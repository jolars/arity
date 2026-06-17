//! Salsa-backed incremental layer: file text → parse tree → semantic model.
//!
//! The CST is cached as a `rowan::GreenNode` (Arc-backed, `Send + Sync`) rather
//! than a `SyntaxNode` (which holds non-`Send` cursor state and is neither
//! `Eq` nor `salsa::Update`). Callers materialize a fresh cursor via
//! [`parsed_tree_root`] — a cheap atomic clone — so each consumer gets its own
//! tree without leaking the salsa cell. The per-file [`semantic_model`] query
//! builds on the cached tree, so the linter and LSP no longer re-parse and
//! rebuild the model from text on every run.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rowan::TextRange;
use salsa::{Durability, Setter};

use crate::parser::{ParseDiagnostic, diff_edit, map_range_through_edit, parse, reparse};
use crate::project::{
    DefKind, PackageInfo, ReadBinding, SourceEdgeKey, TopLevelEvent, collect_source_literal_edges,
    collect_top_level_events, discover_packages, project_defs, project_graph, project_reads,
    relative_path, reverse_source_edges, workspace_project,
};
use crate::rindex::provider::IndexedProvider;
use crate::semantic::{BindingKind, ScopeKind, SemanticModel};
use crate::syntax::{NodePtr, SyntaxNode};

/// An opaque, process-local file identity. Decouples a tracked file from any
/// path: it is allocated once when a file is first seen and never reused, so it
/// is the stable handle the rest of the system can key on without a path leaking
/// in. On-disk files carry one alongside their (immutable) path; in-memory files
/// carry one with no path at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(pub u32);

#[salsa::input]
pub struct SourceFile {
    /// This file's opaque identity, allocated by the [`FileSourceMap`]. Set once
    /// and never mutated.
    pub id: FileId,
    /// The path this file was tracked under, or `None` for an in-memory document.
    /// Set once at creation and never mutated, so path-reading queries (e.g.
    /// [`source_edges`], which resolves relative `source()` targets against
    /// `path.parent()`) don't re-run on a text edit. Equivalent path forms are
    /// deduplicated *before* a file is created (see [`FileSourceMap`]), so two
    /// spellings of the same path never mint two inputs.
    #[returns(ref)]
    pub path: Option<PathBuf>,
    #[returns(ref)]
    pub text: String,
}

