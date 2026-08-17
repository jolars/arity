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
use std::sync::Arc;

use rowan::TextRange;
use smol_str::SmolStr;

use crate::incremental::{
    Descriptions, IncrementalDb, LibraryIndex, PackageGraph, QueryKind, QueryLogEntry, SourceFile,
    Workspace, description_facts, file_class_defs, file_def_sites, file_exports, file_free_reads,
    file_qualified_reads, file_roxygen_topics, loaded_names, package_references, parse_diagnostics,
    source_edges, top_level_events,
};
use crate::project::classes::ClassSystem;
use crate::project::description::DescriptionFacts;
use crate::project::exports::DefKind;
use crate::project::native::dynlib_bound_names;
use crate::project::roxygen::TopicMember;
use crate::project::scope::{FileFacts, FileScope, LayeredSet, ProjectScope, package_root_of_dir};
use crate::project::source::{SourceEdgeKey, SourceTarget};
use crate::rindex::provider::{attach_members, package_indexed, resolve_origin};
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

/// One package root's collation/completeness verdict: whether the analyzed
/// member set covers every R source file the package will load. Disk-derived
/// (like the NAMESPACE texts) and frozen into the interned [`Project`], so the
/// graph queries stay pure and backdate across body edits.
///
/// `complete == false` means a top-level def or read of a name could hide in an
/// `R/*.[RrSsQq]` file we never analyzed (a dropped parse-error member, an
/// unopened sibling, or a `Collate:` entry outside the set), so a multi-def
/// cross-file rename over this package must refuse rather than half-rewrite the
/// flat namespace.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageCollation {
    pub root: PathBuf,
    pub complete: bool,
}

/// One package root's disk-derived metadata, carried as part of the
/// [`PackageGraph`](crate::incremental::PackageGraph) salsa input so
/// [`workspace_project`] can read it without touching the filesystem. Discovered
/// in the write-phase by [`discover_packages`] (the sole disk reader), refreshed
/// in lockstep with workspace membership, and a future `didChangeWatchedFiles`
/// watcher's invalidation target.
#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct PackageInfo {
    /// The package root: a directory with `DESCRIPTION` + `R/` (see
    /// [`package_root`]).
    pub root: PathBuf,
    /// The root's `NAMESPACE` text, or `None` when the file is absent.
    pub namespace: Option<String>,
    /// The R source *basenames* on disk under `R/` ([`r_dir_sources`]).
    ///
    /// The `Collate:` half of the expected source set deliberately lives in
    /// [`description_facts`](crate::incremental::description_facts) instead: a
    /// `Collate` edit then reaches the graph through the backdating firewall,
    /// while an unrelated `DESCRIPTION` edit does not reach it at all.
    pub dir_sources: BTreeSet<String>,
    /// The names this root's `useDynLib()` directives bind, resolved against
    /// `src/` where the NAMESPACE only declares `.registration = TRUE`
    /// ([`dynlib_bound_names`]). Empty for a package with no `useDynLib()`.
    ///
    /// Discovered here rather than derived in [`project_graph`] because it needs
    /// disk, which that query must not touch.
    pub native_routines: BTreeSet<String>,
}

/// One package root's *resolution-relevant* declarations, frozen into the
/// interned [`Project`] so the graph queries stay pure.
///
/// Deliberately a strict subset of
/// [`DescriptionFacts`](crate::project::DescriptionFacts): the compat floors and
/// the `Roxygen` field affect no cross-file scope, so keeping them out means a
/// `RoxygenNote:` bump re-interns to the *same* `Project` id and the
/// [`project_graph`] memo survives it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageDeclarations {
    pub root: PathBuf,
    /// `Depends`, minus `R` — the packages R **attaches** to the search path
    /// when this package loads, so their exports resolve as bare names.
    pub attached: BTreeSet<String>,
    /// Every declared package, any field. *Referenced* — worth harvesting and
    /// fetching — but **not** attached: an `Imports` package is reachable only
    /// through `pkg::` or a NAMESPACE `importFrom`/`import`.
    pub declared: BTreeSet<String>,
}

