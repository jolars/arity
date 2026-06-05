//! Per-file export projection: the top-level binding names a file contributes
//! to a shared scope (an R package namespace, or a `source()` closure).

use std::collections::BTreeSet;

use smol_str::SmolStr;

use crate::semantic::{BindingKind, ScopeKind, SemanticModel};

/// The names bound at file (top) level — the symbols another file in the same
/// package or `source()` closure can see.
///
/// Returned as a `BTreeSet` so equality is order-independent: this is the
/// firewall between per-file analysis and cross-file resolution. Editing a
/// function *body* changes the [`SemanticModel`] but leaves this set unchanged,
/// so downstream cross-file queries short-circuit.
pub fn file_exports(model: &SemanticModel) -> BTreeSet<SmolStr> {
    model
        .bindings()
        .iter()
        .filter(|binding| matches!(binding.kind, BindingKind::Local | BindingKind::Implicit))
        .filter(|binding| model.scope(binding.scope).kind == ScopeKind::File)
        .map(|binding| binding.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn exports_of(src: &str) -> BTreeSet<SmolStr> {
        file_exports(&SemanticModel::build(&parse(src).cst))
    }

    fn names(set: &BTreeSet<SmolStr>) -> Vec<&str> {
        set.iter().map(SmolStr::as_str).collect()
    }

    #[test]
    fn collects_top_level_assignments() {
        let e = exports_of("x <- 1\ny <- function() 2\nz = 3\n");
        assert_eq!(names(&e), vec!["x", "y", "z"]);
    }

    #[test]
    fn excludes_function_locals_and_params() {
        let e = exports_of("f <- function(a) {\n  b <- a + 1\n  b\n}\n");
        // Only `f` is top level; `a` (param) and `b` (function-local) are not.
        assert_eq!(names(&e), vec!["f"]);
    }

    #[test]
    fn includes_top_level_super_assignment() {
        let e = exports_of("g <<- 1\n");
        assert_eq!(names(&e), vec!["g"]);
    }
}
