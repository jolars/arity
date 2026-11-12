//! Ordered S3 lookup over static evidence. Runtime registration and operand
//! conflicts retain candidates; they cannot establish a selected closure.

use smol_str::SmolStr;
use std::path::Path;
use std::sync::OnceLock;

use super::*;
use crate::ast::{ParenExpr, Subset2Expr, SubsetExpr, UnaryExpr};
use crate::semantic::enclosure::Availability;

mod forwarding;
pub use forwarding::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum S3DispatchUncertainty {
    UnknownClass,
    RuntimeRegistration,
    AmbiguousGeneric,
    AmbiguousMethod,
    IncompleteScope,
    ConflictingOperands,
    UnknownContinuation,
    DynamicArguments,
    ExternalMethods,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S3Fallback {
    Internal,
    /// A project-owned generic has no method in the resolved static scope.
    NoMethod,
    /// Classes or library-owned methods/defaults remain outside this evidence.
    Unknown,
}

/// Lexical methods precede the registration table, which precedes parents of
/// the caller's top environment. Exports alone never install a registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S3MethodVisibility {
    Lexical { depth: usize },
    Registered,
    SearchPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3MethodCandidate {
    pub source: S3MethodSource,
    pub implementation: Option<MethodImplementationSource>,
    pub implementation_path: PathBuf,
    pub class: ClassName,
    /// Index in the original dispatch vector; `None` denotes `.default` or an
    /// unknown vector. NextMethod must not search for a dotted-name suffix.
    pub class_index: Option<usize>,
    pub operand: usize,
    pub group: Option<String>,
    pub visibility: S3MethodVisibility,
    pub availability: Availability,
    pub registration_order: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Dispatch {
    pub generic: S3GenericId,
    pub dispatch_name: String,
    pub classes: Vec<S3ClassEvidence>,
    pub candidates: Vec<S3MethodCandidate>,
    pub selected: Option<S3MethodCandidate>,
    pub fallback: S3Fallback,
    pub uncertainty: BTreeSet<S3DispatchUncertainty>,
    pub forwarding: Option<S3Forwarding>,
}

/// Additional execution evidence supplied by the caller of the pure query.
/// An unlisted script registration remains possible, not definitely installed.
/// Loaded namespaces install registrations; attached packages also expose exports.
#[derive(Debug, Clone, Default)]
pub struct S3DispatchContext {
    pub path: PathBuf,
    pub owner: ClassOwner,
    pub deferred: bool,
    pub loaded_packages: BTreeSet<PathBuf>,
    pub attached_packages: BTreeSet<PathBuf>,
    pub executed_registrations: Vec<S3MethodSource>,
    pub runtime_registration: bool,
    pub inactive_methods: Vec<S3MethodSource>,
    pub uncertain_methods: Vec<S3MethodSource>,
}

impl S3DispatchContext {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy)]
pub struct S3MethodFrame<'a> {
    pub path: &'a Path,
    pub root: &'a SyntaxNode,
    pub model: &'a SemanticModel,
}

pub(super) fn known_generic_package(name: &str) -> Option<&'static str> {
    static GENERICS: OnceLock<BTreeMap<&str, &str>> = OnceLock::new();
    GENERICS
        .get_or_init(|| {
            include_str!("../../semantic/base_r/s3_generics.tsv")
                .lines()
                .skip(1)
                .filter_map(|line| {
                    let mut columns = line.split('\t');
                    Some((columns.next()?, columns.next()?))
                })
                .collect()
        })
        .get(name)
        .copied()
}

fn group(name: &str) -> Option<&'static str> {
    Some(match name {
        "+" | "-" | "*" | "/" | "^" | "%%" | "%/%" | "&" | "|" | "!" | "==" | "!=" | "<" | "<="
        | ">=" | ">" => "Ops",
        "%*%" => "matrixOps",
        "abs" | "sign" | "sqrt" | "floor" | "ceiling" | "trunc" | "round" | "signif" | "exp"
        | "log" | "log10" | "log2" | "expm1" | "log1p" | "cos" | "sin" | "tan" | "cospi"
        | "sinpi" | "tanpi" | "acos" | "asin" | "atan" | "cosh" | "sinh" | "tanh" | "acosh"
        | "asinh" | "atanh" | "lgamma" | "gamma" | "digamma" | "trigamma" | "cumsum"
        | "cumprod" | "cummax" | "cummin" => "Math",
        "all" | "any" | "sum" | "prod" | "min" | "max" => "Summary",
        "Arg" | "Conj" | "Im" | "Mod" | "Re" => "Complex",
        _ => return None,
    })
}

