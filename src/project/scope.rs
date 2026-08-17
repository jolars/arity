//! Cross-file visibility: which names a file can see from the rest of its
//! project, and which of its own top-level bindings are used elsewhere.
//!
//! Two models, unified here:
//! - **Package** — files under a common package root (a directory with
//!   `DESCRIPTION` + `R/`) share one namespace: R sources them all together, so
//!   every file sees every other file's top-level bindings.
//! - **Scripts** — files relate through explicit `source()` edges. A file sees
//!   the top-level bindings of the files it (transitively) sources.
//!
//! Resolution runs in both directions:
//! - [`FileScope::resolves`] — a free read here may bind in a file we can see
//!   (so it isn't `undefined-symbol`).
//! - [`FileScope::used_elsewhere`] — a top-level binding here may be read by a
//!   file that can see us (so it isn't `unused-binding`).
//!
//! Package authoring (NAMESPACE) is folded into the same two directions:
//! `importFrom(pkg, name)` makes `name` resolve, and `export(name)` marks a
//! top-level binding as used (it's public API).
//!
//! Visibility can be *incomplete* — a `source()` target that can't be resolved
//! (dynamic argument, or a path outside the analyzed set). Then
//! [`FileScope::resolution_incomplete`] is set and callers must stay
//! conservative (no `undefined-symbol` findings).
//!
//! A wholesale `import(pkg)` is deliberately **not** expressed that way. It is
//! reported as-is by [`FileScope::wildcard_import_packages`], because "can we
//! enumerate pkg's exports" is a question for the library index and this module
//! is pure — that purity is what the project-graph memo depends on. The
//! consumer that holds the index decides, so the file stops being suppressed
//! the moment the package becomes enumerable.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use rowan::TextRange;

use crate::project::source::{SourceEdgeKey, SourceTarget, TopLevelEvent};
use crate::rindex::harvest::parse_namespace;
use crate::semantic::symbols::unbacktick;

// Neither `HashSet::new` nor `Arc::new` is `const`, so these need lazy init.
static EMPTY_PATHS: LazyLock<HashSet<PathBuf>> = LazyLock::new(HashSet::new);
static EMPTY_LAYER: LazyLock<LayeredSet> = LazyLock::new(LayeredSet::default);
static EMPTY_NAMES: LazyLock<Arc<BTreeSet<String>>> = LazyLock::new(Arc::default);

/// A name set held as one layer shared by a whole package plus a small
/// per-file delta, so a package's export union (or read union) is materialized
/// **once** instead of once per member.
///
/// Membership is one uniform predicate:
/// `contains(n) = (shared(n) || added(n)) && !removed(n)`.
///
/// That one formula stands in for two *different* pass orderings inside
/// [`ProjectScope::build`], and it is [`build`](ProjectScope::build) that makes
/// it work, by pre-adjusting `removed` differently for each set:
///
/// - `visible` — the own-export removal runs *after* the `source()` closure is
///   folded in but *before* the NAMESPACE imports and native routines, so
///   `removed` is the file's exports **minus** those two, and deliberately is
///   **not** reduced by `added`.
/// - `read_by_others` — the per-`source()`-edge contribution runs *after* the
///   package clique fold, so it overrides the fold's exclusion, and `removed`
///   **is** reduced by `added`.
///
/// The asymmetry is the encoding of those orderings, not an oversight: making
/// the two uniform flips observable behavior. `own_export_shadowing_a_sourced_export_is_not_visible`
/// and `sourcer_read_beats_the_solo_reader_exclusion` are the tests that catch it.
#[derive(Debug, Default, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct LayeredSet {
    /// One allocation per package root, shared by every member.
    shared: Arc<BTreeSet<String>>,
    /// Names this file gains over `shared`.
    added: BTreeSet<String>,
    /// Names this file loses from `shared`, pre-adjusted (see type docs).
    removed: BTreeSet<String>,
}

impl LayeredSet {
    pub fn new(
        shared: Arc<BTreeSet<String>>,
        added: BTreeSet<String>,
        removed: BTreeSet<String>,
    ) -> Self {
        Self {
            shared,
            added,
            removed,
        }
    }

    #[inline]
    pub fn contains(&self, name: &str) -> bool {
        // `shared` answers almost every query, and the common answer is "no";
        // checking it first keeps the miss to one lookup plus one.
        (self.shared.contains(name) || self.added.contains(name)) && !self.removed.contains(name)
    }

    /// Every name, deduplicated, in no guaranteed order. For tests and
    /// diagnostics — the hot paths ask [`contains`](Self::contains).
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.shared
            .iter()
            .chain(self.added.iter().filter(|n| !self.shared.contains(*n)))
            .map(String::as_str)
            .filter(|n| !self.removed.contains(*n))
    }

    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }
}

/// A set of files answered as one layer shared by a whole package plus a small
/// extra, minus the file it is answered for. Lets `P \ {f}` be a shared handle
/// and a `Path` comparison instead of an owned `HashSet<PathBuf>` per member.
///
/// `members` and `extra` are **disjoint by construction** — [`ProjectScope::build`]
/// filters same-root targets out of `extra` — so [`iter`](Self::iter) and
/// [`len`](Self::len) need no deduplication. A package member that also
/// `source()`s a sibling reaches it both ways and must still count once.
#[derive(Debug, Clone, Copy)]
pub struct PathSetView<'a> {
    members: Option<&'a HashSet<PathBuf>>,
    extra: &'a HashSet<PathBuf>,
    exclude: &'a Path,
}

impl<'a> PathSetView<'a> {
    #[inline]
    pub fn contains(&self, path: &Path) -> bool {
        path != self.exclude
            && (self.members.is_some_and(|m| m.contains(path)) || self.extra.contains(path))
    }

    pub fn len(&self) -> usize {
        self.members
            .map_or(0, |m| m.len() - usize::from(m.contains(self.exclude)))
            + self.extra.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = &'a Path> {
        let exclude = self.exclude;
        self.members
            .into_iter()
            .flatten()
            .chain(self.extra.iter())
            .map(PathBuf::as_path)
            .filter(move |p| *p != exclude)
    }
}

/// One file's contribution to cross-file resolution.
#[derive(Debug, Clone)]
pub struct FileFacts {
    pub path: PathBuf,
    /// Top-level binding names this file defines
    /// (see [`crate::project::file_exports`]).
    pub exports: BTreeSet<String>,
    /// Names this file reads but does not bind locally
    /// (see [`crate::project::exports::file_free_reads`]).
    pub free_reads: BTreeSet<String>,
    /// Names this file reads via `pkg::name` / `pkg:::name`
    /// (see [`crate::project::exports::file_qualified_reads`]). Folded into a
    /// same-package binding's "used by others" set so `pkg:::helper()` in a
    /// sibling counts as a use, but kept out of `free_reads` so it never feeds
    /// name resolution.
    pub qualified_reads: BTreeSet<String>,
    /// Top-level `source()` edges this file declares (range-free).
    pub source_edges: Vec<SourceEdgeKey>,
    /// This file's top-level execution sequence (range-free, order-bearing): the
    /// `define`/`source-edge`/`read` events used to resolve reads through
    /// load order. See [`crate::project::collect_top_level_events`].
    pub top_level_events: Vec<TopLevelEvent>,
    /// The package root this file belongs to, if any. Files sharing a root
    /// share one namespace.
    pub package_root: Option<PathBuf>,
}

/// Cross-file resolution resolved over a set of files.
#[derive(Debug, Default)]
pub struct ProjectScope {
    /// Per file: top-level names reachable from the files it can see.
    visible: HashMap<PathBuf, LayeredSet>,
    /// Per file: names *read* by some file that can see it. Reads only — the
    /// NAMESPACE-export contribution lives in `namespace_exports`, so a caller
    /// can ask "does a sibling call this?" separately from "is this public
    /// API?".
    read_by_others: HashMap<PathBuf, LayeredSet>,
    /// Per package file: the names its package's NAMESPACE `export()`s. Both
    /// sets feed [`FileScope::used_elsewhere`]; only this one marks public API.
    namespace_exports: HashMap<PathBuf, Arc<BTreeSet<String>>>,
    /// Per package file: the subset of `namespace_exports` registered as S3
    /// methods (`S3method(...)`). Reached by dispatch, never by a direct call.
    s3_methods: HashMap<PathBuf, Arc<BTreeSet<String>>>,
    /// Per file: the packages its NAMESPACE `import()`s wholesale. Recorded
    /// rather than resolved — see [`FileScope::wildcard_import_packages`].
    wildcard_imports: HashMap<PathBuf, Arc<BTreeSet<String>>>,
    /// Files whose cross-file visibility is incomplete (unresolved `source()`).
    dynamic: HashSet<PathBuf>,
    /// Per package root: its member set, held once and shared by every member.
    /// [`sees`](Self::sees) and [`package_siblings`](Self::package_siblings) are
    /// this set minus the file being asked about, so neither needs a per-member
    /// copy. Span-free, so it stays body-edit-stable.
    root_members: HashMap<PathBuf, Arc<HashSet<PathBuf>>>,
    /// Per package file: its root, so a view can reach `root_members`.
    file_root: HashMap<PathBuf, PathBuf>,
    /// Per file: the part of its transitive non-local `source()` closure that is
    /// **not** a package co-member. Directional: `a` sourcing `b` puts `b` here
    /// for `a` but not the reverse. Kept disjoint from `root_members` so a
    /// sourced sibling is not counted twice.
    sees_extra: HashMap<PathBuf, HashSet<PathBuf>>,
    /// The reverse of `sees_extra`, so [`seen_by`](Self::seen_by) is a lookup
    /// rather than a scan of every file's closure.
    seen_by_extra: HashMap<PathBuf, HashSet<PathBuf>>,
    /// Per package file: whether its package root's analyzed member set is
    /// *complete* (covers every `R/*.[RrSsQq]` source the package loads). When
    /// false, a def/read could hide in an unanalyzed sibling, so a flat-namespace
    /// rename-all over this package's cohort must refuse. Absent (→ vacuously
    /// complete) for non-package files.
    package_complete: HashMap<PathBuf, bool>,
    /// Per file: its top-level execution sequence, retained so load-order
    /// resolution ([`Self::top_level_read_binding`]) can replay it. Span-free.
    top_level_events: HashMap<PathBuf, Vec<TopLevelEvent>>,
    /// Per file: its top-level binding names, retained so a sourced closure's
    /// contribution of a name can be resolved during the order replay.
    exports: HashMap<PathBuf, BTreeSet<String>>,
    /// Per file: its range-free `source()` edges, retained so the order replay
    /// can walk a sourced file's own (transitive, non-local) closure.
    source_edges: HashMap<PathBuf, Vec<SourceEdgeKey>>,
}

