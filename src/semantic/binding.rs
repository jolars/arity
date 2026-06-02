use rowan::TextRange;
use smol_str::SmolStr;

use crate::semantic::ScopeId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingId(pub(crate) u32);

impl BindingId {
    pub(crate) fn from_index(idx: usize) -> Self {
        Self(idx as u32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind {
    /// A local binding via `<-`, `=`, or `:=`.
    Local,
    /// A function parameter.
    Param,
    /// A `for`-loop variable.
    ForVar,
    /// A binding introduced by `<<-` / `->>` (super-assignment); semantically
    /// scoped to an *enclosing* scope, but tracked here for completeness.
    Implicit,
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub name: SmolStr,
    pub kind: BindingKind,
    pub scope: ScopeId,
    /// Range of the *defining* identifier (or `for`-var, or param name).
    pub def_range: TextRange,
    /// Whether any read of this binding has been observed during resolution.
    pub read: bool,
}
