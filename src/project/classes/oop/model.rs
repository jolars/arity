use std::collections::BTreeSet;
use std::path::PathBuf;

use rowan::TextRange;

use super::super::{
    ClassKey, ClassName, ClassOwner, ClassSystem, FunctionRef, GeneratorRef, ParentRef,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, salsa::SalsaValue)]
pub enum OopSystem {
    S3,
    S4,
    R6,
    RefClass,
    S7,
}

impl From<ClassSystem> for OopSystem {
    fn from(system: ClassSystem) -> Self {
        match system {
            ClassSystem::S4 => Self::S4,
            ClassSystem::R6 => Self::R6,
            ClassSystem::RefClass => Self::RefClass,
            ClassSystem::S7 => Self::S7,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, salsa::SalsaValue)]
pub enum OopKind {
    Generic,
    Method,
    Member,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, salsa::SalsaValue)]
pub struct OopKey {
    pub owner: ClassOwner,
    pub system: OopSystem,
    pub kind: OopKind,
    pub occurrence: u32,
}

/// Project identity adds a file to the lexical declaration identity. The package
/// owner is supplied by `ClassFile::package_root`, never inferred in this module.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, salsa::SalsaValue)]
pub struct OopId {
    pub path: PathBuf,
    pub key: OopKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct CallableParameter {
    pub name: String,
    pub default: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct CallableSignature {
    pub parameters: Vec<CallableParameter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct GenericRef {
    pub system: OopSystem,
    pub owner: ClassOwner,
    pub name: ClassName,
    pub package: Option<String>,
    /// A generator binding may differ from the registered generic name.
    pub by_binding: bool,
    pub local: Option<OopKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct GenericDef {
    pub key: OopKey,
    pub name: ClassName,
    pub binding: Option<String>,
    pub factory: FunctionRef,
    pub signature: Option<CallableSignature>,
    pub dispatch_arguments: Vec<ClassName>,
    /// A literal vector's helper must be confirmed before its shape is known.
    pub dispatch_factory: Option<FunctionRef>,
    pub uncertainty: BTreeSet<OopUncertainty>,
}

impl GenericDef {
    /// An uncertain call cannot establish a definite local alias target.
    pub fn binding_known(&self) -> bool {
        self.binding_possible() && !self.uncertainty.contains(&OopUncertainty::UnresolvedCallee)
    }

    /// Bare factories retain conditional binding facts until the project can
    /// confirm their origins. Ambiguous callee aliases cannot establish them.
    pub(super) fn binding_possible(&self) -> bool {
        !matches!(self.factory, FunctionRef::Unresolved(_))
            && !self.uncertainty.iter().any(|issue| {
                matches!(
                    issue,
                    OopUncertainty::Conditional
                        | OopUncertainty::CustomEnvironment
                        | OopUncertainty::InvalidArguments
                )
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum DispatchClass {
    Formal(String),
    S3(String),
    Generator(GeneratorRef),
    Any,
    Missing,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct DispatchArgument {
    pub argument: Option<String>,
    pub class: DispatchClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct MethodDef {
    pub key: OopKey,
    pub system: OopSystem,
    pub name: ClassName,
    pub class: Option<ClassKey>,
    pub generic: Option<GenericRef>,
    pub dispatch: Vec<DispatchArgument>,
    /// Retain the helper origin just as class inheritance retains `parent_vector`.
    pub dispatch_factory: Option<FunctionRef>,
    pub signature: Option<CallableSignature>,
    pub registration: Option<FunctionRef>,
    pub uncertainty: BTreeSet<OopUncertainty>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum MemberVisibility {
    Public,
    Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum MemberKind {
    Field,
    Method,
    ActiveBinding,
    Slot,
    Property,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct MemberDef {
    pub key: OopKey,
    pub class: ClassKey,
    pub name: ClassName,
    pub visibility: MemberVisibility,
    pub kind: MemberKind,
    pub method: Option<OopKey>,
    pub declared_type: Option<DispatchClass>,
    pub uncertainty: BTreeSet<OopUncertainty>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, salsa::SalsaValue)]
pub enum OopUncertainty {
    Conditional,
    CustomEnvironment,
    InvalidArguments,
    DynamicName,
    DynamicSignature,
    DynamicMembers,
    UnresolvedCallee,
    UnresolvedImplementation,
    UnresolvedAlias,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct GenericBinding {
    pub owner: ClassOwner,
    pub name: String,
    pub target: OopKey,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct OopFacts {
    pub generics: Vec<GenericDef>,
    pub generic_bindings: Vec<GenericBinding>,
    pub methods: Vec<MethodDef>,
    pub members: Vec<MemberDef>,
    pub member_groups: Vec<MemberGroup>,
}

/// Retain a group's constructor origin and incomplete shape even when no member
/// names can be recovered (for example, `public = build_members()`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct MemberGroup {
    pub class: ClassKey,
    pub name: String,
    pub factory: Option<FunctionRef>,
    pub uncertainty: BTreeSet<OopUncertainty>,
}

impl OopFacts {
    /// A candidate list is intentional: neither duplicate declarations nor
    /// competing registrations establish R's runtime choice of implementation.
    pub fn generic_candidates(&self, reference: &GenericRef) -> Vec<&GenericDef> {
        if let Some(key) = &reference.local {
            return self
                .generics
                .iter()
                .filter(|def| &def.key == key && def.key.system == reference.system)
                .collect();
        }
        let ClassName::Known(name) = &reference.name else {
            return Vec::new();
        };
        if reference.package.is_some() {
            return Vec::new();
        }
        for depth in (0..=reference.owner.0.len()).rev() {
            let candidates: Vec<_> = self
                .generics
                .iter()
                .filter(|def| {
                    def.key.system == reference.system
                        && def.key.owner.0 == reference.owner.0[..depth]
                        && if reference.by_binding {
                            self.generic_bindings.iter().any(|binding| {
                                binding.target == def.key
                                    && &binding.name == name
                                    && binding.owner.0 == reference.owner.0[..depth]
                            })
                        } else {
                            def.name == reference.name
                        }
                })
                .collect();
            if !candidates.is_empty() {
                return candidates;
            }
        }
        Vec::new()
    }

    pub fn member_candidates(&self, class: &ClassKey, name: &str) -> Vec<&MemberDef> {
        self.members
            .iter()
            .filter(|def| &def.class == class && def.name == ClassName::Known(name.to_string()))
            .collect()
    }

    pub fn methods_for_generic(&self, generic: &OopKey) -> Vec<&MethodDef> {
        self.methods
            .iter()
            .filter(|method| {
                method.generic.as_ref().is_some_and(|reference| {
                    self.generic_candidates(reference)
                        .iter()
                        .any(|def| &def.key == generic)
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberReceiver {
    Generator(GeneratorRef),
    Binding(String),
    /// This records a syntactic receiver inside a class-owned method. Method
    /// environment analysis must still prove whether a local shadows it.
    Implicit {
        class: ClassKey,
        name: String,
    },
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OopTarget {
    Class(ClassKey),
    Declaration(OopKey),
    ClassReference(ParentRef),
    DispatchClass {
        system: OopSystem,
        class: DispatchClass,
    },
    Generic(GenericRef),
    Member {
        receiver: MemberReceiver,
        name: String,
        slot: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OopSiteRole {
    Declaration,
    Alias,
    Use,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OopSite {
    pub range: TextRange,
    pub role: OopSiteRole,
    pub target: OopTarget,
    pub owner: ClassOwner,
    pub in_function: bool,
}