/// Where a file's top-level reads of a name bind under sequential load order —
/// the result of replaying the file's [`TopLevelEvent`] sequence. Produced by
/// [`ProjectScope::top_level_read_binding`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadBinding {
    /// Every top-level read of the name binds to this one def file.
    Resolved(PathBuf),
    /// There are top-level reads, but none has a live def at its point — they
    /// bind to base R / nothing (the def isn't sourced yet).
    Unresolved,
    /// The name has no top-level read in the file: only function-body reads,
    /// which run at call time against the final scope and are not position-gated.
    NoTopLevelRead,
    /// Top-level reads disagree, or one is poisoned by a dynamic/unanalyzed
    /// source or by two files in a sourced closure defining the same name.
    OrderUnknown,
}

/// What a *single* top-level read occurrence binds to under sequential load
/// order — the per-read counterpart of [`ReadBinding`], produced with the read's
/// span by [`ProjectScope::top_level_read_provenance`]. Where `ReadBinding`
/// aggregates a file's reads into one verdict (forcing a whole-file refusal),
/// this resolves each occurrence so an order-aware rename can co-rename the reads
/// that bind to the cohort and skip the ones that don't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadSite {
    /// The read binds to this def file's top-level definition (live at its
    /// point). Co-renamed iff that file is in the rename cohort.
    Bound(PathBuf),
    /// No def is live at the read's point — it binds to base R / nothing (e.g. a
    /// read before the `source()` that injects the def). Never co-renamed.
    Unbound,
    /// The read is poisoned by a dynamic/unanalyzed source or by two closure
    /// files defining the name: its binding can't be decided, so a sound rename
    /// must refuse rather than guess.
    Unknown,
}

/// One file's view of its project.
pub struct FileScope<'a> {
    visible: &'a LayeredSet,
    read_by_others: &'a LayeredSet,
    namespace_exports: &'a Arc<BTreeSet<String>>,
    s3_methods: &'a Arc<BTreeSet<String>>,
    /// Packages this file's NAMESPACE `import()`s wholesale.
    ///
    /// Every export of such a package is in scope here, which for resolution is
    /// indistinguishable from being attached. Whether those exports are
    /// *enumerable* is a question only the library index can answer, and
    /// [`ProjectScope::build`] is pure — so the packages are reported and the
    /// verdict is left to the caller, which holds the index. Reintroducing an
    /// index lookup inside `build` would look like a simplification and would
    /// silently break the purity the project-graph memo depends on.
    wildcard_imports: &'a Arc<BTreeSet<String>>,
    /// Cross-file visibility is incomplete for a reason nothing can resolve: a
    /// dynamic or unanalyzed `source()` could supply any name, so callers must
    /// not flag them. **No longer set by `import(pkg)`** — see
    /// [`wildcard_import_packages`](Self::wildcard_import_packages).
    pub resolution_incomplete: bool,
}

impl<'a> FileScope<'a> {
    /// Construct a view directly from borrowed visibility sets. Lets the salsa
    /// [`crate::project::Visibility`] memo back a `FileScope` without going
    /// through [`ProjectScope::for_file`].
    pub fn new(
        visible: &'a LayeredSet,
        read_by_others: &'a LayeredSet,
        namespace_exports: &'a Arc<BTreeSet<String>>,
        s3_methods: &'a Arc<BTreeSet<String>>,
        wildcard_imports: &'a Arc<BTreeSet<String>>,
        resolution_incomplete: bool,
    ) -> Self {
        Self {
            visible,
            read_by_others,
            namespace_exports,
            s3_methods,
            wildcard_imports,
            resolution_incomplete,
        }
    }

    /// The packages this file's NAMESPACE `import()`s wholesale.
    pub fn wildcard_import_packages(&self) -> &BTreeSet<String> {
        self.wildcard_imports
    }

    /// The names visible to this file from the rest of the project, as the
    /// shared-plus-delta layers. Handed out whole so
    /// [`Visibility`](crate::project::Visibility) can clone the shared layer by
    /// handle instead of materializing the package's union per member.
    pub fn visible_layer(&self) -> &LayeredSet {
        self.visible
    }

    /// The names of this file's bindings actually *read* by some file that can
    /// see it, as layers. Excludes the NAMESPACE-export contribution — see
    /// [`namespace_export_names`](Self::namespace_export_names).
    pub fn read_layer(&self) -> &LayeredSet {
        self.read_by_others
    }

    /// The names this file's package `export()`s from its NAMESPACE. Kept apart
    /// from [`read_names`](Self::read_names) because the two answer different
    /// questions: an exported name is *public API* (so `unused-binding` must
    /// stay quiet), which is not the same as a name some sibling actually calls
    /// (which is what `unused-function` asks about).
    pub fn namespace_export_names(&self) -> &BTreeSet<String> {
        self.namespace_exports
    }

    /// The subset of [`namespace_export_names`](Self::namespace_export_names)
    /// registered via `S3method()`. A method is reached by dispatch, so the
    /// absence of a direct call to its name means nothing.
    pub fn s3_method_names(&self) -> &BTreeSet<String> {
        self.s3_methods
    }

    /// The three per-package sets as shared handles, so
    /// [`Visibility`](crate::project::Visibility) can take a reference count
    /// instead of copying one NAMESPACE's sets into every member's memo. The
    /// `*_names` accessors above are the same data, borrowed.
    pub fn namespace_exports_handle(&self) -> &Arc<BTreeSet<String>> {
        self.namespace_exports
    }

    pub fn s3_methods_handle(&self) -> &Arc<BTreeSet<String>> {
        self.s3_methods
    }

    pub fn wildcard_imports_handle(&self) -> &Arc<BTreeSet<String>> {
        self.wildcard_imports
    }

    /// True when `name` is bound at top level in a file visible from here.
    pub fn resolves(&self, name: &str) -> bool {
        self.visible.contains(name)
    }

    /// True when `name` (a top-level binding here) is read by a file that can
    /// see this one — so it isn't unused even if unread locally.
    pub fn read_elsewhere(&self, name: &str) -> bool {
        self.read_by_others.contains(unbacktick(name))
    }

    /// True when `name` (a top-level binding here) is `export()`ed by the
    /// package's NAMESPACE, i.e. it is public API.
    pub fn exported_by_namespace(&self, name: &str) -> bool {
        self.namespace_exports.contains(unbacktick(name))
    }

    /// True when `name` is registered as an S3 method by the package's
    /// NAMESPACE (`S3method(generic, class)`).
    pub fn is_s3_method(&self, name: &str) -> bool {
        self.s3_methods.contains(unbacktick(name))
    }

    /// True when `name` (a top-level binding here) must not be reported unused:
    /// either a file that can see this one reads it, or it is exported as
    /// public API.
    pub fn used_elsewhere(&self, name: &str) -> bool {
        self.read_elsewhere(name) || self.exported_by_namespace(name)
    }
}

