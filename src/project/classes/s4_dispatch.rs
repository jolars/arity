//! S4 identities, multiple dispatch, and conservative inherited method selection.

use super::*;
use std::path::Path;

mod builtins;
mod calls;
mod execution;
mod registry;
mod selection;
pub(super) use execution::S4SourceSite;
pub(super) use execution::package_load_sites;
pub use selection::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S4CallKind {
    StandardGeneric,
    NextMethod,
    SelectMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum S4CallUncertainty {
    UnresolvedCallee,
    UnresolvedGeneric,
    AmbiguousGeneric,
    AmbiguousMethodFrame,
    InvalidArguments,
    InvalidFrame,
    DynamicSignature,
    DynamicInheritance,
    CustomMethodTable,
}

/// A live CST query. Ranges and body-local calls never enter the shared index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4DispatchCall {
    pub range: TextRange,
    pub kind: S4CallKind,
    pub reference: Option<GenericRef>,
    pub generics: Vec<S4GenericId>,
    /// Candidate frames for callNextMethod; these retain their declared signatures.
    pub methods: Vec<S4MethodId>,
    /// Literal selectMethod signatures can identify direct declarations without
    /// ranking inherited candidates or choosing between duplicate registrations.
    pub exact_methods: Vec<S4MethodId>,
    /// Inherited candidates, including ambiguity and incomplete static evidence.
    pub resolutions: Vec<S4MethodResolution>,
    pub dispatch: Vec<S4MethodSignature>,
    /// The supplied Boolean override. Omission uses the generic's default mask.
    pub use_inherited: Option<bool>,
    pub uncertainty: BTreeSet<S4CallUncertainty>,
}

pub(super) struct S4PackageLoadSite {
    pub range: TextRange,
    pub function: Option<TextRange>,
    pub load: super::s4::S4NamespaceLoad,
}