/// The syntactic call surface includes replacement functions and operators.
/// Ordinary argument names are kept separately by the forwarding query.
struct CallShape {
    name: String,
    reference: Option<FunctionRef>,
    arguments: Vec<Option<SyntaxElement>>,
    names: Vec<Option<SmolStr>>,
    positional: bool,
}

fn shape(model: &SemanticModel, root: &SyntaxNode, value: &SyntaxElement) -> Option<CallShape> {
    let node = value.as_node()?;
    if let Some(paren) = ParenExpr::cast(node.clone()) {
        return shape(model, root, &paren.inner()?);
    }
    if let Some(assignment) = AssignmentExpr::cast(node.clone()) {
        let mut call = shape(model, root, &assignment.target_element()?)?;
        if !call.name.ends_with("<-") {
            call.name.push_str("<-");
        }
        call.arguments.push(assignment.value_element());
        call.names.push(Some("value".into()));
        return Some(call);
    }
    if let Some(call) = CallExpr::cast(node.clone()) {
        let reference = call_function_ref(model, root, &call);
        let name = match &reference {
            Some(FunctionRef::Qualified { name, .. } | FunctionRef::Bare { name, .. }) => {
                name.clone()
            }
            _ => call
                .callee_name()
                .map(|name| unbacktick(&name).to_string())?,
        };
        let args: Vec<_> = call.arg_list()?.args().collect();
        return Some(CallShape {
            name,
            reference,
            arguments: args.iter().map(|arg| arg.value()).collect(),
            names: args.iter().map(|arg| arg.name()).collect(),
            positional: false,
        });
    }
    let (token, arguments) = if let Some(subset) = SubsetExpr::cast(node.clone()) {
        (subset.open_bracket()?, vec![subset.base()])
    } else if let Some(subset) = Subset2Expr::cast(node.clone()) {
        (subset.open_bracket()?, vec![subset.base()])
    } else if let Some(unary) = UnaryExpr::cast(node.clone()) {
        (unary.op()?, vec![unary.operand()])
    } else {
        let binary = BinaryExpr::cast(node.clone())?;
        let (lhs, op, rhs) = binary.parts()?;
        let arguments = if matches!(op.kind(), SyntaxKind::DOLLAR | SyntaxKind::AT) {
            vec![Some(lhs)]
        } else {
            vec![Some(lhs), Some(rhs)]
        };
        (op, arguments)
    };
    let setter = node
        .parent()
        .and_then(AssignmentExpr::cast)
        .is_some_and(|assignment| {
            assignment
                .target_element()
                .is_some_and(|target| target.text_range() == value.text_range())
        });
    let name = format!("{}{}", token.text(), if setter { "<-" } else { "" });
    let reference = named_function_ref(model, root, &token, &name, &mut BTreeSet::new());
    Some(CallShape {
        name,
        reference,
        names: vec![None; arguments.len()],
        arguments,
        positional: true,
    })
}

impl ClassIndex {
    fn s3_known_call(&self, path: &Path, reference: &FunctionRef) -> Option<S3GenericId> {
        let (name, package) = match reference {
            FunctionRef::Qualified { name, package }
                if known_generic_package(name) == Some(package) =>
            {
                (name, package.as_str())
            }
            FunctionRef::Bare {
                name,
                attached,
                attachment_uncertain: false,
            } => {
                let expected = known_generic_package(name)?;
                let file = self.file(path)?;
                if file.exports.contains(name)
                    || !self.definition_files(path, name, true).is_empty()
                    || !self.class_file_complete(path)
                {
                    return None;
                }
                let mut imports = BTreeSet::new();
                let mut loaded = Vec::new();
                if let Some(root) = &file.package_root {
                    loaded.extend(
                        self.s3_origins
                            .attached
                            .get(root)
                            .into_iter()
                            .flatten()
                            .cloned(),
                    );
                    if let Some(namespace) = self.s3_origins.namespaces.get(root) {
                        if namespace.unknown {
                            return None;
                        }
                        imports.extend(namespace.named.get(name).into_iter().flatten().cloned());
                        for source in &namespace.wildcard {
                            if self.s3_origins.exports.get(source)?.contains(name) {
                                imports.insert(source.clone());
                            }
                        }
                    }
                }
                if !imports.is_empty() {
                    if imports.len() != 1 || !imports.contains(expected) {
                        return None;
                    }
                } else {
                    loaded.extend(attached.iter().cloned());
                    for package in loaded {
                        if self.s3_origins.exports.get(&package)?.contains(name)
                            && package != expected
                        {
                            return None;
                        }
                    }
                }
                (name, expected)
            }
            _ => return None,
        };
        let id = S3GenericId::Known {
            package: package.into(),
            name: name.clone(),
        };
        self.s3
            .generics
            .iter()
            .any(|generic| generic.id == id && generic.kind != S3GenericKind::Group)
            .then_some(id)
    }
    /// Resolve one generic identity in a caller environment. Candidate order is
    /// class-vector order, then member before group, then lookup visibility.
    pub fn s3_dispatch(
        &self,
        context: &S3DispatchContext,
        generic: &S3GenericId,
        classes: &[S3ClassEvidence],
    ) -> S3Dispatch {
        self.s3_dispatch_from(context, generic, classes, 0, false)
    }

