//! Single-file semantic analysis: scope tree, bindings, identifier resolution,
//! and in-file `library()` tracking.
//!
//! Built in one bottom-up CST walk by [`builder::build`]. The output is a
//! [`SemanticModel`] that lint rules and other consumers read; no caching is
//! done internally — the [`crate::incremental`] salsa layer handles that.

pub mod binding;
pub mod builder;
pub mod cfg;
pub mod scope;
pub mod symbols;

use std::collections::{HashMap, HashSet};

use rowan::{TextRange, TextSize};
use smol_str::SmolStr;

pub use binding::{Binding, BindingId, BindingKind};
pub use cfg::{BasicBlock, BlockId, ControlFlowGraph, FileControlFlow, Terminator};
pub use scope::{Scope, ScopeId, ScopeKind};
pub use symbols::{
    LoadedPackage, PackageOrigin, StaticBaseR, SymbolProvider, implicit_attached_packages,
    is_data_masking_callee, is_data_table_arg_name, is_data_table_constructor,
    is_data_table_pronoun, is_model_frame_arg, is_model_frame_arg_prefix, is_model_frame_callee,
    match_args_to_formals, meta_package_members, model_frame_formals,
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
    /// The read is evaluated lazily (an R promise), so intra-frame textual
    /// ordering does not constrain which binding it resolves to. Set for reads in
    /// a parameter default (`function(x, u = hmax)`), an `on.exit(...)` handler,
    /// and the synthesized formal reads of a `NextMethod()` call — each of which
    /// runs after body statements may have assigned a same-name local. Resolution
    /// treats such a read like a closure read: a same-frame binding assigned
    /// *after* it still counts (see the builder's `reads_reached`).
    pub deferred: bool,
}

