//! The cross-file project scope, wrapped as tracked salsa queries.
//!
//! [`crate::project::scope`] holds the *pure* algorithm ([`ProjectScope::build`]);
//! this module wires it into salsa so a function-body edit doesn't rebuild the
//! whole project scope. The layering, from the per-file firewall up:
//!
//! - [`crate::incremental::file_exports`] / `file_free_reads` / `source_edges` —
//!   per-file projections that stay *equal* across a body edit (salsa backdates).
//! - [`project_graph`] — assembles those into the cross-file [`ProjectScope`].
//!   Keyed on the interned [`Project`] (a disk-derived membership snapshot), so
//!   an unchanged project + backdated per-file facts means its memo is reused.
//! - [`visible_symbols`] — one file's owned [`Visibility`] slice of the scope,
//!   the value the linter consumes.
//!
//! Because `project_graph` depends only on the backdated per-file queries and
//! the (re-validated) interned `Project`, editing a body re-runs neither it nor
//! `visible_symbols`. See `tests/salsa_incremental.rs`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use rowan::TextRange;
use smol_str::SmolStr;

use crate::incremental::{
    IncrementalDb, LibraryIndex, QueryKind, QueryLogEntry, SourceFile, Workspace, file_def_sites,
    file_exports, file_free_reads, loaded_names, parse_diagnostics, source_edges,
};
use crate::project::exports::DefKind;
use crate::project::scope::{FileFacts, FileScope, ProjectScope, package_root};
use crate::project::source::{SourceEdgeKey, SourceTarget};
use crate::rindex::provider::{package_indexed, resolve_origin};
use crate::semantic::symbols::{LoadedPackage, PackageOrigin};

/// One member of a project: its tracked input, on-disk path, and enclosing
/// package root (if any). Disk-derived — assembled in the lint write-phase and
/// folded into the interned [`Project`] key, so the graph queries stay pure.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ProjectMember {
    pub file: SourceFile,
    pub path: PathBuf,
    pub package_root: Option<PathBuf>,
}

/// A project as an interned membership snapshot: the set of member files plus
/// the NAMESPACE texts of the packages they belong to. Interning dedups by
/// value, so an unchanged membership yields the same id across lints (a body
/// edit doesn't change the set) and the graph memo survives. Callers must sort
/// `members` and `namespaces` for a stable, dedup-friendly key.
#[salsa::interned]
pub struct Project<'db> {
    #[returns(ref)]
    pub members: Vec<ProjectMember>,
    #[returns(ref)]
    pub namespaces: Vec<(PathBuf, String)>,
}

/// One file's owned view of its project: the names it can see, the names of its
/// own bindings used elsewhere, and whether its visibility is incomplete. Owned
/// (and `Eq`) so the salsa memo backdates when a file's visibility is unchanged.
#[derive(Debug, Default, Clone, PartialEq, Eq, salsa::Update)]
pub struct Visibility {
    pub visible: BTreeSet<String>,
    pub used_by_others: BTreeSet<String>,
    pub incomplete: bool,
}

impl Visibility {
    /// Borrow this as a [`FileScope`] for the lint rules.
    pub fn scope(&self) -> FileScope<'_> {
        FileScope::new(&self.visible, &self.used_by_others, self.incomplete)
    }
}

/// Read the `NAMESPACE` of each distinct package root among `members`, returning
/// `(root, text)` pairs sorted by root (deduped, missing files skipped). Disk
/// work, run inside [`workspace_project`]; the result becomes part of the
/// interned [`Project`] key.
pub(crate) fn read_namespaces(members: &[ProjectMember]) -> Vec<(PathBuf, String)> {
    let mut namespaces: HashMap<PathBuf, String> = HashMap::new();
    for member in members {
        if let Some(root) = &member.package_root
            && !namespaces.contains_key(root)
            && let Ok(text) = std::fs::read_to_string(root.join("NAMESPACE"))
        {
            namespaces.insert(root.clone(), text);
        }
    }
    let mut namespaces: Vec<(PathBuf, String)> = namespaces.into_iter().collect();
    namespaces.sort_by(|a, b| a.0.cmp(&b.0));
    namespaces
}