/// A project as an interned membership snapshot: the set of member files, the
/// NAMESPACE texts of the packages they belong to, each package root's
/// collation/completeness verdict, its declared dependencies, and the native
/// routines it binds. Interning dedups by value, so an unchanged membership
/// yields the same id across lints (a body edit doesn't change the set) and the
/// graph memo survives. Callers must sort every field for a stable,
/// dedup-friendly key.
#[salsa::interned]
pub struct Project<'db> {
    #[returns(ref)]
    pub members: Vec<ProjectMember>,
    #[returns(ref)]
    pub namespaces: Vec<(PathBuf, String)>,
    #[returns(ref)]
    pub collations: Vec<PackageCollation>,
    #[returns(ref)]
    pub declarations: Vec<PackageDeclarations>,
    /// Per package root, the names its `useDynLib()` binds
    /// ([`PackageInfo::native_routines`]). Disk-derived like `namespaces`, and
    /// frozen here for the same reason: [`project_graph`] resolves against them
    /// without touching disk.
    #[returns(ref)]
    pub native_routines: Vec<(PathBuf, BTreeSet<String>)>,
}

/// One file's owned view of its project: the names it can see, the names of its
/// own bindings used elsewhere, and whether its visibility is incomplete. Owned
/// (and `Eq`) so the salsa memo backdates when a file's visibility is unchanged.
#[derive(Debug, Default, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct Visibility {
    /// Held as layers, so the package-wide union this file shares with every
    /// sibling is one `Arc` handle rather than a per-member copy.
    pub visible: LayeredSet,
    /// Names of this file's bindings *read* by a file that can see it.
    pub read_by_others: LayeredSet,
    /// Names this file's package `export()`s (public API). Kept apart from
    /// `read_by_others` so a rule can distinguish "a sibling calls it" from
    /// "it is exported" — see [`FileScope::namespace_export_names`].
    ///
    /// One NAMESPACE's sets are the same for every member of the package, so
    /// these three are shared handles rather than per-file copies. `Arc`
    /// equality is by *contents* (the pointer check is only a fast path for
    /// `true`), so the memo still backdates when a rebuilt set is unchanged.
    pub namespace_exports: Arc<BTreeSet<String>>,
    /// The subset of `namespace_exports` registered via `S3method()` — reached
    /// by dispatch, so no direct call to the name is expected.
    pub s3_methods: Arc<BTreeSet<String>>,
    /// Packages this file's NAMESPACE `import()`s wholesale. Range-free and
    /// `Eq`, so this projection still backdates across a body edit.
    pub wildcard_imports: Arc<BTreeSet<String>>,
    /// An unresolved `source()` left cross-file visibility incomplete. A
    /// wildcard import is *not* expressed here — it is an entry in
    /// `wildcard_imports` that the consumer (which holds the library index)
    /// finds unenumerable.
    pub incomplete: bool,
}

impl Visibility {
    /// Borrow this as a [`FileScope`] for the lint rules.
    pub fn scope(&self) -> FileScope<'_> {
        FileScope::new(
            &self.visible,
            &self.read_by_others,
            &self.namespace_exports,
            &self.s3_methods,
            &self.wildcard_imports,
            self.incomplete,
        )
    }
}

/// Discover the [`PackageInfo`] for every distinct package root among
/// `member_paths`, sorted by root. **The sole filesystem reader** behind the
/// project graph: it walks each member to its [`package_root`] and, per root,
/// reads `NAMESPACE` and lists [`r_dir_sources`]. It deliberately does *not*
/// read `DESCRIPTION` — that file is its own tracked input
/// ([`DescriptionFile`](crate::incremental::DescriptionFile)), so its facts
/// reach the graph through a query that backdates. Run in the write-phase
/// (see [`refresh_package_graph`](crate::incremental::IncrementalDatabase::refresh_package_graph));
/// the result is stored in the [`PackageGraph`](crate::incremental::PackageGraph)
/// input so [`workspace_project`] stays pure.
pub fn discover_packages(member_paths: &[PathBuf]) -> Vec<PackageInfo> {
    // Dedup by parent directory before walking up. The root walk reads only
    // `path.parent()`, so asking once per *file* re-walks one directory chain
    // once per sibling — 86 walks for a package whose members share one `R/`,
    // each stat-ing two entries per ancestor. The root set is identical.
    let roots: BTreeSet<PathBuf> = member_paths
        .iter()
        .filter_map(|p| p.parent())
        .collect::<BTreeSet<&Path>>()
        .into_iter()
        .filter_map(package_root_of_dir)
        .collect();
    roots
        .into_iter()
        .map(|root| {
            let namespace = std::fs::read_to_string(root.join("NAMESPACE")).ok();
            PackageInfo {
                native_routines: namespace
                    .as_deref()
                    .map(|text| dynlib_bound_names(text, &root))
                    .unwrap_or_default(),
                namespace,
                dir_sources: r_dir_sources(&root),
                root,
            }
        })
        .collect()
}

