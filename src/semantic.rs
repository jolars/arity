//! Single-file semantic analysis: scope tree, bindings, identifier resolution,
//! and in-file `library()` tracking.
//!
//! Built in one bottom-up CST walk by [`builder::build`]. The output is a
//! [`SemanticModel`] that lint rules and other consumers read; no caching is
//! done internally — the [`crate::incremental`] salsa layer handles that.

pub mod binding;
pub mod builder;
pub mod scope;
pub mod symbols;

use rowan::TextRange;
use smol_str::SmolStr;

pub use binding::{Binding, BindingId, BindingKind};
pub use scope::{Scope, ScopeId, ScopeKind};
pub use symbols::{LoadedPackage, PackageOrigin, StaticBaseR, SymbolProvider};

use crate::syntax::SyntaxNode;

/// A reference to an identifier read site, paired with its enclosing scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentRef {
    pub name: SmolStr,
    pub range: TextRange,
    pub scope: ScopeId,
}

/// Per-file semantic information derived from the CST.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SemanticModel {
    scopes: Vec<Scope>,
    bindings: Vec<Binding>,
    /// Identifier *read* sites. Definition sites are recorded as `Binding`s.
    idents: Vec<IdentRef>,
    loaded_packages: Vec<LoadedPackage>,
    /// Packages named on the left of `::` / `:::`. Unlike `loaded_packages`,
    /// these are *not* attached to the search path — `pkg::name` is a direct
    /// reference — so they never affect bare-name resolution. They drive
    /// which packages the introspection index should harvest.
    referenced_packages: Vec<SmolStr>,
}