/// Lexically normalize `path` for use as a deduplication key: absolutize it
/// (against the current directory, without touching the filesystem) and collapse
/// `.` / `..` segments. Purely textual — no symlink resolution, no existence
/// check — so it is stable for not-yet-saved buffers and never blocks on I/O.
/// `a.R`, `./a.R`, and `dir/../a.R` all map to the same key.
pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(out.components().next_back(), Some(Component::Normal(_))) =>
            {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Render a path with forward slashes, the separator R source paths use on every
/// platform. A lossy `to_string_lossy` is acceptable: a non-UTF-8 path can't have
/// originated from a `source()` string literal we are rewriting.
fn to_forward_slash(path: &Path) -> String {
    let s = path.to_string_lossy();
    if std::path::MAIN_SEPARATOR == '/' {
        s.into_owned()
    } else {
        s.replace(std::path::MAIN_SEPARATOR, "/")
    }
}

/// The path → input index plus the [`FileId`] allocator. Maps a *normalized*
/// path to the single [`SourceFile`] tracked for it, so reaching the same file
/// by an equivalent path spelling reuses one input (and its cached queries)
/// rather than minting a duplicate. In-memory files get a [`FileId`] but no entry
/// here (nothing looks them up by path).
#[derive(Default)]
struct FileSourceMap {
    by_path: HashMap<PathBuf, SourceFile>,
    next_id: u32,
}

impl FileSourceMap {
    fn alloc_id(&mut self) -> FileId {
        let id = FileId(self.next_id);
        self.next_id += 1;
        id
    }
}

/// The harvested package symbol index, modeled as a salsa **singleton** input at
/// `Durability::HIGH`. Only the harvested layer ([`IndexedProvider`]) varies at
/// runtime — R's default-package lists and the bundled CRAN exports are
/// compile-time constants and stay out of salsa. Because the value is set at
/// HIGH durability, a keystroke (a LOW write to a [`SourceFile`]) skips
/// revalidating any query whose only changing dependency is this index: salsa's
/// version vector compares the HIGH revision in one integer compare and finds it
/// unmoved. The payload is held behind an `Arc` so a swap is a cheap pointer
/// write; input fields are never compared for equality (salsa inputs do not
/// backdate), so the non-`Update` `HashMap`/`SmolStr` inside `IndexedProvider`
/// is fine here.
#[salsa::input(singleton)]
pub struct LibraryIndex {
    #[returns(ref)]
    pub data: Arc<IndexedProvider>,
}

/// The explicit workspace file-set, modeled as a salsa **singleton** input at
/// `Durability::MEDIUM`. The interned [`Project`](crate::project::Project) is
/// *derived* from this (see
/// [`workspace_project`](crate::project::workspace_project)) rather than rebuilt
/// by a per-request disk walk: the member files are discovered once (the LSP
/// seed, the CLI's `collect_r_files`) and reused until the set actually changes.
///
/// MEDIUM durability sits between the HIGH [`LibraryIndex`] and the LOW per-file
/// [`SourceFile`] text. Like every salsa input it never backdates, so the setter
/// ([`set_workspace_members`](IncrementalDatabase::set_workspace_members)) must
/// skip the write when the member set is unchanged — otherwise re-seeding an
/// identical set on each lint would bump the revision needlessly. `members` may
/// include files that currently fail to parse; `workspace_project` filters those
/// out, so membership is stable across a parse error appearing and clearing.
#[salsa::input(singleton)]
pub struct Workspace {
    /// Every tracked file in the workspace, in any order (the setter sorts for a
    /// stable key). Pathless in-memory files are ignored by `workspace_project`.
    #[returns(ref)]
    pub members: Vec<SourceFile>,
    /// The workspace roots the members were discovered under.
    #[returns(ref)]
    pub roots: Vec<PathBuf>,
}

/// The disk-derived package metadata for the workspace's members, modeled as a
/// salsa **singleton** input at `Durability::MEDIUM` — the per-root NAMESPACE
/// texts, expected-source sets, and package-root markers that
/// [`workspace_project`](crate::project::workspace_project) used to read from
/// disk inside the tracked query (re-walking the filesystem on every keystroke).
/// Lifting them into an input makes that query pure: a keystroke re-run does only
/// in-memory work, and a future `didChangeWatchedFiles` watcher can invalidate a
/// real NAMESPACE/DESCRIPTION/`R/` change by refreshing this input.
///
/// Populated in the write-phase by
/// [`refresh_package_graph`](IncrementalDatabase::refresh_package_graph), which
/// runs [`discover_packages`](crate::project::discover_packages) (the sole disk
/// reader) over the current workspace members. Like every salsa input it never
/// backdates, so the setter skips the write when the metadata is unchanged.
#[salsa::input(singleton)]
pub struct PackageGraph {
    /// One entry per distinct package root among the members, sorted by root.
    #[returns(ref)]
    pub packages: Vec<PackageInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueryKind {
    ParsedDocument,
    SemanticModel,
    FileExports,
    FileFreeReads,
    FileDefSites,
    SourceEdges,
    TopLevelEvents,
    ReverseSourceEdges,
    WorkspaceProject,
    ProjectGraph,
    ProjectDefs,
    ProjectReads,
    VisibleSymbols,
    LoadedNames,
    ExternalResolution,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueryLogEntry {
    pub kind: QueryKind,
    /// The per-file query subject, or `None` for project-level queries
    /// ([`QueryKind::ProjectGraph`]) that aren't keyed on a single file.
    pub file: Option<SourceFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnosticData {
    pub message: String,
    pub start: usize,
    pub end: usize,
}

/// A cached parse: the green tree plus parse diagnostics, computed once per
/// `(db, file)`.
///
/// The `GreenNode` is not `Eq`/`salsa::Update`, so [`parsed_document`] is
/// `no_eq, unsafe(non_update_types)`: salsa never compares parse outputs and
/// relies purely on input (text) change detection to invalidate. That is sound
/// because the tree is a pure function of the text.
#[derive(Debug, Clone)]
pub struct ParsedDocument {
    pub green: rowan::GreenNode,
    pub diagnostics: Vec<ParseDiagnosticData>,
}

/// The previous parse of a file, kept outside salsa to drive incremental
/// reparse. `parsed_document` recovers the edit from the old and new text and
/// splices the old green tree instead of re-parsing from scratch. This only ever
/// affects *how fast* `parsed_document` computes, never *what* it returns (a
/// successful reparse is byte-identical to a full parse — see
/// `crate::parser::reparse`), so it is sound to read/write from inside the
/// otherwise-pure tracked query.
#[derive(Debug, Clone)]
pub struct PrevParse {
    pub text: String,
    pub green: rowan::GreenNode,
    pub diagnostics: Vec<ParseDiagnostic>,
}

#[salsa::db]
pub trait IncrementalDb: salsa::Database {
    fn record_query(&self, entry: QueryLogEntry);

    /// The cached previous parse for `file`, if any (the incremental-reparse
    /// base).
    fn reparse_prev(&self, file: SourceFile) -> Option<Arc<PrevParse>>;

    /// Store `prev` as the reparse base for `file`. `incremental` records
    /// whether this parse reused the previous tree (for tests/metrics).
    fn reparse_store(&self, file: SourceFile, prev: PrevParse, incremental: bool);
}

#[salsa::tracked(returns(ref), no_eq, unsafe(non_update_types))]
pub fn parsed_document(db: &dyn IncrementalDb, file: SourceFile) -> ParsedDocument {
    db.record_query(QueryLogEntry {
        kind: QueryKind::ParsedDocument,
        file: Some(file),
    });

    let text = file.text(db);

    // Try an incremental reparse off the previous parse of this file. A miss
    // (first parse, or an edit no strategy handles) falls back to a full parse;
    // either way the result is identical to `parse(text)`.
    let reparsed = db
        .reparse_prev(file)
        .filter(|prev| prev.text != *text)
        .and_then(|prev| {
            let edit = diff_edit(&prev.text, text);
            let old_root = SyntaxNode::new_root(prev.green.clone());
            reparse(&old_root, &prev.text, &prev.diagnostics, &edit)
        });

    let incremental = reparsed.is_some();
    let (green, diagnostics): (rowan::GreenNode, Vec<ParseDiagnostic>) = match reparsed {
        Some(r) => (r.green, r.diagnostics),
        None => {
            let parsed = parse(text.as_str());
            (parsed.cst.green().into_owned(), parsed.diagnostics)
        }
    };

    db.reparse_store(
        file,
        PrevParse {
            text: text.clone(),
            green: green.clone(),
            diagnostics: diagnostics.clone(),
        },
        incremental,
    );

    let diagnostics = diagnostics
        .into_iter()
        .map(|diagnostic| ParseDiagnosticData {
            message: diagnostic.message,
            start: diagnostic.start,
            end: diagnostic.end,
        })
        .collect();

    ParsedDocument { green, diagnostics }
}

/// The parse diagnostics for `file` (empty when the file parses cleanly).
pub fn parse_diagnostics(db: &dyn IncrementalDb, file: SourceFile) -> &[ParseDiagnosticData] {
    &parsed_document(db, file).diagnostics
}

/// Materialize the cached parse for `file` as a fresh `SyntaxNode` cursor.
pub fn parsed_tree_root(db: &dyn IncrementalDb, file: SourceFile) -> SyntaxNode {
    SyntaxNode::new_root(parsed_document(db, file).green.clone())
}

/// The per-file semantic model, built on the cached parse tree. Returned by
/// reference; salsa short-circuits downstream consumers when an edit leaves the
/// model unchanged (`SemanticModel: Eq`).
#[salsa::tracked(returns(ref))]
pub fn semantic_model(db: &dyn IncrementalDb, file: SourceFile) -> SemanticModel {
    db.record_query(QueryLogEntry {
        kind: QueryKind::SemanticModel,
        file: Some(file),
    });
    SemanticModel::build(&parsed_tree_root(db, file))
}

/// The file's top-level exports (a [`crate::project::file_exports`] projection),
/// as a tracked query. This is the cross-file *firewall*: editing a function
/// body changes [`semantic_model`] but leaves this `BTreeSet` equal, so salsa
/// backdates and the project graph that depends on it is not rebuilt.
#[salsa::tracked(returns(ref))]
pub fn file_exports(db: &dyn IncrementalDb, file: SourceFile) -> BTreeSet<String> {
    db.record_query(QueryLogEntry {
        kind: QueryKind::FileExports,
        file: Some(file),
    });
    crate::project::file_exports(semantic_model(db, file))
}

/// The names the file reads but does not bind locally
/// ([`crate::project::file_free_reads`]), as a tracked query. The mirror
/// firewall to [`file_exports`].
#[salsa::tracked(returns(ref))]
pub fn file_free_reads(db: &dyn IncrementalDb, file: SourceFile) -> BTreeSet<String> {
    db.record_query(QueryLogEntry {
        kind: QueryKind::FileFreeReads,
        file: Some(file),
    });
    crate::project::file_free_reads(semantic_model(db, file))
}

/// The file's top-level definitions tagged by [`DefKind`]
/// ([`crate::project::file_def_sites`]), as a tracked query. The name set mirrors
/// [`file_exports`]; the tag enables a symbol index. Range-free, so it backdates
/// across a body edit exactly like [`file_exports`] — a consumer recovers the
/// actual def span from the fresh [`semantic_model`] per request.
#[salsa::tracked(returns(ref))]
pub fn file_def_sites(db: &dyn IncrementalDb, file: SourceFile) -> BTreeMap<String, DefKind> {
    db.record_query(QueryLogEntry {
        kind: QueryKind::FileDefSites,
        file: Some(file),
    });
    crate::project::file_def_sites(semantic_model(db, file), &parsed_tree_root(db, file))
}

/// The names of the packages attached via `library()`/`require()` in the file,
/// a projection of [`semantic_model`]'s loaded packages. A masking firewall for
/// [`external_resolution`](crate::project::external_resolution): editing a body
/// changes the model but leaves this set equal, so resolution backdates. A set
/// (not the source-ordered list) because resolution only asks whether a name
/// resolves to *some* attached package — load order affects only
/// Resolved-vs-Ambiguous, which the undefined-symbol gate does not distinguish.
#[salsa::tracked(returns(ref))]
pub fn loaded_names(db: &dyn IncrementalDb, file: SourceFile) -> BTreeSet<String> {
    db.record_query(QueryLogEntry {
        kind: QueryKind::LoadedNames,
        file: Some(file),
    });
    semantic_model(db, file)
        .loaded_packages()
        .iter()
        .map(|pkg| pkg.name.to_string())
        .collect()
}

/// The file's top-level `source()` edges, range-free
/// ([`crate::project::collect_source_edge_keys`]), as a tracked query. Resolves
/// relative targets against the file's own directory (`path.parent()`); the
/// path is an input field set once, so this re-runs only on a text edit and
/// backdates when the edges are unchanged.
#[salsa::tracked(returns(ref))]
pub fn source_edges(db: &dyn IncrementalDb, file: SourceFile) -> Vec<SourceEdgeKey> {
    db.record_query(QueryLogEntry {
        kind: QueryKind::SourceEdges,
        file: Some(file),
    });
    let root = parsed_tree_root(db, file);
    let base_dir = file.path(db).as_deref().and_then(Path::parent);
    crate::project::collect_source_edge_keys(&root, base_dir)
}

/// The file's top-level execution sequence ([`collect_top_level_events`]): the
/// ordered `define`/`source-edge`/`read` events load-order resolution consumes.
/// Range-free — order is carried by `Vec` position — so it backdates across a
/// body edit exactly like [`source_edges`]: editing inside a function body
/// changes neither the order nor the names of the top-level events, so the
/// re-extracted value compares equal and the memo (and `project_graph` above it)
/// is reused.
#[salsa::tracked(returns(ref))]
pub fn top_level_events(db: &dyn IncrementalDb, file: SourceFile) -> Vec<TopLevelEvent> {
    db.record_query(QueryLogEntry {
        kind: QueryKind::TopLevelEvents,
        file: Some(file),
    });
    let root = parsed_tree_root(db, file);
    let base_dir = file.path(db).as_deref().and_then(Path::parent);
    collect_top_level_events(&root, base_dir, semantic_model(db, file))
}

#[salsa::db]
pub struct IncrementalDatabase {
    storage: salsa::Storage<Self>,
    query_log: Arc<Mutex<Vec<QueryLogEntry>>>,
    /// Normalized-path → input index plus the [`FileId`] allocator, so repeated
    /// edits to the same path (under any equivalent spelling) reuse the same
    /// `SourceFile` input — and thus its cached queries — instead of creating a
    /// fresh one each time. Seeds the cross-file project graph (Phase B).
    source_map: Arc<Mutex<FileSourceMap>>,
    /// Previous parse per file, the base for incremental reparse in
    /// [`parsed_document`]. Outside salsa: a pure performance hint that never
    /// changes query *outputs* (see [`PrevParse`]). Shared across clones.
    reparse_cache: Arc<Mutex<HashMap<SourceFile, Arc<PrevParse>>>>,
    /// Count of parses that reused the previous tree (incremental reparse hits),
    /// for tests and metrics. Shared across clones.
    reparse_hits: Arc<AtomicU64>,
}

impl Default for IncrementalDatabase {
    fn default() -> Self {
        Self {
            storage: salsa::Storage::new(None),
            query_log: Arc::new(Mutex::new(Vec::new())),
            source_map: Arc::new(Mutex::new(FileSourceMap::default())),
            reparse_cache: Arc::new(Mutex::new(HashMap::new())),
            reparse_hits: Arc::new(AtomicU64::new(0)),
        }
    }
}

/// Cloning yields a second handle onto the *same* salsa storage (a cheap
/// `Arc`-bump of the shared `Zalsa`, plus the shared path→input map and query
/// log). This is how the language server runs read-only queries off the lint
/// thread: the owner mints a short-lived clone, hands it to a worker, and the
/// clone is dropped promptly. Salsa is single-writer — a clone outstanding when
/// the owner performs a write blocks that write until the clone drops (and trips
/// `salsa::Cancelled` in any read still in flight), so clones must never be held
/// across a write or parked long-term.
impl Clone for IncrementalDatabase {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            query_log: Arc::clone(&self.query_log),
            source_map: Arc::clone(&self.source_map),
            reparse_cache: Arc::clone(&self.reparse_cache),
            reparse_hits: Arc::clone(&self.reparse_hits),
        }
    }
}

impl std::fmt::Debug for IncrementalDatabase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IncrementalDatabase")
            .finish_non_exhaustive()
    }
}