/// The enclosing package root of `path` among the already-discovered `roots`:
/// the deepest ancestor present in the set, or `None`. The pure counterpart of
/// [`package_root`] — it touches no disk, reproducing the disk walk exactly
/// because `roots` was built by [`discover_packages`] running [`package_root`]
/// over the same member paths.
fn package_root_in(path: &Path, roots: &BTreeSet<&PathBuf>) -> Option<PathBuf> {
    let mut dir = path.parent();
    while let Some(d) = dir {
        if roots.contains(&d.to_path_buf()) {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

/// The extensions R sources from a package's `R/` directory: `.[RrSsQq]`. Note
/// this is broader than [`crate::file_discovery`]'s `.r`/`.R`-only filter — an
/// `.S`/`.q` member that discovery never surfaced must still count toward the
/// package's expected source set, or completeness would silently pass over it.
const R_SOURCE_EXTS: [&str; 6] = ["R", "r", "S", "s", "Q", "q"];

/// The R source file *names* (basenames within `R/`) a package at `root` will
/// load: the union of the on-disk `R/*.[RrSsQq]` listing and any `Collate:`
/// entries from `DESCRIPTION`. Union (not intersection) so neither a stale
/// `Collate:` nor an unlisted file can shrink the expected set and let an
/// incomplete package pass. Touches disk.
/// [`r_dir_sources`] already unions the `Collate:` entries in, so this adds
/// nothing to what it returns — re-reading and re-parsing `DESCRIPTION` here
/// only paid for the same union twice.
pub fn expected_r_sources(root: &Path) -> BTreeSet<String> {
    r_dir_sources(root)
}

/// The R source *basenames* a package at `root` will load: the `R/*.[RrSsQq]`
/// listing on disk, unioned with the `Collate:` entries from its `DESCRIPTION`.
pub fn r_dir_sources(root: &Path) -> BTreeSet<String> {
    let mut expected: BTreeSet<String> = BTreeSet::new();
    if let Ok(entries) = std::fs::read_dir(root.join("R")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| R_SOURCE_EXTS.contains(&e))
                && path.is_file()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                expected.insert(name.to_string());
            }
        }
    }
    expected.extend(crate::project::description::facts_at(root).collate);
    expected
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
/// interning firewall). The query is **pure** (no disk): the per-root NAMESPACE
/// texts, expected-source sets, and package-root markers come from the
/// [`PackageGraph`](crate::incremental::PackageGraph) input (populated in the
/// write-phase by [`discover_packages`]), so a keystroke re-run does only
/// in-memory work and a watcher can invalidate disk changes correctly.
#[salsa::tracked(returns(copy))]
pub fn workspace_project<'db>(db: &'db dyn IncrementalDb) -> Project<'db> {
    db.record_query(QueryLogEntry {
        kind: QueryKind::WorkspaceProject,
        file: None,
    });
    let packages = PackageGraph::try_get(db);
    let infos: &[PackageInfo] = packages.as_ref().map_or(&[], |g| g.packages(db));
    let roots: BTreeSet<&PathBuf> = infos.iter().map(|p| &p.root).collect();

    let mut members: Vec<ProjectMember> = match Workspace::try_get(db) {
        Some(ws) => ws
            .members(db)
            .iter()
            .filter_map(|&file| {
                let path = file.path(db).as_deref()?.to_path_buf();
                if !parse_diagnostics(db, file).is_empty() {
                    return None;
                }
                let package_root = package_root_in(&path, &roots);
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

    // NAMESPACE texts and completeness verdicts, restricted to the roots that
    // actually have a member here — the same shapes the old disk passes built.
    let by_root: HashMap<&PathBuf, &PackageInfo> = infos.iter().map(|p| (&p.root, p)).collect();
    let mut analyzed: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    for member in &members {
        if let Some(root) = &member.package_root
            && let Some(name) = member.path.file_name().and_then(|n| n.to_str())
        {
            analyzed
                .entry(root.clone())
                .or_default()
                .insert(name.to_string());
        }
    }
    let mut namespaces: Vec<(PathBuf, String)> = analyzed
        .keys()
        .filter_map(|root| {
            by_root
                .get(root)
                .and_then(|info| info.namespace.clone())
                .map(|text| (root.clone(), text))
        })
        .collect();
    namespaces.sort_by(|a, b| a.0.cmp(&b.0));
    // Same restriction and ordering as `namespaces` — a root only enters the
    // key when it has an analyzed member. Roots that bind nothing are dropped so
    // the common (no `useDynLib`) case leaves the key untouched.
    let mut native_routines: Vec<(PathBuf, BTreeSet<String>)> = analyzed
        .keys()
        .filter_map(|root| {
            by_root
                .get(root)
                .map(|info| info.native_routines.clone())
                .filter(|routines| !routines.is_empty())
                .map(|routines| (root.clone(), routines))
        })
        .collect();
    native_routines.sort_by(|a, b| a.0.cmp(&b.0));
    // The expected source set is the `R/` listing (from the graph input) union
    // the `Collate:` entries (from the DESCRIPTION facts) — union, not
    // intersection, so neither a stale `Collate:` nor an unlisted file can
    // shrink it and let an incomplete package pass.
    let facts_by_root = description_facts_by_root(db);
    let collations: Vec<PackageCollation> = analyzed
        .into_iter()
        .map(|(root, analyzed_names)| {
            let complete = by_root.get(&root).is_some_and(|info| {
                let collate = facts_by_root
                    .get(&root)
                    .map(|facts| facts.collate.iter())
                    .into_iter()
                    .flatten();
                info.dir_sources
                    .iter()
                    .chain(collate)
                    .all(|name| analyzed_names.contains(name))
            });
            PackageCollation { root, complete }
        })
        .collect();

    // Restricted to roots with an analyzed member, like `namespaces`, and
    // sorted for a stable interning key.
    let mut declarations: Vec<PackageDeclarations> = collations
        .iter()
        .filter_map(|c| {
            facts_by_root.get(&c.root).map(|facts| PackageDeclarations {
                root: c.root.clone(),
                attached: facts.attached_packages(),
                declared: facts.declared_packages(),
            })
        })
        .collect();
    declarations.sort_by(|a, b| a.root.cmp(&b.root));

    Project::new(
        db,
        members,
        namespaces,
        collations,
        declarations,
        native_routines,
    )
}

/// The [`DescriptionFacts`] of the package enclosing `path`, from the tracked
/// `DESCRIPTION` inputs. `None` for a file outside any tracked package.
///
/// Pure — it touches no disk, which is the point: the lint rules used to walk
/// to the package root (stat-ing for `DESCRIPTION` and `R/`) and then re-read
/// and re-parse `DESCRIPTION`, per file and per question. The root is resolved
/// against the *tracked* roots via [`package_root_in`], the same disk-free walk
/// [`workspace_project`] uses.
pub fn package_facts_for<'db>(
    db: &'db dyn IncrementalDb,
    path: &Path,
) -> Option<&'db DescriptionFacts> {
    let set = Descriptions::try_get(db)?;
    let files = set.files(db);
    let roots: BTreeSet<&PathBuf> = files.iter().map(|file| file.root(db)).collect();
    let root = package_root_in(path, &roots)?;
    files
        .iter()
        .find(|file| file.root(db) == &root)
        .map(|&file| description_facts(db, file))
}

