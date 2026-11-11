//! Range-free OOP declarations shared by project analysis and editor consumers.
//!
//! A declaration owns an identity even when its display name duplicates another
//! declaration. Generator bindings, formal class names, and inheritance references
//! are separate facts. Project resolution confirms package origins and visibility;
//! this module only consults the current file's syntax and lexical semantics.
//! [`FileClassFacts::oop`] extends those identities with generics, methods,
//! members, signatures, and symbolic dispatch targets. [`oop_sites`] recovers
//! their declaration and use spans without putting body text in the projection.

mod environments;
mod resolve;
pub(crate) use environments::has_r6_candidates;
pub use environments::{R6MethodEnvironment, apply_method_environments, standalone_semantic_model};
pub use resolve::{ClassFile, ClassIndex, ClassResolution, ResolvedClass};
mod oop;
pub use oop::*;
mod calls;
use calls::*;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use rowan::ast::AstNode as _;
use rowan::{TextRange, TextSize, TokenAtOffset};
use serde::{Deserialize, Serialize};

use crate::ast::{AssignmentExpr, BinaryExpr, CallExpr, FunctionExpr};
use crate::semantic::symbols::unbacktick;
use crate::semantic::{BindingId, BindingKind, SemanticModel};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    salsa::SalsaValue,
)]
pub enum ClassSystem {
    S4,
    R6,
    RefClass,
    S7,
}

impl ClassSystem {
    pub fn label(self) -> &'static str {
        match self {
            Self::S4 => "S4 class",
            Self::R6 => "R6 class",
            Self::RefClass => "reference class",
            Self::S7 => "S7 class",
        }
    }

    pub fn package(self) -> &'static str {
        match self {
            Self::R6 => "R6",
            Self::S4 | Self::RefClass => "methods",
            Self::S7 => "S7",
        }
    }

    pub fn callee(self) -> &'static str {
        match self {
            Self::S4 => "setClass",
            Self::R6 => "R6Class",
            Self::RefClass => "setRefClass",
            Self::S7 => "new_class",
        }
    }

    fn from_callee(name: &str) -> Option<Self> {
        match name {
            "setClass" => Some(Self::S4),
            "R6Class" => Some(Self::R6),
            "setRefClass" => Some(Self::RefClass),
            "new_class" => Some(Self::S7),
            _ => None,
        }
    }

    fn formals(self) -> &'static [&'static str] {
        match self {
            Self::R6 => &[
                "classname",
                "public",
                "private",
                "active",
                "inherit",
                "lock_objects",
                "class",
                "portable",
                "lock_class",
                "cloneable",
                "parent_env",
            ],
            Self::S4 => &[
                "Class",
                "representation",
                "prototype",
                "contains",
                "validity",
                "access",
                "where",
                "version",
                "sealed",
                "package",
                "S3methods",
                "slots",
            ],
            Self::RefClass => &[
                "Class",
                "fields",
                "contains",
                "methods",
                "where",
                "inheritPackage",
                "...",
            ],
            Self::S7 => &[
                "name",
                "parent",
                "package",
                "properties",
                "abstract",
                "constructor",
                "validator",
            ],
        }
    }
}

/// Only function frames containing OOP declarations participate in this key.
/// Adding an unrelated closure or block therefore cannot renumber owners.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    salsa::SalsaValue,
)]
pub struct ClassOwner(pub Vec<OwnerSegment>);

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, salsa::SalsaValue,
)]
pub struct OwnerSegment {
    pub name: Option<String>,
    pub occurrence: u32,
}

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, salsa::SalsaValue,
)]
pub struct ClassKey {
    pub owner: ClassOwner,
    pub system: ClassSystem,
    pub occurrence: u32,
}

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, salsa::SalsaValue,
)]
pub struct ClassId {
    pub path: PathBuf,
    pub key: ClassKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum ClassName {
    Known(String),
    Absent,
    Unknown,
}

/// A bare reference retains attachment order but never package-derived facts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, salsa::SalsaValue)]
pub enum FunctionRef {
    Qualified {
        package: String,
        name: String,
    },
    Bare {
        name: String,
        attached: Vec<String>,
        attachment_uncertain: bool,
    },
    /// Possible origins survive an ambiguous alias without proving a call.
    Unresolved(Vec<FunctionIdentity>),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, salsa::SalsaValue)]
pub struct FunctionIdentity {
    pub package: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum GeneratorRef {
    Local(ClassKey),
    Free(String),
    Qualified { package: String, name: String },
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum ParentRef {
    Formal(String),
    Generator(GeneratorRef),
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, salsa::SalsaValue)]
pub enum ClassUncertainty {
    Conditional,
    CustomEnvironment,
    InvalidArguments,
    DynamicName,
    DynamicParent,
    UnresolvedCallee,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct ClassDef {
    pub key: ClassKey,
    pub name: ClassName,
    pub binding: Option<String>,
    pub factory: FunctionRef,
    pub parents: Vec<ParentRef>,
    /// A literal `c(...)` of parents is useful only if `c` really is base's.
    pub parent_vector: Option<FunctionRef>,
    pub uncertainty: BTreeSet<ClassUncertainty>,
}

impl ClassDef {
    pub fn system(&self) -> ClassSystem {
        self.key.system
    }