impl IncrementalDatabase {
    /// Track an in-memory document with no on-disk path. It gets a fresh
    /// [`FileId`] and a `None` path, so it never aliases another file and never
    /// participates in path-based cross-file resolution. Used by tests and
    /// one-shot single-file checks; the LSP/CLI use
    /// [`upsert_file`](Self::upsert_file) with the real path.
    pub fn add_file(&self, text: impl Into<String>) -> SourceFile {
        let id = self
            .source_map
            .lock()
            .expect("file source map mutex poisoned")
            .alloc_id();
        SourceFile::new(self, id, None, text.into())
    }

    pub fn set_file_text(&mut self, file: SourceFile, text: impl Into<String>) {
        file.set_text(self).to(text.into());
    }

    /// Install (or replace) the harvested package index as the
    /// [`LibraryIndex`] singleton, at `Durability::HIGH`. The sole writer (CLI
    /// setup, the LSP lint thread) calls this; never a read snapshot. Creating
    /// the singleton lands at default durability, so we immediately re-set the
    /// field at HIGH — the first edit after creation then also skips the library
    /// subgraph. Returns the singleton input handle.
    pub fn set_library_index(&mut self, indexed: IndexedProvider) -> LibraryIndex {
        let data = Arc::new(indexed);
        match LibraryIndex::try_get(self) {
            Some(index) => {
                index
                    .set_data(self)
                    .with_durability(Durability::HIGH)
                    .to(data);
                index
            }
            None => {
                let index = LibraryIndex::new(self, Arc::clone(&data));
                index
                    .set_data(self)
                    .with_durability(Durability::HIGH)
                    .to(data);
                index
            }
        }
    }

