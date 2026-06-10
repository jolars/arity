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

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use rowan::TextRange;
use smol_str::SmolStr;

use crate::incremental::{
    IncrementalDb, LibraryIndex, QueryKind, QueryLogEntry, SourceFile, file_exports,
    file_free_reads, loaded_names, source_edges,
};
use crate::project::scope::{FileFacts, FileScope, ProjectScope};
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
    let scope = graph.for_file(file.path(db));
    Visibility {
        visible: scope.visible_names().clone(),
        used_by_others: scope.used_names().clone(),
        incomplete: scope.resolution_incomplete,
    }
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