impl ProjectScope {
    /// Resolve cross-file relationships for `files`. `namespaces` maps a package
    /// root to its NAMESPACE file contents, when present. `package_complete` maps
    /// a package root to whether its analyzed member set is complete (see
    /// [`ProjectScope::package_complete`]); a root absent from the map is treated
    /// as complete. `native_routines` maps a package root to the names its
    /// `useDynLib()` binds ([`crate::project::native`]); resolved by the caller,
    /// which is what keeps this builder pure.
    pub fn build(
        files: &[FileFacts],
        namespaces: &HashMap<PathBuf, String>,
        package_complete: &HashMap<PathBuf, bool>,
        native_routines: &HashMap<PathBuf, BTreeSet<String>>,
    ) -> Self {
        let by_path: HashMap<&Path, &FileFacts> =
            files.iter().map(|f| (f.path.as_path(), f)).collect();

        // Package members keyed by root, so package siblings see each other.
        let mut package_members: HashMap<&Path, Vec<&Path>> = HashMap::new();
        for f in files {
            if let Some(root) = &f.package_root {
                package_members
                    .entry(root.as_path())
                    .or_default()
                    .push(f.path.as_path());
            }
        }

        // Each root's member set, held once. Both file-set relations — the flat
        // shared-namespace one that aliasing/conflict detection needs, and the
        // visibility one — are this set minus the file being asked about, so
        // neither is materialized per member.
        let root_members: HashMap<PathBuf, Arc<HashSet<PathBuf>>> = package_members
            .iter()
            .map(|(&root, members)| {
                (
                    root.to_path_buf(),
                    Arc::new(members.iter().map(|&p| p.to_path_buf()).collect()),
                )
            })
            .collect();
        let file_root: HashMap<PathBuf, PathBuf> = files
            .iter()
            .filter_map(|f| Some((f.path.clone(), f.package_root.clone()?)))
            .collect();

        // Per package file, its root's completeness verdict (a root absent from
        // the map is vacuously complete). Only recorded for package files.
        let package_complete: HashMap<PathBuf, bool> = files
            .iter()
            .filter_map(|f| {
                let root = f.package_root.as_ref()?;
                Some((
                    f.path.clone(),
                    package_complete.get(root).copied().unwrap_or(true),
                ))
            })
            .collect();

        // Per-file data retained for the load-order replay (`source()` position).
        let top_level_events: HashMap<PathBuf, Vec<TopLevelEvent>> = files
            .iter()
            .map(|f| (f.path.clone(), f.top_level_events.clone()))
            .collect();
        let exports_by_path: HashMap<PathBuf, BTreeSet<String>> = files
            .iter()
            .map(|f| (f.path.clone(), f.exports.clone()))
            .collect();
        let source_edges_by_path: HashMap<PathBuf, Vec<SourceEdgeKey>> = files
            .iter()
            .map(|f| (f.path.clone(), f.source_edges.clone()))
            .collect();

        // For each file, the set of *other* files it can see — and, kept apart,
        // just the part of it reached through `source()`. The split is what lets
        // the two derivations below treat a package as a clique: within a
        // package every member sees every other, so the clique's contribution is
        // the same for all of them and can be folded once instead of once per
        // ordered pair. `source()` edges are directional and sparse, so they stay
        // a per-edge fold.
        let mut sees_extra: HashMap<PathBuf, HashSet<PathBuf>> = HashMap::new();
        let mut seen_by_extra: HashMap<PathBuf, HashSet<PathBuf>> = HashMap::new();
        let mut sourced: Vec<Vec<&Path>> = Vec::with_capacity(files.len());
        let mut dynamic: HashSet<PathBuf> = HashSet::new();
        let mut wildcard_imports: HashMap<PathBuf, Arc<BTreeSet<String>>> = HashMap::new();
        for f in files {
            let mut via_source: Vec<&Path> = Vec::new();
            let mut unresolved = false;
            let mut visited: HashSet<&Path> = HashSet::from([f.path.as_path()]);
            let mut queue: Vec<&FileFacts> = vec![f];
            while let Some(cur) = queue.pop() {
                for edge in &cur.source_edges {
                    match source_dependency(edge) {
                        Dependency::Skip => {}
                        Dependency::Unresolved => unresolved = true,
                        Dependency::Path(p) => match by_path.get(p) {
                            Some(target) if visited.insert(target.path.as_path()) => {
                                via_source.push(target.path.as_path());
                                queue.push(target);
                            }
                            Some(_) => {}
                            // A resolved path to a file we didn't analyze is just
                            // as opaque as a dynamic source.
                            None => unresolved = true,
                        },
                    }
                }
            }

            if unresolved {
                dynamic.insert(f.path.clone());
            }
            // Keep the closure disjoint from the root's member set: a member
            // that also `source()`s a sibling reaches it both ways, and a view
            // that stored it twice would over-count.
            // Two rootless scripts both have `package_root == None`, which is
            // *not* shared membership — only a `Some` root that matches is.
            let extra: HashSet<PathBuf> = via_source
                .iter()
                .filter(|t| f.package_root.is_none() || by_path[*t].package_root != f.package_root)
                .map(|t| t.to_path_buf())
                .collect();
            for target in &extra {
                seen_by_extra
                    .entry(target.clone())
                    .or_default()
                    .insert(f.path.clone());
            }
            sees_extra.insert(f.path.clone(), extra);
            sourced.push(via_source);
        }

        // A file's reads, normalized once here rather than once per file that
        // sees it. Backticking is a *spelling*, not a different name: `foo` and
        // `` `foo` `` are one binding. Reads are recorded as spelled, so
        // normalize here — the accessors strip the queried name to match. A
        // `pkg::name` / `pkg:::name` access counts as a use too (e.g.
        // `pkg:::helper()` in a test): fold in the qualified reads, which resolve
        // to a same-package sibling's binding.
        let reads: Vec<BTreeSet<String>> = files
            .iter()
            .map(|f| {
                f.free_reads
                    .iter()
                    .chain(f.qualified_reads.iter())
                    .map(|n| unbacktick(n).to_string())
                    .collect()
            })
            .collect();

        // The two per-package folds the clique relation collapses to: the union
        // of the members' exports, and per read name how many members read it.
        let mut package_exports: HashMap<&Path, BTreeSet<String>> = HashMap::new();
        let mut package_read_counts: HashMap<&Path, HashMap<&str, usize>> = HashMap::new();
        for (f, reads) in files.iter().zip(&reads) {
            let Some(root) = f.package_root.as_deref() else {
                continue;
            };
            package_exports
                .entry(root)
                .or_default()
                .extend(f.exports.iter().cloned());
            let counts = package_read_counts.entry(root).or_default();
            for name in reads {
                *counts.entry(name.as_str()).or_default() += 1;
            }
        }

        // NAMESPACE declarations, parsed once per root. This runs *before* the
        // two derivations because the imported names belong in `visible`'s
        // shared layer; the per-member fan-out stays below.
        let mut ns_exported: HashMap<&Path, Arc<BTreeSet<String>>> = HashMap::new();
        let mut ns_s3: HashMap<&Path, Arc<BTreeSet<String>>> = HashMap::new();
        let mut ns_imported: HashMap<&Path, BTreeSet<String>> = HashMap::new();
        let mut ns_wildcards: HashMap<&Path, Arc<BTreeSet<String>>> = HashMap::new();
        for (root, text) in namespaces {
            let Some(members) = package_members.get(root.as_path()) else {
                continue;
            };
            let object_names: Vec<String> = members
                .iter()
                .filter_map(|m| by_path.get(m))
                .flat_map(|f| f.exports.iter().map(|n| n.to_string()))
                .collect();
            let info = parse_namespace(text, &object_names);
            let root = root.as_path();
            ns_exported.insert(root, Arc::new(info.exports.iter().cloned().collect()));
            ns_s3.insert(root, Arc::new(info.s3_methods.iter().cloned().collect()));
            ns_imported.insert(root, info.imported_names.iter().cloned().collect());
            let wildcards: BTreeSet<String> = info.imported_packages.iter().cloned().collect();
            if !wildcards.is_empty() {
                ns_wildcards.insert(root, Arc::new(wildcards));
            }
        }

        // Derive the two directions from `sees`, each as one layer shared by the
        // whole package plus a per-file delta.

        // `visible`'s shared layer: the package's export union, plus what its
        // NAMESPACE `importFrom`s and its `useDynLib()` binds. Native routines
        // belong here rather than being reached through `sees` because nothing
        // in the R sources defines them, yet a reference to one resolves
        // anywhere in the package — not only in a `.Call()` head.
        //
        // Those last two are also kept apart as `exempt`: they are folded in
        // *after* the own-export removal, so a name that is both an own export
        // and an import (or a native routine) stays visible.
        let mut visible_shared: HashMap<&Path, Arc<BTreeSet<String>>> = HashMap::new();
        let mut visible_exempt: HashMap<&Path, BTreeSet<String>> = HashMap::new();
        for (&root, exports) in &package_exports {
            let mut exempt = BTreeSet::new();
            if let Some(imported) = ns_imported.get(root) {
                exempt.extend(imported.iter().cloned());
            }
            if let Some(routines) = native_routines.get(root) {
                exempt.extend(routines.iter().cloned());
            }
            let mut shared = exports.clone();
            shared.extend(exempt.iter().cloned());
            visible_shared.insert(root, Arc::new(shared));
            visible_exempt.insert(root, exempt);
        }

        let empty_shared: Arc<BTreeSet<String>> = Arc::new(BTreeSet::new());
        let mut visible: HashMap<PathBuf, LayeredSet> = HashMap::with_capacity(files.len());
        for (i, f) in files.iter().enumerate() {
            let root = f.package_root.as_deref();
            let shared = root
                .and_then(|r| visible_shared.get(r))
                .unwrap_or(&empty_shared)
                .clone();
            // `source()` edges are directional and sparse, so the closure's
            // exports stay a per-file layer.
            let mut added = BTreeSet::new();
            for target in &sourced[i] {
                if let Some(target) = by_path.get(target) {
                    added.extend(target.exports.iter().cloned());
                }
            }
            // Own bindings resolve locally, which is what makes `visible`
            // strictly cross-file. `removed` outranks `added` — a file that
            // redefines a name it also sources does not see the sourced one —
            // but not `exempt`, which is folded in later.
            let exempt = root.and_then(|r| visible_exempt.get(r));
            let removed: BTreeSet<String> = f
                .exports
                .iter()
                .filter(|n| exempt.is_none_or(|e| !e.contains(*n)))
                .cloned()
                .collect();
            visible.insert(f.path.clone(), LayeredSet::new(shared, added, removed));
        }

        // Every file `f` sees contributes `f`'s reads to that file's "used by
        // others" set. For a package member that is every sibling, so the shared
        // layer is the package-wide read union and the per-file delta drops only
        // the names *this* member alone reads. `source()` edges stay per-file,
        // and are computed first because they *win* over that exclusion.
        let mut read_added: HashMap<&Path, BTreeSet<String>> = HashMap::new();
        for i in 0..files.len() {
            for target in &sourced[i] {
                read_added
                    .entry(*target)
                    .or_default()
                    .extend(reads[i].iter().cloned());
            }
        }
        let read_shared: HashMap<&Path, Arc<BTreeSet<String>>> = package_read_counts
            .iter()
            .map(|(&root, counts)| {
                (
                    root,
                    Arc::new(counts.keys().map(|n| (*n).to_string()).collect()),
                )
            })
            .collect();

        let mut read_by_others: HashMap<PathBuf, LayeredSet> = HashMap::with_capacity(files.len());
        for (i, f) in files.iter().enumerate() {
            let root = f.package_root.as_deref();
            let shared = root
                .and_then(|r| read_shared.get(r))
                .unwrap_or(&empty_shared)
                .clone();
            let added = read_added.remove(f.path.as_path()).unwrap_or_default();
            // A name with two or more readers survives the exclusion of any
            // single one; a name with one reader survives unless that reader is
            // us — and even then, a file that `source()`s us may read it, which
            // `added` has already recorded.
            let removed: BTreeSet<String> = match root.and_then(|r| package_read_counts.get(r)) {
                Some(counts) => reads[i]
                    .iter()
                    .filter(|n| counts.get(n.as_str()) == Some(&1) && !added.contains(*n))
                    .cloned()
                    .collect(),
                None => BTreeSet::new(),
            };
            read_by_others.insert(f.path.clone(), LayeredSet::new(shared, added, removed));
        }

        // Fan the parsed NAMESPACE facts out to each root's members. Both
        // resolution directions are already folded in above — imported names sit
        // in `visible`'s shared layer — so what is left is the record of what the
        // package exports. A wholesale `import(pkg)` is *recorded*, not resolved:
        // whether pkg's exports are enumerable needs the library index, which
        // this pure builder does not have.
        let mut namespace_exports: HashMap<PathBuf, Arc<BTreeSet<String>>> = HashMap::new();
        let mut s3_methods: HashMap<PathBuf, Arc<BTreeSet<String>>> = HashMap::new();
        for (&root, members) in &package_members {
            for member in members {
                let path = member.to_path_buf();
                if let Some(exported) = ns_exported.get(root) {
                    namespace_exports.insert(path.clone(), Arc::clone(exported));
                }
                if let Some(s3) = ns_s3.get(root) {
                    s3_methods.insert(path.clone(), Arc::clone(s3));
                }
                if let Some(wildcards) = ns_wildcards.get(root) {
                    wildcard_imports.insert(path, Arc::clone(wildcards));
                }
            }
        }

        Self {
            visible,
            read_by_others,
            namespace_exports,
            s3_methods,
            wildcard_imports,
            dynamic,
            root_members,
            file_root,
            sees_extra,
            seen_by_extra,
            package_complete,
            top_level_events,
            exports: exports_by_path,
            source_edges: source_edges_by_path,
        }
    }