/// One package root's dependency-usage verdict: every package its own code
/// reaches, and whether the run is entitled to conclude anything from a
/// package's *absence* from that set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageUsage {
    /// Every package this root's R sources, NAMESPACE, and roxygen tags reach.
    pub used: BTreeSet<String>,
    /// Package-shaped string literals anywhere in the root's sources. Not a
    /// reference — the guard a consumer consults so a dynamic use cannot become
    /// a false report of disuse
    /// ([`PackageReferences::string_mentions`](crate::project::PackageReferences::string_mentions)).
    pub mentioned: BTreeSet<String>,
    /// **The guard that makes reporting on absence safe.** `true` only when the
    /// run actually analyzed every R source this package will load, read its
    /// NAMESPACE, and found at least one source.
    ///
    /// Without it, `arity lint R/one.R` would declare every dependency the
    /// other files use unused. In practice single-file runs are still complete
    /// — the driver adds a package's whole expected `R/` set as scope-only
    /// members — so this is the backstop for an unrelated directory, for a
    /// package with a parse error in `R/`, and for a package mid-`document()`
    /// with no NAMESPACE yet.
    pub complete: bool,
}

/// Every tracked package root's [`PackageUsage`], keyed by root.
///
/// Keyed on the interned [`Project`] and folded from the backdated per-file
/// [`package_references`], so it backdates across a body edit exactly like
/// [`project_graph`]: editing inside a function changes neither the packages a
/// file names nor the membership, so this is not re-executed.
#[salsa::tracked(returns(ref))]
pub fn package_usage<'db>(
    db: &'db dyn IncrementalDb,
    project: Project<'db>,
) -> BTreeMap<PathBuf, PackageUsage> {
    db.record_query(QueryLogEntry {
        kind: QueryKind::PackageUsage,
        file: None,
    });

    let mut usage: BTreeMap<PathBuf, PackageUsage> = BTreeMap::new();
    let mut sources: BTreeMap<PathBuf, usize> = BTreeMap::new();
    for member in project.members(db) {
        let Some(root) = member.package_root.clone() else {
            continue;
        };
        let refs = package_references(db, member.file);
        let entry = usage.entry(root.clone()).or_default();
        entry.used.extend(refs.direct.iter().cloned());
        entry.used.extend(refs.roxygen_imports.iter().cloned());
        entry.mentioned.extend(refs.string_mentions.iter().cloned());
        if refs.uses_methods {
            // R's own `uses_methods`: an S4 or reference class makes
            // `Imports: methods` mandatory with nothing naming it.
            entry.used.insert("methods".to_string());
        }
        *sources.entry(root).or_default() += 1;
    }

    // A NAMESPACE `import()`/`importFrom()` reaches its package as surely as a
    // `::` does, and for many packages it is the *only* thing that does.
    for (root, namespace) in project.namespaces(db) {
        let info = crate::rindex::harvest::parse_namespace(namespace, &[]);
        let entry = usage.entry(root.clone()).or_default();
        entry.used.extend(info.imported_packages);
        entry.used.extend(info.imported_from_packages);
    }

    let complete_roots: BTreeSet<&PathBuf> = project
        .collations(db)
        .iter()
        .filter(|collation| collation.complete)
        .map(|collation| &collation.root)
        .collect();
    let with_namespace: BTreeSet<&PathBuf> = project
        .namespaces(db)
        .iter()
        .map(|(root, _)| root)
        .collect();
    for (root, entry) in &mut usage {
        entry.complete = complete_roots.contains(root)
            && with_namespace.contains(root)
            && sources.get(root).is_some_and(|count| *count > 0);
    }
    usage
}