    /// Continue with the original `.Class` vector after the selected occurrence.
    /// A changed receiver class in a method does not change this cursor.
    pub fn s3_next_method(
        &self,
        context: &S3DispatchContext,
        previous: &S3Dispatch,
        current: &S3MethodCandidate,
        generic: Option<&S3GenericId>,
    ) -> S3Dispatch {
        let classes = previous
            .classes
            .get(current.operand)
            .map(std::slice::from_ref)
            .unwrap_or_default();
        let mut result = self.s3_dispatch_from(
            context,
            generic.unwrap_or(&previous.generic),
            classes,
            current.class_index.map_or(usize::MAX, |index| index + 1),
            true,
        );
        result
            .uncertainty
            .extend(previous.uncertainty.iter().copied());
        if !previous.candidates.contains(current) || classes.is_empty() {
            result
                .uncertainty
                .insert(S3DispatchUncertainty::UnknownContinuation);
        }
        if !result.uncertainty.is_empty() {
            result.selected = None;
        }
        if let Some(signature) = self.s3_implementation_signature(current)
            && let Some(forwarding) = &previous.forwarding
        {
            result.forwarding = Some(forwarding.next_method(signature, &[]));
        }
        result
    }

    /// Resolve a particular NextMethod call using its originating dispatch.
    /// The caller environment and class vector come from that dispatch, while
    /// argument overrides and metadata writes come from the current method.
    pub fn s3_next_method_at(
        &self,
        context: &S3DispatchContext,
        previous: &S3Dispatch,
        current: &S3MethodCandidate,
        frame: S3MethodFrame<'_>,
        call: &CallExpr,
    ) -> S3Dispatch {
        let S3MethodFrame {
            path: source_path,
            root,
            model,
        } = frame;
        let reference = call_function_ref(model, root, call);
        let confirmed = reference.as_ref().is_some_and(|reference| match reference {
            FunctionRef::Qualified { package, name } => package == "base" && name == "NextMethod",
            _ => self
                .callee_identity(source_path, reference)
                .is_some_and(|origin| origin.package == "base" && origin.name == "NextMethod"),
        });
        let args: Vec<_> = call
            .arg_list()
            .map(|args| args.args().collect())
            .unwrap_or_default();
        let names: Vec<_> = args.iter().map(|arg| arg.name()).collect();
        let matched =
            crate::semantic::checked_match_args_to_formals(&names, &["generic", "object", "..."]);
        let mut generic = None;
        let mut uncertain =
            !confirmed || matched.is_err() || !self.s3_method_frame_matches(current, frame, call);
        let mut extras = Vec::new();
        if let Ok(matched) = matched {
            for (arg, formal) in args.iter().zip(matched) {
                match formal {
                    Some("generic") => {
                        if let Some(token) = arg.value().and_then(SyntaxElement::into_token) {
                            if token.kind() == SyntaxKind::STRING {
                                let name = crate::semantic::decode_string_literal(token.text());
                                let candidates: Vec<_> = self
                                    .s3
                                    .generics
                                    .iter()
                                    .filter(|candidate| Some(&candidate.name) == name.as_ref())
                                    .collect();
                                if candidates.len() == 1 {
                                    generic = Some(candidates[0].id.clone());
                                } else {
                                    uncertain = true;
                                }
                            } else if token.text() != "NULL" {
                                uncertain = true;
                            }
                        } else {
                            uncertain = true;
                        }
                    }
                    Some("object") => {
                        // An explicit object cannot establish a new `.Class`
                        // vector in an existing dispatch context.
                        uncertain |= !arg.value().is_some_and(|value| value.to_string() == "NULL");
                    }
                    _ => {
                        uncertain |= arg.value().is_none_or(|value| {
                            value.as_token().is_some_and(|token| token.text() == "...")
                        });
                        extras.push(arg.name().map(|name| name.to_string()));
                    }
                }
            }
        }
        let scope = model.innermost_scope_at(call.syntax().text_range().start());
        uncertain |= opaque_binding_frame(root, call.syntax().text_range());
        uncertain |= model.bindings().iter().any(|binding| {
            binding.scope == scope
                && matches!(
                    unbacktick(&binding.name),
                    ".Class" | ".Generic" | ".Group" | ".Method"
                )
                && binding.def_range.start() < call.syntax().text_range().start()
        });
        let mut result = self.s3_next_method(context, previous, current, generic.as_ref());
        if let Some(signature) = self.s3_implementation_signature(current)
            && let Some(forwarding) = &previous.forwarding
        {
            result.forwarding = Some(forwarding.next_method(signature, &extras));
        }
        if uncertain {
            result
                .uncertainty
                .insert(S3DispatchUncertainty::UnknownContinuation);
            result.selected = None;
            if let Some(forwarding) = &mut result.forwarding {
                forwarding.uncertain = true;
            }
        }
        result
    }

