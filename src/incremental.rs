//! Salsa-backed incremental layer: file text → parse tree → semantic model.
//!
//! The CST is cached as a `rowan::GreenNode` (Arc-backed, `Send + Sync`) rather
//! than a `SyntaxNode` (which holds non-`Send` cursor state and is neither
//! `Eq` nor `salsa::Update`). Callers materialize a fresh cursor via
//! [`parsed_tree_root`] — a cheap atomic clone — so each consumer gets its own
//! tree without leaking the salsa cell. The per-file [`semantic_model`] query
//! builds on the cached tree, so the linter and LSP no longer re-parse and
//! rebuild the model from text on every run.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use salsa::{Durability, Setter};

use crate::parser::{ParseDiagnostic, diff_edit, parse, reparse};
use crate::project::SourceEdgeKey;
use crate::rindex::provider::IndexedProvider;
use crate::semantic::SemanticModel;
use crate::syntax::SyntaxNode;

#[salsa::input]
pub struct SourceFile {
    /// The path this file was tracked under. Set once at creation and never
    /// mutated, so path-keyed queries (e.g. [`source_edges`], which resolves
    /// relative `source()` targets against `path.parent()`) don't re-run on a
    /// text edit. In-memory files (see [`IncrementalDatabase::add_file`]) get a
    /// unique synthetic path so they never collide.
    #[returns(ref)]
    pub path: PathBuf,
    #[returns(ref)]
    pub text: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueryKind {
    ParsedDocument,
    SemanticModel,
    FileExports,
    FileFreeReads,
    SourceEdges,
    ProjectGraph,
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
    crate::project::collect_source_edge_keys(&root, file.path(db).parent())
}

#[salsa::db]
pub struct IncrementalDatabase {
    storage: salsa::Storage<Self>,
    query_log: Arc<Mutex<Vec<QueryLogEntry>>>,
    /// Path → input mapping, so repeated edits to the same path reuse the same
    /// `SourceFile` input (and thus its cached queries) instead of creating a
    /// fresh one each time. Seeds the cross-file project graph (Phase B).
    files: Arc<Mutex<HashMap<PathBuf, SourceFile>>>,
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
            files: Arc::new(Mutex::new(HashMap::new())),
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
            files: Arc::clone(&self.files),
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
    /// Track an in-memory document with no on-disk path. Each call mints a
    /// unique synthetic path so two in-memory files never alias in a path-keyed
    /// query. Used by tests and one-shot single-file checks; the LSP/CLI use
    /// [`upsert_file`](Self::upsert_file) with the real path.
    pub fn add_file(&self, text: impl Into<String>) -> SourceFile {
        let path = PathBuf::from(format!("<mem>/{}.R", uuid::Uuid::new_v4()));
        SourceFile::new(self, path, text.into())
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

    /// The harvested package index payload, if installed. A cheap `Arc` clone.
    pub fn library_data(&self) -> Option<Arc<IndexedProvider>> {
        LibraryIndex::try_get(self).map(|index| index.data(self).clone())
    }

    /// Insert or update the input for `path`, reusing the existing `SourceFile`
    /// when one is already tracked. The hot path for editor buffers: a keystroke
    /// updates the text of an existing input so unchanged downstream queries stay
    /// cached.
    pub fn upsert_file(&mut self, path: &Path, text: String) -> SourceFile {
        let existing = self
            .files
            .lock()
            .expect("file cache mutex poisoned")
            .get(path)
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
                let file = SourceFile::new(self, path.to_path_buf(), text);
                self.files
                    .lock()
                    .expect("file cache mutex poisoned")
                    .insert(path.to_path_buf(), file);
                file
            }
        }
    }

    /// The `SourceFile` input currently tracked for `path`, if any. Read-only:
    /// unlike [`upsert_file`](Self::upsert_file) it never inserts, so it is safe
    /// to call on a shared clone (the language server's read path uses it to find
    /// the cached parse for the buffer under the cursor).
    pub fn lookup_file(&self, path: &Path) -> Option<SourceFile> {
        self.files
            .lock()
            .expect("file cache mutex poisoned")
            .get(path)
            .copied()
    }

    /// The text currently tracked for `file`.
    pub fn file_text(&self, file: SourceFile) -> &str {
        file.text(self)
    }

    /// The path `file` is tracked under.
    pub fn file_path(&self, file: SourceFile) -> &Path {
        file.path(self)
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

    /// The path `file` is tracked under.
    pub fn file_path(&self, file: SourceFile) -> &Path {
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