    /// One file's view of the project. Files not in the analyzed set get an
    /// empty, non-dynamic scope.
    pub fn for_file(&self, path: &Path) -> FileScope<'_> {
        FileScope {
            visible: self.visible.get(path).unwrap_or(&EMPTY_LAYER),
            read_by_others: self.read_by_others.get(path).unwrap_or(&EMPTY_LAYER),
            namespace_exports: self.namespace_exports.get(path).unwrap_or(&EMPTY_NAMES),
            s3_methods: self.s3_methods.get(path).unwrap_or(&EMPTY_NAMES),
            wildcard_imports: self.wildcard_imports.get(path).unwrap_or(&EMPTY_NAMES),
            resolution_incomplete: self.dynamic.contains(path),
        }
    }

    /// The members of `path`'s package root, if it has one. The base of every
    /// file-set view: each one is this set minus `path` itself.
    fn members_of(&self, path: &Path) -> Option<&HashSet<PathBuf>> {
        self.file_root
            .get(path)
            .and_then(|root| self.root_members.get(root))
            .map(|members| &**members)
    }

    /// The set of *other* files `path` can see (package siblings + non-local
    /// `source()` closure). Directional. Empty for files outside the analyzed
    /// set.
    pub fn sees<'a>(&'a self, path: &'a Path) -> PathSetView<'a> {
        PathSetView {
            members: self.members_of(path),
            extra: self.sees_extra.get(path).unwrap_or(&EMPTY_PATHS),
            exclude: path,
        }
    }

    /// The inverse of [`sees`](Self::sees): the files that can see `path` (i.e.
    /// resolve `path`'s top-level bindings). For renaming a binding defined in
    /// `path`, these are the files whose reads can bind to it.
    ///
    /// The package half is symmetric (siblings see each other), so it is the
    /// same member set; only the `source()` half needs inverting, and that
    /// inverse is stored.
    pub fn seen_by<'a>(&'a self, path: &'a Path) -> PathSetView<'a> {
        PathSetView {
            members: self.members_of(path),
            extra: self.seen_by_extra.get(path).unwrap_or(&EMPTY_PATHS),
            exclude: path,
        }
    }

    /// The package co-members of `path` (excluding itself), which share one flat
    /// namespace with it. Empty for non-package files. Unlike [`sees`](Self::sees),
    /// this is the *aliasing* relation: two siblings defining the same top-level
    /// name are the same binding slot.
    pub fn package_siblings<'a>(&'a self, path: &'a Path) -> PathSetView<'a> {
        PathSetView {
            members: self.members_of(path),
            extra: &EMPTY_PATHS,
            exclude: path,
        }
    }

    /// Whether `path`'s package root has a *complete* analyzed member set — i.e.
    /// every `R/*.[RrSsQq]` source the package loads was analyzed. Vacuously
    /// `true` for a non-package file (it has no flat package namespace to be
    /// incomplete over). A `false` here is what makes a multi-def package cohort
    /// refuse rename instead of half-rewriting the namespace.
    pub fn package_complete(&self, path: &Path) -> bool {
        self.package_complete.get(path).copied().unwrap_or(true)
    }

    /// Resolve, by sequential load order, what `name`'s *top-level* reads in
    /// `from_file` bind to. Replays the file's [`TopLevelEvent`] sequence: a
    /// `source()` edge folds its (transitive, non-local) closure's defs of `name`
    /// into the live binding, a later top-level def shadows it, and each
    /// top-level read records the live binding at its point. A dynamic/unanalyzed
    /// source poisons every later read; two closure files defining `name` make it
    /// ambiguous. Function-body reads aren't in the sequence (they run against the
    /// final scope), so a file with only those is [`ReadBinding::NoTopLevelRead`].
    pub fn top_level_read_binding(&self, from_file: &Path, name: &str) -> ReadBinding {
        let Some(events) = self.top_level_events.get(from_file) else {
            return ReadBinding::NoTopLevelRead;
        };
        let mut live: Option<PathBuf> = None;
        let mut name_ambiguous = false;
        let mut poisoned = false;
        let mut saw_read = false;
        let mut resolved: BTreeSet<PathBuf> = BTreeSet::new();
        let mut saw_unresolved = false;
        let mut saw_unknown = false;
        for event in events {
            match event {
                TopLevelEvent::Define(n) if n == name => {
                    live = Some(from_file.to_path_buf());
                    name_ambiguous = false;
                }
                TopLevelEvent::SourceEdge(key) => match source_dependency(key) {
                    Dependency::Skip => {}
                    Dependency::Unresolved => poisoned = true,
                    Dependency::Path(p) => {
                        let mut definers = self.closure_definers(p, name);
                        match definers.len() {
                            0 => {}
                            1 => {
                                live = definers.pop();
                                name_ambiguous = false;
                            }
                            _ => name_ambiguous = true,
                        }
                    }
                },
                TopLevelEvent::Read(n) if n == name => {
                    saw_read = true;
                    if poisoned || name_ambiguous {
                        saw_unknown = true;
                    } else if let Some(p) = &live {
                        resolved.insert(p.clone());
                    } else {
                        saw_unresolved = true;
                    }
                }
                _ => {}
            }
        }
        if !saw_read {
            return ReadBinding::NoTopLevelRead;
        }
        if saw_unknown {
            return ReadBinding::OrderUnknown;
        }
        match (resolved.len(), saw_unresolved) {
            (0, _) => ReadBinding::Unresolved,
            (1, false) => ReadBinding::Resolved(resolved.into_iter().next().expect("len == 1")),
            _ => ReadBinding::OrderUnknown,
        }
    }

    /// The per-occurrence binding of each top-level read of `name` in
    /// `from_file`, paired with its span. The span-aware refinement of
    /// [`top_level_read_binding`](Self::top_level_read_binding): same load-order
    /// replay (a `source()` edge folds its closure's def into the live binding, a
    /// later top-level def shadows it, a dynamic/unanalyzed source poisons every
    /// later read, two closure definers make it ambiguous), but instead of
    /// aggregating it emits one [`ReadSite`] per read so an order-aware rename can
    /// co-rename the cohort-bound reads and skip the rest.
    ///
    /// `spanned` is `from_file`'s own top-level sequence *with* read spans (from
    /// [`crate::project::collect_top_level_events_spanned`] off the current
    /// tree); the stored, range-free sequence can't supply spans. Cross-file
    /// closure resolution still reads `self`'s range-free data, so the two views
    /// agree on event order by construction. Reads of other names contribute
    /// their `Define`/`SourceEdge` effect to the replay but emit no `ReadSite`.
    pub fn top_level_read_provenance(
        &self,
        from_file: &Path,
        name: &str,
        spanned: &[(TopLevelEvent, Option<TextRange>)],
    ) -> Vec<(TextRange, ReadSite)> {
        let mut live: Option<PathBuf> = None;
        let mut name_ambiguous = false;
        let mut poisoned = false;
        let mut sites: Vec<(TextRange, ReadSite)> = Vec::new();
        for (event, span) in spanned {
            match event {
                TopLevelEvent::Define(n) if n == name => {
                    live = Some(from_file.to_path_buf());
                    name_ambiguous = false;
                }
                TopLevelEvent::SourceEdge(key) => match source_dependency(key) {
                    Dependency::Skip => {}
                    Dependency::Unresolved => poisoned = true,
                    Dependency::Path(p) => {
                        let mut definers = self.closure_definers(p, name);
                        match definers.len() {
                            0 => {}
                            1 => {
                                live = definers.pop();
                                name_ambiguous = false;
                            }
                            _ => name_ambiguous = true,
                        }
                    }
                },
                TopLevelEvent::Read(n) if n == name => {
                    let range = span.expect("a Read event always carries its span");
                    let site = if poisoned || name_ambiguous {
                        ReadSite::Unknown
                    } else if let Some(p) = &live {
                        ReadSite::Bound(p.clone())
                    } else {
                        ReadSite::Unbound
                    };
                    sites.push((range, site));
                }
                _ => {}
            }
        }
        sites
    }

    /// What `name` binds to in `from_file`'s *final* post-execution scope: the
    /// same load-order replay as
    /// [`top_level_read_binding`](Self::top_level_read_binding), read at
    /// end-of-file rather than per top-level read. This is the binding a
    /// **function-body** read of `name` sees — bodies run at call time against
    /// the fully executed scope, so they aren't position-gated and a later
    /// `source()` of a same-name def shadows an earlier one.
    ///
    /// [`ReadSite::Bound`] names the last live definer; [`ReadSite::Unbound`]
    /// means nothing in the file's own sequence defines it (it then comes from a
    /// package sibling's flat namespace, if at all — so a rename treats this as
    /// "binds to the cohort"); [`ReadSite::Unknown`] means a dynamic/unanalyzed
    /// source or two closure definers leave it undecidable. Span-free (reads the
    /// stored `top_level_events`), so it backdates across body edits like the
    /// other replays.
    pub fn final_scope_binding(&self, from_file: &Path, name: &str) -> ReadSite {
        let Some(events) = self.top_level_events.get(from_file) else {
            return ReadSite::Unbound;
        };
        let mut live: Option<PathBuf> = None;
        let mut name_ambiguous = false;
        let mut poisoned = false;
        for event in events {
            match event {
                TopLevelEvent::Define(n) if n == name => {
                    live = Some(from_file.to_path_buf());
                    name_ambiguous = false;
                }
                TopLevelEvent::SourceEdge(key) => match source_dependency(key) {
                    Dependency::Skip => {}
                    Dependency::Unresolved => poisoned = true,
                    Dependency::Path(p) => {
                        let mut definers = self.closure_definers(p, name);
                        match definers.len() {
                            0 => {}
                            1 => {
                                live = definers.pop();
                                name_ambiguous = false;
                            }
                            _ => name_ambiguous = true,
                        }
                    }
                },
                _ => {}
            }
        }
        if poisoned || name_ambiguous {
            ReadSite::Unknown
        } else if let Some(p) = live {
            ReadSite::Bound(p)
        } else {
            ReadSite::Unbound
        }
    }

    /// The analyzed files in the transitive, non-local `source()` closure rooted
    /// at `start` (including `start` itself) whose top-level exports include
    /// `name`. Cycle-guarded with a `visited` set like [`ProjectScope::build`].
    fn closure_definers(&self, start: &Path, name: &str) -> Vec<PathBuf> {
        let mut definers: Vec<PathBuf> = Vec::new();
        let mut visited: HashSet<PathBuf> = HashSet::new();
        let mut stack: Vec<PathBuf> = vec![start.to_path_buf()];
        while let Some(cur) = stack.pop() {
            if !visited.insert(cur.clone()) {
                continue;
            }
            if self.exports.get(&cur).is_some_and(|e| e.contains(name)) {
                definers.push(cur.clone());
            }
            if let Some(edges) = self.source_edges.get(&cur) {
                for edge in edges {
                    if let SourceTarget::Path(target) = &edge.target
                        && !edge.local
                    {
                        stack.push(target.clone());
                    }
                }
            }
        }
        definers
    }
}

