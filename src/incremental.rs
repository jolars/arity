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
use crate::project::{DefKind, SourceEdgeKey, project_defs, workspace_project};
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
fn normalize_path(path: &Path) -> PathBuf {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueryKind {
    ParsedDocument,
    SemanticModel,
    FileExports,
    FileFreeReads,
    FileDefSites,
    SourceEdges,
    ReverseSourceEdges,
    WorkspaceProject,
    ProjectGraph,
    ProjectDefs,
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
        match Workspace::try_get(self) {
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
