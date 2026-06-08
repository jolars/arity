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
use std::sync::{Arc, Mutex};

use salsa::Setter;

use crate::parser::parse;
use crate::project::SourceEdgeKey;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueryKind {
    ParsedDocument,
    SemanticModel,
    FileExports,
    FileFreeReads,
    SourceEdges,
    ProjectGraph,
    VisibleSymbols,
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

#[salsa::db]
pub trait IncrementalDb: salsa::Database {
    fn record_query(&self, entry: QueryLogEntry);
}

#[salsa::tracked(returns(ref), no_eq, unsafe(non_update_types))]
pub fn parsed_document(db: &dyn IncrementalDb, file: SourceFile) -> ParsedDocument {
    db.record_query(QueryLogEntry {
        kind: QueryKind::ParsedDocument,
        file: Some(file),
    });

    let parsed = parse(file.text(db).as_str());
    let diagnostics = parsed
        .diagnostics
        .into_iter()
        .map(|diagnostic| ParseDiagnosticData {
            message: diagnostic.message,
            start: diagnostic.start,
            end: diagnostic.end,
        })
        .collect();

    ParsedDocument {
        green: parsed.cst.green().into_owned(),
        diagnostics,
    }
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
}

impl Default for IncrementalDatabase {
    fn default() -> Self {
        Self {
            storage: salsa::Storage::new(None),
            query_log: Arc::new(Mutex::new(Vec::new())),
            files: Arc::new(Mutex::new(HashMap::new())),
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
}