/// Derive the interned [`Project`] from the explicit [`Workspace`] file-set,
/// replacing the per-request disk walk and imperative interning. Membership is
/// the workspace's cleanly-parsing, on-disk members, sorted by path; pathless
/// in-memory files and files with parse errors are dropped (the long-standing
/// invariant — a broken file contributes nothing to cross-file scope).
///
/// Re-runs when the workspace input changes or a member's parse status flips, but
/// backdates to the *same* interned `Project` id when the derived membership is
/// unchanged, so a body edit doesn't rebuild [`project_graph`] (the existing
/// interning firewall). `package_root`/NAMESPACE reads touch disk (model (a));
/// carrying those as inputs so the query is fully pure is a follow-up.
#[salsa::tracked]
pub fn workspace_project<'db>(db: &'db dyn IncrementalDb) -> Project<'db> {
    db.record_query(QueryLogEntry {
        kind: QueryKind::WorkspaceProject,
        file: None,
    });
    let mut members: Vec<ProjectMember> = match Workspace::try_get(db) {
        Some(ws) => ws
            .members(db)
            .iter()
            .filter_map(|&file| {
                let path = file.path(db).as_deref()?.to_path_buf();
                if !parse_diagnostics(db, file).is_empty() {
                    return None;
                }
                let package_root = package_root(&path);
                Some(ProjectMember {
                    file,
                    path,
                    package_root,
                })
            })
            .collect(),
        None => Vec::new(),
    };
    members.sort_by(|a, b| a.path.cmp(&b.path));
    let namespaces = read_namespaces(&members);
    Project::new(db, members, namespaces)
}

/// The cross-file scope for `project`, built from the per-file firewall queries.
///
/// `no_eq` because its output ([`ProjectScope`]) holds `HashMap`s that aren't
/// `salsa::Update`/`Eq`-comparable here; `unsafe(non_update_types)` asserts it
/// carries no salsa references. This costs nothing for the firewall: a body edit
/// leaves the per-file inputs backdated, so this query simply isn't re-executed.
/// `no_eq` only forgoes backdating *when it does re-run* (an export actually
/// changed), and [`visible_symbols`] re-establishes per-file backdating above it.
#[salsa::tracked(returns(ref), no_eq, unsafe(non_update_types))]
pub fn project_graph<'db>(db: &'db dyn IncrementalDb, project: Project<'db>) -> ProjectScope {
    db.record_query(QueryLogEntry {
        kind: QueryKind::ProjectGraph,
        file: None,
    });

    let facts: Vec<FileFacts> = project
        .members(db)
        .iter()
        .map(|m| FileFacts {
            path: m.path.clone(),
            exports: file_exports(db, m.file).clone(),
            free_reads: file_free_reads(db, m.file).clone(),
            source_edges: source_edges(db, m.file).clone(),
            package_root: m.package_root.clone(),
        })
        .collect();

    let namespaces: HashMap<PathBuf, String> = project.namespaces(db).iter().cloned().collect();
    ProjectScope::build(&facts, &namespaces)
}

/// One file's [`Visibility`] within `project`. Depends only on [`project_graph`]
/// and the file's (stable) input path, so it backdates across body edits and
/// re-runs only when the file's actual cross-file visibility changes.
#[salsa::tracked(returns(ref))]
pub fn visible_symbols<'db>(
    db: &'db dyn IncrementalDb,
    project: Project<'db>,
    file: SourceFile,
) -> Visibility {
    db.record_query(QueryLogEntry {
        kind: QueryKind::VisibleSymbols,
        file: Some(file),
    });

    let graph = project_graph(db, project);
    // A project member always has a path; a pathless (in-memory) file never
    // enters a project, so it simply has no cross-file visibility.
    let Some(path) = file.path(db).as_deref() else {
        return Visibility::default();
    };
    let scope = graph.for_file(path);
    Visibility {
        visible: scope.visible_names().clone(),
        used_by_others: scope.used_names().clone(),
        incomplete: scope.resolution_incomplete,
    }
}

/// The reverse of the forward `source()` graph: for each statically-resolved
/// target path, the set of member files that `source()` it ("who sources me").
///
/// Deliberately broader than the forward scope builder ([`ProjectScope::build`])
/// in two ways, because file-rename and cross-file references care about the
/// *dependency*, not scope contribution:
/// - `local = TRUE` edges are **kept** (the forward builder skips them, since
///   they don't fold bindings into global scope — `src/project/scope.rs`).
/// - targets **outside** the analyzed member set are **kept** (the forward
///   builder treats them as incomplete visibility), so renaming an as-yet-
///   unopened file still finds its sourcers.
///
/// `BTreeMap`/`BTreeSet` so the type is `Eq`/`salsa::Update` and the query
/// backdates: a body edit leaves every `source_edges` unchanged (it is
/// range-free), so this re-runs only when a `source()` call is actually
/// added/removed/retargeted.
#[derive(Debug, Default, Clone, PartialEq, Eq, salsa::Update)]
pub struct ReverseSources {
    /// Target path → the member paths that `source()` it.
    pub sourced_by: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    /// Members with a `Dynamic` `source()` argument: their outgoing edge can't
    /// be resolved to a path, so they can't be recorded as a sourcer of any
    /// specific target. Tracked so a consumer knows the reverse map is partial.
    pub dynamic_sources: BTreeSet<PathBuf>,
}