    fn s3_implementation_signature(
        &self,
        candidate: &S3MethodCandidate,
    ) -> Option<&CallableSignature> {
        if let S3MethodSource::Declaration(id) = &candidate.source {
            return self.method(id)?.signature.as_ref();
        }
        let MethodImplementationSource::Binding { name, .. } = candidate.implementation.as_ref()?
        else {
            return None;
        };
        let mut signatures = self
            .oop_files()
            .filter(|file| file.package_root.as_ref() == Some(&candidate.implementation_path))
            .flat_map(|file| &file.facts.oop.callables)
            .filter(|callable| &callable.name == name);
        let signature = &signatures.next()?.signature;
        signatures.next().is_none().then_some(signature)
    }

    fn s3_method_frame_matches(
        &self,
        current: &S3MethodCandidate,
        frame: S3MethodFrame<'_>,
        call: &CallExpr,
    ) -> bool {
        let Some(function) = call.syntax().ancestors().find_map(FunctionExpr::cast) else {
            return false;
        };
        if let S3MethodSource::Declaration(id) = &current.source
            && id.path == frame.path
        {
            return method_implementation_sites(frame.model, frame.root)
                .iter()
                .any(|site| {
                    site.method == id.key && site.function_range == function.syntax().text_range()
                });
        }
        let (name, owner) = match current.implementation.as_ref() {
            Some(MethodImplementationSource::Binding { name, owner }) => (name, owner.clone()),
            Some(MethodImplementationSource::Reference { name }) => (name, ClassOwner::default()),
            _ => return false,
        };
        let visible = match &current.source {
            S3MethodSource::Namespace { root, .. } => self
                .file(frame.path)
                .is_some_and(|file| file.package_root.as_ref() == Some(root)),
            S3MethodSource::Declaration(id) => self
                .implementation_files(&id.path, name, None)
                .contains(frame.path),
        };
        if !visible {
            return false;
        }
        let owners = (!owner.0.is_empty()).then(|| extract(frame.model, frame.root, false).owners);
        frame
            .model
            .bindings()
            .iter()
            .enumerate()
            .filter(|(_, binding)| unbacktick(&binding.name) == name)
            .filter(|(index, binding)| {
                if owner.0.is_empty() {
                    return frame
                        .model
                        .binding_is_file_scope(BindingId::from_index(*index));
                }
                frame
                    .root
                    .covering_element(binding.def_range)
                    .parent()
                    .and_then(|node| node.ancestors().find_map(FunctionExpr::cast))
                    .and_then(|function| owners.as_ref()?.get(&function.syntax().text_range()))
                    == Some(&owner)
            })
            .any(|(index, _)| {
                let mut binding = BindingId::from_index(index);
                let mut visited = BTreeSet::new();
                while visited.len() < 64 && visited.insert(binding.0) {
                    let Some(value) = stable_value(frame.model, frame.root, binding) else {
                        return false;
                    };
                    if value
                        .as_node()
                        .is_some_and(|node| node == function.syntax())
                    {
                        return true;
                    }
                    let Some(next) = value
                        .as_token()
                        .and_then(|token| read_binding(frame.model, token))
                    else {
                        return false;
                    };
                    binding = next;
                }
                false
            })
    }