enum Dependency<'a> {
    /// Contributes the target file's top-level bindings to global scope.
    Path(&'a Path),
    /// Unresolvable (dynamic argument); visibility is incomplete.
    Unresolved,
    /// `local = TRUE`: loads into the calling env, never global scope.
    Skip,
}

fn source_dependency(edge: &SourceEdgeKey) -> Dependency<'_> {
    match &edge.target {
        SourceTarget::Dynamic => Dependency::Unresolved,
        SourceTarget::Path(_) if edge.local => Dependency::Skip,
        SourceTarget::Path(p) => Dependency::Path(p.as_path()),
    }
}

/// Whether `dir` is an R package root: it holds both a `DESCRIPTION` file and an
/// `R/` subdirectory. Touches the filesystem.
///
/// The one statement of what arity considers a package. File discovery asks it
/// directly (to tell a package's `DESCRIPTION` from an `inst/extdata` fixture of
/// the same name), and [`package_root`] is the walk-upward form.
pub fn is_package_root(dir: &Path) -> bool {
    dir.join("DESCRIPTION").is_file() && dir.join("R").is_dir()
}

/// Walk up from `path` to find an enclosing R package root: a directory with
/// both a `DESCRIPTION` file and an `R/` subdirectory. Touches the filesystem.
pub fn package_root(path: &Path) -> Option<PathBuf> {
    path.parent().and_then(package_root_of_dir)
}