/// The [`PackageUsage`] of the package enclosing `path` — a `DESCRIPTION`, or
/// any file under a package root. `None` outside every tracked package.
pub fn package_usage_for<'db>(
    db: &'db dyn IncrementalDb,
    project: Project<'db>,
    path: &Path,
) -> Option<&'db PackageUsage> {
    let usage = package_usage(db, project);
    let roots: BTreeSet<&PathBuf> = usage.keys().collect();
    let root = package_root_in(path, &roots)?;
    usage.get(&root)
}

/// Every tracked package root's [`DescriptionFacts`], keyed by root.
///
/// Pure: it reads the [`Descriptions`] input's handles and goes through
/// [`description_facts`], so an unrelated `DESCRIPTION` edit backdates there
/// and never reaches this caller.
fn description_facts_by_root(db: &dyn IncrementalDb) -> HashMap<PathBuf, &DescriptionFacts> {
    let Some(set) = Descriptions::try_get(db) else {
        return HashMap::new();
    };
    set.files(db)
        .iter()
        .map(|&file| (file.root(db).clone(), description_facts(db, file)))
        .collect()
}

/// The cross-file scope for `project`, built from the per-file firewall queries.
///
/// `no_eq` because its output ([`ProjectScope`]) holds `HashMap`s that aren't
/// `salsa::SalsaValue`/`Eq`-comparable here; `unsafe(non_salsa_values)` asserts it
/// carries no salsa references. This costs nothing for the firewall: a body edit
/// leaves the per-file inputs backdated, so this query simply isn't re-executed.
/// `no_eq` only forgoes backdating *when it does re-run* (an export actually
/// changed), and [`visible_symbols`] re-establishes per-file backdating above it.
#[salsa::tracked(returns(ref), no_eq, unsafe(non_salsa_values))]
pub fn project_graph<'db>(db: &'db dyn IncrementalDb, project: Project<'db>) -> ProjectScope {
    db.record_query(QueryLogEntry {
        kind: QueryKind::ProjectGraph,
        file: None,
    });

    let facts: Vec<FileFacts> = project
        .members(db)
        .iter()
        // `Arc::clone`, not a deep copy: these five are the per-file memos
        // themselves, and three of them are retained in the `ProjectScope`
        // afterward, so an owned `FileFacts` copied every name twice per member.
        .map(|m| FileFacts {
            path: m.path.clone(),
            exports: Arc::clone(file_exports(db, m.file)),
            free_reads: Arc::clone(file_free_reads(db, m.file)),
            qualified_reads: Arc::clone(file_qualified_reads(db, m.file)),
            source_edges: Arc::clone(source_edges(db, m.file)),
            top_level_events: Arc::clone(top_level_events(db, m.file)),
            package_root: m.package_root.clone(),
        })
        .collect();

    let namespaces: HashMap<PathBuf, String> = project.namespaces(db).iter().cloned().collect();
    let package_complete: HashMap<PathBuf, bool> = project
        .collations(db)
        .iter()
        .map(|c| (c.root.clone(), c.complete))
        .collect();
    let native_routines: HashMap<PathBuf, BTreeSet<String>> =
        project.native_routines(db).iter().cloned().collect();
    ProjectScope::build(&facts, &namespaces, &package_complete, &native_routines)
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
        visible: scope.visible_layer().clone(),
        read_by_others: scope.read_layer().clone(),
        namespace_exports: Arc::clone(scope.namespace_exports_handle()),
        s3_methods: Arc::clone(scope.s3_methods_handle()),
        wildcard_imports: Arc::clone(scope.wildcard_imports_handle()),
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
/// `BTreeMap`/`BTreeSet` so the type is `Eq`/`salsa::SalsaValue` and the query
/// backdates: a body edit leaves every `source_edges` unchanged (it is
/// range-free), so this re-runs only when a `source()` call is actually
/// added/removed/retargeted.
#[derive(Debug, Default, Clone, PartialEq, Eq, salsa::SalsaValue)]
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
#[derive(Debug, Default, Clone, PartialEq, Eq, salsa::SalsaValue)]
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