pub(in crate::project::classes) struct S4CallSite {
    pub range: TextRange,
    pub kind: S4CallKind,
    pub callee: FunctionRef,
    pub reference: Option<GenericRef>,
    pub frames: Vec<OopKey>,
    /// Generic implementation sites that actually supply the recorded default.
    pub default_frames: Vec<OopKey>,
    /// Index of the named closure's callable binding in this file's projection.
    pub named_frame: Option<usize>,
    pub function: Option<TextRange>,
    pub dispatch: Vec<DispatchArgument>,
    pub dispatch_factory: Option<FunctionRef>,
    pub inheritance: S4Inheritance,
    pub uncertainty: BTreeSet<S4CallUncertainty>,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::project::classes) enum S4Inheritance {
    Default,
    Explicit(bool),
    Dynamic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum S4DefaultKind {
    None,
    Definition,
    Explicit,
    Existing,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S4DefaultMethod {
    pub kind: S4DefaultKind,
    pub signature: Option<CallableSignature>,
    pub implementation: Option<MethodImplementationSource>,
}

/// R compares both the generic's class and its body before reusing an imported
/// object. Ordinary definitions become standard bodies; nonstandard generics
/// retain a range-free structural body with trivia removed and strings decoded.
#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum S4GenericSkeleton {
    Standard,
    Nonstandard(S4GenericBody),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum S4GenericBody {
    Node(SyntaxKind, Vec<S4GenericBody>),
    Token(SyntaxKind, String),
}

impl S4GenericSkeleton {
    fn compatibility(&self, other: &Self) -> Option<bool> {
        match (self, other) {
            (Self::Standard, Self::Standard) => Some(true),
            (Self::Nonstandard(new), Self::Nonstandard(old)) => new.compatibility(old),
            _ => Some(false),
        }
    }
}

impl S4GenericBody {
    pub(in crate::project::classes) fn from_syntax(value: SyntaxElement) -> Option<Self> {
        match value {
            SyntaxElement::Node(node) => {
                let children = node
                    .children_with_tokens()
                    .filter(|child| {
                        !crate::ast::kinds::is_trivia(child.kind())
                            && !matches!(
                                child.kind(),
                                SyntaxKind::COMMENT
                                    | SyntaxKind::ROXYGEN_BLOCK
                                    | SyntaxKind::SEMICOLON
                            )
                    })
                    .map(Self::from_syntax)
                    .collect::<Option<Vec<_>>>()?;
                Some(Self::Node(node.kind(), children))
            }
            SyntaxElement::Token(token) => {
                let value = match token.kind() {
                    SyntaxKind::STRING => crate::semantic::decode_string_literal(token.text())?,
                    _ => token.text().to_string(),
                };
                Some(Self::Token(token.kind(), value))
            }
        }
    }

    fn compatibility(&self, other: &Self) -> Option<bool> {
        if self == other {
            return Some(true);
        }
        match (self, other) {
            (Self::Node(new_kind, new), Self::Node(old_kind, old))
                if new_kind == old_kind && new.len() == old.len() =>
            {
                let mut known = true;
                for (new, old) in new.iter().zip(old) {
                    match new.compatibility(old) {
                        Some(false) => return Some(false),
                        None => known = false,
                        Some(true) => {}
                    }
                }
                known.then_some(true)
            }
            (Self::Token(SyntaxKind::STRING, _), Self::Token(SyntaxKind::STRING, _)) => Some(false),
            // Different syntax can represent identical R language objects,
            // including numeric spellings and assignment directions. Only a
            // proven difference may establish a new package identity.
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S4GenericDefinition {
    /// None means the owner comes from the existing function or package context.
    pub package: Option<ClassName>,
    pub groups: Vec<ClassName>,
    pub group_factory: Option<FunctionRef>,
    pub definition: Option<MethodImplementationSource>,
    pub definition_supplied: bool,
    /// No definition or options were supplied, so an existing object can be reused.
    pub simple_redeclaration: bool,
    pub signature_only: bool,
    pub simple_inheritance_only: Option<bool>,
    /// The free lookup is combined with bindings at the installation event.
    pub lookup: Option<FunctionRef>,
    pub previous: Option<MethodImplementationSource>,
    pub default: S4DefaultMethod,
    pub dispatch_explicit: bool,
    pub default_disabled: bool,
    pub standard_dispatch: Option<bool>,
    pub standard_factory: Option<FunctionRef>,
    pub skeleton: Option<S4GenericSkeleton>,
}

pub(in crate::project::classes) fn s4_dispatch_arguments(
    signature: &CallableSignature,
) -> Vec<ClassName> {
    signature
        .parameters
        .iter()
        .filter(|parameter| parameter.name != "..." || signature.parameters.len() == 1)
        .map(|parameter| ClassName::Known(parameter.name.clone()))
        .collect()
}

/// S4's identity includes the generic's original package. A local or unresolved
/// declaration retains its site instead of acquiring a guessed package.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, salsa::SalsaValue)]
pub enum S4GenericId {
    Named { package: String, name: String },
    Declaration(OopId),
}

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct S4Generic {
    pub id: S4GenericId,
    pub declaration: Option<OopId>,
    /// The installation that created the object, retained by redeclarations.
    /// Built-in groups have no source declaration.
    pub object: Option<OopId>,
    pub signature: Option<CallableSignature>,
    pub skeleton: Option<S4GenericSkeleton>,
    pub dispatch_arguments: Vec<ClassName>,
    pub groups: Vec<ClassName>,
    pub group: bool,
    pub simple_inheritance_only: Option<bool>,
    pub default: S4DefaultMethod,
    pub uncertainty: BTreeSet<OopUncertainty>,
}

impl S4Generic {
    fn dispatches_on_dots(&self) -> bool {
        matches!(self.dispatch_arguments.as_slice(), [ClassName::Known(name)] if name == "...")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, salsa::SalsaValue)]
pub enum S4MethodId {
    Registration(OopId),
    Default(OopId),
}

impl S4MethodId {
    fn source(&self) -> &OopId {
        let (Self::Registration(source) | Self::Default(source)) = self;
        source
    }
}

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct S4MethodOwner {
    pub path: PathBuf,
    pub package_root: Option<PathBuf>,
    pub package: Option<String>,
    pub lexical: ClassOwner,
}

/// Normalize separately for each candidate declaration, preserving competing
/// generic definitions with different selected arguments.
#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct S4MethodSignature {
    pub generic: S4GenericId,
    pub declaration: Option<OopId>,
    pub object: Option<OopId>,
    pub arguments: Vec<DispatchArgument>,
    pub complete: bool,
    pub valid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct S4Method {
    pub id: S4MethodId,
    pub owner: S4MethodOwner,
    pub generic: GenericRef,
    pub generics: Vec<S4GenericId>,
    pub supplied_signature: Vec<DispatchArgument>,
    pub dispatch: Vec<S4MethodSignature>,
    pub signature: Option<CallableSignature>,
    pub implementation: Option<MethodImplementationSource>,
    pub uncertainty: BTreeSet<OopUncertainty>,
}

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
struct S4Removal {
    source: OopId,
    generic: GenericRef,
    dispatch: Vec<S4MethodSignature>,
    uncertainty: BTreeSet<OopUncertainty>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, salsa::SalsaValue)]
pub struct S4Registry {
    pub generics: Vec<S4Generic>,
    pub methods: Vec<S4Method>,
    removals: Vec<S4Removal>,
}

impl ClassIndex {
    pub(super) fn s4_parameter_classes(
        &self,
        path: &Path,
        root: &SyntaxNode,
        binding: &crate::semantic::Binding,
        extracted: &Extracted,
    ) -> (BTreeSet<ClassId>, bool) {
        let mut classes = BTreeSet::new();
        let mut unknown = false;
        let Some(function) = root
            .covering_element(binding.def_range)
            .parent()
            .and_then(|node| node.ancestors().find_map(FunctionExpr::cast))
        else {
            return (classes, true);
        };
        let range = function.syntax().text_range();
        let named_frame = extracted.callable_frames.get(&range).copied();
        for method in &self.s4.methods {
            let registration = method.id.source();
            let inline = registration.path == path
                && extracted
                    .method_implementations
                    .iter()
                    .any(|site| site.method == registration.key && site.function_range == range);
            let named = named_frame.is_some_and(|frame| {
                method
                    .implementation
                    .as_ref()
                    .is_some_and(|source| self.s4_named_frame(path, frame, registration, source))
            });
            if !inline && !named {
                continue;
            }
            unknown |= method.dispatch.is_empty();
            for signature in &method.dispatch {
                let argument = signature.arguments.iter().find(|argument| {
                    argument.argument.as_deref() == Some(unbacktick(&binding.name))
                });
                let Some(DispatchClass::Formal(name)) = argument.map(|arg| &arg.class) else {
                    unknown = true;
                    continue;
                };
                if !signature.valid || !signature.complete {
                    unknown = true;
                    continue;
                }
                let candidates = self.resolve_reference(
                    &method.owner.path,
                    &method.owner.lexical,
                    &ParentRef::Formal(name.clone()),
                    true,
                );
                unknown |= candidates.is_empty();
                classes.extend(candidates);
            }
        }
        (classes, unknown)
    }
}

impl S4Registry {
    pub fn methods_for_generic(&self, generic: &S4GenericId) -> Vec<&S4Method> {
        self.methods
            .iter()
            .filter(|method| method.generics.contains(generic))
            .collect()
    }
}

/// Named signatures use exact then unambiguous partial matching; unnamed
/// signatures follow the generic's selected argument order. R treats every
/// omitted position as ANY.
pub fn s4_method_signature(
    generic: &S4Generic,
    supplied: &[DispatchArgument],
) -> S4MethodSignature {
    let mut result = S4MethodSignature {
        generic: generic.id.clone(),
        declaration: generic.declaration.clone(),
        object: generic.object.clone(),
        arguments: generic
            .dispatch_arguments
            .iter()
            .map(|name| DispatchArgument {
                argument: match name {
                    ClassName::Known(name) => Some(name.clone()),
                    _ => None,
                },
                class: DispatchClass::Any,
            })
            .collect(),
        complete: !generic
            .uncertainty
            .contains(&OopUncertainty::DynamicSignature),
        valid: !generic
            .uncertainty
            .contains(&OopUncertainty::InvalidArguments),
    };
    result.complete &= result.arguments.iter().all(|arg| arg.argument.is_some());
    if !result.complete {
        // Unknown selected arguments cannot invalidate a supplied name or imply
        // an ANY position. Retain the complete supplied vector until lookup wins.
        result.arguments = supplied.to_vec();
        return result;
    }
    let mut seen = BTreeSet::new();
    for arg in supplied
        .iter()
        .filter(|arg| arg.argument.is_some())
        .chain(supplied.iter().filter(|arg| arg.argument.is_none()))
    {
        let position = if let Some(name) = &arg.argument {
            result
                .arguments
                .iter()
                .position(|arg| arg.argument.as_ref() == Some(name))
                .or_else(|| {
                    let mut candidates = result.arguments.iter().enumerate().filter(|(_, arg)| {
                        !name.is_empty()
                            && arg
                                .argument
                                .as_ref()
                                .is_some_and(|formal| formal.starts_with(name.as_str()))
                    });
                    match (candidates.next(), candidates.next()) {
                        (Some((position, _)), None) => Some(position),
                        _ => None,
                    }
                })
        } else {
            (0..result.arguments.len()).find(|position| !seen.contains(position))
        };
        let Some(position) = position.filter(|position| *position < result.arguments.len()) else {
            result.valid = false;
            continue;
        };
        if !seen.insert(position) {
            result.valid = false;
        }
        result.complete &= !matches!(
            arg.class,
            DispatchClass::Unknown | DispatchClass::Generator(_)
        );
        result.arguments[position].class = arg.class.clone();
    }
    result.complete &= result.valid;
    result
}
