//! Formal class metadata and structural slot schemas. Coercion and validity
//! functions are recorded as declarations, never evaluated to discover structure.

use super::*;

mod extract;
mod slots;
pub(super) use extract::{class_shape, declaration_count, formals, project, relation_formals};
pub(crate) use slots::slot_call;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum S4Definition {
    Class,
    Union,
    OldClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum S4Default {
    String(String),
    Literal(String),
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S4Prototype {
    pub provided: bool,
    pub slots: BTreeMap<String, S4Default>,
    pub data: Option<S4Default>,
    pub complete: bool,
    pub helpers: Vec<FunctionRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S4OldClass {
    pub classes: Vec<ClassName>,
    pub index: usize,
    pub test: Option<bool>,
    /// An explicit S4 definition promises a schema for the old class's attributes.
    pub s4_class: Option<ParentRef>,
    /// Parents at this offset in ClassDef::parents come from this registration.
    /// A self-template keeps the formal declaration's earlier parents separate.
    pub parent_start: usize,
    pub factory: FunctionRef,
    pub helpers: Vec<FunctionRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S4Class {
    pub definition: S4Definition,
    pub virtual_class: Option<bool>,
    pub sealed: Option<bool>,
    pub package: ClassName,
    pub prototype: S4Prototype,
    pub validity: bool,
    pub schema_helpers: Vec<FunctionRef>,
    pub slot_helpers: Vec<FunctionRef>,
    pub slots_complete: bool,
    pub parents_complete: bool,
    pub union_members: Vec<ClassName>,
    pub old_class: Option<S4OldClass>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum S4RelationKind {
    UnionMember,
    Is,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S4Relation {
    pub owner: ClassOwner,
    pub subclass: ClassName,
    pub superclass: ClassName,
    pub kind: S4RelationKind,
    pub simple: Option<bool>,
    pub do_complete: Option<bool>,
    pub factory: FunctionRef,
    pub conditional: bool,
    pub coerce: bool,
    pub replace: bool,
    pub by: Vec<ClassName>,
    pub helpers: Vec<FunctionRef>,
    pub uncertainty: BTreeSet<OopUncertainty>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S4Coercion {
    pub owner: ClassOwner,
    pub from: ClassName,
    pub to: ClassName,
    pub factory: FunctionRef,
    pub coerce: bool,
    pub replace: bool,
    pub uncertainty: BTreeSet<OopUncertainty>,
    pub(super) definition_valid: Option<bool>,
}

impl S4Coercion {
    pub(super) fn method_key(&self, ordinal: usize) -> OopKey {
        OopKey {
            owner: self.owner.clone(),
            system: OopSystem::S4,
            kind: OopKind::Coercion,
            occurrence: ordinal as u32,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S4Facts {
    pub relations: Vec<S4Relation>,
    pub coercions: Vec<S4Coercion>,
    pub(in crate::project::classes) namespace_loads: Vec<S4NamespaceLoad>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub(in crate::project::classes) struct S4NamespaceLoad {
    pub package: Option<String>,
    pub unconditional: bool,
    pub kind: S4NamespaceLoadKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub(in crate::project::classes) enum S4NamespaceLoadKind {
    Access,
    Call {
        callee: Option<FunctionRef>,
        attach_depends: Option<bool>,
    },
}

impl S4NamespaceLoad {
    pub fn attach_depends(&self) -> Option<bool> {
        match self.kind {
            S4NamespaceLoadKind::Access => Some(false),
            S4NamespaceLoadKind::Call { attach_depends, .. } => attach_depends,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4Slot {
    /// Built-in data slots have no editable declaration.
    pub declaration: Option<OopId>,
    pub declared_type: DispatchClass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4Slots {
    pub entries: BTreeMap<String, Vec<S4Slot>>,
    /// Effective order in R's class definition, including inherited slots.
    pub order: Vec<String>,
    pub complete: bool,
}

impl Default for S4Slots {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            order: Vec::new(),
            complete: true,
        }
    }
}

impl S4Slots {
    pub(super) fn slot_entries(&mut self, name: &str, inherited: bool) -> &mut Vec<S4Slot> {
        self.entries.entry(name.to_string()).or_insert_with(|| {
            // R prepends an inherited data part. An explicitly declared .Data
            // slot stays where it appears in the declaration.
            if inherited && name == ".Data" {
                self.order.insert(0, name.to_string());
            } else {
                self.order.push(name.to_string());
            }
            Vec::new()
        })
    }

    pub(super) fn into_ordered_entries(mut self) -> impl Iterator<Item = (String, Vec<S4Slot>)> {
        self.order.into_iter().filter_map(move |name| {
            let slots = self.entries.remove(&name)?;
            Some((name, slots))
        })
    }
}

pub(super) struct SlotEntry {
    pub name: Option<String>,
    pub value: Option<SyntaxElement>,
    pub range: TextRange,
    pub untyped: bool,
}

pub(super) struct ClassShape {
    pub metadata: S4Class,
    pub name: ClassName,
    pub name_range: Option<TextRange>,
    pub parents: Vec<(String, TextRange)>,
    pub slots: Vec<SlotEntry>,
}