/// A project-wide OOP class index: for each declared class, the `(member path,
/// system)` sites it is defined at, its declared supertypes, and the inverse
/// subtype edges. Range-free (no spans), aggregated from the per-file
/// [`file_class_defs`] firewall, so it backdates across body edits; a consumer
/// recovers a class's span per request via
/// [`crate::project::locate_class_def`].
///
/// The class-hierarchy analog of [`DefIndex`]: it backs LSP type hierarchy
/// (`prepareTypeHierarchy` + supertypes/subtypes).
#[derive(Debug, Default, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct ClassIndex {
    /// Class name -> the sites (path, system) it is defined at.
    pub def_sites: BTreeMap<String, BTreeSet<(PathBuf, ClassSystem)>>,
    /// Class name -> its declared supertype names.
    pub supertypes: BTreeMap<String, BTreeSet<String>>,
    /// Parent class name -> the classes that declare it a supertype (the
    /// inverse of [`supertypes`](Self::supertypes)).
    pub subtypes: BTreeMap<String, BTreeSet<String>>,
}

/// Aggregate every member's [`file_class_defs`] into the project-wide
/// [`ClassIndex`], recording both the forward supertype edges and their inverse.
/// Keyed on the interned [`Project`] and the per-file firewall, so it backdates
/// across body edits and re-runs only when some file's class definitions change.
#[salsa::tracked(returns(ref))]
pub fn project_classes<'db>(db: &'db dyn IncrementalDb, project: Project<'db>) -> ClassIndex {
    db.record_query(QueryLogEntry {
        kind: QueryKind::ProjectClasses,
        file: None,
    });
    let mut index = ClassIndex::default();
    for member in project.members(db) {
        for (name, def) in file_class_defs(db, member.file) {
            index
                .def_sites
                .entry(name.clone())
                .or_default()
                .insert((member.path.clone(), def.system));
            for parent in &def.parents {
                index
                    .supertypes
                    .entry(name.clone())
                    .or_default()
                    .insert(parent.clone());
                index
                    .subtypes
                    .entry(parent.clone())
                    .or_default()
                    .insert(name.clone());
            }
        }
    }
    index
}

/// One package's Rd topics: each topic name mapped to the blocks that merge
/// into it, across every `R/` file of that package.
///
/// The borrowed view a rule consults — the [`FileScope`] analog for
/// documentation. Wrapping the map rather than exposing it keeps the lookup
/// ("what merges into this topic?") the only question callers can ask.
#[derive(Debug, Default, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct PackageTopics {
    by_topic: BTreeMap<String, Vec<TopicMember>>,
}

impl PackageTopics {
    /// The blocks merging into `key`, in package order, or empty when no block
    /// resolves to that topic.
    pub fn members(&self, key: &str) -> &[TopicMember] {
        self.by_topic.get(key).map_or(&[], Vec::as_slice)
    }
}