    /// The [`LibraryIndex`] singleton, if one has been installed. Read-only.
    pub fn library_index(&self) -> Option<LibraryIndex> {
        LibraryIndex::try_get(self)
    }

    /// Install (or update) the explicit workspace file-set as the [`Workspace`]
    /// singleton, at `Durability::MEDIUM`. `members` are sorted by [`FileId`] for
    /// a stable key; the write is **skipped when the set is unchanged**, because a
    /// salsa input always bumps its revision on a `set_*` and re-seeding an
    /// identical membership on each lint would needlessly invalidate
    /// [`workspace_project`](crate::project::workspace_project). The sole writer
    /// (CLI setup, the LSP lint thread) calls this; never a read snapshot.
    pub fn set_workspace_members(
        &mut self,
        mut members: Vec<SourceFile>,
        roots: Vec<PathBuf>,
    ) -> Workspace {
        members.sort_by_key(|file| file.id(self));
        members.dedup();
        let ws = match Workspace::try_get(self) {
            Some(ws) => {
                if ws.members(self) != &members {
                    ws.set_members(self)
                        .with_durability(Durability::MEDIUM)
                        .to(members);
                }
                if ws.roots(self) != &roots {
                    ws.set_roots(self)
                        .with_durability(Durability::MEDIUM)
                        .to(roots);
                }
                ws
            }
            None => {
                let ws = Workspace::new(self, members.clone(), roots.clone());
                // Creation lands at default durability; re-set at MEDIUM so the
                // first edit after seeding also skips revalidating the file-set.
                ws.set_members(self)
                    .with_durability(Durability::MEDIUM)
                    .to(members);
                ws.set_roots(self)
                    .with_durability(Durability::MEDIUM)
                    .to(roots);
                ws
            }
        };
        // Keep the disk-derived package metadata in lockstep with membership, so
        // `workspace_project` always sees a `PackageGraph` consistent with the
        // members it derives from (no stale-root window).
        self.refresh_package_graph();
        ws
    }

    /// Re-read the workspace members' package metadata (NAMESPACE texts,
    /// expected-source sets, package roots) from disk and install it as the
    /// [`PackageGraph`] singleton, at `Durability::MEDIUM`. The **sole disk
    /// reader** behind `workspace_project`: [`set_workspace_members`] calls it on
    /// every membership change, and a future `didChangeWatchedFiles` watcher calls
    /// it directly to pick up a NAMESPACE/DESCRIPTION/`R/` edit without touching
    /// membership. The write is **skipped when the metadata is unchanged**, since
    /// a salsa input always bumps its revision on a `set_*`.
    pub fn refresh_package_graph(&mut self) -> PackageGraph {
        let member_paths: Vec<PathBuf> = Workspace::try_get(self)
            .map(|ws| {
                ws.members(self)
                    .iter()
                    .filter_map(|file| file.path(self).clone())
                    .collect()
            })
            .unwrap_or_default();
        let packages = discover_packages(&member_paths);
        match PackageGraph::try_get(self) {
            Some(graph) => {
                if graph.packages(self) != &packages {
                    graph
                        .set_packages(self)
                        .with_durability(Durability::MEDIUM)
                        .to(packages);
                }
                graph
            }
            None => {
                let graph = PackageGraph::new(self, packages.clone());
                // Creation lands at default durability; re-set at MEDIUM so the
                // first edit after seeding also skips revalidating the metadata.
                graph
                    .set_packages(self)
                    .with_durability(Durability::MEDIUM)
                    .to(packages);
                graph
            }
        }
    }

