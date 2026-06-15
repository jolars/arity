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
//! (dynamic argument, or a path outside the analyzed set), or a wholesale
//! `import(pkg)` whose exports we can't enumerate. Then
//! [`FileScope::resolution_incomplete`] is set and callers must stay
//! conservative (no `undefined-symbol` findings).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::project::source::{SourceEdgeKey, SourceTarget};
use crate::rindex::harvest::parse_namespace;

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
    /// Top-level `source()` edges this file declares (range-free).
    pub source_edges: Vec<SourceEdgeKey>,
    /// The package root this file belongs to, if any. Files sharing a root
    /// share one namespace.
    pub package_root: Option<PathBuf>,
}

/// Cross-file resolution resolved over a set of files.
#[derive(Debug, Default)]
pub struct ProjectScope {
    /// Per file: top-level names reachable from the files it can see.
    visible: HashMap<PathBuf, BTreeSet<String>>,
    /// Per file: names read by some file that can see it.
    used_by_others: HashMap<PathBuf, BTreeSet<String>>,
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
}

/// One file's view of its project.
pub struct FileScope<'a> {
    visible: &'a BTreeSet<String>,
    used_by_others: &'a BTreeSet<String>,
    /// Cross-file visibility is incomplete — an unresolved `source()` or a
    /// wholesale `import(pkg)` could supply otherwise-unresolved names — so
    /// callers must not flag them.
    pub resolution_incomplete: bool,
}

impl<'a> FileScope<'a> {
    /// Construct a view directly from borrowed visibility sets. Lets the salsa
    /// [`crate::project::Visibility`] memo back a `FileScope` without going
    /// through [`ProjectScope::for_file`].
    pub fn new(
        visible: &'a BTreeSet<String>,
        used_by_others: &'a BTreeSet<String>,
        resolution_incomplete: bool,
    ) -> Self {
        Self {
            visible,
            used_by_others,
            resolution_incomplete,
        }
    }

    /// The names visible to this file from the rest of the project.
    pub fn visible_names(&self) -> &BTreeSet<String> {
        self.visible
    }

    /// The names of this file's bindings read by some file that can see it.
    pub fn used_names(&self) -> &BTreeSet<String> {
        self.used_by_others
    }

    /// True when `name` is bound at top level in a file visible from here.
    pub fn resolves(&self, name: &str) -> bool {
        self.visible.contains(name)
    }

    /// True when `name` (a top-level binding here) is read by a file that can
    /// see this one — so it isn't unused even if unread locally.
    pub fn used_elsewhere(&self, name: &str) -> bool {
        self.used_by_others.contains(name)
    }
}

impl ProjectScope {
    /// Resolve cross-file relationships for `files`. `namespaces` maps a package
    /// root to its NAMESPACE file contents, when present.
    pub fn build(files: &[FileFacts], namespaces: &HashMap<PathBuf, String>) -> Self {
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

        // For each file, the set of *other* files it can see.
        let mut sees: HashMap<PathBuf, HashSet<PathBuf>> = HashMap::new();
        let mut dynamic: HashSet<PathBuf> = HashSet::new();
        for f in files {
            let mut seen: HashSet<PathBuf> = HashSet::new();
            if let Some(root) = &f.package_root {
                for member in &package_members[root.as_path()] {
                    if *member != f.path {
                        seen.insert(member.to_path_buf());
                    }
                }
            }

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
        }

        // Derive the two directions from `sees`.
        let mut visible: HashMap<PathBuf, BTreeSet<String>> = HashMap::new();
        let mut used_by_others: HashMap<PathBuf, BTreeSet<String>> = files
            .iter()
            .map(|f| (f.path.clone(), BTreeSet::new()))
            .collect();
        for f in files {
            let mut defs = BTreeSet::new();
            for seen in &sees[&f.path] {
                if let Some(target) = by_path.get(seen.as_path()) {
                    defs.extend(target.exports.iter().cloned());
                }
            }
            // `visible` is strictly cross-file; own bindings resolve locally.
            for name in &f.exports {
                defs.remove(name);
            }
            visible.insert(f.path.clone(), defs);

            // Every file `f` sees contributes `f`'s free reads to that file's
            // "used by others" set.
            for seen in &sees[&f.path] {
                if let Some(used) = used_by_others.get_mut(seen) {
                    used.extend(f.free_reads.iter().cloned());
                }
            }
        }

        // Fold NAMESPACE declarations into the same two directions: imported
        // names resolve (visible), exported names count as used (used_by_others),
        // and a wholesale `import(pkg)` makes resolution incomplete.
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
            let incomplete = !info.imported_packages.is_empty();

            for member in members {
                let path = member.to_path_buf();
                if let Some(used) = used_by_others.get_mut(&path) {
                    used.extend(exported.iter().cloned());
                }
                if let Some(vis) = visible.get_mut(&path) {
                    vis.extend(imported.iter().cloned());
                }
                if incomplete {
                    dynamic.insert(path);
                }
            }
        }

        Self {
            visible,
            used_by_others,
            dynamic,
            sees,
            package_siblings,
        }
    }

    /// One file's view of the project. Files not in the analyzed set get an
    /// empty, non-dynamic scope.
    pub fn for_file(&self, path: &Path) -> FileScope<'_> {
        FileScope {
            visible: self.visible.get(path).unwrap_or(&EMPTY),
            used_by_others: self.used_by_others.get(path).unwrap_or(&EMPTY),
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

/// Walk up from `path` to find an enclosing R package root: a directory with
/// both a `DESCRIPTION` file and an `R/` subdirectory. Touches the filesystem.
pub fn package_root(path: &Path) -> Option<PathBuf> {
    let mut dir = path.parent();
    while let Some(d) = dir {
        if d.join("DESCRIPTION").is_file() && d.join("R").is_dir() {
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
            source_edges: edges,
            package_root: root.map(PathBuf::from),
        }
    }

    fn names(set: &BTreeSet<String>) -> Vec<String> {
        let mut v: Vec<String> = set.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    }

    /// Build a scope with no NAMESPACE data.
    fn build_scope(files: &[FileFacts]) -> ProjectScope {
        ProjectScope::build(files, &HashMap::new())
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
        let scope = ProjectScope::build(&files, &ns);
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
        let scope = ProjectScope::build(&files, &ns);
        let a = scope.for_file(Path::new("/pkg/R/a.R"));
        assert!(a.resolves("filter"));
        assert!(!a.resolution_incomplete);
    }

    #[test]
    fn namespace_wholesale_import_marks_resolution_incomplete() {
        let files = [facts("/pkg/R/a.R", &[], &["abort"], vec![], Some("/pkg"))];
        let ns = namespaces(&[("/pkg", "import(rlang)\n")]);
        let scope = ProjectScope::build(&files, &ns);
        assert!(
            scope
                .for_file(Path::new("/pkg/R/a.R"))
                .resolution_incomplete
        );
    }
}