/// A project-wide roxygen topic index, keyed by package root.
///
/// Keyed by *package* and not flat because roxygen2 merges topics within one
/// package: two packages under one workspace each own their `add` topic, and
/// [`topic_key`](crate::project::topic_key) falls back to the documented
/// binding's name, so a flat map would merge unrelated `helper()`s. Members
/// outside any package contribute nothing — roxygen2 only runs on packages, so
/// a loose script keeps the file-local view.
///
/// Range-free (no spans), aggregated from the per-file
/// [`file_roxygen_topics`](crate::incremental::file_roxygen_topics) firewall, so
/// it backdates across body edits.
#[derive(Debug, Default, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct RoxygenTopicIndex {
    by_package: BTreeMap<PathBuf, PackageTopics>,
}

impl RoxygenTopicIndex {
    /// The topics of the package rooted at `root`, or `None` when that root has
    /// no members here.
    pub fn for_package(&self, root: &Path) -> Option<&PackageTopics> {
        self.by_package.get(root)
    }
}

/// Aggregate every package member's [`file_roxygen_topics`] into the
/// package-wide [`RoxygenTopicIndex`]. Keyed on the interned [`Project`] and the
/// per-file firewall, so it backdates across body edits and re-runs only when
/// some file's documentation actually changes.
///
/// Members are visited in `project.members` order, which is sorted by path, so
/// a topic's member list is deterministic.
#[salsa::tracked(returns(ref))]
pub fn project_roxygen_topics<'db>(
    db: &'db dyn IncrementalDb,
    project: Project<'db>,
) -> RoxygenTopicIndex {
    db.record_query(QueryLogEntry {
        kind: QueryKind::ProjectRoxygenTopics,
        file: None,
    });
    let mut index = RoxygenTopicIndex::default();
    for member in project.members(db) {
        let Some(root) = member.package_root.as_ref() else {
            continue;
        };
        let package = index.by_package.entry(root.clone()).or_default();
        for (topic, members) in file_roxygen_topics(db, member.file) {
            package
                .by_topic
                .entry(topic.clone())
                .or_default()
                .extend(members.iter().cloned());
        }
    }
    index
}

/// The [`PackageTopics`] of the package enclosing `path`. `None` outside every
/// tracked package, where topic resolution falls back to the file-local view.
pub fn roxygen_topics_for<'db>(
    db: &'db dyn IncrementalDb,
    project: Project<'db>,
    path: &Path,
) -> Option<&'db PackageTopics> {
    let index = project_roxygen_topics(db, project);
    let roots: BTreeSet<&PathBuf> = index.by_package.keys().collect();
    let root = package_root_in(path, &roots)?;
    index.by_package.get(&root)
}

/// A project-wide name → read-site index: for each name a member *free-reads*
/// (reads without binding it locally), the set of member paths that read it.
/// Range-free, aggregated from the per-file [`file_free_reads`] firewall, so it
/// backdates across body edits; a consumer recovers the actual read spans per
/// request via [`Analysis::read_ranges_in`](crate::incremental::Analysis::read_ranges_in).
///
/// The read-site mirror of [`DefIndex`]: it backs cross-file find-references (the
/// inverse of the def index that backs cross-file go-to-definition).
#[derive(Debug, Default, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct ReadIndex {
    pub by_name: BTreeMap<String, BTreeSet<PathBuf>>,
}