    /// The [`Workspace`] singleton, if one has been seeded. Read-only.
    pub fn workspace(&self) -> Option<Workspace> {
        Workspace::try_get(self)
    }

    /// The harvested package index payload, if installed. A cheap `Arc` clone.
    pub fn library_data(&self) -> Option<Arc<IndexedProvider>> {
        LibraryIndex::try_get(self).map(|index| index.data(self).clone())
    }

    /// Insert or update the input for `path`, reusing the existing `SourceFile`
    /// when one is already tracked. The hot path for editor buffers: a keystroke
    /// updates the text of an existing input so unchanged downstream queries stay
    /// cached.
    pub fn upsert_file(&mut self, path: &Path, text: String) -> SourceFile {
        let key = normalize_path(path);
        let existing = self
            .source_map
            .lock()
            .expect("file source map mutex poisoned")
            .by_path
            .get(&key)
            .copied();
        match existing {
            Some(file) => {
                // Skip the write when the text is unchanged: setting an input
                // unconditionally bumps the revision and would re-run every
                // downstream query (a sibling file re-read on each keystroke).
                if file.text(self) != &text {
                    file.set_text(self).to(text);
                }
                file
            }
            None => {
                let id = self
                    .source_map
                    .lock()
                    .expect("file source map mutex poisoned")
                    .alloc_id();
                // Store the first-seen spelling as the file's path; the index is
                // keyed by the normalized form, so later equivalent spellings
                // resolve to this same input and its first-seen path.
                let file = SourceFile::new(self, id, Some(path.to_path_buf()), text);
                self.source_map
                    .lock()
                    .expect("file source map mutex poisoned")
                    .by_path
                    .insert(key, file);
                file
            }
        }
    }

    /// The `SourceFile` input currently tracked for `path`, if any. Read-only:
    /// unlike [`upsert_file`](Self::upsert_file) it never inserts, so it is safe
    /// to call on a shared clone (the language server's read path uses it to find
    /// the cached parse for the buffer under the cursor).
    pub fn lookup_file(&self, path: &Path) -> Option<SourceFile> {
        self.source_map
            .lock()
            .expect("file source map mutex poisoned")
            .by_path
            .get(&normalize_path(path))
            .copied()
    }

    /// The text currently tracked for `file`.
    pub fn file_text(&self, file: SourceFile) -> &str {
        file.text(self)
    }

    /// The path `file` is tracked under, or `None` for an in-memory document.
    pub fn file_path(&self, file: SourceFile) -> Option<&Path> {
        file.path(self).as_deref()
    }

    /// Parse diagnostics for `file` (empty when it parses cleanly).
    pub fn parse_diagnostics(&self, file: SourceFile) -> &[ParseDiagnosticData] {
        parse_diagnostics(self, file)
    }

    /// A fresh `SyntaxNode` over the cached parse tree.
    pub fn parsed_tree(&self, file: SourceFile) -> SyntaxNode {
        parsed_tree_root(self, file)
    }

    /// The cached per-file semantic model.
    pub fn semantic_model(&self, file: SourceFile) -> &SemanticModel {
        semantic_model(self, file)
    }

    pub fn clear_query_log(&self) {
        self.query_log
            .lock()
            .expect("query log mutex poisoned")
            .clear();
    }

    pub fn query_log(&self) -> Vec<QueryLogEntry> {
        self.query_log
            .lock()
            .expect("query log mutex poisoned")
            .clone()
    }

    /// Number of parses served by an incremental reparse (reused the previous
    /// tree) since construction. For tests and metrics.
    pub fn reparse_hits(&self) -> u64 {
        self.reparse_hits.load(Ordering::Relaxed)
    }

    /// Mint a read-only [`Analysis`] snapshot: a short-lived db clone wrapped so
    /// callers can only *read*. Drop it promptly --- an outstanding clone blocks
    /// the next write (salsa is single-writer; see the [`Clone`] impl).
    pub fn snapshot(&self) -> Analysis {
        Analysis(self.clone())
    }
}

/// A read-only handle onto the incremental database, à la rust-analyzer's
/// `Analysis` (vs. its writer `AnalysisHost`). Wraps a short-lived clone of the
/// lint thread's [`IncrementalDatabase`] and exposes *only* read queries, so a
/// read job cannot call `upsert_file` / salsa setters --- the single-writer
/// invariant is encoded in the type system rather than left to convention.
///
/// Handed to the language server's read jobs and the cross-file read-phase
/// ([`analyze_prepared`](crate::linter::check::analyze_prepared)); the
/// `&mut`-capable [`IncrementalDatabase`] stays private to the lint worker.
/// The scope-aware resolution of a cross-file binding: the workspace partition a
/// top-level name actually belongs to, as opposed to the global name-keyed view
/// of [`Analysis::workspace_def_sites`]/[`Analysis::workspace_read_sites`].
///
/// Produced by [`Analysis::cross_file_binding`] for a `(def_file, name)` pair.
/// Span-free (paths only); consumers recover spans per file via
/// [`Analysis::def_range_in`]/[`Analysis::read_ranges_in`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CrossFileBinding {
    /// Member files whose top-level definition of the name aliases `def_file`'s
    /// — `def_file` itself plus the files sharing its flat-namespace component
    /// (package siblings, or `source()`-connected scripts) that also define it.
    pub cohort: Vec<PathBuf>,
    /// Member files that can see `def_file`, free-read the name, and do not
    /// shadow it with their own top-level definition — the reads that bind to
    /// `def_file`'s definition.
    pub readers: Vec<PathBuf>,
    /// The name is defined by more than one file in `def_file`'s component
    /// (`cohort.len() > 1`). In R's flat per-component namespace these are
    /// aliases of one slot (a redefinition). References over-reports the whole
    /// cohort on this; rename is sound as a rename-all *unless* the picture is
    /// incomplete (see `cohort_incomplete`).
    pub conflict: bool,
    /// A multi-def cohort whose package picture is *incomplete* — an unanalyzed
    /// or dropped (parse-error) `R/*.[RrSsQq]` member could define or read the
    /// name, so renaming the visible aliases would half-rewrite the flat
    /// namespace. A sound rename refuses; references still over-reports.
    pub cohort_incomplete: bool,
    /// A reader has a *top-level* read of the name that, under sequential
    /// `source()` load order, does **not** bind to this cohort — it sits before
    /// the `source()` that injects the def, binds to a different file, or is
    /// poisoned by a dynamic/ambiguous source. Rewriting that reader would touch
    /// a read that isn't this binding, and we can't selectively skip it without
    /// per-read spans, so a sound rename refuses. References still over-reports.
    pub order_ambiguous: bool,
    /// Some member has an unresolvable dynamic `source()`, so a hidden reader
    /// could exist. A sound rename refuses project-wide to avoid a missed read.
    pub project_has_dynamic_source: bool,
}