    fn s3_dispatch_from(
        &self,
        context: &S3DispatchContext,
        id: &S3GenericId,
        classes: &[S3ClassEvidence],
        start: usize,
        next: bool,
    ) -> S3Dispatch {
        let generic = self.s3.generics.iter().find(|generic| &generic.id == id);
        let internal = generic.is_some_and(|generic| generic.kind == S3GenericKind::Internal);
        let entry_name = generic.map(|generic| generic.name.as_str()).unwrap_or("");
        let name = if matches!(id, S3GenericId::Known { package, .. } if package == "base") {
            match entry_name {
                "as.numeric" => "as.double",
                "seq.int" => "seq",
                _ => entry_name,
            }
        } else {
            entry_name
        };
        let dispatch_id = match id {
            S3GenericId::Known { package, .. } => S3GenericId::Known {
                package: package.clone(),
                name: name.into(),
            },
            _ => id.clone(),
        };
        // A user-defined closure named like a primitive does not acquire group dispatch.
        let group = matches!(id, S3GenericId::Known { package, .. } if package == "base")
            .then(|| group(name))
            .flatten();
        let bind = internal && matches!(name, "cbind" | "rbind");
        let mut result = S3Dispatch {
            generic: id.clone(),
            dispatch_name: name.into(),
            classes: classes.to_vec(),
            candidates: Vec::new(),
            selected: None,
            fallback: if internal {
                S3Fallback::Internal
            } else if matches!(id, S3GenericId::Declaration(_)) {
                S3Fallback::NoMethod
            } else {
                S3Fallback::Unknown
            },
            uncertainty: BTreeSet::new(),
            forwarding: None,
        };
        if generic.is_none()
            || generic.is_some_and(|generic| {
                generic
                    .uncertainty
                    .iter()
                    .any(|issue| *issue != OopUncertainty::DynamicSignature)
            })
        {
            result
                .uncertainty
                .insert(S3DispatchUncertainty::AmbiguousGeneric);
        }
        if !self.class_file_complete(&context.path) {
            result
                .uncertainty
                .insert(S3DispatchUncertainty::IncompleteScope);
        }
        if context.runtime_registration
            || context.loaded_packages.iter().any(|root| {
                self.s3_origins
                    .namespaces
                    .get(root)
                    .is_some_and(|namespace| namespace.unknown)
            })
        {
            result
                .uncertainty
                .insert(S3DispatchUncertainty::RuntimeRegistration);
        }
        let operands = if bind {
            classes.len()
        } else if matches!(group, Some("Ops" | "matrixOps")) {
            classes.len().min(2)
        } else {
            classes.len().min(1)
        };
        if operands == 0 && !internal {
            result
                .uncertainty
                .insert(S3DispatchUncertainty::UnknownClass);
        }
        let mut chosen = Vec::new();
        for (operand, evidence) in classes.iter().take(operands).enumerate() {
            if internal && !next && *evidence == S3ClassEvidence::NoExplicitClass {
                continue;
            }
            let ordered = match evidence {
                S3ClassEvidence::Explicit(classes) if !classes.is_empty() => Some(classes),
                _ => {
                    result
                        .uncertainty
                        .insert(S3DispatchUncertainty::UnknownClass);
                    None
                }
            };
            let mut candidates = Vec::new();
            for method in &self.s3.methods {
                let ClassName::Known(method_generic) = &method.generic.name else {
                    if method.registered && self.s3_visibility(context, method).is_some() {
                        result
                            .uncertainty
                            .insert(S3DispatchUncertainty::RuntimeRegistration);
                    }
                    continue;
                };
                let via_group = group == Some(method_generic.as_str());
                if method_generic != name && !via_group {
                    continue;
                }
                if method.registered
                    && !method.generics.contains(&dispatch_id)
                    && !via_group
                    && !method.generics.is_empty()
                {
                    continue;
                }
                let Some((visibility, mut availability)) = self.s3_visibility(context, method)
                else {
                    continue;
                };
                if method.registered && availability == Availability::Unknown {
                    result
                        .uncertainty
                        .insert(S3DispatchUncertainty::RuntimeRegistration);
                }
                if method.registered && method.generics.is_empty() {
                    availability = Availability::Unknown;
                }
                let class = match &method.class {
                    ClassName::Known(class) => class,
                    _ => {
                        result
                            .uncertainty
                            .insert(S3DispatchUncertainty::RuntimeRegistration);
                        continue;
                    }
                };
                let indices: Vec<_> = if let Some(classes) = ordered {
                    let mut indices: Vec<_> = classes
                        .iter()
                        .enumerate()
                        .filter(|(i, name)| *i >= start && *name == class)
                        .map(|(i, _)| Some(i))
                        .collect();
                    if class == "default"
                        && (group.is_none() && !bind || next)
                        && start != usize::MAX
                    {
                        indices.push(None);
                    }
                    indices
                } else {
                    vec![None]
                };
                for class_index in indices {
                    candidates.push(S3MethodCandidate {
                        source: method.source.clone(),
                        implementation: method.implementation.clone(),
                        implementation_path: method
                            .owner
                            .package_root
                            .clone()
                            .unwrap_or_else(|| method.owner.path.clone()),
                        class: method.class.clone(),
                        class_index,
                        operand,
                        group: via_group.then(|| method_generic.clone()),
                        visibility,
                        availability,
                        registration_order: context
                            .executed_registrations
                            .iter()
                            .position(|source| *source == method.source),
                    });
                }
            }
            candidates.sort_by_key(candidate_rank);
            // The project registry cannot prove the absence of a library's
            // methods for classes skipped before this candidate. That also
            // matters when a different operand might supply a method.
            if matches!(id, S3GenericId::Known { .. })
                && ordered.is_some()
                && candidates
                    .first()
                    .is_none_or(|first| first.class_index != Some(start))
            {
                result
                    .uncertainty
                    .insert(S3DispatchUncertainty::ExternalMethods);
            }
            if let Some(first) = candidates.first() {
                let rank = candidate_rank(first);
                let tier: Vec<_> = candidates
                    .iter()
                    .take_while(|candidate| candidate_rank(candidate) == rank)
                    .collect();
                if tier.len() == 1 && first.availability == Availability::Present {
                    chosen.push(first.clone());
                } else {
                    result
                        .uncertainty
                        .insert(S3DispatchUncertainty::AmbiguousMethod);
                }
            }
            result.candidates.extend(candidates);
        }
        if let Some(first) = chosen.first() {
            if !bind
                && chosen
                    .iter()
                    .skip(1)
                    .any(|other| !same_implementation(first, other))
            {
                result
                    .uncertainty
                    .insert(S3DispatchUncertainty::ConflictingOperands);
            }
            if result.uncertainty.is_empty() {
                result.selected = Some(first.clone());
            }
        }
        if result
            .uncertainty
            .contains(&S3DispatchUncertainty::UnknownClass)
        {
            result.fallback = S3Fallback::Unknown;
        }
        result
    }

