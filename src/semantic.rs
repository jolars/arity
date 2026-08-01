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

use std::collections::{HashMap, HashSet};

use rowan::{TextRange, TextSize};
use smol_str::SmolStr;

pub use binding::{Binding, BindingId, BindingKind};
pub use scope::{Scope, ScopeId, ScopeKind};
pub use symbols::{
    LoadedPackage, PackageOrigin, StaticBaseR, SymbolProvider, implicit_attached_packages,
    is_data_masking_callee, meta_package_members,
};

use crate::syntax::SyntaxNode;

/// A reference to an identifier read site, paired with its enclosing scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentRef {
    pub name: SmolStr,
    pub range: TextRange,
    pub scope: ScopeId,
    /// The read sits inside the masked arguments of a data-masking call (e.g. a
    /// dplyr verb), so a bare name here may be a data-frame column rather than a
    /// binding or package export. `undefined-symbol` skips these; the read is
    /// still recorded so it can mark an enclosing binding used (see
    /// [`is_data_masking_callee`]).
    pub data_masked: bool,
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
    /// Names accessed on the right of `::` / `:::` (`pkg::name` -> `name`). Kept
    /// out of [`idents`](Self::idents) so they never resolve locally or reach
    /// `undefined-symbol`; they exist only to mark a same-package binding *used*
    /// across files (e.g. `pkg:::helper()` in a test reads `helper`).
    qualified_reads: Vec<SmolStr>,
    /// Each scope's directly-declared bindings keyed by name, in
    /// [`Scope::bindings`] order — the constant-time backend for
    /// [`resolve_local`](Self::resolve_local) and the builder's read marking
    /// (a linear scan of a scope's bindings per lookup is quadratic over a
    /// file's worth of idents). Fully derived from [`bindings`](Self::bindings),
    /// maintained by the builder alongside it.
    bindings_by_name: HashMap<(ScopeId, SmolStr), Vec<BindingId>>,
    /// Reverse def-use edges: parallel to [`bindings`](Self::bindings), each
    /// entry holds the indices into [`idents`](Self::idents) of the reads bound
    /// to that binding. The reverse of the map the builder's `resolve_reads`
    /// pass computes (`reads_reached`), materialized in that same pass — so it
    /// is frame-aware/flow-insensitive, exactly matching how the `read` flag is
    /// set. Read via [`read_sites`](Self::read_sites).
    binding_reads: Vec<Vec<u32>>,
    /// Forward def-use edges: parallel to [`idents`](Self::idents), each entry
    /// holds the binding(s) that read resolves to. A read can reach several
    /// (a conservative reassignment; see `reads_reached`), and a free/undefined
    /// read reaches none. Read via [`ident_bindings`](Self::ident_bindings).
    ident_bindings: Vec<Vec<BindingId>>,
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

    /// Whether `id` is a top-level (file-scope) binding — the gate cross-file
    /// find-references uses to decide a local binding can also be read from
    /// sibling files. Nested locals (params, `for`-vars, function-body locals)
    /// are file-private, so references for them stay intra-file.
    pub fn binding_is_file_scope(&self, id: BindingId) -> bool {
        self.scope(self.binding(id).scope).kind == ScopeKind::File
    }

    pub fn idents(&self) -> &[IdentRef] {
        &self.idents
    }

    /// The identifier read sites bound to `id`, in [`idents`](Self::idents)
    /// order. Empty when the binding is never read. The reverse def-use edge —
    /// the concrete read set behind the `read` flag.
    pub fn read_sites(&self, id: BindingId) -> impl Iterator<Item = &IdentRef> + '_ {
        self.binding_reads[id.0 as usize]
            .iter()
            .map(move |&i| &self.idents[i as usize])
    }

    /// The binding(s) the read at `ident_index` (an index into
    /// [`idents`](Self::idents), as yielded in order) resolves to. Several on a
    /// conservative reassignment; empty for a free/undefined read. The forward
    /// def-use edge.
    pub fn ident_bindings(&self, ident_index: usize) -> &[BindingId] {
        &self.ident_bindings[ident_index]
    }

    pub fn loaded_packages(&self) -> &[LoadedPackage] {
        &self.loaded_packages
    }

    /// Packages referenced via `pkg::name` / `pkg:::name`, in source order
    /// (with duplicates preserved as encountered).
    pub fn referenced_packages(&self) -> &[SmolStr] {
        &self.referenced_packages
    }

    /// Names accessed via `pkg::name` / `pkg:::name` (the right operand), in
    /// source order. Feeds cross-file *use* detection only (see the field doc).
    pub fn qualified_reads(&self) -> &[SmolStr] {
        &self.qualified_reads
    }

    /// The innermost scope whose range contains `offset`. Falls back to the
    /// file scope (id 0) when no narrower scope matches — every model has one.
    /// Drives completion's scope-visible name enumeration.
    pub fn innermost_scope_at(&self, offset: TextSize) -> ScopeId {
        let mut best: Option<(ScopeId, TextSize)> = None;
        for (idx, scope) in self.scopes.iter().enumerate() {
            if !scope.range.contains_inclusive(offset) {
                continue;
            }
            let len = scope.range.len();
            match best {
                Some((_, best_len)) if best_len <= len => {}
                _ => best = Some((ScopeId::from_index(idx), len)),
            }
        }
        best.map_or(ScopeId::from_index(0), |(id, _)| id)
    }

    /// Names visible from the scope enclosing `offset`, inner scopes shadowing
    /// outer ones. Walks outward via `parent` from [`innermost_scope_at`],
    /// collecting each scope's directly-declared bindings; the first (innermost)
    /// occurrence of a name wins.
    pub fn names_in_scope_at(&self, offset: TextSize) -> Vec<(SmolStr, BindingKind)> {
        let mut seen: HashSet<SmolStr> = HashSet::new();
        let mut out = Vec::new();
        let mut current = Some(self.innermost_scope_at(offset));
        while let Some(scope_id) = current {
            let scope = self.scope(scope_id);
            for binding in &scope.bindings {
                let b = self.binding(*binding);
                if seen.insert(b.name.clone()) {
                    out.push((b.name.clone(), b.kind));
                }
            }
            current = scope.parent;
        }
        out
    }

    /// The bindings named `name` directly declared in `scope`, in declaration
    /// ([`Scope::bindings`]) order. Empty if the scope declares no such name.
    pub(crate) fn scope_bindings_named(&self, scope: ScopeId, name: &SmolStr) -> &[BindingId] {
        self.bindings_by_name
            .get(&(scope, name.clone()))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Resolve a single identifier read against the scope tree. Walks
    /// outward from `ident.scope` looking for a matching binding. Returns
    /// `None` if no binding is found within any enclosing scope.
    pub fn resolve_local(&self, ident: &IdentRef) -> Option<BindingId> {
        let mut current = Some(ident.scope);
        while let Some(scope_id) = current {
            if let Some(first) = self.scope_bindings_named(scope_id, &ident.name).first() {
                return Some(*first);
            }
            current = self.scope(scope_id).parent;
        }
        None
    }

    /// The frame a scope belongs to: the nearest enclosing `Function`/`File`
    /// scope, collapsing `For`/`Block` scopes (which share their function's
    /// execution frame) into it. Returns `scope` itself when it is already a
    /// frame root. Distinct from the builder's `enclosing_function_or_file`,
    /// which steps *past* a function (for `<<-` semantics).
    fn frame_scope(&self, scope: ScopeId) -> ScopeId {
        let mut current = scope;
        loop {
            match self.scope(current).kind {
                ScopeKind::File | ScopeKind::Function => return current,
                _ => match self.scope(current).parent {
                    Some(parent) => current = parent,
                    None => return current,
                },
            }
        }
    }

    /// Every binding naming the same variable as `id`: same name, same enclosing
    /// function/file frame (`for`/block scopes collapse into their frame). In R a
    /// name is one mutable variable per frame, so these are its reassignments;
    /// rename/references treat them as a unit. Includes `id`. Returned in
    /// [`bindings`](Self::bindings) order.
    pub fn variable_cohort(&self, id: BindingId) -> Vec<BindingId> {
        let binding = self.binding(id);
        let frame = self.frame_scope(binding.scope);
        (0..self.bindings.len())
            .map(BindingId::from_index)
            .filter(|other| {
                let b = self.binding(*other);
                b.name == binding.name && self.frame_scope(b.scope) == frame
            })
            .collect()
    }

    /// Bindings that were defined but never read in the same file.
    /// Excludes parameters and `for`-loop variables (those have semantic
    /// meaning even when unused) and names starting with `.` (R convention).
    ///
    /// Also excludes `Implicit` (super-assignment, `<<-`) targets: a `<<-` is a
    /// stateful write to an *enclosing* (or global) binding, not a fresh local,
    /// so its non-use here does not mean dead code — the read may live in the
    /// outer scope or a later invocation of a closure.
    pub fn unused_local_bindings(&self) -> impl Iterator<Item = BindingId> + '_ {
        (0..self.bindings.len())
            .map(BindingId::from_index)
            .filter(move |id| {
                let binding = self.binding(*id);
                matches!(binding.kind, BindingKind::Local)
                    && self.read_sites(*id).next().is_none()
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

    fn ident_named<'a>(model: &'a SemanticModel, name: &str) -> &'a IdentRef {
        model
            .idents
            .iter()
            .find(|ident| ident.name == name)
            .unwrap_or_else(|| panic!("no ident read of `{name}`"))
    }

    #[test]
    fn resolve_local_returns_the_first_same_scope_binding() {
        // Two defs of `x` in the same (file) scope: resolution pins to the
        // first-recorded binding, not the nearest.
        let m = model_of("x <- 1\nx <- 2\ny <- x\n");
        let resolved = m.resolve_local(ident_named(&m, "x")).expect("x resolves");
        let first_x = m
            .bindings
            .iter()
            .position(|b| b.name == "x")
            .map(BindingId::from_index)
            .unwrap();
        assert_eq!(resolved, first_x);
    }

    #[test]
    fn resolve_local_prefers_the_innermost_scope() {
        // The body read of `x` resolves to the parameter, not the file-scope
        // def; the top-level read resolves to the file-scope def.
        let m = model_of("x <- 1\nf <- function(x) x + 1\ng <- x\n");
        let param = m.bindings.iter().position(|b| b.kind == BindingKind::Param);
        let param = BindingId::from_index(param.unwrap());
        let body_read = m
            .idents
            .iter()
            .find(|ident| ident.name == "x" && m.scope(ident.scope).kind != ScopeKind::File)
            .expect("a body read of x");
        assert_eq!(m.resolve_local(body_read), Some(param));

        let top_read = m
            .idents
            .iter()
            .find(|ident| ident.name == "x" && m.scope(ident.scope).kind == ScopeKind::File)
            .expect("a top-level read of x");
        let file_x = m
            .bindings
            .iter()
            .position(|b| b.name == "x" && m.scope(b.scope).kind == ScopeKind::File)
            .map(BindingId::from_index)
            .unwrap();
        assert_eq!(m.resolve_local(top_read), Some(file_x));
    }

    #[test]
    fn resolve_local_walks_out_to_enclosing_scopes_or_none() {
        // `y` in the body resolves outward to the file scope; `zz` resolves
        // nowhere.
        let m = model_of("y <- 1\nf <- function() y + zz\n");
        let y_read = ident_named(&m, "y");
        let file_y = m
            .bindings
            .iter()
            .position(|b| b.name == "y")
            .map(BindingId::from_index)
            .unwrap();
        assert_eq!(m.resolve_local(y_read), Some(file_y));
        assert_eq!(m.resolve_local(ident_named(&m, "zz")), None);
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
    fn colon_reference_records_qualified_read_name() {
        // The accessed name (right of `::`/`:::`) is captured as a qualified read
        // for cross-file use detection, but stays out of `idents` so it never
        // resolves locally or reaches `undefined-symbol`.
        let m = model_of("dplyr::filter(x)\nrlang:::abort");
        let qr: Vec<&str> = m.qualified_reads().iter().map(|s| s.as_str()).collect();
        assert!(qr.contains(&"filter"), "qualified_reads: {qr:?}");
        assert!(qr.contains(&"abort"), "qualified_reads: {qr:?}");
        assert!(!m.idents().iter().any(|i| i.name == "filter"));
        assert!(!m.idents().iter().any(|i| i.name == "abort"));
        // The call argument still resolves as a normal read.
        assert!(m.idents().iter().any(|i| i.name == "x"));
    }

    #[test]
    fn read_marks_binding_used() {
        let m = model_of("x <- 1\nprint(x)");
        let x_binding = m.bindings.iter().find(|b| b.name == "x").unwrap();
        assert!(x_binding.read);
    }

    fn binding_id_named(model: &SemanticModel, name: &str) -> BindingId {
        model
            .bindings
            .iter()
            .position(|b| b.name == name)
            .map(BindingId::from_index)
            .unwrap_or_else(|| panic!("no binding named `{name}`"))
    }

    fn ident_index(model: &SemanticModel, name: &str) -> usize {
        model
            .idents
            .iter()
            .position(|i| i.name == name)
            .unwrap_or_else(|| panic!("no ident read of `{name}`"))
    }

    #[test]
    fn read_sites_lists_a_bindings_reads() {
        // Both reads of `x` (in `print(x)` and `y <- x`) are its read sites.
        let m = model_of("x <- 1\nprint(x)\ny <- x\n");
        let x = binding_id_named(&m, "x");
        let ranges: Vec<TextRange> = m.read_sites(x).map(|i| i.range).collect();
        let expected: Vec<TextRange> = m
            .idents
            .iter()
            .filter(|i| i.name == "x")
            .map(|i| i.range)
            .collect();
        assert_eq!(expected.len(), 2, "sanity: two reads of x");
        assert_eq!(ranges, expected);
    }

    #[test]
    fn read_sites_empty_for_unread_binding() {
        // `x` is never read; `y` is. `read_sites` must agree with the `read` flag.
        let m = model_of("x <- 1\ny <- 2\nprint(y)\n");
        let x = binding_id_named(&m, "x");
        let y = binding_id_named(&m, "y");
        assert!(m.read_sites(x).next().is_none());
        assert!(!m.binding(x).read);
        assert!(m.read_sites(y).next().is_some());
        assert!(m.binding(y).read);
    }

    #[test]
    fn ident_bindings_resolves_a_read_to_its_binding() {
        let m = model_of("x <- 1\ny <- x\n");
        let x = binding_id_named(&m, "x");
        let idx = ident_index(&m, "x");
        assert_eq!(m.ident_bindings(idx), &[x]);
    }

    #[test]
    fn ident_bindings_empty_for_free_read() {
        let m = model_of("f(zz)\n");
        let idx = ident_index(&m, "zz");
        assert!(m.ident_bindings(idx).is_empty());
    }

    #[test]
    fn reassignment_read_binds_conservatively() {
        // Mirrors the `reads_reached` doc example: within the frame a read marks
        // *every* preceding same-name binding. The second `f(x)` reaches both
        // `x` bindings; both bindings' read sets include it.
        let m = model_of("x <- 1\nf(x)\nx <- 2\nf(x)\n");
        let x_defs: Vec<BindingId> = m
            .bindings
            .iter()
            .enumerate()
            .filter(|(_, b)| b.name == "x")
            .map(|(i, _)| BindingId::from_index(i))
            .collect();
        assert_eq!(x_defs.len(), 2, "sanity: two defs of x");
        let x_reads: Vec<usize> = m
            .idents
            .iter()
            .enumerate()
            .filter(|(_, i)| i.name == "x")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(x_reads.len(), 2, "sanity: two reads of x");
        let (first_read, second_read) = (x_reads[0], x_reads[1]);
        // First read sees only the first def; second read sees both.
        assert_eq!(m.ident_bindings(first_read), &[x_defs[0]]);
        assert_eq!(m.ident_bindings(second_read), &[x_defs[0], x_defs[1]]);
        // Reverse edges: the second read appears in both bindings' read sets.
        for def in &x_defs {
            let reads: Vec<usize> = m
                .read_sites(*def)
                .map(|site| ident_index_of(&m, site.range))
                .collect();
            assert!(
                reads.contains(&second_read),
                "the second read binds to both x defs"
            );
        }
    }

    fn ident_index_of(model: &SemanticModel, range: TextRange) -> usize {
        model
            .idents
            .iter()
            .position(|i| i.range == range)
            .expect("ident with range")
    }

    /// The cohort of `id` as a set of `bindings()` indices, for order-independent
    /// comparison.
    fn cohort_indices(model: &SemanticModel, id: BindingId) -> std::collections::BTreeSet<usize> {
        model
            .variable_cohort(id)
            .into_iter()
            .map(|b| b.0 as usize)
            .collect()
    }

    #[test]
    fn variable_cohort_groups_frame_reassignments() {
        // Two file-scope defs of `x` are one variable; from either member the
        // cohort is both. `y` is its own cohort.
        let m = model_of("x <- 1\nf(x)\nx <- 2\ng(x)\ny <- 3\n");
        let x_defs: Vec<usize> = m
            .bindings
            .iter()
            .enumerate()
            .filter(|(_, b)| b.name == "x")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(x_defs.len(), 2, "sanity: two defs of x");
        let both: std::collections::BTreeSet<usize> = x_defs.iter().copied().collect();
        assert_eq!(cohort_indices(&m, BindingId::from_index(x_defs[0])), both);
        assert_eq!(cohort_indices(&m, BindingId::from_index(x_defs[1])), both);
        let y = binding_id_named(&m, "y");
        assert_eq!(m.variable_cohort(y), vec![y]);
    }

    #[test]
    fn variable_cohort_excludes_a_shadowing_inner_param() {
        // The file-scope `x` and a nested function's parameter `x` are distinct
        // variables (different frames), so neither cohort includes the other.
        let m = model_of("x <- 1\nf <- function(x) x + 1\n");
        let file_x = m
            .bindings
            .iter()
            .position(|b| b.name == "x" && m.scope(b.scope).kind == ScopeKind::File)
            .map(BindingId::from_index)
            .expect("file-scope x");
        let param_x = m
            .bindings
            .iter()
            .position(|b| b.name == "x" && b.kind == BindingKind::Param)
            .map(BindingId::from_index)
            .expect("param x");
        assert_eq!(m.variable_cohort(file_x), vec![file_x]);
        assert_eq!(m.variable_cohort(param_x), vec![param_x]);
    }

    #[test]
    fn variable_cohort_groups_a_for_var_with_the_frame() {
        // A `for` loop variable shares its enclosing frame, so a same-name
        // reassignment outside the loop is the same variable.
        let m = model_of("for (i in 1:3) print(i)\ni <- 0\n");
        let i_defs: Vec<usize> = m
            .bindings
            .iter()
            .enumerate()
            .filter(|(_, b)| b.name == "i")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(i_defs.len(), 2, "sanity: for-var plus reassignment");
        let both: std::collections::BTreeSet<usize> = i_defs.iter().copied().collect();
        assert_eq!(cohort_indices(&m, BindingId::from_index(i_defs[0])), both);
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
    fn data_masking_call_args_marked_masked() {
        // Bare names in a data-masking verb's argument expressions are columns
        // in the data mask, recorded as reads but flagged `data_masked` so
        // `undefined-symbol` won't treat them as undefined.
        let m = model_of("mutate(df, b = a + 1)");
        let a = m.idents().iter().find(|i| i.name == "a").unwrap();
        assert!(a.data_masked, "column read `a` should be data-masked");
        let df = m.idents().iter().find(|i| i.name == "df").unwrap();
        assert!(df.data_masked, "data argument `df` should be data-masked");
        // The callee itself is not masked: a typo'd verb name stays flaggable.
        let mutate = m.idents().iter().find(|i| i.name == "mutate").unwrap();
        assert!(!mutate.data_masked, "callee should not be data-masked");
    }

    #[test]
    fn data_masking_propagates_to_qualified_call() {
        // `pkg::mutate(...)` masks its arguments just like the bare form.
        let m = model_of("dplyr::mutate(df, b = a + 1)");
        let a = m.idents().iter().find(|i| i.name == "a").unwrap();
        assert!(a.data_masked, "column read `a` should be data-masked");
    }

    #[test]
    fn non_masking_call_args_not_masked() {
        let m = model_of("paste(a, b)");
        assert!(m.idents().iter().all(|i| !i.data_masked));
    }

    #[test]
    fn custom_infix_operands_masked() {
        // A user-defined `%...%` operator is opaque (it may capture its operands
        // symbolically, e.g. caugi's `A %---% B`), so its operands are recorded
        // as reads but flagged `data_masked` so `undefined-symbol` stays silent.
        let m = model_of("A %---% B");
        let a = m.idents().iter().find(|i| i.name == "A").unwrap();
        assert!(
            a.data_masked,
            "lhs operand of `%---%` should be data-masked"
        );
        let b = m.idents().iter().find(|i| i.name == "B").unwrap();
        assert!(
            b.data_masked,
            "rhs operand of `%---%` should be data-masked"
        );
    }

    #[test]
    fn builtin_infix_operands_not_masked() {
        // Base special operators evaluate both operands normally, so their
        // operands stay flaggable.
        for src in [
            "x %in% y", "x %% y", "x %*% y", "x %o% y", "x %/% y", "x %>% y",
        ] {
            let m = model_of(src);
            assert!(
                m.idents().iter().all(|i| !i.data_masked),
                "no operand of `{src}` should be data-masked",
            );
        }
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
    fn reserved_constants_are_not_reads() {
        // `TRUE`/`FALSE`/`NA`/`NULL`/`Inf`/`NaN`/`NA_*` are reserved literals,
        // not symbol references — they must not be recorded as reads (else
        // `undefined-symbol` flags them). `T`/`F` are rebindable base bindings,
        // so they remain reads.
        let m = model_of("print(c(TRUE, FALSE, NA, NULL, Inf, NaN, NA_integer_, T))");
        let names: Vec<&str> = m.idents.iter().map(|i| i.name.as_str()).collect();
        for constant in ["TRUE", "FALSE", "NA", "NULL", "Inf", "NaN", "NA_integer_"] {
            assert!(
                !names.contains(&constant),
                "{constant} should not be a read"
            );
        }
        assert!(
            names.contains(&"T"),
            "T is a rebindable binding, still a read"
        );
        assert!(names.contains(&"print"));
    }

    #[test]
    fn names_in_scope_at_respects_function_scope() {
        // `a` (param of f) is visible inside f's body but not inside g; `b`
        // (param of g) is visible inside g but not f. Both functions and the
        // file-scope `f`/`g` are visible from within.
        let src = "f <- function(a) {\n  a\n}\ng <- function(b) {\n  b\n}\n";
        let m = model_of(src);
        let names_at = |needle: &str| -> Vec<String> {
            let offset = TextSize::new(src.find(needle).unwrap() as u32);
            m.names_in_scope_at(offset)
                .into_iter()
                .map(|(n, _)| n.to_string())
                .collect()
        };
        let in_f = names_at("  a");
        assert!(in_f.contains(&"a".to_string()), "f sees param a: {in_f:?}");
        assert!(!in_f.contains(&"b".to_string()), "f hides g's b: {in_f:?}");
        assert!(in_f.contains(&"f".to_string()) && in_f.contains(&"g".to_string()));
        let in_g = names_at("  b");
        assert!(in_g.contains(&"b".to_string()), "g sees param b: {in_g:?}");
        assert!(!in_g.contains(&"a".to_string()), "g hides f's a: {in_g:?}");
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