pub struct Analysis(IncrementalDatabase);

impl Analysis {
    /// The `SourceFile` input currently tracked for `path`, if any.
    pub fn lookup_file(&self, path: &Path) -> Option<SourceFile> {
        self.0.lookup_file(path)
    }

    /// The text currently tracked for `file`.
    pub fn file_text(&self, file: SourceFile) -> &str {
        self.0.file_text(file)
    }

    /// The path `file` is tracked under, or `None` for an in-memory document.
    pub fn file_path(&self, file: SourceFile) -> Option<&Path> {
        self.0.file_path(file)
    }

    /// Parse diagnostics for `file` (empty when it parses cleanly).
    pub fn parse_diagnostics(&self, file: SourceFile) -> &[ParseDiagnosticData] {
        self.0.parse_diagnostics(file)
    }

    /// A fresh `SyntaxNode` over the cached parse tree.
    pub fn parsed_tree(&self, file: SourceFile) -> SyntaxNode {
        self.0.parsed_tree(file)
    }

    /// The cached per-file semantic model.
    pub fn semantic_model(&self, file: SourceFile) -> &SemanticModel {
        self.0.semantic_model(file)
    }

    /// The definition span of the top-level binding named `name` in `file`, read
    /// from the *fresh* semantic model so the range always indexes the current
    /// text (never a stale memo). Mirrors the def-site filter of
    /// [`crate::project::file_def_sites`] — this is how a consumer recovers the
    /// actual span the range-free
    /// [`project_defs`](crate::project::project_defs) aggregate omits. Returns the
    /// first matching top-level binding.
    pub fn def_range_in(&self, file: SourceFile, name: &str) -> Option<TextRange> {
        let model = self.0.semantic_model(file);
        model
            .bindings()
            .iter()
            .find(|binding| {
                matches!(binding.kind, BindingKind::Local | BindingKind::Implicit)
                    && model.scope(binding.scope).kind == ScopeKind::File
                    && binding.name.as_str() == name
            })
            .map(|binding| binding.def_range)
    }

    /// The top-level definition sites for `name` across the workspace, as
    /// `(member path, def span)`. Empty when no workspace is seeded or `name`
    /// matches no top-level binding. The first consumer of
    /// [`project_defs`](crate::project::project_defs): it supplies the
    /// range-free `(path, kind)` set, and each span is recovered per site via
    /// [`def_range_in`](Self::def_range_in) so it indexes that file's *current*
    /// text. Backs cross-file go-to-definition. A pure read — the caller wraps
    /// it in [`salsa::Cancelled::catch`] (as hover wraps its read).
    pub fn workspace_def_sites(&self, name: &str) -> Vec<(PathBuf, TextRange)> {
        if self.0.workspace().is_none() {
            return Vec::new();
        }
        let project = workspace_project(&self.0);
        let index = project_defs(&self.0, project);
        let Some(sites) = index.by_name.get(name) else {
            return Vec::new();
        };
        sites
            .iter()
            .filter_map(|(path, _kind)| {
                let file = self.0.lookup_file(path)?;
                Some((path.clone(), self.def_range_in(file, name)?))
            })
            .collect()
    }

    /// The free-read sites of `name` in `file`, read from the *fresh* semantic
    /// model so each span indexes the current text. A "free read" is an identifier
    /// occurrence that binds to no local binding — the same predicate
    /// [`crate::project::file_free_reads`] uses, so a name in that (range-free) set
    /// is exactly one this recovers spans for. The read-site mirror of
    /// [`def_range_in`](Self::def_range_in).
    pub fn read_ranges_in(&self, file: SourceFile, name: &str) -> Vec<TextRange> {
        let model = self.0.semantic_model(file);
        model
            .idents()
            .iter()
            .filter(|ident| ident.name.as_str() == name && model.resolve_local(ident).is_none())
            .map(|ident| ident.range)
            .collect()
    }

    /// The cross-file read sites of `name` across the workspace, as `(member path,
    /// read span)`. Empty when no workspace is seeded or no member free-reads
    /// `name`. The read-site mirror of [`workspace_def_sites`](Self::workspace_def_sites)
    /// and the first consumer of [`project_reads`](crate::project::project_reads):
    /// it supplies the range-free set of reading files, and each span is recovered
    /// per file via [`read_ranges_in`](Self::read_ranges_in) against its current
    /// text. Backs cross-file find-references. A pure read — the caller wraps it in
    /// [`salsa::Cancelled::catch`].
    pub fn workspace_read_sites(&self, name: &str) -> Vec<(PathBuf, TextRange)> {
        if self.0.workspace().is_none() {
            return Vec::new();
        }
        let project = workspace_project(&self.0);
        let index = project_reads(&self.0, project);
        let Some(paths) = index.by_name.get(name) else {
            return Vec::new();
        };
        paths
            .iter()
            .filter_map(|path| {
                let file = self.0.lookup_file(path)?;
                Some((path.clone(), file))
            })
            .flat_map(|(path, file)| {
                self.read_ranges_in(file, name)
                    .into_iter()
                    .map(move |range| (path.clone(), range))
            })
            .collect()
    }