    fn s3_visibility(
        &self,
        context: &S3DispatchContext,
        method: &S3Method,
    ) -> Option<(S3MethodVisibility, Availability)> {
        if context.inactive_methods.contains(&method.source) {
            return None;
        }
        let caller = self.file(&context.path)?;
        let mut availability = if method.uncertainty.iter().any(|issue| {
            !matches!(
                issue,
                OopUncertainty::UnprovenMethod | OopUncertainty::DynamicSignature
            )
        }) {
            Availability::Unknown
        } else {
            Availability::Present
        };
        if context.uncertain_methods.contains(&method.source) {
            availability = Availability::Unknown;
        }
        if !self.class_file_complete(&method.owner.path)
            && !matches!(method.source, S3MethodSource::Namespace { .. })
        {
            availability = Availability::Unknown;
        }
        if method.registered {
            if let S3MethodSource::Namespace { root, .. } = &method.source {
                return (caller.package_root.as_ref() == Some(root)
                    || context.loaded_packages.contains(root))
                .then_some((S3MethodVisibility::Registered, availability));
            }
            if context.executed_registrations.contains(&method.source) {
                return Some((S3MethodVisibility::Registered, availability));
            }
            if method.owner.path == context.path
                || method.owner.package_root.is_some()
                    && method.owner.package_root == caller.package_root
                || self.sees_file(&context.path, &method.owner.path)
            {
                return Some((S3MethodVisibility::Registered, Availability::Unknown));
            }
            return None;
        }
        let ClassName::Known(class) = &method.class else {
            return None;
        };
        let ClassName::Known(generic) = &method.generic.name else {
            return None;
        };
        let name = format!("{generic}.{class}");
        if method.owner.path == context.path && context.owner.0.starts_with(&method.owner.lexical.0)
        {
            return Some((
                S3MethodVisibility::Lexical {
                    depth: method.owner.lexical.0.len(),
                },
                availability,
            ));
        }
        if !method.owner.lexical.0.is_empty() {
            return None;
        }
        if self
            .definition_files(
                &context.path,
                &name,
                context.deferred || !context.owner.0.is_empty(),
            )
            .contains(&method.owner.path)
        {
            return Some((S3MethodVisibility::Lexical { depth: 0 }, availability));
        }
        if self
            .implementation_files(&context.path, &name, None)
            .contains(&method.owner.path)
        {
            return Some((S3MethodVisibility::SearchPath, availability));
        }
        if method.exported
            && method
                .owner
                .package_root
                .as_ref()
                .is_some_and(|root| context.attached_packages.contains(root))
        {
            return Some((S3MethodVisibility::SearchPath, availability));
        }
        None
    }

