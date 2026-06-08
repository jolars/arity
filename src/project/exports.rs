//! Per-file export projection: the top-level binding names a file contributes
//! to a shared scope (an R package namespace, or a `source()` closure).

use std::collections::BTreeSet;

use crate::semantic::{BindingKind, ScopeKind, SemanticModel};

/// The names bound at file (top) level — the symbols another file in the same
/// package or `source()` closure can see.
///
/// Returned as a `BTreeSet` so equality is order-independent: this is the
/// firewall between per-file analysis and cross-file resolution. Editing a
/// function *body* changes the [`SemanticModel`] but leaves this set unchanged,
/// so downstream cross-file queries short-circuit.
pub fn file_exports(model: &SemanticModel) -> BTreeSet<String> {
    model
        .bindings()
        .iter()
        .filter(|binding| matches!(binding.kind, BindingKind::Local | BindingKind::Implicit))
        .filter(|binding| model.scope(binding.scope).kind == ScopeKind::File)
        .map(|binding| binding.name.to_string())
        .collect()
}

/// The names a file reads but does not bind locally — candidates for resolution
/// against another file in the same package or `source()` closure. The mirror of
/// [`file_exports`]: it drives cross-file *use* (so a binding read only in a
/// sibling file isn't flagged unused).
pub fn file_free_reads(model: &SemanticModel) -> BTreeSet<String> {
    model
        .idents()
        .iter()
        .filter(|ident| model.resolve_local(ident).is_none())
        .map(|ident| ident.name.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn exports_of(src: &str) -> BTreeSet<String> {
        file_exports(&SemanticModel::build(&parse(src).cst))
    }

    fn names(set: &BTreeSet<String>) -> Vec<&str> {
        set.iter().map(String::as_str).collect()
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

    #[test]
    fn free_reads_exclude_locally_resolved_names() {
        let model = SemanticModel::build(&parse("x <- 1\nfoo(x, y)\n").cst);
        let reads = file_free_reads(&model);
        // `foo` and `y` are free; `x` resolves to the local binding.
        assert_eq!(names(&reads), vec!["foo", "y"]);
    }
}