/// Invert per-file forward `source()` edges into a [`ReverseSources`] map. Pure
/// over `(path, edges)` pairs so it is unit-testable without a salsa db.
fn invert_source_edges<'a>(
    members: impl IntoIterator<Item = (&'a Path, &'a [SourceEdgeKey])>,
) -> ReverseSources {
    let mut rev = ReverseSources::default();
    for (path, edges) in members {
        for edge in edges {
            match &edge.target {
                SourceTarget::Dynamic => {
                    rev.dynamic_sources.insert(path.to_path_buf());
                }
                SourceTarget::Path(target) => {
                    rev.sourced_by
                        .entry(target.clone())
                        .or_default()
                        .insert(path.to_path_buf());
                }
            }
        }
    }
    rev
}

/// The "who sources me" index for `project`, inverting every member's forward
/// `source_edges`. Keyed on the interned [`Project`] and the per-member
/// (range-free) `source_edges` firewall, so it backdates across body edits.
#[salsa::tracked(returns(ref))]
pub fn reverse_source_edges<'db>(
    db: &'db dyn IncrementalDb,
    project: Project<'db>,
) -> ReverseSources {
    db.record_query(QueryLogEntry {
        kind: QueryKind::ReverseSourceEdges,
        file: None,
    });
    invert_source_edges(
        project
            .members(db)
            .iter()
            .map(|m| (m.path.as_path(), source_edges(db, m.file).as_slice())),
    )
}

/// A project-wide name → definition-site index: for each top-level binding name,
/// the set of `(member path, kind)` it is defined at. Range-free, aggregated from
/// the per-file [`file_def_sites`] firewall, so it backdates across body edits;
/// a consumer recovers the actual span per request via
/// [`Analysis::def_range_in`](crate::incremental::Analysis::def_range_in).
///
/// This is the index that backs workspace symbols, cross-file go-to-definition
/// and references, and call hierarchy.
#[derive(Debug, Default, Clone, PartialEq, Eq, salsa::Update)]
pub struct DefIndex {
    pub by_name: BTreeMap<String, BTreeSet<(PathBuf, DefKind)>>,
}

/// Aggregate every member's [`file_def_sites`] into the project-wide
/// [`DefIndex`]. Keyed on the interned [`Project`] and the per-file firewall, so
/// it backdates across body edits and re-runs only when some file's top-level
/// definitions change.
#[salsa::tracked(returns(ref))]
pub fn project_defs<'db>(db: &'db dyn IncrementalDb, project: Project<'db>) -> DefIndex {
    db.record_query(QueryLogEntry {
        kind: QueryKind::ProjectDefs,
        file: None,
    });
    let mut index = DefIndex::default();
    for member in project.members(db) {
        for (name, kind) in file_def_sites(db, member.file) {
            index
                .by_name
                .entry(name.clone())
                .or_default()
                .insert((member.path.clone(), *kind));
        }
    }
    index
}

/// The free-read names in `file` that resolve to nothing — neither a sibling /
/// `source()`-closure binding (cross-file visibility) nor any attached package
/// (default, harvested, or bundled). These are the `undefined-symbol`
/// candidates, keyed by name (range-free) so the memo backdates across body
/// edits. Empty when the rule's conservative gates trip — an attached package
/// whose exports are unknown, or incomplete cross-file visibility — since either
/// could supply the otherwise-unresolved names.
#[derive(Debug, Default, Clone, PartialEq, Eq, salsa::Update)]
pub struct ExternalResolution {
    pub unresolved: BTreeSet<String>,
}