    /// Resolve a call, operator, subset, or replacement in the current CST.
    /// The returned forwarding plan describes promises, without forcing them.
    pub fn s3_dispatch_at(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        value: &SyntaxElement,
    ) -> Option<S3Dispatch> {
        let call = shape(model, root, value)?;
        let mut context = self.s3_dispatch_context_at(path, root, model, value);
        let mut uncertain_callee = false;
        if let Some(syntax_call) = value.as_node().cloned().and_then(CallExpr::cast)
            && replacement_assignment(&syntax_call).is_none()
            && let Some(binding) = syntax_call
                .callee_token()
                .and_then(|token| read_binding(model, &token))
        {
            let stable = stable_value(model, root, binding);
            uncertain_callee = stable.is_none();
            if let Some(function) = stable
                .and_then(SyntaxElement::into_node)
                .and_then(FunctionExpr::cast)
            {
                let has_dispatch = factories(model, root).iter().any(|(call, _, name)| {
                    name == "UseMethod"
                        && call
                            .syntax()
                            .ancestors()
                            .find_map(FunctionExpr::cast)
                            .as_ref()
                            == Some(&function)
                });
                if !has_dispatch {
                    return None;
                }
            }
        }
        let reference = GenericRef {
            system: OopSystem::S3,
            owner: context.owner.clone(),
            name: ClassName::Known(call.name.clone()),
            package: match &call.reference {
                Some(FunctionRef::Qualified { package, .. }) => Some(package.clone()),
                _ => None,
            },
            by_binding: true,
            local: None,
            lookup: call.reference.clone(),
        };
        let mut ids = self
            .generic_candidates(
                path,
                &reference,
                context.deferred || !context.owner.0.is_empty(),
            )
            .into_iter()
            .map(S3GenericId::Declaration)
            .collect::<Vec<_>>();
        if ids.is_empty()
            && let Some(reference) = &call.reference
        {
            ids.extend(self.s3_known_call(path, reference));
        }
        let id = ids.first()?;
        if let S3GenericId::Declaration(id) = id
            && let Some(root) = self
                .file(&id.path)
                .and_then(|file| file.package_root.as_ref())
        {
            self.s3_load_namespace(root, &mut context.loaded_packages);
        }
        let (classes, forwarding) = self.s3_call_arguments(path, root, model, value, &call, id);
        let mut result = self.s3_dispatch(&context, id, &classes);
        result.forwarding = forwarding;
        if call.arguments.iter().flatten().any(|argument| {
            argument
                .as_token()
                .is_some_and(|token| token.text() == "...")
        }) || result
            .forwarding
            .as_ref()
            .is_some_and(|forwarding| forwarding.uncertain)
        {
            result
                .uncertainty
                .insert(S3DispatchUncertainty::DynamicArguments);
            result.selected = None;
            if let Some(forwarding) = &mut result.forwarding {
                forwarding.uncertain = true;
            }
        }
        if ids.len() != 1 || uncertain_callee {
            for alternative in ids.iter().skip(1) {
                let (classes, _) =
                    self.s3_call_arguments(path, root, model, value, &call, alternative);
                let alternative = self.s3_dispatch(&context, alternative, &classes);
                for candidate in alternative.candidates {
                    if !result.candidates.contains(&candidate) {
                        result.candidates.push(candidate);
                    }
                }
            }
            result
                .uncertainty
                .insert(S3DispatchUncertainty::AmbiguousGeneric);
            result.selected = None;
        }
        Some(result)
    }

