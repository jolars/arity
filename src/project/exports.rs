//! Per-file export projection: the top-level binding names a file contributes
//! to a shared scope (an R package namespace, or a `source()` closure).

use std::collections::{BTreeMap, BTreeSet};

use rowan::NodeOrToken;
use rowan::ast::AstNode as _;

use crate::ast::{AssignmentExpr, FunctionExpr};
use crate::semantic::{BindingKind, ScopeKind, SemanticModel};
use crate::syntax::SyntaxNode;

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

/// How a top-level binding was defined: a function (`f <- function(...)` or a
/// lambda) versus any other value. Stable across body edits — the classification
/// turns on the *shape* of the right-hand side, not its contents — so it keeps
/// [`file_def_sites`] a backdating firewall like [`file_exports`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, salsa::Update)]
pub enum DefKind {
    Function,
    Value,
}

/// The top-level binding names a file defines, each tagged with its [`DefKind`].
///
/// The name set mirrors [`file_exports`] exactly (file-scope `Local`/`Implicit`
/// bindings); this adds the function-vs-value tag a symbol index needs. It is
/// deliberately **range-free**: a `def_range` shifts on any earlier edit, so a
/// consumer that needs the actual span re-resolves it from the fresh
/// [`SemanticModel`] per request (see the project-wide aggregate). Keeping ranges
/// out of the map is what lets this query backdate across a body edit.
///
/// `root` is the file's parse tree, used only to inspect each binding's
/// right-hand side; the authoritative name/scope set still comes from `model`.
/// When a name is bound more than once, a function definition wins over a value.
pub fn file_def_sites(model: &SemanticModel, root: &SyntaxNode) -> BTreeMap<String, DefKind> {
    let mut defs: BTreeMap<String, DefKind> = BTreeMap::new();
    for binding in model.bindings() {
        if !matches!(binding.kind, BindingKind::Local | BindingKind::Implicit) {
            continue;
        }
        if model.scope(binding.scope).kind != ScopeKind::File {
            continue;
        }
        let name = binding.name.to_string();
        match classify_def(root, binding.def_range) {
            DefKind::Function => {
                defs.insert(name, DefKind::Function);
            }
            DefKind::Value => {
                defs.entry(name).or_insert(DefKind::Value);
            }
        }
    }
    defs
}

/// Classify the binding whose defining identifier spans `def_range`: a
/// [`DefKind::Function`] when the enclosing assignment's value is a function or
/// lambda, otherwise [`DefKind::Value`].
fn classify_def(root: &SyntaxNode, def_range: rowan::TextRange) -> DefKind {
    let element = root.covering_element(def_range);
    let start = match element {
        NodeOrToken::Node(node) => node,
        NodeOrToken::Token(token) => match token.parent() {
            Some(parent) => parent,
            None => return DefKind::Value,
        },
    };
    for ancestor in start.ancestors() {
        if let Some(assign) = AssignmentExpr::cast(ancestor) {
            return match assign.value_element() {
                Some(NodeOrToken::Node(value)) if FunctionExpr::can_cast(value.kind()) => {
                    DefKind::Function
                }
                _ => DefKind::Value,
            };
        }
    }
    DefKind::Value
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

    fn def_sites_of(src: &str) -> BTreeMap<String, DefKind> {
        let cst = parse(src).cst;
        file_def_sites(&SemanticModel::build(&cst), &cst)
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

    #[test]
    fn def_sites_tag_functions_and_values() {
        let d = def_sites_of("x <- 1\nf <- function() 2\ng <- \\(a) a\nh = function(b) b\n");
        assert_eq!(d.get("x"), Some(&DefKind::Value));
        assert_eq!(d.get("f"), Some(&DefKind::Function));
        assert_eq!(d.get("g"), Some(&DefKind::Function));
        assert_eq!(d.get("h"), Some(&DefKind::Function));
    }

    #[test]
    fn def_sites_match_export_names_and_exclude_locals() {
        let src = "f <- function(a) {\n  b <- function() a\n  b\n}\ny <<- 3\n";
        let d = def_sites_of(src);
        // Same name set as file_exports: top-level `f` and `y` only, not the
        // function-local `b` or the param `a`.
        let mut names: Vec<&str> = d.keys().map(String::as_str).collect();
        names.sort();
        assert_eq!(names, vec!["f", "y"]);
        assert_eq!(d.get("f"), Some(&DefKind::Function));
        assert_eq!(d.get("y"), Some(&DefKind::Value));
    }

    #[test]
    fn def_sites_function_wins_over_value_on_rebind() {
        // A name bound as both a value and a function classifies as Function.
        let d = def_sites_of("p <- 1\np <- function() 2\n");
        assert_eq!(d.get("p"), Some(&DefKind::Function));
        let d = def_sites_of("q <- function() 2\nq <- 1\n");
        assert_eq!(d.get("q"), Some(&DefKind::Function));
    }
}