/// Aggregate every member's [`file_free_reads`] into the project-wide
/// [`ReadIndex`]. Keyed on the interned [`Project`] and the per-file firewall, so
/// it backdates across body edits and re-runs only when some file's free-read
/// name set changes.
#[salsa::tracked(returns(ref))]
pub fn project_reads<'db>(db: &'db dyn IncrementalDb, project: Project<'db>) -> ReadIndex {
    db.record_query(QueryLogEntry {
        kind: QueryKind::ProjectReads,
        file: None,
    });
    let mut index = ReadIndex::default();
    for member in project.members(db) {
        for name in file_free_reads(db, member.file).iter() {
            index
                .by_name
                .entry(name.clone())
                .or_default()
                .insert(member.path.clone());
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
#[derive(Debug, Default, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct ExternalResolution {
    pub unresolved: BTreeSet<String>,
}

/// Every package whose exports resolve as **bare names** in `file`.
///
/// The union of two provenances: the file's own `library()`/`require()` calls
/// and location-implied attaches ([`loaded_names`], a per-file firewall), and
/// the `Depends` its package declares (this project's interned declarations).
///
/// Deliberately **not** `Imports`: R does not attach an `Imports` package, and
/// resolving its exports as bare names here would accept code that fails under
/// `R CMD check`. Those reach a file only through `pkg::` or a NAMESPACE
/// `importFrom`/`import`, all of which the project layer models separately.
///
/// A `BTreeSet<String>` like `loaded_names`, so it backdates: a body edit
/// leaves it equal and [`external_resolution`] is not re-run.
#[salsa::tracked(returns(ref))]
pub fn attached_names<'db>(
    db: &'db dyn IncrementalDb,
    project: Project<'db>,
    file: SourceFile,
) -> BTreeSet<String> {
    db.record_query(QueryLogEntry {
        kind: QueryKind::AttachedNames,
        file: Some(file),
    });
    let mut names = loaded_names(db, file).clone();
    // A NAMESPACE `import(pkg)` puts every export of pkg in this file's scope,
    // which for name resolution is indistinguishable from attaching it.
    names.extend(
        visible_symbols(db, project, file)
            .wildcard_imports
            .iter()
            .cloned(),
    );
    let root = project
        .members(db)
        .iter()
        .find(|m| m.file == file)
        .and_then(|m| m.package_root.as_ref());
    if let Some(root) = root
        && let Some(decl) = project.declarations(db).iter().find(|d| &d.root == root)
    {
        names.extend(decl.attached.iter().cloned());
    }
    names
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
    let remote: &crate::rindex::remote::RemoteExports = manifest.remote(db);
    let loaded = attached_names(db, project, file);

    // Gate: an attached package whose exports we don't fully know could define
    // any of the unresolved names — suppress the whole file. A meta-package
    // (e.g. tidyverse) also attaches its core members (harvested attach set,
    // static table as fallback), so each of those must be indexed too, or one
    // of them could be the otherwise-unresolved name's home.
    //
    // This gate is also where a wholesale `import(pkg)` lands: `attached_names`
    // folds those packages in, so an unenumerable one suppresses here — the old
    // unconditional poison, now conditional on the index, and therefore lifting
    // the moment the sidecar can enumerate the package.
    if loaded.iter().any(|pkg| {
        !package_indexed(index, remote, pkg)
            || attach_members(index, pkg).any(|m| !package_indexed(index, remote, m))
    }) {
        return ExternalResolution::default();
    }

    let visibility = visible_symbols(db, project, file);
    // Gate: cross-file visibility left incomplete by something nothing can
    // resolve — a dynamic or unanalyzed `source()` — could supply any name.
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
                resolve_origin(index, remote, name, &loaded_pkgs),
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

    /// `Visibility` now holds shared layers behind `Arc`, and salsa backdates a
    /// memo by comparing the old and new values. Two distinct allocations with
    /// equal contents must compare equal, or a rebuilt-but-unchanged project
    /// graph would stop backdating and cost every keystroke a re-lint of every
    /// file.
    #[test]
    fn visibility_equality_is_content_not_pointer() {
        let names = || -> BTreeSet<String> { ["foo".to_string()].into_iter().collect() };
        let layer = || {
            crate::project::LayeredSet::new(
                std::sync::Arc::new(names()),
                BTreeSet::new(),
                BTreeSet::new(),
            )
        };
        let a = Visibility {
            visible: layer(),
            read_by_others: layer(),
            namespace_exports: Arc::new(names()),
            s3_methods: Arc::default(),
            wildcard_imports: Arc::default(),
            incomplete: false,
        };
        let b = a.clone();
        // `b` shares `a`'s allocations; `c` is rebuilt from scratch.
        let c = Visibility {
            visible: layer(),
            read_by_others: layer(),
            namespace_exports: Arc::new(names()),
            s3_methods: Arc::default(),
            wildcard_imports: Arc::default(),
            incomplete: false,
        };
        assert_eq!(a, b);
        assert_eq!(a, c);
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

    #[test]
    fn expected_r_sources_unions_dir_glob_and_collate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::create_dir(root.join("R")).expect("R/");
        std::fs::write(root.join("R/a.R"), "").expect("a.R");
        std::fs::write(root.join("R/b.r"), "").expect("b.r"); // lowercase ext counts
        std::fs::write(root.join("R/notes.md"), "").expect("notes.md"); // non-source ignored
        std::fs::write(root.join("DESCRIPTION"), "Package: p\nCollate: a.R 'c.R'\n")
            .expect("DESCRIPTION");

        let expected = expected_r_sources(root);
        assert!(expected.contains("a.R"), "dir-glob R source");
        assert!(expected.contains("b.r"), "lowercase .r extension counts");
        assert!(!expected.contains("notes.md"), "non-R file is excluded");
        // From `Collate:` (quote-stripped), even though c.R is absent on disk —
        // the union makes the package incomplete when c.R isn't analyzed.
        assert!(expected.contains("c.R"), "Collate entry, quote-stripped");
    }
}