    pub fn s3_dispatch_context_at(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        value: &SyntaxElement,
    ) -> S3DispatchContext {
        let extracted = extract(model, root, true);
        let owner = value
            .as_node()
            .map(|node| owner_at(node, &extracted.owners))
            .unwrap_or_default();
        let mut context = S3DispatchContext {
            owner,
            deferred: value.parent().is_some_and(|node| {
                node.ancestors()
                    .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR)
            }),
            ..S3DispatchContext::new(path)
        };
        if let Some(package) = self.file(path).and_then(|file| file.package_root.as_ref()) {
            self.s3_load_namespace(package, &mut context.loaded_packages);
        }
        for loaded in model
            .loaded_packages()
            .iter()
            .filter(|package| context.deferred || package.range.end() < value.text_range().start())
        {
            let Some((package, _)) = self
                .s3_origins
                .package_names
                .iter()
                .find(|(_, name)| **name == loaded.name)
            else {
                continue;
            };
            let Some(call) = root
                .covering_element(loaded.range)
                .parent()
                .and_then(|node| node.ancestors().find_map(CallExpr::cast))
            else {
                continue;
            };
            if unconditional(call.syntax())
                && call
                    .callee_token()
                    .is_some_and(|token| read_binding(model, &token).is_none())
            {
                self.s3_load_namespace(package, &mut context.loaded_packages);
                self.s3_attach_package(package, &mut context.attached_packages);
            } else {
                context.runtime_registration = true;
            }
        }
        let mut executed = Vec::new();
        for method in self
            .s3
            .methods
            .iter()
            .filter(|method| method.owner.path == path)
        {
            let S3MethodSource::Declaration(id) = &method.source else {
                continue;
            };
            let Some(site) = extracted.oop_sites.iter().find(|site| {
                site.role == OopSiteRole::Declaration
                    && site.target == OopTarget::Declaration(id.key.clone())
            }) else {
                continue;
            };
            let element = root.covering_element(site.range);
            let declaration = element.parent().and_then(|node| {
                node.ancestors().find(|node| {
                    if method.registered {
                        node.kind() == SyntaxKind::CALL_EXPR
                    } else {
                        node.kind() == SyntaxKind::ASSIGNMENT_EXPR
                    }
                })
            });
            if let Some(declaration) = declaration {
                if model.innermost_scope_at(site.range.start())
                    == model.innermost_scope_at(value.text_range().start())
                    && declaration.text_range().end() > value.text_range().start()
                {
                    context.inactive_methods.push(method.source.clone());
                } else if method.registered
                    && method.owner.lexical.0.is_empty()
                    && declaration.text_range().end() < value.text_range().start()
                {
                    executed.push((declaration.text_range().start(), method.source.clone()));
                }
            }
            if !method.registered
                && let Some(token) = element.as_token()
                && let Some(binding) = model
                    .bindings()
                    .iter()
                    .enumerate()
                    .find(|(_, binding)| binding.def_range == token.text_range())
                    .map(|(index, _)| BindingId::from_index(index))
                && (model.variable_cohort(binding).len() != 1
                    || stable_value(model, root, binding).is_none())
            {
                context.uncertain_methods.push(method.source.clone());
            }
        }
        executed.sort_by_key(|(offset, _)| *offset);
        context.executed_registrations = executed.into_iter().map(|(_, source)| source).collect();
        context
    }

    fn s3_load_namespace(&self, root: &Path, loaded: &mut BTreeSet<PathBuf>) {
        if !loaded.insert(root.into()) {
            return;
        }
        let mut names = self
            .s3_origins
            .attached
            .get(root)
            .cloned()
            .unwrap_or_default();
        if let Some(namespace) = self.s3_origins.namespaces.get(root) {
            names.extend(namespace.loaded.iter().cloned());
        }
        for (dependency, _) in self
            .s3_origins
            .package_names
            .iter()
            .filter(|(_, name)| names.contains(*name))
        {
            self.s3_load_namespace(dependency, loaded);
        }
    }

    fn s3_attach_package(&self, root: &Path, attached: &mut BTreeSet<PathBuf>) {
        if !attached.insert(root.into()) {
            return;
        }
        let dependencies = self.s3_origins.attached.get(root);
        for (dependency, _) in self.s3_origins.package_names.iter().filter(|(_, name)| {
            dependencies.is_some_and(|dependencies| dependencies.contains(*name))
        }) {
            self.s3_attach_package(dependency, attached);
        }
    }
}

fn candidate_rank(candidate: &S3MethodCandidate) -> (usize, bool, u8, std::cmp::Reverse<usize>) {
    let (tier, depth) = match candidate.visibility {
        S3MethodVisibility::Lexical { depth } => (0, depth),
        S3MethodVisibility::Registered => {
            (1, candidate.registration_order.map_or(0, |order| order + 1))
        }
        S3MethodVisibility::SearchPath => (2, 0),
    };
    (
        candidate.class_index.unwrap_or(usize::MAX),
        candidate.group.is_some(),
        tier,
        std::cmp::Reverse(depth),
    )
}

fn same_implementation(left: &S3MethodCandidate, right: &S3MethodCandidate) -> bool {
    left.implementation.is_some()
        && left.implementation == right.implementation
        && left.implementation_path == right.implementation_path
}