/// Resolve a file's free reads against the project graph and the
/// HIGH-durability [`LibraryIndex`], yielding the `undefined-symbol` candidate
/// names.
///
/// The library index is set at `Durability::HIGH`, and every other dependency
/// ([`file_free_reads`], [`loaded_names`], [`visible_symbols`]) is an `Eq`
/// firewall projection that backdates on a body edit. So a keystroke that leaves
/// the free-read / loaded / visibility sets unchanged re-runs neither this query
/// nor any masking work: salsa skips the HIGH library subgraph in a single
/// version-vector compare. The result is range-free — the rule re-attaches
/// diagnostic spans from the fresh [`semantic_model`](crate::incremental::semantic_model)
/// and re-applies the per-occurrence local-binding check, so a name bound in one
/// scope but free in another is handled correctly.
#[salsa::tracked(returns(ref))]
pub fn external_resolution<'db>(
    db: &'db dyn IncrementalDb,
    manifest: LibraryIndex,
    project: Project<'db>,
    file: SourceFile,
) -> ExternalResolution {
    db.record_query(QueryLogEntry {
        kind: QueryKind::ExternalResolution,
        file: Some(file),
    });

    let index: &crate::rindex::provider::IndexedProvider = manifest.data(db);
    let loaded = loaded_names(db, file);

    // Gate: an attached package whose exports we don't fully know could define
    // any of the unresolved names — suppress the whole file.
    if loaded.iter().any(|pkg| !package_indexed(index, pkg)) {
        return ExternalResolution::default();
    }

    let visibility = visible_symbols(db, project, file);
    // Gate: incomplete cross-file visibility (an unresolved `source()` or a
    // wholesale `import(pkg)`) could supply otherwise-unresolved names.
    if visibility.incomplete {
        return ExternalResolution::default();
    }

    // Resolution only asks whether a name resolves to *some* attached package, so
    // load order is irrelevant here; rebuild lightweight `LoadedPackage`s (the
    // ranges are unused by `resolve_origin`).
    let loaded_pkgs: Vec<LoadedPackage> = loaded
        .iter()
        .map(|name| LoadedPackage {
            name: SmolStr::new(name),
            range: TextRange::default(),
        })
        .collect();

    let unresolved = file_free_reads(db, file)
        .iter()
        .filter(|name| !visibility.visible.contains(name.as_str()))
        .filter(|name| {
            matches!(
                resolve_origin(index, name, &loaded_pkgs),
                PackageOrigin::Unknown
            )
        })
        .cloned()
        .collect();

    ExternalResolution { unresolved }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::source::{SourceEdgeKey, SourceTarget};

    fn path_edge(target: &str, local: bool) -> SourceEdgeKey {
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

    fn invert(members: &[(&str, Vec<SourceEdgeKey>)]) -> ReverseSources {
        invert_source_edges(
            members
                .iter()
                .map(|(p, edges)| (Path::new(*p), edges.as_slice())),
        )
    }

    fn sourcers<'a>(rev: &'a ReverseSources, target: &str) -> Vec<&'a str> {
        rev.sourced_by
            .get(Path::new(target))
            .into_iter()
            .flat_map(|set| set.iter().map(|p| p.to_str().unwrap()))
            .collect()
    }

    #[test]
    fn single_edge_inverts() {
        let rev = invert(&[
            ("/s/a.R", vec![path_edge("/s/b.R", false)]),
            ("/s/b.R", vec![]),
        ]);
        assert_eq!(sourcers(&rev, "/s/b.R"), vec!["/s/a.R"]);
        // The sourcer itself is never keyed as a target.
        assert!(!rev.sourced_by.contains_key(Path::new("/s/a.R")));
        assert!(rev.dynamic_sources.is_empty());
    }

    #[test]
    fn multiple_sourcers_aggregate() {
        let rev = invert(&[
            ("/s/a.R", vec![path_edge("/s/c.R", false)]),
            ("/s/b.R", vec![path_edge("/s/c.R", false)]),
        ]);
        assert_eq!(sourcers(&rev, "/s/c.R"), vec!["/s/a.R", "/s/b.R"]);
    }

    #[test]
    fn local_edge_is_retained() {
        // Unlike the forward scope builder, a local=TRUE edge is still a file
        // dependency the reverse map records.
        let rev = invert(&[("/s/a.R", vec![path_edge("/s/b.R", true)])]);
        assert_eq!(sourcers(&rev, "/s/b.R"), vec!["/s/a.R"]);
    }

    #[test]
    fn dynamic_edge_recorded_separately() {
        let rev = invert(&[("/s/a.R", vec![dynamic_edge()])]);
        assert!(rev.sourced_by.is_empty());
        assert!(rev.dynamic_sources.contains(Path::new("/s/a.R")));
    }

    #[test]
    fn target_outside_member_set_is_retained() {
        // /s/gen.R is not itself a member, but its sourcer is still recorded.
        let rev = invert(&[("/s/a.R", vec![path_edge("/s/gen.R", false)])]);
        assert_eq!(sourcers(&rev, "/s/gen.R"), vec!["/s/a.R"]);
    }
}
