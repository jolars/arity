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
use std::sync::LazyLock;

use rowan::TextRange;

use crate::project::source::{SourceEdgeKey, SourceTarget, TopLevelEvent};
use crate::rindex::harvest::parse_namespace;
use crate::semantic::symbols::unbacktick;

static EMPTY: BTreeSet<String> = BTreeSet::new();
// `HashSet::new` isn't `const` (its hasher state isn't const-constructible), so
// unlike `EMPTY` this needs lazy init.
static EMPTY_PATHS: LazyLock<HashSet<PathBuf>> = LazyLock::new(HashSet::new);

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
    visible: HashMap<PathBuf, BTreeSet<String>>,
    /// Per file: names *read* by some file that can see it. Reads only — the
    /// NAMESPACE-export contribution lives in `namespace_exports`, so a caller
    /// can ask "does a sibling call this?" separately from "is this public
    /// API?".
    read_by_others: HashMap<PathBuf, BTreeSet<String>>,
    /// Per package file: the names its package's NAMESPACE `export()`s. Both
    /// sets feed [`FileScope::used_elsewhere`]; only this one marks public API.
    namespace_exports: HashMap<PathBuf, BTreeSet<String>>,
    /// Per package file: the subset of `namespace_exports` registered as S3
    /// methods (`S3method(...)`). Reached by dispatch, never by a direct call.
    s3_methods: HashMap<PathBuf, BTreeSet<String>>,
    /// Per file: the packages its NAMESPACE `import()`s wholesale. Recorded
    /// rather than resolved — see [`FileScope::wildcard_import_packages`].
    wildcard_imports: HashMap<PathBuf, BTreeSet<String>>,
    /// Files whose cross-file visibility is incomplete (unresolved `source()`).
    dynamic: HashSet<PathBuf>,
    /// Per file: the set of *other* files it can see (package siblings, plus the
    /// transitive non-local `source()` closure). Directional: `a` sourcing `b`
    /// puts `b` in `sees[a]` but not the reverse. The raw reachability relation
    /// `visible`/`used_by_others` are derived from; retained so scope-aware
    /// cross-file resolution (rename/references) can partition by visibility
    /// component. Span-free, so it stays body-edit-stable.
    sees: HashMap<PathBuf, HashSet<PathBuf>>,
    /// Per package file: its co-members under the same package root (excluding
    /// itself). Package siblings share one *flat* namespace, so two siblings
    /// defining the same top-level name are the same binding slot (a
    /// redefinition) — unlike `source()` edges, which only make a name *visible*
    /// and shadow by order. Absent for non-package files. Span-free.
    package_siblings: HashMap<PathBuf, HashSet<PathBuf>>,
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
    visible: &'a BTreeSet<String>,
    read_by_others: &'a BTreeSet<String>,
    namespace_exports: &'a BTreeSet<String>,
    s3_methods: &'a BTreeSet<String>,
    /// Packages this file's NAMESPACE `import()`s wholesale.
    ///
    /// Every export of such a package is in scope here, which for resolution is
    /// indistinguishable from being attached. Whether those exports are
    /// *enumerable* is a question only the library index can answer, and
    /// [`ProjectScope::build`] is pure — so the packages are reported and the
    /// verdict is left to the caller, which holds the index. Reintroducing an
    /// index lookup inside `build` would look like a simplification and would
    /// silently break the purity the project-graph memo depends on.
    wildcard_imports: &'a BTreeSet<String>,
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
        visible: &'a BTreeSet<String>,
        read_by_others: &'a BTreeSet<String>,
        namespace_exports: &'a BTreeSet<String>,
        s3_methods: &'a BTreeSet<String>,
        wildcard_imports: &'a BTreeSet<String>,
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

    /// The names visible to this file from the rest of the project.
    pub fn visible_names(&self) -> &BTreeSet<String> {
        self.visible
    }

    /// The names of this file's bindings actually *read* by some file that can
    /// see it. Excludes the NAMESPACE-export contribution — see
    /// [`namespace_export_names`](Self::namespace_export_names).
    pub fn read_names(&self) -> &BTreeSet<String> {
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

        // Each package file's co-members (excluding itself), for the flat
        // shared-namespace relation that aliasing/conflict detection needs.
        let mut package_siblings: HashMap<PathBuf, HashSet<PathBuf>> = HashMap::new();
        for members in package_members.values() {
            for &member in members {
                let siblings = members
                    .iter()
                    .filter(|&&other| other != member)
                    .map(|&other| other.to_path_buf())
                    .collect();
                package_siblings.insert(member.to_path_buf(), siblings);
            }
        }

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
        let mut sees: HashMap<PathBuf, HashSet<PathBuf>> = HashMap::new();
        let mut sourced: Vec<Vec<&Path>> = Vec::with_capacity(files.len());
        let mut dynamic: HashSet<PathBuf> = HashSet::new();
        let mut wildcard_imports: HashMap<PathBuf, BTreeSet<String>> = HashMap::new();
        for f in files {
            let mut seen: HashSet<PathBuf> = HashSet::new();
            if let Some(root) = &f.package_root {
                for member in &package_members[root.as_path()] {
                    if *member != f.path {
                        seen.insert(member.to_path_buf());
                    }
                }
            }

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
                                seen.insert(target.path.clone());
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
            sees.insert(f.path.clone(), seen);
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

        // Derive the two directions from `sees`.
        let mut visible: HashMap<PathBuf, BTreeSet<String>> = HashMap::new();
        let mut read_by_others: HashMap<PathBuf, BTreeSet<String>> = files
            .iter()
            .map(|f| (f.path.clone(), BTreeSet::new()))
            .collect();
        let mut namespace_exports: HashMap<PathBuf, BTreeSet<String>> = HashMap::new();
        let mut s3_methods: HashMap<PathBuf, BTreeSet<String>> = HashMap::new();
        for (i, f) in files.iter().enumerate() {
            // Starting from the whole package's exports rather than the
            // siblings' is the same set: the member's own exports are removed
            // again below, which is also what makes `visible` strictly
            // cross-file — own bindings resolve locally.
            let mut defs = match f
                .package_root
                .as_deref()
                .and_then(|r| package_exports.get(r))
            {
                Some(union) => union.clone(),
                None => BTreeSet::new(),
            };
            for target in &sourced[i] {
                if let Some(target) = by_path.get(target) {
                    defs.extend(target.exports.iter().cloned());
                }
            }
            for name in &f.exports {
                defs.remove(name);
            }
            visible.insert(f.path.clone(), defs);
        }

        // Every file `f` sees contributes `f`'s reads to that file's "used by
        // others" set. For a package member that is every sibling, so take the
        // package-wide union and drop only the names *this* member alone reads:
        // a name with two or more readers survives the exclusion of any single
        // one, and a name with one reader survives unless that reader is us.
        for (i, f) in files.iter().enumerate() {
            let Some(counts) = f
                .package_root
                .as_deref()
                .and_then(|r| package_read_counts.get(r))
            else {
                continue;
            };
            let used = read_by_others
                .get_mut(&f.path)
                .expect("every file is seeded above");
            for (&name, &count) in counts {
                if count > 1 || !reads[i].contains(name) {
                    used.insert(name.to_string());
                }
            }
        }
        for i in 0..files.len() {
            for target in &sourced[i] {
                if let Some(used) = read_by_others.get_mut(*target) {
                    used.extend(reads[i].iter().cloned());
                }
            }
        }

        // Fold NAMESPACE declarations into the same two directions: imported
        // names resolve (visible) and exported names count as used (via
        // `namespace_exports`). A wholesale `import(pkg)` is *recorded*, not
        // resolved: whether pkg's exports are enumerable needs the library
        // index, which this pure builder does not have.
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
            let exported: BTreeSet<String> = info.exports.iter().cloned().collect();
            let imported: BTreeSet<String> = info.imported_names.iter().cloned().collect();
            let wildcards: BTreeSet<String> = info.imported_packages.iter().cloned().collect();

            for member in members {
                let path = member.to_path_buf();
                namespace_exports
                    .entry(path.clone())
                    .or_default()
                    .extend(exported.iter().cloned());
                s3_methods
                    .entry(path.clone())
                    .or_default()
                    .extend(info.s3_methods.iter().cloned());
                if let Some(vis) = visible.get_mut(&path) {
                    vis.extend(imported.iter().cloned());
                }
                if !wildcards.is_empty() {
                    wildcard_imports
                        .entry(path)
                        .or_default()
                        .extend(wildcards.iter().cloned());
                }
            }
        }

        // `useDynLib()` binds native routines in the package namespace, so a
        // reference to one resolves anywhere in the package — not only in a
        // `.Call()` head. Nothing in the R sources defines them, which is why
        // they are injected here rather than reached through `sees`.
        for (root, routines) in native_routines {
            let Some(members) = package_members.get(root.as_path()) else {
                continue;
            };
            for member in members {
                if let Some(vis) = visible.get_mut(*member) {
                    vis.extend(routines.iter().cloned());
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
            sees,
            package_siblings,
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
            visible: self.visible.get(path).unwrap_or(&EMPTY),
            read_by_others: self.read_by_others.get(path).unwrap_or(&EMPTY),
            namespace_exports: self.namespace_exports.get(path).unwrap_or(&EMPTY),
            s3_methods: self.s3_methods.get(path).unwrap_or(&EMPTY),
            wildcard_imports: self.wildcard_imports.get(path).unwrap_or(&EMPTY),
            resolution_incomplete: self.dynamic.contains(path),
        }
    }

    /// The set of *other* files `path` can see (package siblings + non-local
    /// `source()` closure). Directional. Empty for files outside the analyzed
    /// set.
    pub fn sees(&self, path: &Path) -> &HashSet<PathBuf> {
        match self.sees.get(path) {
            Some(seen) => seen,
            None => &EMPTY_PATHS,
        }
    }

    /// The inverse of [`sees`](Self::sees): the files that can see `path` (i.e.
    /// resolve `path`'s top-level bindings). For renaming a binding defined in
    /// `path`, these are the files whose reads can bind to it.
    pub fn seen_by(&self, path: &Path) -> HashSet<PathBuf> {
        self.sees
            .iter()
            .filter(|(_, seen)| seen.contains(path))
            .map(|(p, _)| p.clone())
            .collect()
    }

    /// The package co-members of `path` (excluding itself), which share one flat
    /// namespace with it. Empty for non-package files. Unlike [`sees`](Self::sees),
    /// this is the *aliasing* relation: two siblings defining the same top-level
    /// name are the same binding slot.
    pub fn package_siblings(&self, path: &Path) -> &HashSet<PathBuf> {
        match self.package_siblings.get(path) {
            Some(siblings) => siblings,
            None => &EMPTY_PATHS,
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
    let mut dir = path.parent();
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

    fn names(set: &BTreeSet<String>) -> Vec<String> {
        let mut v: Vec<String> = set.iter().map(|s| s.to_string()).collect();
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
            names(scope.for_file(Path::new("/s/a.R")).visible),
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