    /// Resolve the cross-file binding for the top-level `name` defined in
    /// `def_file`, scoped to the visibility component instead of the global
    /// name-keyed [`workspace_def_sites`](Self::workspace_def_sites)/
    /// [`workspace_read_sites`](Self::workspace_read_sites) view.
    ///
    /// The cohort is `def_file` plus the files in its component
    /// ([`ProjectScope::sees`](crate::project::ProjectScope::sees), in either
    /// direction) that also define `name`; the readers are the files that can
    /// see `def_file` ([`ProjectScope::seen_by`](crate::project::ProjectScope::seen_by)),
    /// free-read `name`, and don't shadow it with their own top-level
    /// definition. Span-free: spans are recovered per site downstream. A pure
    /// read — the caller wraps it in [`salsa::Cancelled::catch`]. Empty when no
    /// workspace is seeded.
    pub fn cross_file_binding(&self, def_file: &Path, name: &str) -> CrossFileBinding {
        if self.0.workspace().is_none() {
            return CrossFileBinding::default();
        }
        let project = workspace_project(&self.0);
        let graph = project_graph(&self.0, project);
        let defs = project_defs(&self.0, project);
        let reads = project_reads(&self.0, project);

        let def_paths: BTreeSet<PathBuf> = defs
            .by_name
            .get(name)
            .map(|sites| sites.iter().map(|(path, _kind)| path.clone()).collect())
            .unwrap_or_default();

        // Cohort: the defining files that share def_file's flat namespace slot —
        // def_file plus its package siblings that also define the name. A
        // `source()`-connected file that defines the same name is a *shadow*
        // (visible but order-resolved), not an alias, so it stays out; and a
        // disjoint script defining the same name is unrelated.
        let siblings = graph.package_siblings(def_file);
        let cohort: Vec<PathBuf> = def_paths
            .iter()
            .filter(|d| d.as_path() == def_file || siblings.contains(d.as_path()))
            .cloned()
            .collect();
        let conflict = cohort.len() > 1;
        // A multi-def cohort is, by construction, all package siblings (the only
        // multi-member source — `source()`-shadows and disjoint scripts are
        // filtered out above), so every extra member shares def_file's package
        // root. Lock that invariant: it is what makes rename-all sound and the
        // package-completeness gate the right (and only needed) refusal.
        debug_assert!(
            cohort.len() <= 1
                || cohort.iter().all(|d| d.as_path() == def_file
                    || graph.package_siblings(def_file).contains(d.as_path())),
            "multi-def cohort must be pure package siblings of def_file"
        );
        // Aliases of one flat slot rename together safely *iff* the package's
        // member set is complete; otherwise an unanalyzed sibling could hide a
        // def/read and the rename-all would be partial.
        let cohort_incomplete = conflict && !graph.package_complete(def_file);

        // Readers: files that can see def_file, free-read the name, and don't
        // shadow it with their own top-level definition (those are cohort defs,
        // not external readers).
        let seen_by = graph.seen_by(def_file);
        let readers: Vec<PathBuf> = reads
            .by_name
            .get(name)
            .map(|paths| {
                paths
                    .iter()
                    .filter(|r| seen_by.contains(r.as_path()) && !def_paths.contains(r.as_path()))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        // Order-aware refinement: a reader's *top-level* reads of the name must
        // all bind to a cohort member under sequential load order. A read before
        // the `source()` that injects the def (or one bound elsewhere / poisoned)
        // would be wrongly co-renamed, and we can't skip it selectively without
        // per-read spans — so flag the rename as ambiguous (it refuses). Reads in
        // function bodies run against the final scope (`NoTopLevelRead`) and are
        // resolved by the position-blind `seen_by` membership above.
        let order_ambiguous =
            readers
                .iter()
                .any(|reader| match graph.top_level_read_binding(reader, name) {
                    ReadBinding::NoTopLevelRead => false,
                    ReadBinding::Resolved(def) => !cohort.contains(&def),
                    ReadBinding::Unresolved | ReadBinding::OrderUnknown => true,
                });

        let project_has_dynamic_source = !reverse_source_edges(&self.0, project)
            .dynamic_sources
            .is_empty();

        CrossFileBinding {
            cohort,
            readers,
            conflict,
            cohort_incomplete,
            order_ambiguous,
            project_has_dynamic_source,
        }
    }

    /// The member files that define top-level `name` and are visible from
    /// `from_file` (its [`sees`](crate::project::ProjectScope::sees) set). Used to
    /// resolve a bare free read to the definition it binds to: exactly one
    /// visible def is an unambiguous resolution; zero or more than one is not. A
    /// pure read — the caller wraps it in [`salsa::Cancelled::catch`].
    pub fn visible_def_files(&self, from_file: &Path, name: &str) -> Vec<PathBuf> {
        if self.0.workspace().is_none() {
            return Vec::new();
        }
        let project = workspace_project(&self.0);
        let graph = project_graph(&self.0, project);
        let defs = project_defs(&self.0, project);
        let seen = graph.sees(from_file);
        defs.by_name
            .get(name)
            .map(|sites| {
                sites
                    .iter()
                    .map(|(path, _kind)| path.clone())
                    .filter(|path| seen.contains(path.as_path()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Edits that rewrite `source("old")` literals in dependents when files are
    /// renamed/moved (`workspace/willRenameFiles`). Each `(old, new)` pair is a
    /// file rename; the result is `(sourcer path, literal token range, new
    /// quoted literal)` triples, range-bearing so the LSP layer can position
    /// them (it stays free of `lsp_types`, mirroring
    /// [`cross_file_rename_edits`](crate::lsp)).
    ///
    /// Found via the reverse `source()` graph
    /// ([`reverse_source_edges`]): its keys are the un-normalized resolved
    /// targets, so matching against an incoming `old` path normalizes both sides
    /// ([`normalize_path`]). A dynamic `source(var)` is never in the forward
    /// `sourced_by` map, so it is left untouched. A pure read — the caller wraps
    /// it in [`salsa::Cancelled::catch`].
    pub fn source_rename_edits(
        &self,
        renames: &[(PathBuf, PathBuf)],
    ) -> Vec<(PathBuf, TextRange, String)> {
        if self.0.workspace().is_none() {
            return Vec::new();
        }
        // Normalized old → new, dropping no-op renames.
        let targets: Vec<(PathBuf, PathBuf)> = renames
            .iter()
            .map(|(old, new)| (normalize_path(old), normalize_path(new)))
            .filter(|(old, new)| old != new)
            .collect();
        if targets.is_empty() {
            return Vec::new();
        }

        let project = workspace_project(&self.0);
        let rev = reverse_source_edges(&self.0, project);

        // Candidate sourcers per normalized target: the reverse map keys are
        // un-normalized, so normalize each before matching.
        let mut sourcers: BTreeSet<PathBuf> = BTreeSet::new();
        for (key, members) in &rev.sourced_by {
            if targets.iter().any(|(old, _)| normalize_path(key) == *old) {
                sourcers.extend(members.iter().cloned());
            }
        }

        let mut edits = Vec::new();
        for sourcer in sourcers {
            let Some(file) = self.lookup_file(&sourcer) else {
                continue;
            };
            let text = self.file_text(file);
            let root = parse(text).cst;
            let base_dir = sourcer.parent();
            for edge in collect_source_literal_edges(&root, base_dir) {
                let edge_norm = normalize_path(&edge.target);
                let Some((_, new)) = targets.iter().find(|(old, _)| *old == edge_norm) else {
                    continue;
                };
                // Preserve the original quote; recompute the spelling, keeping the
                // relative/absolute shape the author wrote.
                let new_spelling = if edge.was_relative {
                    base_dir
                        .map(normalize_path)
                        .and_then(|dir| relative_path(&dir, new))
                        .map(|rel| to_forward_slash(&rel))
                        .unwrap_or_else(|| to_forward_slash(new))
                } else {
                    to_forward_slash(new)
                };
                let quote = edge.quote as char;
                // No escaping: skip rather than corrupt if the path carries the
                // quote byte (vanishingly rare for R source paths).
                if new_spelling.as_bytes().contains(&edge.quote) {
                    continue;
                }
                edits.push((
                    sourcer.clone(),
                    edge.literal_range,
                    format!("{quote}{new_spelling}{quote}"),
                ));
            }
        }
        edits
    }

    /// Re-resolve a [`NodePtr`] taken against `taken_at_text` to a node in
    /// `file`'s *current* parse tree.
    ///
    /// When the snapshot's text still equals `taken_at_text` the handle resolves
    /// directly. Otherwise the stored range is first mapped through the single
    /// edit separating the two texts ([`diff_edit`] + [`map_range_through_edit`])
    /// before resolving against the new tree — the same `file_text != text`
    /// staleness signal hover uses, here turned into an offset fix-up rather than
    /// a bail-out. Returns `None` (caller falls back to position/name
    /// re-resolution) when the node was edited or the mapped range no longer names
    /// a node of that kind. A pure read: the caller wraps it in
    /// [`salsa::Cancelled::catch`], as hover wraps `hover_from_node`.
    pub fn resolve_ptr(
        &self,
        file: SourceFile,
        ptr: NodePtr,
        taken_at_text: &str,
    ) -> Option<SyntaxNode> {
        let root = self.parsed_tree(file);
        if self.file_text(file) == taken_at_text {
            return ptr.try_to_node(&root);
        }
        let edit = diff_edit(taken_at_text, self.file_text(file));
        let mapped = map_range_through_edit(ptr.text_range(), &edit)?;
        ptr.with_range(mapped).try_to_node(&root)
    }

    /// The installed [`LibraryIndex`] singleton handle, if any. The read-phase
    /// uses it to key the [`external_resolution`](crate::project::external_resolution)
    /// query.
    pub fn library_index(&self) -> Option<LibraryIndex> {
        self.0.library_index()
    }

    /// The harvested package index payload, if installed. Hover reads the rich
    /// per-symbol data from this snapshot rather than carrying a separate `Arc`,
    /// so it sees exactly the index the lint thread last set.
    pub fn library_data(&self) -> Option<Arc<IndexedProvider>> {
        self.0
            .library_index()
            .map(|index| index.data(&self.0).clone())
    }

    /// Borrow the underlying db as the salsa query trait, for read-phase free
    /// functions (`intern_project`, `visible_symbols`). A shared borrow can't
    /// mutate, so this preserves the read-only guarantee; crate-private so read
    /// jobs go through the methods above and never reach the trait.
    pub(crate) fn as_db(&self) -> &dyn IncrementalDb {
        &self.0
    }
}

#[salsa::db]
impl salsa::Database for IncrementalDatabase {}

#[salsa::db]
impl IncrementalDb for IncrementalDatabase {
    fn record_query(&self, entry: QueryLogEntry) {
        self.query_log
            .lock()
            .expect("query log mutex poisoned")
            .push(entry);
    }

    fn reparse_prev(&self, file: SourceFile) -> Option<Arc<PrevParse>> {
        self.reparse_cache
            .lock()
            .expect("reparse cache mutex poisoned")
            .get(&file)
            .cloned()
    }

    fn reparse_store(&self, file: SourceFile, prev: PrevParse, incremental: bool) {
        if incremental {
            self.reparse_hits.fetch_add(1, Ordering::Relaxed);
        }
        self.reparse_cache
            .lock()
            .expect("reparse cache mutex poisoned")
            .insert(file, Arc::new(prev));
    }
}
