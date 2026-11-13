//! Formal class metadata and structural slot schemas. Coercion and validity
//! functions are recorded as declarations, never evaluated to discover structure.

use super::*;

mod extract;
mod slots;
pub(super) use extract::{class_shape, declaration_count, formals, project};
pub(super) use slots::slot_call;

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
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S4Facts {
    pub relations: Vec<S4Relation>,
    pub coercions: Vec<S4Coercion>,
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
    pub complete: bool,
}

impl Default for S4Slots {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            complete: true,
        }
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