impl SemanticModel {
    /// Build a fresh model from a parsed file root.
    pub fn build(root: &SyntaxNode) -> Self {
        builder::build(root)
    }

    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }

    pub fn scope(&self, id: ScopeId) -> &Scope {
        &self.scopes[id.0 as usize]
    }

    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    pub fn binding(&self, id: BindingId) -> &Binding {
        &self.bindings[id.0 as usize]
    }

    pub fn idents(&self) -> &[IdentRef] {
        &self.idents
    }

    pub fn loaded_packages(&self) -> &[LoadedPackage] {
        &self.loaded_packages
    }

    /// Packages referenced via `pkg::name` / `pkg:::name`, in source order
    /// (with duplicates preserved as encountered).
    pub fn referenced_packages(&self) -> &[SmolStr] {
        &self.referenced_packages
    }

    /// Resolve a single identifier read against the scope tree. Walks
    /// outward from `ident.scope` looking for a matching binding. Returns
    /// `None` if no binding is found within any enclosing scope.
    pub fn resolve_local(&self, ident: &IdentRef) -> Option<BindingId> {
        let mut current = Some(ident.scope);
        while let Some(scope_id) = current {
            for binding in &self.scope(scope_id).bindings {
                if self.binding(*binding).name == ident.name {
                    return Some(*binding);
                }
            }
            current = self.scope(scope_id).parent;
        }
        None
    }

    /// Bindings that were defined but never read in the same file.
    /// Excludes parameters and `for`-loop variables (those have semantic
    /// meaning even when unused) and names starting with `.` (R convention).
    pub fn unused_local_bindings(&self) -> impl Iterator<Item = BindingId> + '_ {
        (0..self.bindings.len())
            .map(BindingId::from_index)
            .filter(move |id| {
                let binding = self.binding(*id);
                matches!(binding.kind, BindingKind::Local | BindingKind::Implicit)
                    && !binding.read
                    && !binding.name.starts_with('.')
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn model_of(src: &str) -> SemanticModel {
        let parsed = parse(src);
        SemanticModel::build(&parsed.cst)
    }

    fn binding_names(model: &SemanticModel) -> Vec<&str> {
        model.bindings.iter().map(|b| b.name.as_str()).collect()
    }

    #[test]
    fn top_level_assignment_creates_binding() {
        let m = model_of("x <- 1");
        assert_eq!(binding_names(&m), vec!["x"]);
        assert_eq!(m.bindings[0].kind, BindingKind::Local);
    }

    #[test]
    fn function_params_create_bindings() {
        let m = model_of("f <- function(a, b = 2) a + b");
        let names = binding_names(&m);
        assert!(names.contains(&"f"));
        assert!(names.contains(&"a"));
        assert!(names.contains(&"b"));
        let f_binding = m.bindings.iter().find(|b| b.name == "f").unwrap();
        assert_eq!(f_binding.kind, BindingKind::Local);
        let a_binding = m.bindings.iter().find(|b| b.name == "a").unwrap();
        assert_eq!(a_binding.kind, BindingKind::Param);
    }

    #[test]
    fn for_loop_var_creates_binding() {
        let m = model_of("for (i in 1:10) print(i)");
        let i_binding = m.bindings.iter().find(|b| b.name == "i").unwrap();
        assert_eq!(i_binding.kind, BindingKind::ForVar);
    }

    #[test]
    fn library_call_at_top_level_tracked() {
        let m = model_of("library(dplyr)\nx <- 1");
        assert_eq!(m.loaded_packages.len(), 1);
        assert_eq!(m.loaded_packages[0].name.as_str(), "dplyr");
    }

    #[test]
    fn library_call_with_string_tracked() {
        let m = model_of("library(\"dplyr\")");
        assert_eq!(m.loaded_packages.len(), 1);
        assert_eq!(m.loaded_packages[0].name.as_str(), "dplyr");
    }

    #[test]
    fn library_call_inside_function_ignored() {
        let m = model_of("f <- function() { library(dplyr); 1 }");
        assert_eq!(m.loaded_packages.len(), 0);
    }

    #[test]
    fn library_package_name_is_not_a_read() {
        // The bare package name must not be recorded as an identifier read
        // (otherwise `undefined-symbol` flags it).
        let m = model_of("library(dplyr)");
        assert!(
            !m.idents().iter().any(|i| i.name == "dplyr"),
            "package name should be suppressed, got {:?}",
            m.idents()
        );
    }

    #[test]
    fn library_other_args_still_read() {
        // Only the package-name argument is suppressed; later args resolve as
        // normal reads.
        let m = model_of("library(dplyr, character.only = flag)");
        assert!(!m.idents().iter().any(|i| i.name == "dplyr"));
        assert!(m.idents().iter().any(|i| i.name == "flag"));
    }

    #[test]
    fn colon_reference_records_referenced_package() {
        let m = model_of("dplyr::filter(x)\nrlang:::abort(\"e\")");
        let refs: Vec<&str> = m.referenced_packages().iter().map(|s| s.as_str()).collect();
        assert!(refs.contains(&"dplyr"));
        assert!(refs.contains(&"rlang"));
        // A `::` reference does not attach the package to the search path.
        assert!(m.loaded_packages.is_empty());
    }

    #[test]
    fn read_marks_binding_used() {
        let m = model_of("x <- 1\nprint(x)");
        let x_binding = m.bindings.iter().find(|b| b.name == "x").unwrap();
        assert!(x_binding.read);
    }

    #[test]
    fn unused_binding_not_read() {
        let m = model_of("x <- 1\ny <- 2\nprint(y)");
        let unused: Vec<_> = m
            .unused_local_bindings()
            .map(|id| m.binding(id).name.as_str())
            .collect();
        assert_eq!(unused, vec!["x"]);
    }

    #[test]
    fn dotted_unused_binding_skipped() {
        let m = model_of(".x <- 1");
        let unused: Vec<_> = m.unused_local_bindings().collect();
        assert!(unused.is_empty());
    }

    #[test]
    fn shadowing_uses_inner_binding() {
        // Inner `x` is not unused because it's read; outer `x` is read by the print.
        let m = model_of("x <- 1\nf <- function() { x <- 2; x }\nprint(x)");
        let inner = m
            .bindings
            .iter()
            .filter(|b| b.name == "x")
            .find(|b| {
                b.kind == BindingKind::Local && {
                    let scope = m.scope(b.scope);
                    scope.kind == ScopeKind::Function
                }
            })
            .unwrap();
        assert!(inner.read);
    }

    #[test]
    fn rhs_self_reference_marks_binding_read() {
        // `x <- x + 1` at top-level: the LHS-defined `x` does end up read by
        // the RHS. We don't model "value depends on prior `x`"; the unused
        // binding rule only cares that *some* read site references the name.
        let m = model_of("x <- x + 1");
        let x_binding = m.bindings.iter().find(|b| b.name == "x").unwrap();
        assert!(x_binding.read);
    }

    #[test]
    fn namespace_operands_not_reads() {
        let m = model_of("dplyr::filter(x, y)");
        let names: Vec<&str> = m.idents.iter().map(|i| i.name.as_str()).collect();
        assert!(!names.contains(&"dplyr"));
        assert!(!names.contains(&"filter"));
        assert!(names.contains(&"x"));
        assert!(names.contains(&"y"));
    }

    #[test]
    fn member_access_rhs_not_read() {
        let m = model_of("obj$field");
        let names: Vec<&str> = m.idents.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"obj"));
        assert!(!names.contains(&"field"));
    }

    #[test]
    fn named_arg_name_not_read() {
        let m = model_of("f(x = 1, y)");
        let names: Vec<&str> = m.idents.iter().map(|i| i.name.as_str()).collect();
        // `x` is an arg name (not a read); `y` is a positional arg (a read);
        // `f` is the callee (a read).
        assert!(!names.contains(&"x"));
        assert!(names.contains(&"y"));
        assert!(names.contains(&"f"));
    }

    #[test]
    fn super_assign_binds_outer_scope() {
        let m = model_of("f <- function() { x <<- 1 }");
        // The `x` super-assignment creates an `Implicit` binding scoped to
        // the file (the nearest scope outside the function).
        let x_binding = m.bindings.iter().find(|b| b.name == "x").unwrap();
        assert_eq!(x_binding.kind, BindingKind::Implicit);
        let scope = m.scope(x_binding.scope);
        assert_eq!(scope.kind, ScopeKind::File);
    }
}
