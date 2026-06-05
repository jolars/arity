use rowan::TextRange;

use crate::semantic::BindingId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(pub(crate) u32);

impl ScopeId {
    pub(crate) fn from_index(idx: usize) -> Self {
        Self(idx as u32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    File,
    Function,
    Block,
    For,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub kind: ScopeKind,
    pub parent: Option<ScopeId>,
    pub range: TextRange,
    /// Bindings declared *directly* in this scope (not in child scopes).
    pub bindings: Vec<BindingId>,
}