/// [`package_root`] for a directory already in hand: the nearest ancestor of (or
/// equal to) `dir` that is a package root.
///
/// Split out because the answer depends only on the directory. Each step stats
/// two entries, and the walk runs to the filesystem root when there is no
/// package at all, so a caller holding many files should dedup by parent
/// directory and ask once per directory rather than once per file.
pub fn package_root_of_dir(dir: &Path) -> Option<PathBuf> {
    let mut dir = Some(dir);
    while let Some(d) = dir {
        if is_package_root(d) {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    fn source_path(target: &str, local: bool) -> SourceEdgeKey {
        SourceEdgeKey {
            target: SourceTarget::Path(PathBuf::from(target)),
            local,
        }
    }

    fn dynamic_edge() -> SourceEdgeKey {
        SourceEdgeKey {
            target: SourceTarget::Dynamic,
            local: false,
        }
    }

    /// Build `FileFacts` with `path`, exports, free reads, source edges, root.
    fn facts(
        path: &str,
        exp: &[&str],
        reads: &[&str],
        edges: Vec<SourceEdgeKey>,
        root: Option<&str>,
    ) -> FileFacts {
        FileFacts {
            path: PathBuf::from(path),
            exports: set(exp),
            free_reads: set(reads),
            qualified_reads: BTreeSet::new(),
            source_edges: edges,
            top_level_events: Vec::new(),
            package_root: root.map(PathBuf::from),
        }
    }

    fn layer_names(layer: &LayeredSet) -> Vec<String> {
        let mut v: Vec<String> = layer.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    }

    fn read_ev(name: &str) -> TopLevelEvent {
        TopLevelEvent::Read(name.to_string())
    }
    fn def_ev(name: &str) -> TopLevelEvent {
        TopLevelEvent::Define(name.to_string())
    }
    fn src_ev(target: &str) -> TopLevelEvent {
        TopLevelEvent::SourceEdge(source_path(target, false))
    }
    fn dyn_src_ev() -> TopLevelEvent {
        TopLevelEvent::SourceEdge(dynamic_edge())
    }

    /// `FileFacts` carrying an explicit top-level event sequence (and matching
    /// `source_edges`), for load-order resolution tests.
    fn facts_seq(
        path: &str,
        exp: &[&str],
        edges: Vec<SourceEdgeKey>,
        events: Vec<TopLevelEvent>,
    ) -> FileFacts {
        FileFacts {
            path: PathBuf::from(path),
            exports: set(exp),
            free_reads: BTreeSet::new(),
            qualified_reads: BTreeSet::new(),
            source_edges: edges,
            top_level_events: events,
            package_root: None,
        }
    }

    /// Build a scope with no NAMESPACE data.
    fn build_scope(files: &[FileFacts]) -> ProjectScope {
        ProjectScope::build(files, &HashMap::new(), &HashMap::new(), &HashMap::new())
    }

    #[test]
    fn package_files_share_one_namespace() {
        let files = [
            facts("/pkg/R/a.R", &["foo"], &[], vec![], Some("/pkg")),
            facts("/pkg/R/b.R", &["bar"], &["foo"], vec![], Some("/pkg")),
        ];
        let scope = build_scope(&files);
        // b reads foo, which a defines: resolves cross-file.
        assert!(scope.for_file(Path::new("/pkg/R/b.R")).resolves("foo"));
        // foo is used by b, so a's foo isn't unused.
        assert!(
            scope
                .for_file(Path::new("/pkg/R/a.R"))
                .used_elsewhere("foo")
        );
        // bar is defined by b but read by nobody.
        assert!(
            !scope
                .for_file(Path::new("/pkg/R/b.R"))
                .used_elsewhere("bar")
        );
    }

    /// A package member's *own* reads must never land in its own
    /// "used by others" set. `sees` excludes self, and the clique fold that
    /// replaces the per-pair walk has to reproduce that exclusion: a name only
    /// one member reads is not read *by others* for that member, but is for
    /// every other member.
    ///
    /// Observable through `qualified_reads` because that is the one way a file
    /// can reference a name it also defines (`pkg:::foo()` alongside `foo <-`);
    /// a free read of an own binding resolves locally and never appears.
    #[test]
    fn a_members_own_qualified_read_is_not_a_use_by_others() {
        let mut a = facts("/pkg/R/a.R", &["foo"], &[], vec![], Some("/pkg"));
        a.qualified_reads = set(&["foo"]);
        let b = facts("/pkg/R/b.R", &["bar"], &[], vec![], Some("/pkg"));
        let scope = build_scope(&[a.clone(), b.clone()]);
        // Only a references `foo`, and a is a's own file.
        assert!(
            !scope
                .for_file(Path::new("/pkg/R/a.R"))
                .used_elsewhere("foo")
        );
        // The same name still counts as a use for every *other* member.
        assert!(
            scope
                .for_file(Path::new("/pkg/R/b.R"))
                .read_elsewhere("foo")
        );

        // A second referencing member makes it a use for a too.
        let mut c = facts("/pkg/R/c.R", &["baz"], &[], vec![], Some("/pkg"));
        c.qualified_reads = set(&["foo"]);
        let scope = build_scope(&[a, b, c]);
        assert!(
            scope
                .for_file(Path::new("/pkg/R/a.R"))
                .used_elsewhere("foo")
        );
    }

    #[test]
    fn source_closure_is_directional() {
        // a.R sources b.R: a sees bar; b does not see foo.
        let files = [
            facts(
                "/s/a.R",
                &["foo"],
                &["bar"],
                vec![source_path("/s/b.R", false)],
                None,
            ),
            facts("/s/b.R", &["bar"], &[], vec![], None),
        ];
        let scope = build_scope(&files);
        assert!(scope.for_file(Path::new("/s/a.R")).resolves("bar"));
        assert!(!scope.for_file(Path::new("/s/b.R")).resolves("foo"));
        // a reads bar, and a sees b, so b's bar is used elsewhere.
        assert!(scope.for_file(Path::new("/s/b.R")).used_elsewhere("bar"));
        assert!(!scope.for_file(Path::new("/s/a.R")).resolution_incomplete);
    }

    #[test]
    fn source_closure_is_transitive_and_cycle_safe() {
        // a -> b -> c, plus c -> a (cycle). a sees bar + baz.
        let files = [
            facts(
                "/s/a.R",
                &["foo"],
                &[],
                vec![source_path("/s/b.R", false)],
                None,
            ),
            facts(
                "/s/b.R",
                &["bar"],
                &[],
                vec![source_path("/s/c.R", false)],
                None,
            ),
            facts(
                "/s/c.R",
                &["baz"],
                &[],
                vec![source_path("/s/a.R", false)],
                None,
            ),
        ];
        let scope = build_scope(&files);
        assert_eq!(
            layer_names(scope.for_file(Path::new("/s/a.R")).visible_layer()),
            vec!["bar", "baz"]
        );
    }

    #[test]
    fn seen_by_is_inverse_of_sees() {
        // a sources b: a sees b, so b is seen_by a; nobody sees a.
        let files = [
            facts(
                "/s/a.R",
                &["foo"],
                &[],
                vec![source_path("/s/b.R", false)],
                None,
            ),
            facts("/s/b.R", &["bar"], &[], vec![], None),
        ];
        let scope = build_scope(&files);
        assert!(
            scope
                .sees(Path::new("/s/a.R"))
                .contains(Path::new("/s/b.R"))
        );
        assert!(scope.sees(Path::new("/s/b.R")).is_empty());
        assert!(
            scope
                .seen_by(Path::new("/s/b.R"))
                .contains(Path::new("/s/a.R"))
        );
        assert!(scope.seen_by(Path::new("/s/a.R")).is_empty());
    }

    #[test]
    fn seen_by_includes_package_siblings_symmetrically() {
        let files = [
            facts("/pkg/R/a.R", &["foo"], &[], vec![], Some("/pkg")),
            facts("/pkg/R/b.R", &["bar"], &[], vec![], Some("/pkg")),
        ];
        let scope = build_scope(&files);
        assert!(
            scope
                .sees(Path::new("/pkg/R/a.R"))
                .contains(Path::new("/pkg/R/b.R"))
        );
        assert!(
            scope
                .sees(Path::new("/pkg/R/b.R"))
                .contains(Path::new("/pkg/R/a.R"))
        );
        assert!(
            scope
                .seen_by(Path::new("/pkg/R/a.R"))
                .contains(Path::new("/pkg/R/b.R"))
        );
    }

    #[test]
    fn seen_by_excludes_unconnected_file() {
        // Two flat scripts, same export name, no edge: neither sees the other.
        let files = [
            facts("/s/a.R", &["foo"], &[], vec![], None),
            facts("/s/b.R", &["foo"], &[], vec![], None),
        ];
        let scope = build_scope(&files);
        assert!(scope.sees(Path::new("/s/a.R")).is_empty());
        assert!(scope.seen_by(Path::new("/s/a.R")).is_empty());
    }

    /// A package member that also `source()`s a sibling reaches it two ways.
    /// `sees` is a set, so the sibling is still one entry — a representation
    /// that keeps the member half and the `source()` half apart has to keep them
    /// disjoint or it double-counts here.
    #[test]
    fn sees_does_not_double_count_a_sourced_sibling() {
        let files = [
            facts(
                "/pkg/R/a.R",
                &[],
                &[],
                vec![source_path("/pkg/R/b.R", false)],
                Some("/pkg"),
            ),
            facts("/pkg/R/b.R", &["bar"], &[], vec![], Some("/pkg")),
        ];
        let scope = build_scope(&files);
        let a = scope.sees(Path::new("/pkg/R/a.R"));
        assert_eq!(a.len(), 1);
        assert!(a.contains(Path::new("/pkg/R/b.R")));
    }

    #[test]
    fn seen_by_is_siblings_plus_outside_sourcers() {
        let files = [
            facts("/pkg/R/a.R", &["foo"], &[], vec![], Some("/pkg")),
            facts("/pkg/R/b.R", &["bar"], &[], vec![], Some("/pkg")),
            facts(
                "/s/x.R",
                &[],
                &["foo"],
                vec![source_path("/pkg/R/a.R", false)],
                None,
            ),
        ];
        let scope = build_scope(&files);
        let seen_by = scope.seen_by(Path::new("/pkg/R/a.R"));
        assert_eq!(seen_by.len(), 2);
        assert!(seen_by.contains(Path::new("/pkg/R/b.R")));
        assert!(seen_by.contains(Path::new("/s/x.R")));
        assert!(!seen_by.contains(Path::new("/pkg/R/a.R")));
    }

    /// `package_siblings` is the flat-namespace aliasing relation, so unlike
    /// `sees` it must not pick up `source()` targets.
    #[test]
    fn package_siblings_excludes_self_and_source_targets() {
        let files = [
            facts(
                "/pkg/R/a.R",
                &[],
                &[],
                vec![source_path("/s/c.R", false)],
                Some("/pkg"),
            ),
            facts("/pkg/R/b.R", &["bar"], &[], vec![], Some("/pkg")),
            facts("/s/c.R", &["baz"], &[], vec![], None),
        ];
        let scope = build_scope(&files);
        let siblings = scope.package_siblings(Path::new("/pkg/R/a.R"));
        assert_eq!(siblings.len(), 1);
        assert!(siblings.contains(Path::new("/pkg/R/b.R")));
        assert!(!siblings.contains(Path::new("/s/c.R")));
        assert!(!siblings.contains(Path::new("/pkg/R/a.R")));
        // A non-package file has no siblings at all.
        assert!(scope.package_siblings(Path::new("/s/c.R")).is_empty());
    }

    #[test]
    fn dynamic_source_marks_scope_incomplete() {
        let files = [facts("/s/a.R", &[], &[], vec![dynamic_edge()], None)];
        let scope = build_scope(&files);
        assert!(scope.for_file(Path::new("/s/a.R")).resolution_incomplete);
    }

    #[test]
    fn source_to_unanalyzed_file_marks_scope_incomplete() {
        let files = [facts(
            "/s/a.R",
            &[],
            &[],
            vec![source_path("/s/missing.R", false)],
            None,
        )];
        let scope = build_scope(&files);
        assert!(scope.for_file(Path::new("/s/a.R")).resolution_incomplete);
    }

    #[test]
    fn local_source_neither_contributes_nor_marks_dynamic() {
        let files = [
            facts(
                "/s/a.R",
                &[],
                &["bar"],
                vec![source_path("/s/b.R", true)],
                None,
            ),
            facts("/s/b.R", &["bar"], &[], vec![], None),
        ];
        let scope = build_scope(&files);
        let a = scope.for_file(Path::new("/s/a.R"));
        assert!(!a.resolves("bar"));
        assert!(!a.resolution_incomplete);
        // A local source doesn't make b's bar "used elsewhere".
        assert!(!scope.for_file(Path::new("/s/b.R")).used_elsewhere("bar"));
    }

    fn namespaces(entries: &[(&str, &str)]) -> HashMap<PathBuf, String> {
        entries
            .iter()
            .map(|(root, text)| (PathBuf::from(*root), text.to_string()))
            .collect()
    }

    #[test]
    fn namespace_export_marks_binding_used() {
        // `foo` is exported, so it isn't unused even though no file reads it.
        let files = [facts("/pkg/R/a.R", &["foo"], &[], vec![], Some("/pkg"))];
        let ns = namespaces(&[("/pkg", "export(foo)\n")]);
        let scope = ProjectScope::build(&files, &ns, &HashMap::new(), &HashMap::new());
        assert!(
            scope
                .for_file(Path::new("/pkg/R/a.R"))
                .used_elsewhere("foo")
        );
    }

    #[test]
    fn namespace_import_from_resolves_name() {
        let files = [facts("/pkg/R/a.R", &[], &["filter"], vec![], Some("/pkg"))];
        let ns = namespaces(&[("/pkg", "importFrom(dplyr, filter)\n")]);
        let scope = ProjectScope::build(&files, &ns, &HashMap::new(), &HashMap::new());
        let a = scope.for_file(Path::new("/pkg/R/a.R"));
        assert!(a.resolves("filter"));
        assert!(!a.resolution_incomplete);
    }

    #[test]
    fn namespace_wholesale_import_is_recorded_not_poisoned() {
        // `import(pkg)` used to poison the file unconditionally. It now reports
        // the package instead: whether pkg's exports are enumerable needs the
        // library index, which this pure builder deliberately does not have.
        let files = [facts("/pkg/R/a.R", &[], &["abort"], vec![], Some("/pkg"))];
        let ns = namespaces(&[("/pkg", "import(rlang)\n")]);
        let scope = ProjectScope::build(&files, &ns, &HashMap::new(), &HashMap::new());
        let a = scope.for_file(Path::new("/pkg/R/a.R"));
        assert_eq!(
            a.wildcard_import_packages(),
            &["rlang".to_string()].into_iter().collect()
        );
        assert!(
            !a.resolution_incomplete,
            "a wildcard import is a question for the index, not an unresolvable"
        );
    }

    fn routines(entries: &[(&str, &[&str])]) -> HashMap<PathBuf, BTreeSet<String>> {
        entries
            .iter()
            .map(|(root, names)| (PathBuf::from(*root), set(names)))
            .collect()
    }

    // The tests below pin the *precedence* rules inside `build`. `visible` and
    // `read_by_others` are each assembled by several passes, and in both cases a
    // later pass overrides an earlier one — which pass wins is observable, and
    // any representation change has to reproduce it exactly.

    /// `importFrom` is folded in *after* a file's own exports are removed, so a
    /// name that is both is visible. Pins the direction: the own-export removal
    /// must not outrank the NAMESPACE import.
    #[test]
    fn own_export_that_is_also_importfrom_stays_visible() {
        let files = [facts("/pkg/R/a.R", &["filter"], &[], vec![], Some("/pkg"))];
        let ns = namespaces(&[("/pkg", "importFrom(dplyr, filter)\n")]);
        let scope = ProjectScope::build(&files, &ns, &HashMap::new(), &HashMap::new());
        assert!(scope.for_file(Path::new("/pkg/R/a.R")).resolves("filter"));
    }

    /// The `useDynLib()` twin of the case above: native routines are injected
    /// after the own-export removal too.
    #[test]
    fn own_export_that_is_also_a_native_routine_stays_visible() {
        let files = [facts("/pkg/R/a.R", &["c_foo"], &[], vec![], Some("/pkg"))];
        let native = routines(&[("/pkg", &["c_foo"])]);
        let scope = ProjectScope::build(&files, &HashMap::new(), &HashMap::new(), &native);
        assert!(scope.for_file(Path::new("/pkg/R/a.R")).resolves("c_foo"));
    }

    #[test]
    fn native_routine_is_visible_to_every_member() {
        let files = [
            facts("/pkg/R/a.R", &["foo"], &[], vec![], Some("/pkg")),
            facts("/pkg/R/b.R", &["bar"], &[], vec![], Some("/pkg")),
        ];
        let native = routines(&[("/pkg", &["c_foo"])]);
        let scope = ProjectScope::build(&files, &HashMap::new(), &HashMap::new(), &native);
        assert!(scope.for_file(Path::new("/pkg/R/a.R")).resolves("c_foo"));
        assert!(scope.for_file(Path::new("/pkg/R/b.R")).resolves("c_foo"));
    }

    /// The other direction: the own-export removal runs *after* the `source()`
    /// closure is folded in, so a file that redefines a name it also sources
    /// does not see the sourced one. `visible` is strictly cross-file — the own
    /// binding resolves locally.
    #[test]
    fn own_export_shadowing_a_sourced_export_is_not_visible() {
        let files = [
            facts("/s/a.R", &["foo"], &[], vec![], None),
            facts(
                "/s/b.R",
                &["foo"],
                &[],
                vec![source_path("/s/a.R", false)],
                None,
            ),
        ];
        let scope = build_scope(&files);
        assert!(!scope.for_file(Path::new("/s/b.R")).resolves("foo"));
    }

    #[test]
    fn own_export_is_never_self_visible() {
        let files = [facts("/pkg/R/a.R", &["foo"], &[], vec![], Some("/pkg"))];
        let scope = build_scope(&files);
        assert!(!scope.for_file(Path::new("/pkg/R/a.R")).resolves("foo"));
    }

    /// Every per-package set is keyed by root; two roots in one analyzed set
    /// must not bleed into each other.
    #[test]
    fn two_roots_do_not_leak() {
        let files = [
            facts("/p1/R/a.R", &["one"], &["shared"], vec![], Some("/p1")),
            facts("/p2/R/a.R", &["two"], &[], vec![], Some("/p2")),
        ];
        let ns = namespaces(&[
            (
                "/p1",
                "export(one)\nimportFrom(dplyr, filter)\nimport(rlang)\n",
            ),
            ("/p2", "S3method(print, two)\n"),
        ]);
        let native = routines(&[("/p1", &["c_one"])]);
        let scope = ProjectScope::build(&files, &ns, &HashMap::new(), &native);
        let p2 = scope.for_file(Path::new("/p2/R/a.R"));
        assert!(!p2.resolves("one"));
        assert!(!p2.resolves("filter"));
        assert!(!p2.resolves("c_one"));
        assert!(!p2.exported_by_namespace("one"));
        assert!(!p2.read_elsewhere("shared"));
        assert!(p2.wildcard_import_packages().is_empty());
        let p1 = scope.for_file(Path::new("/p1/R/a.R"));
        assert!(!p1.resolves("two"));
        assert!(!p1.is_s3_method("print.two"));
    }

    /// A root with no analyzed member contributes nothing — neither loop has a
    /// member list to fan out over.
    #[test]
    fn facts_for_a_root_with_no_members_are_ignored() {
        let files = [facts("/s/a.R", &[], &["filter"], vec![], None)];
        let ns = namespaces(&[("/pkg", "importFrom(dplyr, filter)\nexport(foo)\n")]);
        let native = routines(&[("/pkg", &["c_foo"])]);
        let scope = ProjectScope::build(&files, &ns, &HashMap::new(), &native);
        let a = scope.for_file(Path::new("/s/a.R"));
        assert!(!a.resolves("filter"));
        assert!(!a.resolves("c_foo"));
        assert!(!a.exported_by_namespace("foo"));
    }

    /// The per-`source()`-edge contribution to `read_by_others` runs *after* the
    /// package clique fold, so it overrides the fold's "only this member reads
    /// it" exclusion. Pins the one case where the two disagree.
    #[test]
    fn sourcer_read_beats_the_solo_reader_exclusion() {
        let mut a = facts("/pkg/R/a.R", &["foo"], &[], vec![], Some("/pkg"));
        a.qualified_reads = set(&["foo"]);
        let files = [
            a,
            facts("/pkg/R/b.R", &["bar"], &[], vec![], Some("/pkg")),
            facts(
                "/s/x.R",
                &[],
                &["foo"],
                vec![source_path("/pkg/R/a.R", false)],
                None,
            ),
        ];
        let scope = build_scope(&files);
        assert!(
            scope
                .for_file(Path::new("/pkg/R/a.R"))
                .read_elsewhere("foo")
        );
    }

    /// The negative control for the case above: with no sourcer reading the
    /// name, the clique fold's exclusion stands.
    #[test]
    fn sourcer_that_does_not_read_the_name_leaves_it_excluded() {
        let mut a = facts("/pkg/R/a.R", &["foo"], &[], vec![], Some("/pkg"));
        a.qualified_reads = set(&["foo"]);
        let files = [
            a,
            facts("/pkg/R/b.R", &["bar"], &[], vec![], Some("/pkg")),
            facts(
                "/s/x.R",
                &[],
                &["other"],
                vec![source_path("/pkg/R/a.R", false)],
                None,
            ),
        ];
        let scope = build_scope(&files);
        assert!(
            !scope
                .for_file(Path::new("/pkg/R/a.R"))
                .read_elsewhere("foo")
        );
    }

    /// A file with no package root has no clique fold at all: its "read by
    /// others" set comes only from the files that `source()` it, and never from
    /// its own reads.
    #[test]
    fn nonpackage_target_gets_reads_only_from_its_sourcers() {
        let files = [
            facts("/s/a.R", &["foo"], &["qux"], vec![], None),
            facts(
                "/s/x.R",
                &[],
                &["foo"],
                vec![source_path("/s/a.R", false)],
                None,
            ),
        ];
        let scope = build_scope(&files);
        let a = scope.for_file(Path::new("/s/a.R"));
        assert!(a.read_elsewhere("foo"));
        assert!(!a.read_elsewhere("qux"));
    }

    /// The `source()` closure walk seeds `visited` with the file itself, so a
    /// cycle never routes a file's own reads back into its own "used by others"
    /// set.
    #[test]
    fn a_cycle_between_two_package_members_does_not_self_source() {
        let mut a = facts(
            "/pkg/R/a.R",
            &["foo"],
            &[],
            vec![source_path("/pkg/R/b.R", false)],
            Some("/pkg"),
        );
        a.qualified_reads = set(&["foo"]);
        let files = [
            a,
            facts(
                "/pkg/R/b.R",
                &["bar"],
                &[],
                vec![source_path("/pkg/R/a.R", false)],
                Some("/pkg"),
            ),
        ];
        let scope = build_scope(&files);
        assert!(
            !scope
                .for_file(Path::new("/pkg/R/a.R"))
                .read_elsewhere("foo")
        );
        assert!(
            scope
                .for_file(Path::new("/pkg/R/b.R"))
                .read_elsewhere("foo")
        );
    }

    #[test]
    fn read_before_source_is_unresolved() {
        // b reads foo before sourcing a (which defines it): foo isn't live yet.
        let files = [
            facts("/s/a.R", &["foo"], &[], vec![], None),
            facts_seq(
                "/s/b.R",
                &[],
                vec![source_path("/s/a.R", false)],
                vec![read_ev("foo"), src_ev("/s/a.R")],
            ),
        ];
        let scope = build_scope(&files);
        assert_eq!(
            scope.top_level_read_binding(Path::new("/s/b.R"), "foo"),
            ReadBinding::Unresolved
        );
    }

    #[test]
    fn read_after_source_resolves_to_the_sourced_def() {
        let files = [
            facts("/s/a.R", &["foo"], &[], vec![], None),
            facts_seq(
                "/s/b.R",
                &[],
                vec![source_path("/s/a.R", false)],
                vec![src_ev("/s/a.R"), read_ev("foo")],
            ),
        ];
        let scope = build_scope(&files);
        assert_eq!(
            scope.top_level_read_binding(Path::new("/s/b.R"), "foo"),
            ReadBinding::Resolved(PathBuf::from("/s/a.R"))
        );
    }

    #[test]
    fn local_def_after_source_shadows_the_sourced_def() {
        // b sources a (foo), then defines its own foo: a later read binds to b.
        let files = [
            facts("/s/a.R", &["foo"], &[], vec![], None),
            facts_seq(
                "/s/b.R",
                &["foo"],
                vec![source_path("/s/a.R", false)],
                vec![src_ev("/s/a.R"), def_ev("foo"), read_ev("foo")],
            ),
        ];
        let scope = build_scope(&files);
        assert_eq!(
            scope.top_level_read_binding(Path::new("/s/b.R"), "foo"),
            ReadBinding::Resolved(PathBuf::from("/s/b.R"))
        );
    }

    #[test]
    fn dynamic_source_before_read_is_order_unknown() {
        let files = [facts_seq(
            "/s/b.R",
            &[],
            vec![dynamic_edge()],
            vec![dyn_src_ev(), read_ev("foo")],
        )];
        let scope = build_scope(&files);
        assert_eq!(
            scope.top_level_read_binding(Path::new("/s/b.R"), "foo"),
            ReadBinding::OrderUnknown
        );
    }

    #[test]
    fn body_only_read_has_no_top_level_event() {
        // b sources a but only reads foo inside a function body: no Read event, so
        // it falls back to the final-scope (position-blind) model.
        let files = [
            facts("/s/a.R", &["foo"], &[], vec![], None),
            facts_seq(
                "/s/b.R",
                &[],
                vec![source_path("/s/a.R", false)],
                vec![src_ev("/s/a.R")],
            ),
        ];
        let scope = build_scope(&files);
        assert_eq!(
            scope.top_level_read_binding(Path::new("/s/b.R"), "foo"),
            ReadBinding::NoTopLevelRead
        );
    }

    #[test]
    fn same_name_in_one_sourced_closure_is_order_unknown() {
        // b sources d, which sources both a and c — both define foo. Which one
        // wins is order-dependent inside the closure, so resolution gives up.
        let files = [
            facts("/s/a.R", &["foo"], &[], vec![], None),
            facts("/s/c.R", &["foo"], &[], vec![], None),
            facts(
                "/s/d.R",
                &[],
                &[],
                vec![source_path("/s/a.R", false), source_path("/s/c.R", false)],
                None,
            ),
            facts_seq(
                "/s/b.R",
                &[],
                vec![source_path("/s/d.R", false)],
                vec![src_ev("/s/d.R"), read_ev("foo")],
            ),
        ];
        let scope = build_scope(&files);
        assert_eq!(
            scope.top_level_read_binding(Path::new("/s/b.R"), "foo"),
            ReadBinding::OrderUnknown
        );
    }

    // --- per-read provenance (`top_level_read_provenance`) ---
    //
    // Spans are synthetic and arbitrary here (the replay just carries them
    // through); only their *classification* matters. `n` keeps each read span
    // distinct so a test can assert order.
    fn span(n: u32) -> TextRange {
        TextRange::new(n.into(), (n + 1).into())
    }
    fn s_read(name: &str, at: u32) -> (TopLevelEvent, Option<TextRange>) {
        (read_ev(name), Some(span(at)))
    }
    fn s_def(name: &str) -> (TopLevelEvent, Option<TextRange>) {
        (def_ev(name), None)
    }
    fn s_src(target: &str) -> (TopLevelEvent, Option<TextRange>) {
        (src_ev(target), None)
    }
    fn s_dyn() -> (TopLevelEvent, Option<TextRange>) {
        (dyn_src_ev(), None)
    }

    #[test]
    fn provenance_read_before_source_is_unbound() {
        let files = [facts("/s/a.R", &["foo"], &[], vec![], None)];
        let scope = build_scope(&files);
        let events = [s_read("foo", 1), s_src("/s/a.R")];
        assert_eq!(
            scope.top_level_read_provenance(Path::new("/s/b.R"), "foo", &events),
            vec![(span(1), ReadSite::Unbound)]
        );
    }

    #[test]
    fn provenance_read_after_source_binds_to_the_def() {
        let files = [facts("/s/a.R", &["foo"], &[], vec![], None)];
        let scope = build_scope(&files);
        let events = [s_src("/s/a.R"), s_read("foo", 1)];
        assert_eq!(
            scope.top_level_read_provenance(Path::new("/s/b.R"), "foo", &events),
            vec![(span(1), ReadSite::Bound(PathBuf::from("/s/a.R")))]
        );
    }

    #[test]
    fn provenance_local_shadow_binds_to_self() {
        let files = [facts("/s/a.R", &["foo"], &[], vec![], None)];
        let scope = build_scope(&files);
        let events = [s_src("/s/a.R"), s_def("foo"), s_read("foo", 1)];
        assert_eq!(
            scope.top_level_read_provenance(Path::new("/s/b.R"), "foo", &events),
            vec![(span(1), ReadSite::Bound(PathBuf::from("/s/b.R")))]
        );
    }

    #[test]
    fn provenance_dynamic_source_poisons_the_read() {
        let scope = build_scope(&[]);
        let events = [s_dyn(), s_read("foo", 1)];
        assert_eq!(
            scope.top_level_read_provenance(Path::new("/s/b.R"), "foo", &events),
            vec![(span(1), ReadSite::Unknown)]
        );
    }

    #[test]
    fn provenance_two_closure_definers_is_unknown() {
        // d sources both a and c, which both define foo: ambiguous which wins.
        let files = [
            facts("/s/a.R", &["foo"], &[], vec![], None),
            facts("/s/c.R", &["foo"], &[], vec![], None),
            facts(
                "/s/d.R",
                &[],
                &[],
                vec![source_path("/s/a.R", false), source_path("/s/c.R", false)],
                None,
            ),
        ];
        let scope = build_scope(&files);
        let events = [s_src("/s/d.R"), s_read("foo", 1)];
        assert_eq!(
            scope.top_level_read_provenance(Path::new("/s/b.R"), "foo", &events),
            vec![(span(1), ReadSite::Unknown)]
        );
    }

    #[test]
    fn provenance_distinguishes_pre_and_post_source_reads() {
        // The whole point: one read before the source (unbound) and one after
        // (bound) resolve *separately*, where `top_level_read_binding` would
        // collapse both into a single `OrderUnknown` refusal.
        let files = [facts("/s/a.R", &["foo"], &[], vec![], None)];
        let scope = build_scope(&files);
        let events = [s_read("foo", 1), s_src("/s/a.R"), s_read("foo", 2)];
        assert_eq!(
            scope.top_level_read_provenance(Path::new("/s/b.R"), "foo", &events),
            vec![
                (span(1), ReadSite::Unbound),
                (span(2), ReadSite::Bound(PathBuf::from("/s/a.R"))),
            ]
        );
    }

    #[test]
    fn provenance_ignores_reads_of_other_names() {
        let files = [facts("/s/a.R", &["foo"], &[], vec![], None)];
        let scope = build_scope(&files);
        let events = [s_src("/s/a.R"), s_read("other", 1), s_read("foo", 2)];
        assert_eq!(
            scope.top_level_read_provenance(Path::new("/s/b.R"), "foo", &events),
            vec![(span(2), ReadSite::Bound(PathBuf::from("/s/a.R")))]
        );
    }

    // --- final-scope binding (`final_scope_binding`) ---

    #[test]
    fn final_scope_binds_to_the_last_sourced_definer() {
        // b sources a then z, both defining foo: the final scope binds foo to z
        // (last writer wins), so b's body reads bind to z, not a.
        let files = [
            facts("/s/a.R", &["foo"], &[], vec![], None),
            facts("/s/z.R", &["foo"], &[], vec![], None),
            facts_seq(
                "/s/b.R",
                &[],
                vec![source_path("/s/a.R", false), source_path("/s/z.R", false)],
                vec![src_ev("/s/a.R"), src_ev("/s/z.R")],
            ),
        ];
        let scope = build_scope(&files);
        assert_eq!(
            scope.final_scope_binding(Path::new("/s/b.R"), "foo"),
            ReadSite::Bound(PathBuf::from("/s/z.R"))
        );
    }

    #[test]
    fn final_scope_binds_to_the_sole_sourced_definer() {
        let files = [
            facts("/s/a.R", &["foo"], &[], vec![], None),
            facts_seq(
                "/s/b.R",
                &[],
                vec![source_path("/s/a.R", false)],
                vec![src_ev("/s/a.R")],
            ),
        ];
        let scope = build_scope(&files);
        assert_eq!(
            scope.final_scope_binding(Path::new("/s/b.R"), "foo"),
            ReadSite::Bound(PathBuf::from("/s/a.R"))
        );
    }

    #[test]
    fn final_scope_ignores_a_pre_source_read() {
        // A read before the source doesn't move the final scope: still bound to a.
        let files = [
            facts("/s/a.R", &["foo"], &[], vec![], None),
            facts_seq(
                "/s/b.R",
                &[],
                vec![source_path("/s/a.R", false)],
                vec![read_ev("foo"), src_ev("/s/a.R")],
            ),
        ];
        let scope = build_scope(&files);
        assert_eq!(
            scope.final_scope_binding(Path::new("/s/b.R"), "foo"),
            ReadSite::Bound(PathBuf::from("/s/a.R"))
        );
    }

    #[test]
    fn final_scope_is_unbound_without_a_definer() {
        // No source edge or local def of foo in the file's own sequence: the def,
        // if any, comes from a package sibling's flat namespace — treated as
        // "binds to the cohort" by the rename caller.
        let files = [facts_seq("/s/b.R", &[], vec![], vec![read_ev("foo")])];
        let scope = build_scope(&files);
        assert_eq!(
            scope.final_scope_binding(Path::new("/s/b.R"), "foo"),
            ReadSite::Unbound
        );
    }

    #[test]
    fn final_scope_is_unbound_for_a_file_without_events() {
        let scope = build_scope(&[]);
        assert_eq!(
            scope.final_scope_binding(Path::new("/s/b.R"), "foo"),
            ReadSite::Unbound
        );
    }

    #[test]
    fn final_scope_is_unknown_under_a_dynamic_source() {
        let files = [facts_seq(
            "/s/b.R",
            &[],
            vec![dynamic_edge()],
            vec![dyn_src_ev()],
        )];
        let scope = build_scope(&files);
        assert_eq!(
            scope.final_scope_binding(Path::new("/s/b.R"), "foo"),
            ReadSite::Unknown
        );
    }

    #[test]
    fn final_scope_is_unknown_with_two_closure_definers() {
        // b sources d, which sources both a and c — both define foo: ambiguous.
        let files = [
            facts("/s/a.R", &["foo"], &[], vec![], None),
            facts("/s/c.R", &["foo"], &[], vec![], None),
            facts(
                "/s/d.R",
                &[],
                &[],
                vec![source_path("/s/a.R", false), source_path("/s/c.R", false)],
                None,
            ),
            facts_seq(
                "/s/b.R",
                &[],
                vec![source_path("/s/d.R", false)],
                vec![src_ev("/s/d.R")],
            ),
        ];
        let scope = build_scope(&files);
        assert_eq!(
            scope.final_scope_binding(Path::new("/s/b.R"), "foo"),
            ReadSite::Unknown
        );
    }

    #[test]
    fn final_scope_local_def_shadows_a_sourced_def() {
        // A local define of foo after the source rebinds the final scope to self.
        let files = [
            facts("/s/a.R", &["foo"], &[], vec![], None),
            facts_seq(
                "/s/b.R",
                &["foo"],
                vec![source_path("/s/a.R", false)],
                vec![src_ev("/s/a.R"), def_ev("foo")],
            ),
        ];
        let scope = build_scope(&files);
        assert_eq!(
            scope.final_scope_binding(Path::new("/s/b.R"), "foo"),
            ReadSite::Bound(PathBuf::from("/s/b.R"))
        );
    }
}