/// Per-file semantic information derived from the CST.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SemanticModel {
    scopes: Vec<Scope>,
    bindings: Vec<Binding>,
    /// Identifier *read* sites. Definition sites are recorded as `Binding`s.
    idents: Vec<IdentRef>,
    loaded_packages: Vec<LoadedPackage>,
    /// Packages this file *names*: the left of every `::` / `:::`, and the
    /// package argument of every
    /// [`PACKAGE_LOAD_CALLS`](symbols::PACKAGE_LOAD_CALLS) call **at any
    /// depth**.
    ///
    /// Deliberately wider than `loaded_packages`, which is top-level-only
    /// because it models *attachment*: a `requireNamespace()` inside a function
    /// body is the conditional-dependency idiom, and it references a package
    /// without attaching it. Naming a package never affects bare-name
    /// resolution, so this set only drives which packages the introspection
    /// index should harvest and which ones a package's code can be said to
    /// reach.
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
    /// Whether the file calls `attach()` or `load()` — scope introducers whose
    /// bindings arity can't enumerate statically (`attach`'s data-frame columns
    /// go on the search path; `load` restores arbitrary names from an `.rda`).
    /// `undefined-symbol` gates the whole file when set, since any otherwise-
    /// unresolved bare name might be one of those opaquely-introduced bindings.
    attaches_opaque_env: bool,
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

    /// Whether the file calls `attach()` or `load()`, opaquely introducing
    /// bindings arity can't enumerate. `undefined-symbol` gates the whole file
    /// when this is set (see the field doc).
    pub fn attaches_opaque_env(&self) -> bool {
        self.attaches_opaque_env
    }

    /// Packages named via `pkg::name` / `pkg:::name` or by a `library` /
    /// `require` / `requireNamespace` / `loadNamespace` call at any depth, in
    /// source order (with duplicates preserved as encountered). See the field
    /// doc for why this is wider than [`loaded_packages`](Self::loaded_packages).
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

    #[test]
    fn for_body_binding_read_after_loop_is_used() {
        // A `for`-body assignment leaks into the enclosing frame (R has no
        // loop scope), so a read after the loop marks it used.
        let m = model_of("for (i in xs) last <- i\nprint(last)\n");
        let last = m.bindings.iter().find(|b| b.name == "last").unwrap();
        assert!(last.read, "`last` assigned in the loop is read afterward");
        // And the trailing read resolves to a binding, not a free/undefined read.
        let idx = ident_index(&m, "last");
        assert!(!m.ident_bindings(idx).is_empty());
    }

    #[test]
    fn loop_carried_read_before_reassignment_is_used() {
        // `prev` is read before it is (re)assigned in the same loop body: on the
        // next iteration the assignment precedes the read, so it is used.
        let m = model_of("for (i in xs) {\n  print(prev)\n  prev <- i\n}\n");
        let prev = m.bindings.iter().find(|b| b.name == "prev").unwrap();
        assert!(prev.read, "loop-carried read marks the reassignment used");
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

    /// A closure body's read carries no ordering relative to the enclosing
    /// frame's assignments: the closure runs when it is *called*, so any of the
    /// same-name bindings there may be the one it sees. Marking only the first
    /// made every later reassignment look unread.
    #[test]
    fn closure_read_marks_every_enclosing_reassignment() {
        let m = model_of("fit <- 1\nprint(fit)\n\nfit <- 2\nh <- function() print(fit)\nh()\n");
        assert!(
            !unused_names(&m).contains("fit"),
            "`fit <- 2` is what `h()` reads; it is not unused"
        );
    }

    #[test]
    fn dotted_unused_binding_skipped() {
        let m = model_of(".x <- 1");
        let unused: Vec<_> = m.unused_local_bindings().collect();
        assert!(unused.is_empty());
    }

    /// The names `unused_local_bindings()` reports, as a set.
    fn unused_names(model: &SemanticModel) -> std::collections::BTreeSet<String> {
        model
            .unused_local_bindings()
            .map(|id| model.binding(id).name.to_string())
            .collect()
    }

    #[test]
    fn default_arg_read_marks_body_local_used() {
        // A parameter default is a promise evaluated in the function's own frame,
        // so `upper = hmax` reads the body-local `hmax` assigned *after* it. The
        // read is order-free within the frame; `hmax` is not unused.
        let m = model_of("f <- function(x, upper = hmax) { hmax <- sqrt(x); upper }\n");
        assert!(
            !unused_names(&m).contains("hmax"),
            "default-arg read marks `hmax` used: {:?}",
            unused_names(&m)
        );
    }

    #[test]
    fn default_arg_read_marks_body_local_closure_used() {
        // `panel = panel.lda` reads a body-local closure defined later.
        let m = model_of(
            "f <- function(panel = panel.lda) {\n  panel.lda <- function() 1\n  panel()\n}\n",
        );
        assert!(
            !unused_names(&m).contains("panel.lda"),
            "default-arg read marks the body-local closure used: {:?}",
            unused_names(&m)
        );
    }

    #[test]
    fn on_exit_read_marks_later_local_used() {
        // `on.exit(par(oldpar))` is a promise evaluated at function exit, so it
        // reads `oldpar` assigned on the next line. `oldpar` is not unused.
        let m = model_of(
            "f <- function() {\n  on.exit(par(oldpar))\n  oldpar <- par(pty = \"s\")\n  plot(1)\n}\n",
        );
        assert!(
            !unused_names(&m).contains("oldpar"),
            "on.exit read marks `oldpar` used: {:?}",
            unused_names(&m)
        );
    }

    #[test]
    fn next_method_marks_reassigned_formal_used() {
        // `NextMethod()` passes the current frame values of the formals to the
        // next method, so the reassigned formal `x <- M` is used. Neither `x`
        // nor `M` is unused.
        let m = model_of(
            "print.foo <- function(x, ...) {\n  M <- cbind(x)\n  x <- M\n  NextMethod(\"print\")\n}\n",
        );
        let unused = unused_names(&m);
        assert!(
            !unused.contains("x") && !unused.contains("M"),
            "NextMethod marks the reassigned formal used: {unused:?}"
        );
    }

    #[test]
    fn expression_body_assignment_is_not_a_binding() {
        // Assignments inside `expression({ ... })` are captured unevaluated, so
        // the inner `n <-` is not an analyzable local binding at all.
        let m = model_of("f <- function() {\n  e <- expression({ n <- rep(1, nobs) })\n  e\n}\n");
        assert!(
            !unused_names(&m).contains("n"),
            "quoted assignment is not a local binding: {:?}",
            unused_names(&m)
        );
    }

    #[test]
    fn genuine_unused_still_flagged_beside_a_deferred_read() {
        // The deferral/quote suppression is scoped, not global: a genuinely
        // unused local in a function that *also* has a param default is still
        // flagged.
        let m = model_of(
            "f <- function(x, upper = hmax) {\n  hmax <- sqrt(x)\n  dead <- 1\n  upper\n}\n",
        );
        assert!(
            unused_names(&m).contains("dead"),
            "a genuinely unused local is still flagged: {:?}",
            unused_names(&m)
        );
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
    fn data_table_by_argument_masks_subset_args() {
        // data.table's masking is `[`-shaped, not a call: `dt[i, j, by]`
        // evaluates every slot in the frame's column mask. A `by =` argument is
        // data.table-only syntax, so it identifies the shape unambiguously.
        let m = model_of("dt[grp == 1, .(m = mean(val)), by = key]");
        for name in ["grp", "val", "key"] {
            let i = ident_named(&m, name);
            assert!(i.data_masked, "column read `{name}` should be data-masked");
        }
        // The table itself is a genuine read: a typo'd `dt` stays flaggable.
        assert!(!ident_named(&m, "dt").data_masked, "base is not masked");
    }

    #[test]
    fn data_table_walrus_masks_subset_args() {
        // `:=` only exists inside data.table's `[`, so it identifies the shape.
        let m = model_of("dt[, newcol := old * 2]");
        assert!(ident_named(&m, "old").data_masked);
    }

    #[test]
    fn data_table_walrus_target_is_a_column_not_a_binding() {
        // `dt[, newcol := 1]` adds a *column*; it binds nothing in the frame, so
        // recording a binding would make `newcol` a false unused-binding.
        let m = model_of("dt[, newcol := 1]");
        assert!(
            !m.bindings.iter().any(|b| b.name == "newcol"),
            "`:=` target must not become a binding"
        );
        assert!(
            ident_named(&m, "newcol").data_masked,
            "`:=` target is a masked column read"
        );
    }

    #[test]
    fn data_table_pronoun_masks_subset_args() {
        // `.N`/`.SD`/… are data.table's pronouns, bound only inside its `[`.
        let m = model_of("dt[, .N]");
        assert!(ident_named(&m, ".N").data_masked);
        let m = model_of("dt[, lapply(.SD, sum)]");
        assert!(ident_named(&m, ".SD").data_masked);
    }

    #[test]
    fn data_table_constructor_binding_masks_bare_subset() {
        // The marker-free filter idiom `dt[x > 3]` is shaped exactly like plain
        // vector indexing, so it masks only when the base is known to hold a
        // data.table.
        let m = model_of("dt <- data.table(a = 1)\ndt[col > 3]");
        assert!(ident_named(&m, "col").data_masked);
    }

    #[test]
    fn set_dt_marks_base_as_data_table() {
        // `setDT(df)` converts in place, so `df` is a data.table afterwards.
        let m = model_of("setDT(df)\ndf[col > 3]");
        assert!(ident_named(&m, "col").data_masked);
    }

    #[test]
    fn data_table_identity_propagates_through_subsets() {
        // `dt <- data.table(…)[, x := y][]` is the common build-then-modify
        // idiom: a subscript of a table is still a table, so the name it lands
        // in must be recognized too — and so must a name assigned from *that*.
        let src = "en <- data.table(f = 1)[, ef := rm(f)][]\n\
                   link <- en[, cap(ef), by = ef]\n\
                   link[first != second]\n";
        let m = model_of(src);
        for name in ["first", "second"] {
            assert!(
                ident_named(&m, name).data_masked,
                "`{name}` is a column of a derived table"
            );
        }
    }

    #[test]
    fn chained_data_table_subset_stays_masked() {
        // `dt[...][...]`: the second `[`'s base is the first subset, which is
        // itself a data.table expression.
        let m = model_of("dt[, .N, by = g][order(cnt)]");
        assert!(ident_named(&m, "cnt").data_masked);
    }

    #[test]
    fn data_table_method_called_directly_masks_args() {
        // Calling the `[` method by name bypasses the `SUBSET_EXPR` path, but
        // the arguments are still data.table's `i`/`j`/`by` slots.
        let m = model_of("data.table:::`[.data.table`(dt, , 1, by = grp)");
        assert!(ident_named(&m, "grp").data_masked);
    }

    #[test]
    fn plain_subset_args_not_masked() {
        // Ordinary indexing must stay flaggable: `v[i]` with an undefined `i` is
        // a genuine error, and it carries no data.table marker.
        let m = model_of("v[i]");
        assert!(m.idents().iter().all(|i| !i.data_masked));
        let m = model_of("m[rows, cols]");
        assert!(m.idents().iter().all(|i| !i.data_masked));
    }

    #[test]
    fn double_bracket_subset_args_not_masked() {
        // `[[` is not data.table's NSE form.
        let m = model_of("x[[i]]");
        assert!(m.idents().iter().all(|i| !i.data_masked));
    }

    #[test]
    fn locally_shadowed_masking_verb_unmasks_args() {
        // The masking table is name-only. When the file defines its own
        // `filter`, the call is *that* function — an ordinary one that evaluates
        // its arguments — so its bare names are genuine reads again.
        let m = model_of("filter <- function(x, y) x\nfilter(d, a)");
        assert!(!ident_named(&m, "a").data_masked, "shadowed verb unmasks");
        assert!(!ident_named(&m, "d").data_masked);
    }

    #[test]
    fn masking_verb_defined_after_use_stays_masked() {
        // A top-level call runs before a definition placed below it, so the call
        // really is dplyr's `filter`. Resolution is frame-ordered, so this falls
        // out for free — and errs toward suppression either way.
        let m = model_of("filter(d, a)\nfilter <- function(x, y) x");
        assert!(ident_named(&m, "a").data_masked);
    }

    #[test]
    fn qualified_masking_verb_ignores_local_shadowing() {
        // `dplyr::filter(...)` names the package's function outright; a local
        // `filter` cannot shadow it.
        let m = model_of("filter <- function(x, y) x\ndplyr::filter(d, a)");
        assert!(ident_named(&m, "a").data_masked);
    }

    #[test]
    fn nested_masking_verb_keeps_args_masked_when_outer_shadowed() {
        // Unmasking is attributed to a single enclosing verb; a read nested in a
        // second, unshadowed verb keeps its mask.
        let m = model_of("filter <- function(x, y) x\nfilter(d, mutate(d2, a))");
        assert!(
            ident_named(&m, "a").data_masked,
            "`a` is masked by the inner `mutate`"
        );
        assert!(!ident_named(&m, "d").data_masked);
    }

    #[test]
    fn shadowed_quoting_callee_keeps_args_masked() {
        // Only data-masking verbs are gated. A quoting callee doesn't evaluate
        // its argument at all, so the mask holds regardless of shadowing.
        let m = model_of("quote <- function(x) x\nquote(a)");
        assert!(ident_named(&m, "a").data_masked);
    }

    #[test]
    fn rlang_defusing_body_is_masked() {
        // rlang's `quo`/`quos`/`expr`/`exprs` defuse their body exactly as base
        // `quote` does, so a bare name inside is not a resolvable read.
        for src in [
            "quo(fn(this, that))",
            "quos(fn(this, that))",
            "expr(fn(this, that))",
            "exprs(this, that)",
        ] {
            let m = model_of(src);
            for name in ["this", "that"] {
                assert!(
                    ident_named(&m, name).data_masked,
                    "`{name}` should be masked in {src:?}"
                );
            }
        }
        // The callee itself is still an ordinary read: a typo'd `qou(...)` is a
        // genuine undefined symbol.
        let m = model_of("quo(fn(this))");
        assert!(!ident_named(&m, "quo").data_masked);
    }

    #[test]
    fn unquoted_operand_inside_defusing_is_evaluated() {
        // `!!`, `!!!`, and `{{ }}` are evaluated when the quosure is built, so
        // their operands are real reads even though the body around them is not.
        let m = model_of("quo(!!a)");
        assert!(!ident_named(&m, "a").data_masked);

        let m = model_of("expr(g(!!!b))");
        assert!(!ident_named(&m, "b").data_masked);
        assert!(ident_named(&m, "g").data_masked, "`g` is still defused");

        let m = model_of("quo(mean({{ c }}, na.rm = TRUE))");
        assert!(!ident_named(&m, "c").data_masked);
        assert!(ident_named(&m, "mean").data_masked);
    }

    #[test]
    fn unquote_escape_does_not_apply_under_base_quote() {
        // `quote()` evaluates nothing at all, so `!!x` there is plain double
        // negation of code that never runs — not an unquote.
        let m = model_of("quote(!!a)");
        assert!(ident_named(&m, "a").data_masked);

        let m = model_of("substitute({{ c }})");
        assert!(ident_named(&m, "c").data_masked);
    }

    #[test]
    fn bquote_dot_escape_is_evaluated() {
        // `bquote` unquotes with `.()`/`..()`, not `!!`.
        let m = model_of("bquote(x + .(y) + ..(z))");
        assert!(ident_named(&m, "x").data_masked);
        assert!(!ident_named(&m, "y").data_masked);
        assert!(!ident_named(&m, "z").data_masked);
        // The escape's own `.`/`..` head names no binding, so it is not a read.
        assert!(m.idents.iter().all(|i| i.name != "." && i.name != ".."));

        // `!!` is not an escape here.
        let m = model_of("bquote(!!w)");
        assert!(ident_named(&m, "w").data_masked);
    }

    #[test]
    fn embrace_requires_a_single_symbol() {
        // A nested block holding real statements is ordinary quoted code, not
        // the curly-curly operator, which rlang accepts only around a symbol.
        let m = model_of("expr({ { f(); g() } })");
        assert!(ident_named(&m, "f").data_masked);
        assert!(ident_named(&m, "g").data_masked);
    }

    #[test]
    fn qualified_quoting_callee_defuses_like_the_bare_one() {
        // `base::quote({n <- 1})` captures the assignment unevaluated, so it
        // binds nothing analyzable — same as the unqualified spelling.
        let m = model_of("base::quote({ n <- 1 })");
        assert!(
            !binding_names(&m).contains(&"n"),
            "quoted assignment is not a binding: {:?}",
            binding_names(&m)
        );

        // And the unquote escape reaches through the qualified form too.
        let m = model_of("rlang::quo(fn(!!a, that))");
        assert!(!ident_named(&m, "a").data_masked);
        assert!(ident_named(&m, "that").data_masked);
    }

    #[test]
    fn mask_carries_into_inline_function_body() {
        // A closure written inside a masked argument is *created in* the data
        // mask, so the mask is its lexical parent and a bare column name in its
        // body resolves. Verified against R:
        // `with(d, sapply(col, function(v) v + other[1]))` finds `other` in `d`.
        // The mask must therefore not stop at the closure boundary.
        let m = model_of("mutate(df, y = sapply(x, function(v) v + z))");
        assert!(ident_named(&m, "x").data_masked, "`x` is a column");
        assert!(
            ident_named(&m, "z").data_masked,
            "a closure body inherits the enclosing data mask"
        );
    }

    #[test]
    fn mask_carries_into_inline_function_inside_quote() {
        // Nothing inside `quote()` is evaluated at all.
        let m = model_of("quote(function(x) y)");
        assert!(ident_named(&m, "y").data_masked);
    }

    #[test]
    fn mask_carries_into_inline_function_inside_opaque_infix() {
        // An opaque `%op%` may capture its operands symbolically, closure
        // included.
        let m = model_of("A %---% sapply(v, function(x) y)");
        assert!(ident_named(&m, "y").data_masked);
    }

    #[test]
    fn model_frame_args_marked_masked() {
        // A model-fitting call with `data =` evaluates `weights`/`subset`/
        // `offset` in the model frame, so bare names there may be columns of
        // the data frame. Only those arguments are masked — the `data` value
        // itself and the callee stay resolvable reads.
        let m = model_of("lm(y ~ x, data = d, weights = w, subset = s > 1, offset = o)");
        for name in ["w", "s", "o"] {
            let i = m.idents().iter().find(|i| i.name == name).unwrap();
            assert!(i.data_masked, "model-frame arg `{name}` should be masked");
        }
        let d = m.idents().iter().find(|i| i.name == "d").unwrap();
        assert!(!d.data_masked, "`data` value should stay a resolvable read");
        let lm = m.idents().iter().find(|i| i.name == "lm").unwrap();
        assert!(!lm.data_masked, "callee should not be data-masked");
    }

    #[test]
    fn model_frame_args_unmasked_without_data() {
        // Without `data`, R evaluates `weights` in the calling environment, so
        // a bare name there is a genuine read and must stay flaggable.
        let m = model_of("lm(y ~ x, weights = w)");
        let w = m.idents().iter().find(|i| i.name == "w").unwrap();
        assert!(
            !w.data_masked,
            "`weights` without `data` should not be masked"
        );
    }

    #[test]
    fn model_frame_masking_propagates_to_qualified_call() {
        // `MASS::polr(...)` masks its model-frame arguments just like the bare
        // form (the `CALL_EXPR` nests under the `::`, a separate walk path).
        let m = model_of("MASS::polr(size ~ carrier, data = tonsils, weights = count)");
        let count = m.idents().iter().find(|i| i.name == "count").unwrap();
        assert!(count.data_masked, "`weights` column read should be masked");
        let tonsils = m.idents().iter().find(|i| i.name == "tonsils").unwrap();
        assert!(
            !tonsils.data_masked,
            "`data` value should stay a resolvable read"
        );
    }

    #[test]
    fn model_frame_args_masked_with_positional_data() {
        // `data` supplied positionally: `lm`'s second formal is `data`, so the
        // gate opens without a named `data =`.
        let m = model_of("lm(y ~ x, d, weights = w)");
        let w = m.idents().iter().find(|i| i.name == "w").unwrap();
        assert!(
            w.data_masked,
            "`weights` with positional `data` should mask"
        );
        let d = m.idents().iter().find(|i| i.name == "d").unwrap();
        assert!(!d.data_masked, "positional `data` stays a resolvable read");
        // `glm`'s `data` is its *third* formal, after `family`.
        let m = model_of("glm(y ~ x, poisson, d, weights = w)");
        let w = m.idents().iter().find(|i| i.name == "w").unwrap();
        assert!(w.data_masked, "`glm`'s third positional arg is `data`");
        // A positional third argument to `lm` lands on `subset` and is
        // model-frame-evaluated too.
        let m = model_of("lm(y ~ x, d, s > 1)");
        let s = m.idents().iter().find(|i| i.name == "s").unwrap();
        assert!(s.data_masked, "positional `subset` should mask");
    }

    #[test]
    fn model_frame_gate_needs_data_slot_filled() {
        // `glm`'s second formal is `family`, not `data`: two positional args
        // leave `data` unsupplied, so `weights` stays a plain read.
        let m = model_of("glm(y ~ x, d, weights = w)");
        let w = m.idents().iter().find(|i| i.name == "w").unwrap();
        assert!(!w.data_masked, "`glm`'s second positional arg is `family`");
        // An argument hole consumes `data`'s position but supplies nothing.
        let m = model_of("lm(y ~ x, , weights = w)");
        let w = m.idents().iter().find(|i| i.name == "w").unwrap();
        assert!(!w.data_masked, "a hole in the `data` slot supplies no data");
    }

    #[test]
    fn model_frame_args_partial_names() {
        // R's partial argument matching: `weight =` is a unique prefix of
        // `weights`, and `dat =` of `data`.
        let m = model_of("lm(y ~ x, data = d, weight = w)");
        let w = m.idents().iter().find(|i| i.name == "w").unwrap();
        assert!(w.data_masked, "`weight =` partial-matches `weights`");
        let m = model_of("lm(y ~ x, dat = d, weights = w)");
        let w = m.idents().iter().find(|i| i.name == "w").unwrap();
        assert!(w.data_masked, "`dat =` partial-matches `data`");
    }

    #[test]
    fn model_frame_args_forwarded_through_dots() {
        // `aov` has no `weights` formal of its own; the named argument passes
        // through `...` to `lm`, which evaluates it in the model frame.
        let m = model_of("aov(y ~ x, data = d, weights = w)");
        let w = m.idents().iter().find(|i| i.name == "w").unwrap();
        assert!(w.data_masked, "dots-forwarded `weights` should mask");
    }

    #[test]
    fn model_frame_post_dots_formal_matched_by_exact_name() {
        // `polr` declares `...` right after `start`, so `subset` sits behind
        // the dots and is only reachable by its exact name — which still masks,
        // here with `data` supplied positionally as well.
        let m = model_of("polr(size ~ carrier, tonsils, subset = s > 1)");
        let s = m.idents().iter().find(|i| i.name == "s").unwrap();
        assert!(s.data_masked, "post-dots `subset` should mask");
    }

    #[test]
    fn non_model_frame_args_not_masked() {
        // Masking is confined to the model-frame argument names: every other
        // argument of the same call is walked normally.
        let m = model_of("lm(y ~ x, data = d, method = qq)");
        let qq = m.idents().iter().find(|i| i.name == "qq").unwrap();
        assert!(!qq.data_masked, "`method` is not a model-frame argument");
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

    #[test]
    fn native_routine_head_arg_is_not_a_read() {
        // A bare IDENT in the head (first-argument) position of `.C`/`.Call`/
        // `.Fortran`/`.External` names a native routine registered by the
        // NAMESPACE, not a scope read, so it must not be recorded as an ident.
        for callee in [".C", ".Call", ".Fortran", ".External"] {
            let src = format!("f <- function(x) {callee}(VR_sammon, as.double(x))");
            let m = model_of(&src);
            assert!(
                !m.idents().iter().any(|i| i.name == "VR_sammon"),
                "{callee}: native routine head must be suppressed, got {:?}",
                m.idents()
            );
            // The remaining arguments stay ordinary reads.
            assert!(
                m.idents().iter().any(|i| i.name == "x"),
                "{callee}: later args stay reads"
            );
        }
    }

    #[test]
    fn native_routine_only_head_suppressed() {
        // Only the first argument is a routine name; a bare name elsewhere is a
        // normal read (a string head has no IDENT to suppress).
        let m = model_of(".Call(\"routine\", bogus)");
        assert!(
            m.idents().iter().any(|i| i.name == "bogus"),
            "non-head arg must stay a read, got {:?}",
            m.idents()
        );
    }

    #[test]
    fn data_loader_introduces_binding() {
        // `data(sole)` binds `sole` in the caller's frame, so a later `sole`
        // read resolves rather than dangling.
        let m = model_of("data(sole)\nsole$off <- 1\n");
        let sole = m
            .bindings
            .iter()
            .find(|b| b.name == "sole")
            .expect("data() should introduce a `sole` binding");
        assert_eq!(sole.kind, BindingKind::Implicit);
        // The later member-access read of `sole` resolves to that binding.
        let read = m
            .idents()
            .iter()
            .find(|i| i.name == "sole")
            .expect("a `sole` read");
        assert!(m.resolve_local(read).is_some(), "`sole` read must resolve");
    }

    #[test]
    fn data_loader_binds_each_bare_name() {
        // Multiple positional bare names each become a binding; a `package=`
        // string introduces nothing.
        let m = model_of("data(painters, sole, package = \"MASS\")\n");
        for name in ["painters", "sole"] {
            assert!(
                m.bindings.iter().any(|b| b.name == name),
                "expected a binding for `{name}`, got {:?}",
                m.bindings
                    .iter()
                    .map(|b| b.name.as_str())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn attach_sets_opaque_env_flag() {
        assert!(model_of("attach(painters)").attaches_opaque_env());
    }

    #[test]
    fn load_sets_opaque_env_flag() {
        assert!(model_of("load(\"x.rda\")").attaches_opaque_env());
    }

    #[test]
    fn plain_file_leaves_opaque_env_flag_unset() {
        assert!(!model_of("x <- 1\nprint(x)\n").attaches_opaque_env());
    }
}