    pub fn display_name(&self) -> &str {
        match &self.name {
            ClassName::Known(name) => name,
            ClassName::Absent | ClassName::Unknown => {
                self.binding.as_deref().unwrap_or("<anonymous class>")
            }
        }
    }

    /// An uncertain execution environment cannot establish a definite binding.
    pub fn binding_known(&self) -> bool {
        !self.uncertainty.iter().any(|issue| {
            matches!(
                issue,
                ClassUncertainty::Conditional
                    | ClassUncertainty::CustomEnvironment
                    | ClassUncertainty::InvalidArguments
                    | ClassUncertainty::UnresolvedCallee
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct GeneratorBinding {
    pub owner: ClassOwner,
    pub name: String,
    pub target: ClassKey,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct FileClassFacts {
    pub declarations: Vec<ClassDef>,
    pub generators: Vec<GeneratorBinding>,
    pub oop: OopFacts,
}

impl FileClassFacts {
    pub fn declaration(&self, key: &ClassKey) -> Option<&ClassDef> {
        self.declarations.iter().find(|def| &def.key == key)
    }

    /// Session-local validation for opaque LSP handles, independent of offsets.
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

#[derive(Debug, Clone)]
pub struct ClassLocation {
    pub name_range: TextRange,
    pub full_range: TextRange,
    call_range: TextRange,
}

#[derive(Debug, Clone)]
pub enum ClassOccurrence {
    Declaration(ClassKey),
    Reference {
        owner: ClassOwner,
        reference: ParentRef,
        /// Closures and R6 inheritance look up names in the completed environment.
        deferred: bool,
    },
}

struct Extracted {
    facts: FileClassFacts,
    locations: Vec<ClassLocation>,
    occurrences: Vec<(TextRange, ClassOccurrence)>,
    oop_sites: Vec<OopSite>,
    method_implementations: Vec<MethodImplementationSite>,
}

pub fn file_class_defs(model: &SemanticModel, root: &SyntaxNode) -> FileClassFacts {
    extract(model, root, false).facts
}

pub fn locate_class_def(root: &SyntaxNode, key: &ClassKey) -> Option<ClassLocation> {
    let model = SemanticModel::build(root);
    let extracted = extract(&model, root, false);
    extracted
        .facts
        .declarations
        .iter()
        .position(|def| &def.key == key)
        .map(|index| extracted.locations[index].clone())
}

pub fn class_occurrence_at_offset(
    model: &SemanticModel,
    root: &SyntaxNode,
    offset: TextSize,
) -> Option<ClassOccurrence> {
    let token = token_at(root, offset)?;
    let value = if let Some(binary) = token.parent().and_then(BinaryExpr::cast)
        && let Some(namespace) = binary.namespace_access()
    {
        if namespace.name_token != token {
            return None;
        }
        SyntaxElement::Node(binary.syntax().clone())
    } else {
        SyntaxElement::Token(token.clone())
    };
    let extracted = extract(model, root, false);
    if let Some((_, occurrence)) = extracted
        .occurrences
        .iter()
        .find(|(range, _)| *range == value.text_range())
    {
        return Some(occurrence.clone());
    }
    if value
        .as_token()
        .is_some_and(|token| token.kind() != SyntaxKind::IDENT)
    {
        return None;
    }
    let keys: HashMap<_, _> = extracted
        .facts
        .declarations
        .iter()
        .zip(&extracted.locations)
        .filter(|(def, _)| !matches!(def.factory, FunctionRef::Unresolved(_)))
        .map(|(def, loc)| (loc.call_range, def.key.clone()))
        .collect();
    // Resolve ordinary generator uses through the same lexical binding proof as
    // inheritance, rather than treating an arbitrary identifier as a class name.
    let parent = token.parent()?;
    let deferred = parent
        .ancestors()
        .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR);
    let target = generator_ref(model, root, &value, &keys, &mut BTreeSet::new(), deferred);
    let owner = owner_at(
        &parent,
        &owner_map(root, &extracted.facts.declarations, &extracted.locations),
    );
    Some(ClassOccurrence::Reference {
        owner,
        reference: ParentRef::Generator(target),
        deferred,
    })
}

fn extract(model: &SemanticModel, root: &SyntaxNode, include_sites: bool) -> Extracted {
    let calls: Vec<_> = root
        .descendants()
        .filter_map(CallExpr::cast)
        .flat_map(|call| {
            let Some(factory) = call_function_ref(model, root, &call) else {
                return Vec::new();
            };
            let systems: Vec<_> = match &factory {
                FunctionRef::Qualified { package, name } => ClassSystem::from_callee(name)
                    .filter(|system| package == system.package())
                    .into_iter()
                    .collect(),
                FunctionRef::Bare { name, .. } => {
                    ClassSystem::from_callee(name).into_iter().collect()
                }
                FunctionRef::Unresolved(candidates) => candidates
                    .iter()
                    .filter_map(|candidate| {
                        ClassSystem::from_callee(&candidate.name)
                            .filter(|system| candidate.package == system.package())
                    })
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            };
            systems
                .into_iter()
                .map(|system| (call.clone(), system, factory.clone()))
                .collect::<Vec<_>>()
        })
        .collect();
    let oop_roots = oop::declaration_roots(model, root);
    let mut owners: HashMap<TextRange, ClassOwner> = HashMap::new();
    let mut owner_counts = BTreeMap::new();
    for node in root
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::FUNCTION_EXPR)
    {
        if !calls
            .iter()
            .any(|(call, _, _)| node.text_range().contains_range(call.syntax().text_range()))
            && !oop_roots
                .iter()
                .any(|decl| node.text_range().contains_range(decl.text_range()))
        {
            continue;
        }
        let parent = owner_at(&node.parent().unwrap_or_else(|| root.clone()), &owners);
        let name = direct_assignment(&SyntaxElement::Node(node.clone()))
            .and_then(|assignment| assignment.target_name())
            .map(|name| unbacktick(&name).to_string());
        let occurrence = owner_counts
            .entry((parent.clone(), name.clone()))
            .or_insert(0);
        let mut owner = parent;
        owner.0.push(OwnerSegment {
            name,
            occurrence: *occurrence,
        });
        *occurrence += 1;
        owners.insert(node.text_range(), owner);
    }
    let mut counts = BTreeMap::new();
    let calls: Vec<_> = calls
        .into_iter()
        .map(|(call, system, factory)| {
            let owner = owner_at(call.syntax(), &owners);
            let occurrence = counts.entry((owner.clone(), system)).or_insert(0);
            let key = ClassKey {
                owner,
                system,
                occurrence: *occurrence,
            };
            *occurrence += 1;
            (call, system, factory, key)
        })
        .collect();
    let keys: HashMap<_, _> = calls
        .iter()
        .filter(|(_, _, factory, _)| !matches!(factory, FunctionRef::Unresolved(_)))
        .map(|(call, _, _, key)| (call.syntax().text_range(), key.clone()))
        .collect();
    let mut result = Extracted {
        facts: FileClassFacts::default(),
        locations: Vec::new(),
        occurrences: Vec::new(),
        oop_sites: Vec::new(),
        method_implementations: Vec::new(),
    };
    for (call, system, factory, key) in calls {
        let assignment = direct_assignment(&SyntaxElement::Node(call.syntax().clone()));
        let binding_token = assignment
            .as_ref()
            .and_then(AssignmentExpr::target_name_token);
        let mut def = ClassDef {
            key: key.clone(),
            name: ClassName::Unknown,
            binding: assignment
                .as_ref()
                .and_then(AssignmentExpr::target_name)
                .map(|name| unbacktick(&name).to_string()),
            factory,
            parents: Vec::new(),
            parent_vector: None,
            uncertainty: BTreeSet::new(),
        };
        if matches!(def.factory, FunctionRef::Unresolved(_)) {
            def.uncertainty.insert(ClassUncertainty::UnresolvedCallee);
        }
        if !unconditional(call.syntax()) {
            def.uncertainty.insert(ClassUncertainty::Conditional);
        }
        let mut selection = binding_token
            .as_ref()
            .map(SyntaxToken::text_range)
            .unwrap_or_else(|| {
                call.callee_token()
                    .map_or(call.syntax().text_range(), |token| token.text_range())
            });
        if let Some(args) = matched_args(&call, system.formals()) {
            let argument = |formal: &str| argument(&args, formal);
            let name_arg = argument(match system {
                ClassSystem::R6 => "classname",
                ClassSystem::S7 => "name",
                _ => "Class",
            });
            def.name = match name_arg {
                None if system == ClassSystem::R6 => ClassName::Absent,
                Some(SyntaxElement::Token(token))
                    if system == ClassSystem::R6 && token.text() == "NULL" =>
                {
                    ClassName::Absent
                }
                Some(SyntaxElement::Token(token)) if token.kind() == SyntaxKind::STRING => {
                    if let Some(name) = crate::semantic::decode_string_literal(token.text()) {
                        selection = token.text_range();
                        result
                            .occurrences
                            .push((selection, ClassOccurrence::Declaration(key.clone())));
                        ClassName::Known(name)
                    } else {
                        ClassName::Unknown
                    }
                }
                _ => ClassName::Unknown,
            };
            if def.name == ClassName::Unknown {
                def.uncertainty.insert(ClassUncertainty::DynamicName);
            }
            if argument(if system == ClassSystem::R6 {
                "parent_env"
            } else {
                "where"
            })
            .is_some()
            {
                def.uncertainty.insert(ClassUncertainty::CustomEnvironment);
            }
            if let Some(parent) = argument(match system {
                ClassSystem::R6 => "inherit",
                ClassSystem::S7 => "parent",
                _ => "contains",
            }) {
                if matches!(system, ClassSystem::R6 | ClassSystem::S7) {
                    if !matches!(parent, SyntaxElement::Token(ref token) if token.text() == "NULL")
                    {
                        let target = generator_ref(
                            model,
                            root,
                            &parent,
                            &keys,
                            &mut BTreeSet::new(),
                            system == ClassSystem::R6,
                        );
                        if target == GeneratorRef::Unknown {
                            def.uncertainty.insert(ClassUncertainty::DynamicParent);
                        }
                        let reference = ParentRef::Generator(target);
                        result.occurrences.push((
                            parent.text_range(),
                            ClassOccurrence::Reference {
                                owner: key.owner.clone(),
                                reference: reference.clone(),
                                deferred: system == ClassSystem::R6 || !key.owner.0.is_empty(),
                            },
                        ));
                        def.parents.push(reference);
                    }
                } else {
                    match formal_parents(model, root, &parent) {
                        Some(parents) => {
                            def.parent_vector = parents.vector;
                            for (name, range) in parents.names {
                                let reference = ParentRef::Formal(name);
                                result.occurrences.push((
                                    range,
                                    ClassOccurrence::Reference {
                                        owner: key.owner.clone(),
                                        reference: reference.clone(),
                                        deferred: !key.owner.0.is_empty(),
                                    },
                                ));
                                def.parents.push(reference);
                            }
                        }
                        None => {
                            def.parents.push(ParentRef::Unknown);
                            def.uncertainty.insert(ClassUncertainty::DynamicParent);
                        }
                    }
                }
            }
        } else {
            def.uncertainty.insert(ClassUncertainty::InvalidArguments);
        }
        if let Some(token) = binding_token {
            result
                .occurrences
                .push((token.text_range(), ClassOccurrence::Declaration(key)));
        }
        result.locations.push(ClassLocation {
            name_range: selection,
            full_range: assignment.as_ref().map_or_else(
                || call.syntax().text_range(),
                |assignment| assignment.syntax().text_range(),
            ),
            call_range: call.syntax().text_range(),
        });
        result.facts.declarations.push(def);
    }
    for (index, binding) in model.bindings().iter().enumerate() {
        let frame = root
            .covering_element(binding.def_range)
            .parent()
            .and_then(|node| {
                node.ancestors()
                    .find(|node| node.kind() == SyntaxKind::FUNCTION_EXPR)
            });
        if frame
            .as_ref()
            .is_some_and(|frame| !owners.contains_key(&frame.text_range()))
        {
            continue;
        }
        let id = BindingId::from_index(index);
        let Some(value) = stable_value(model, root, id) else {
            continue;
        };
        if let GeneratorRef::Local(target) =
            generator_ref(model, root, &value, &keys, &mut BTreeSet::new(), false)
        {
            if result
                .facts
                .declaration(&target)
                .is_none_or(|def| !def.binding_known())
            {
                continue;
            }
            let owner = owner_at(
                &root
                    .covering_element(binding.def_range)
                    .parent()
                    .unwrap_or_else(|| root.clone()),
                &owners,
            );
            result.facts.generators.push(GeneratorBinding {
                owner,
                name: unbacktick(&binding.name).to_string(),
                target,
            });
        }
    }
    result
        .facts
        .generators
        .sort_by(|a, b| (&a.owner, &a.name, &a.target).cmp(&(&b.owner, &b.name, &b.target)));
    let (facts, sites, implementations) =
        oop::project(model, root, &owners, &result, include_sites);
    result.method_implementations = implementations;
    result.facts.oop = facts;
    result.oop_sites = sites;
    result
}

fn owner_at(node: &SyntaxNode, owners: &HashMap<TextRange, ClassOwner>) -> ClassOwner {
    node.ancestors()
        .find_map(|ancestor| owners.get(&ancestor.text_range()).cloned())
        .unwrap_or_default()
}

fn owner_map(
    root: &SyntaxNode,
    defs: &[ClassDef],
    locations: &[ClassLocation],
) -> HashMap<TextRange, ClassOwner> {
    let mut owners = HashMap::new();
    for (def, location) in defs.iter().zip(locations) {
        if let Some(node) = root.covering_element(location.full_range).into_node() {
            let functions: Vec<_> = node
                .ancestors()
                .filter(|node| node.kind() == SyntaxKind::FUNCTION_EXPR)
                .collect();
            for (depth, node) in functions.into_iter().rev().enumerate() {
                owners.insert(
                    node.text_range(),
                    ClassOwner(def.key.owner.0[..=depth].to_vec()),
                );
            }
        }
    }
    owners
}

fn unconditional(node: &SyntaxNode) -> bool {
    for ancestor in node.ancestors().skip(1) {
        match ancestor.kind() {
            SyntaxKind::ROOT => return true,
            SyntaxKind::FUNCTION_EXPR => {
                return FunctionExpr::cast(ancestor)
                    .and_then(|function| function.body())
                    .is_some_and(|body| body.text_range().contains_range(node.text_range()));
            }
            SyntaxKind::BLOCK_EXPR | SyntaxKind::ASSIGNMENT_EXPR => {}
            _ => return false,
        }
    }
    true
}

fn direct_assignment(value: &SyntaxElement) -> Option<AssignmentExpr> {
    let assignment = value.parent().and_then(AssignmentExpr::cast)?;
    (assignment.value_element()?.text_range() == value.text_range()).then_some(assignment)
}

fn stable_value(model: &SemanticModel, root: &SyntaxNode, id: BindingId) -> Option<SyntaxElement> {
    let binding = model.binding(id);
    if binding.kind != BindingKind::Local
        || model.variable_cohort(id).len() != 1
        || model.bindings().iter().any(|other| {
            other.kind == BindingKind::Implicit
                && unbacktick(&other.name) == unbacktick(&binding.name)
        })
    {
        return None;
    }
    if crate::semantic::calls::has_dynamic_bindings(model)
        && opaque_binding_frame(root, binding.def_range)
    {
        return None;
    }
    let assignment = binding_assignment(root, binding.def_range)?;
    unconditional(assignment.syntax())
        .then(|| assignment.value_element())
        .flatten()
}

fn binding_assignment(root: &SyntaxNode, range: TextRange) -> Option<AssignmentExpr> {
    root.covering_element(range)
        .into_token()?
        .parent()
        .and_then(AssignmentExpr::cast)
}

fn opaque_binding_frame(root: &SyntaxNode, range: TextRange) -> bool {
    let frame = |node: SyntaxNode| {
        node.ancestors()
            .find(|node| node.kind() == SyntaxKind::FUNCTION_EXPR)
            .map(|node| node.text_range())
    };
    let binding_frame = root.covering_element(range).parent().and_then(frame);
    root.descendants().filter_map(CallExpr::cast).any(|call| {
        call.callee_name().is_some_and(|name| {
            if replacement_assignment(&call).is_some() {
                crate::semantic::calls::changes_bindings(&format!("{name}<-"))
            } else {
                crate::semantic::calls::changes_bindings(unbacktick(&name))
            }
        }) && frame(call.syntax().clone()) == binding_frame
    })
}

fn read_binding(model: &SemanticModel, token: &SyntaxToken) -> Option<BindingId> {
    if let Some(ident) = model
        .idents()
        .iter()
        .find(|ident| ident.range == token.text_range())
    {
        return model.resolve_local(ident);
    }
    if token.kind() != SyntaxKind::STRING {
        return None;
    }
    let name = crate::semantic::decode_string_literal(token.text())?;
    read_named_binding(model, token, &name)
}

fn read_named_binding(model: &SemanticModel, token: &SyntaxToken, name: &str) -> Option<BindingId> {
    let mut scope = Some(model.innermost_scope_at(token.text_range().start()));
    while let Some(id) = scope {
        let frame = model.scope(id);
        if let Some(binding) = frame
            .bindings
            .iter()
            .find(|id| unbacktick(&model.binding(**id).name) == name)
        {
            return Some(*binding);
        }
        scope = frame.parent;
    }
    None
}

fn function_ref(
    model: &SemanticModel,
    root: &SyntaxNode,
    value: &SyntaxElement,
    visiting: &mut BTreeSet<u32>,
) -> Option<FunctionRef> {
    if let SyntaxElement::Node(node) = value {
        if let Some(paren) = crate::ast::ParenExpr::cast(node.clone()) {
            return function_ref(model, root, &paren.inner()?, visiting);
        }
        if let Some(branch) = crate::ast::IfExpr::cast(node.clone()) {
            let mut candidates = BTreeSet::new();
            for value in branch
                .then_elements()
                .into_iter()
                .flatten()
                .chain(branch.else_elements().into_iter().flatten())
            {
                if let Some(reference) = function_ref(model, root, &value, &mut visiting.clone()) {
                    candidates.extend(possible_functions(&reference));
                }
            }
            return (!candidates.is_empty())
                .then(|| FunctionRef::Unresolved(candidates.into_iter().collect()));
        }
        let namespace = BinaryExpr::cast(node.clone())?.namespace_access()?;
        return Some(FunctionRef::Qualified {
            package: unbacktick(&namespace.package).to_string(),
            name: unbacktick(&namespace.name).to_string(),
        });
    }
    let token = value.as_token()?;
    if !matches!(token.kind(), SyntaxKind::IDENT | SyntaxKind::STRING) {
        return None;
    }
    let name = if token.kind() == SyntaxKind::STRING {
        crate::semantic::decode_string_literal(token.text())?
    } else {
        unbacktick(token.text()).to_string()
    };
    named_function_ref(model, root, token, &name, visiting)
}

fn named_function_ref(
    model: &SemanticModel,
    root: &SyntaxNode,
    token: &SyntaxToken,
    name: &str,
    visiting: &mut BTreeSet<u32>,
) -> Option<FunctionRef> {
    if let Some(id) = read_named_binding(model, token, name) {
        if visiting.len() >= 64 || !visiting.insert(id.0) {
            return None;
        }
        if let Some(target) = stable_value(model, root, id)
            && model.binding(id).def_range.start() < token.text_range().start()
        {
            return function_ref(model, root, &target, visiting);
        }
        let mut candidates = BTreeSet::new();
        for candidate in model.variable_cohort(id) {
            let Some(target) = binding_assignment(root, model.binding(candidate).def_range)
                .and_then(|assignment| assignment.value_element())
            else {
                continue;
            };
            if let Some(reference) = function_ref(model, root, &target, &mut visiting.clone()) {
                candidates.extend(possible_functions(&reference));
            }
        }
        return (!candidates.is_empty())
            .then(|| FunctionRef::Unresolved(candidates.into_iter().collect()));
    }
    let in_function = token
        .parent()?
        .ancestors()
        .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR);
    let mut attached = Vec::new();
    let mut attachment_uncertain = false;
    for package in model
        .loaded_packages()
        .iter()
        .filter(|package| in_function || package.range.start() < token.text_range().start())
    {
        let call = root
            .covering_element(package.range)
            .parent()
            .and_then(|node| node.ancestors().find_map(CallExpr::cast));
        let trusted = call.as_ref().is_some_and(|call| {
            let Some(callee) = call.callee_token() else {
                return false;
            };
            let origin_known = match call.base() {
                Some(SyntaxElement::Node(node)) => BinaryExpr::cast(node)
                    .and_then(|binary| binary.namespace_access())
                    .is_some_and(|namespace| namespace.package == "base"),
                Some(SyntaxElement::Token(_)) => read_binding(model, &callee).is_none(),
                None => false,
            };
            origin_known && unconditional(call.syntax())
        });
        if trusted {
            attached.push(package.name.to_string());
        } else {
            attachment_uncertain = true;
        }
    }
    Some(FunctionRef::Bare {
        name: name.into(),
        attached,
        attachment_uncertain,
    })
}

fn generator_ref(
    model: &SemanticModel,
    root: &SyntaxNode,
    value: &SyntaxElement,
    keys: &HashMap<TextRange, ClassKey>,
    visiting: &mut BTreeSet<u32>,
    deferred: bool,
) -> GeneratorRef {
    if let Some(key) = keys.get(&value.text_range()) {
        return GeneratorRef::Local(key.clone());
    }
    if let Some(namespace) = value
        .as_node()
        .cloned()
        .and_then(BinaryExpr::cast)
        .and_then(|binary| binary.namespace_access())
    {
        return GeneratorRef::Qualified {
            package: namespace.package.to_string(),
            name: namespace.name.to_string(),
        };
    }
    let Some(token) = value
        .as_token()
        .filter(|token| token.kind() == SyntaxKind::IDENT)
    else {
        return GeneratorRef::Unknown;
    };
    let Some(id) = read_binding(model, token) else {
        return GeneratorRef::Free(unbacktick(token.text()).to_string());
    };
    if visiting.len() >= 64
        || !visiting.insert(id.0)
        || (!deferred && model.binding(id).def_range.start() >= token.text_range().start())
    {
        return GeneratorRef::Unknown;
    }
    let Some(target) = stable_value(model, root, id) else {
        return GeneratorRef::Unknown;
    };
    if target.text_range().contains_range(token.text_range()) {
        return GeneratorRef::Unknown;
    }
    // R6 captures the inheritance expression, but an alias assignment still
    // evaluates its right-hand side when that assignment executes.
    generator_ref(model, root, &target, keys, visiting, false)
}

struct FormalParents {
    names: Vec<(String, TextRange)>,
    vector: Option<FunctionRef>,
}

fn formal_parents(
    model: &SemanticModel,
    root: &SyntaxNode,
    value: &SyntaxElement,
) -> Option<FormalParents> {
    if let SyntaxElement::Token(token) = value {
        return (token.kind() == SyntaxKind::STRING)
            .then(|| {
                crate::semantic::decode_string_literal(token.text()).map(|name| FormalParents {
                    names: vec![(name, token.text_range())],
                    vector: None,
                })
            })
            .flatten();
    }
    let call = value.as_node().cloned().and_then(CallExpr::cast)?;
    let function = function_ref(model, root, &call.base()?, &mut BTreeSet::new())?;
    match &function {
        FunctionRef::Qualified { package, name } if package == "base" && name == "c" => {}
        FunctionRef::Bare { name, .. } if name == "c" => {}
        _ => return None,
    }
    let mut names = Vec::new();
    for arg in call.arg_list()?.args() {
        let token = arg.value()?.into_token()?;
        if token.kind() != SyntaxKind::STRING {
            return None;
        }
        names.push((
            crate::semantic::decode_string_literal(token.text())?,
            token.text_range(),
        ));
    }
    Some(FormalParents {
        names,
        vector: Some(function),
    })
}

fn token_at(root: &SyntaxNode, offset: TextSize) -> Option<SyntaxToken> {
    match root.token_at_offset(offset) {
        TokenAtOffset::None => None,
        TokenAtOffset::Single(token) => Some(token),
        TokenAtOffset::Between(left, right) => {
            if matches!(right.kind(), SyntaxKind::IDENT | SyntaxKind::STRING) {
                Some(right)
            } else {
                Some(left)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    struct Projection {
        system: ClassSystem,
        parents: Vec<String>,
    }

    fn facts_of(src: &str) -> FileClassFacts {
        let root = parse(src).cst;
        assert_eq!(root.text().to_string(), src);
        file_class_defs(&SemanticModel::build(&root), &root)
    }

    fn defs_of(src: &str) -> BTreeMap<String, Projection> {
        let facts = facts_of(src);
        facts
            .declarations
            .iter()
            .map(|def| {
                (
                    def.display_name().to_string(),
                    Projection {
                        system: def.system(),
                        parents: def
                            .parents
                            .iter()
                            .filter_map(|parent| match parent {
                                ParentRef::Formal(name)
                                | ParentRef::Generator(GeneratorRef::Free(name)) => {
                                    Some(name.clone())
                                }
                                ParentRef::Generator(GeneratorRef::Local(key)) => facts
                                    .declaration(key)
                                    .map(|def| def.display_name().to_string()),
                                _ => None,
                            })
                            .collect(),
                    },
                )
            })
            .collect()
    }

    fn parents_of<'a>(defs: &'a BTreeMap<String, Projection>, name: &str) -> &'a [String] {
        &defs.get(name).expect("class def").parents
    }

    #[test]
    fn s4_contains_single_string() {
        let defs = defs_of("setClass(\"Child\", contains = \"Parent\")\n");
        assert_eq!(defs["Child"].system, ClassSystem::S4);
        assert_eq!(parents_of(&defs, "Child"), ["Parent"]);
    }

    #[test]
    fn s4_contains_vector() {
        let defs = defs_of("setClass(\"C\", contains = c(\"A\", \"B\"))\n");
        assert_eq!(parents_of(&defs, "C"), ["A", "B"]);
    }

    #[test]
    fn s4_without_contains_has_no_parents() {
        let defs = defs_of("setClass(\"Root\")\n");
        assert_eq!(defs["Root"].system, ClassSystem::S4);
        assert!(parents_of(&defs, "Root").is_empty());
    }

    #[test]
    fn r6_inherit_identifier() {
        let defs = defs_of("Child <- R6Class(\"Child\", inherit = Parent)\n");
        assert_eq!(defs["Child"].system, ClassSystem::R6);
        assert_eq!(parents_of(&defs, "Child"), ["Parent"]);
    }

    #[test]
    fn r6_without_inherit_has_no_parents() {
        let defs = defs_of("Animal <- R6Class(\"Animal\", public = list())\n");
        assert_eq!(defs["Animal"].system, ClassSystem::R6);
        assert!(parents_of(&defs, "Animal").is_empty());
    }

    #[test]
    fn refclass_contains_string() {
        let defs = defs_of("Child <- setRefClass(\"Child\", contains = \"Parent\")\n");
        assert_eq!(defs["Child"].system, ClassSystem::RefClass);
        assert_eq!(parents_of(&defs, "Child"), ["Parent"]);
    }

    #[test]
    fn single_quoted_class_name_is_unquoted() {
        let defs = defs_of("setClass('Child', contains = 'Parent')\n");
        assert_eq!(parents_of(&defs, "Child"), ["Parent"]);
    }

    #[test]
    fn non_class_calls_are_ignored() {
        let defs = defs_of("print(\"hello\")\nx <- list(contains = \"y\")\n");
        assert!(defs.is_empty());
    }

    #[test]
    fn locate_yields_name_and_statement_spans() {
        let src = "Child <- R6Class(\"Child\", inherit = Parent)\n";
        let root = parse(src).cst;
        let facts = file_class_defs(&SemanticModel::build(&root), &root);
        let loc = locate_class_def(&root, &facts.declarations[0].key).expect("located");
        assert_eq!(facts.declarations[0].system(), ClassSystem::R6);
        // Selection is the class-name string literal (quotes included).
        assert_eq!(&src[loc.name_range], "\"Child\"");
        // Full range is the whole assignment statement (sans trailing newline).
        assert_eq!(
            &src[loc.full_range],
            "Child <- R6Class(\"Child\", inherit = Parent)"
        );
    }

    fn name_at(src: &str, needle: &str) -> Option<String> {
        let root = parse(src).cst;
        let offset = TextSize::new(src.find(needle).expect("needle") as u32 + 1);
        let model = SemanticModel::build(&root);
        let facts = file_class_defs(&model, &root);
        match class_occurrence_at_offset(&model, &root, offset)? {
            ClassOccurrence::Declaration(key) => facts
                .declaration(&key)
                .map(|def| def.display_name().to_string()),
            ClassOccurrence::Reference {
                reference: ParentRef::Formal(name) | ParentRef::Generator(GeneratorRef::Free(name)),
                ..
            } => Some(name),
            ClassOccurrence::Reference {
                reference: ParentRef::Generator(GeneratorRef::Local(key)),
                ..
            } => facts
                .declaration(&key)
                .map(|def| def.display_name().to_string()),
            _ => None,
        }
    }

    #[test]
    fn cursor_on_class_name_string() {
        assert_eq!(
            name_at("setClass(\"Child\", contains = \"Parent\")\n", "\"Child\""),
            Some("Child".to_string())
        );
    }

    #[test]
    fn cursor_on_contains_string() {
        assert_eq!(
            name_at("setClass(\"Child\", contains = \"Parent\")\n", "\"Parent\""),
            Some("Parent".to_string())
        );
    }

    #[test]
    fn cursor_on_contains_vector_element() {
        assert_eq!(
            name_at("setClass(\"C\", contains = c(\"A\", \"B\"))\n", "\"B\""),
            Some("B".to_string())
        );
    }

    #[test]
    fn cursor_on_r6_binding_identifier() {
        assert_eq!(
            name_at(
                "Child <- R6Class(\"Child\", inherit = Parent)\n",
                "Child <-"
            ),
            Some("Child".to_string())
        );
    }

    #[test]
    fn cursor_on_inherit_identifier() {
        assert_eq!(
            name_at("Child <- R6Class(\"Child\", inherit = Parent)\n", "Parent)"),
            Some("Parent".to_string())
        );
    }

    #[test]
    fn cursor_on_plain_string_names_nothing() {
        assert_eq!(name_at("x <- \"Parent\"\n", "\"Parent\""), None);
    }

    #[test]
    fn class_facts_retain_dynamic_conditional_and_custom_environment_declarations() {
        let facts = facts_of(
            "if (flag) A <- R6::R6Class(classname = name, inherit = choose())\nmethods::setRefClass(\"Local\", where = env)\n",
        );
        assert_eq!(facts.declarations.len(), 2);
        assert_eq!(facts.declarations[0].name, ClassName::Unknown);
        assert!(
            facts.declarations[0]
                .uncertainty
                .contains(&ClassUncertainty::Conditional)
        );
        assert!(
            facts.declarations[0]
                .uncertainty
                .contains(&ClassUncertainty::DynamicParent)
        );
        assert!(
            facts.declarations[1]
                .uncertainty
                .contains(&ClassUncertainty::CustomEnvironment)
        );
    }

    #[test]
    fn class_facts_reject_invalid_argument_matching() {
        for source in [
            "R6::R6Class(clas = \"A\")",
            "R6::R6Class(classname = \"A\", classname = \"B\")",
            "methods::setClass(\"A\", nonsense = 1)",
            "R6::R6Class(\"A\", ...)",
        ] {
            let facts = facts_of(source);
            assert_eq!(facts.declarations.len(), 1, "{source}");
            assert!(
                facts.declarations[0]
                    .uncertainty
                    .contains(&ClassUncertainty::InvalidArguments),
                "{source}"
            );
        }
    }

    #[test]
    fn class_facts_match_partial_positional_and_backticked_arguments() {
        let facts = facts_of(
            "`make` <- R6::R6Class\n`Generator<-` <- `make`(public = list(), classn = \"Display\")\nChild <- R6::R6Class(\"Child\", list(), NULL, NULL, `Generator<-`)\n",
        );
        assert_eq!(facts.declarations.len(), 2);
        assert_eq!(
            facts.declarations[0].binding.as_deref(),
            Some("Generator<-")
        );
        assert_eq!(
            facts.declarations[0].name,
            ClassName::Known("Display".into())
        );
        assert_eq!(
            facts.declarations[1].parents,
            [ParentRef::Generator(GeneratorRef::Local(
                facts.declarations[0].key.clone()
            ))]
        );
    }

    #[test]
    fn class_owner_keys_ignore_unrelated_closures_and_prose() {
        let before = "f <- function() { A <- R6::R6Class(\"A\"); A }\n";
        let after = "# Prose.\ng <- function() 2\nf <- function() { h <- function() 1; A <- R6::R6Class(\"A\"); A }\n";
        assert_eq!(facts_of(before), facts_of(after));
        let before = "A <- R6::R6Class(\"A\")\nf <- function() 1\n";
        let after = "A <- R6::R6Class(\"A\")\nf <- function() { alias <- A; alias }\n";
        assert_eq!(facts_of(before), facts_of(after));
    }

    #[test]
    fn class_facts_decode_literal_names_without_evaluating_expressions() {
        let facts = facts_of(r#"methods::setClass("\u0041", contains = r"(Parent)")"#);
        assert_eq!(facts.declarations[0].name, ClassName::Known("A".into()));
        assert_eq!(
            facts.declarations[0].parents,
            [ParentRef::Formal("Parent".into())]
        );
    }
    #[test]
    fn class_facts_keep_uncertain_callee_aliases_without_proving_generators() {
        for source in [
            "if (flag) make <- R6::R6Class\nA <- make(\"A\")\n",
            "make <- R6::R6Class\nmake <- function(...) NULL\nA <- make(\"A\")\n",
            "make <- R6::R6Class\nsource(path)\nA <- make(\"A\")\n",
        ] {
            let facts = facts_of(source);
            assert_eq!(facts.declarations.len(), 1, "{source}");
            assert!(
                facts.declarations[0]
                    .uncertainty
                    .contains(&ClassUncertainty::UnresolvedCallee),
                "{source}"
            );
            assert!(facts.generators.is_empty(), "{source}");
        }
        let facts = facts_of("make <- R6::R6Class\nmake <- methods::setClass\nA <- make(\"A\")\n");
        assert_eq!(facts.declarations.len(), 2);
        assert_ne!(facts.declarations[0].key, facts.declarations[1].key);
    }

    #[test]
    fn class_facts_reject_shadowed_quoted_callees() {
        assert!(
            facts_of("setClass <- function(...) NULL\n\"setClass\"(\"A\")\n")
                .declarations
                .is_empty()
        );
    }
    #[test]
    fn class_facts_decode_utf8_byte_escapes_as_r_strings() {
        for source in [
            r#"methods::setClass("\303\251")"#,
            r#"methods::setClass("\xc3\xa9")"#,
        ] {
            assert_eq!(
                facts_of(source).declarations[0].name,
                ClassName::Known("é".into())
            );
        }
        for source in ["R6::R6Class(", "R6::R6Class(\"", "R6::R6Class(classname = "] {
            let _ = facts_of(source);
        }
    }
}
