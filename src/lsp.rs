//! Stdio-based LSP server (built on `lsp-server`): formatting, pushed
//! diagnostics, quick-fix code actions, and hover backed by the introspection
//! index.
//!
//! Architecture (see the dedicated-lint-thread design): the main loop owns no
//! salsa database. A dedicated thread owns the persistent [`IncrementalDatabase`]
//! and is the sole *writer* — salsa is strictly single-writer, and cross-file
//! linting writes sibling files into the db. Each lint is split into a cheap
//! **write-phase** ([`prepare_document_in_project`](crate::linter::check::prepare_document_in_project),
//! `&mut db`, on the lint thread: upsert the live buffer + siblings) and an
//! expensive **read-phase** ([`analyze_prepared`](crate::linter::check::analyze_prepared),
//! `&db` only) that runs on the read pool holding a short-lived db clone. The
//! lint thread returns to its `select!` right after the write-phase, so a long
//! analyze no longer blocks queued reads.
//!
//! Threading uses two purpose-built [`TaskPool`](task_pool::TaskPool)s rather
//! than rayon's global pool (which has no priority concept): a **read pool**
//! sized to the machine's parallelism serves latency-sensitive work (formatting,
//! hover, the analyze read-phase, code actions), and a **single-thread index
//! pool** isolates the one unbounded-duration job — background package indexing
//! ([`build_index`]) — so a long harvest can never *slot-block* a read.
//! `build_index` itself fans the per-package harvest across rayon underneath
//! that single index thread, shortening the build's CPU-contention window
//! without ever competing for read-pool slots. (CLI format/lint stays
//! sequential.)
//!
//! Requests are *coalesced* (latest version per URI; stale edits dropped) into a
//! pending queue. A [`decide`] scheduler keeps at most one analyze in flight: a
//! strictly-newer edit of the *same* URI cancels the running analyze via
//! [`salsa::Database::trigger_cancellation`] (the worker's [`salsa::Cancelled`]
//! catch then publishes nothing), while a *different* pending URI waits its turn
//! — never cross-cancelled, so a multi-URI [`Outbound::RelintAll`] still publishes
//! every file. Diagnostics route back through the main loop, which drops publishes
//! for closed or superseded documents (a version gate that backstops the rare
//! finish-during-cancel race).
//!
//! Read-only requests reuse the lint thread's cached work rather than re-parsing:
//! - **Formatting and hover** are sent to the lint thread as [`ReadJob`]s; it
//!   mints a short-lived db clone and runs the job on the read pool ([`run_read`]),
//!   formatting/hovering off the cached parse tree when the tracked buffer still
//!   matches the live text. A clone outstanding when the lint thread writes trips
//!   [`salsa::Cancelled`]; both that and a cache miss fall back to a fresh parse,
//!   so reads are always correct, only sometimes warm.
//! - **Code actions** are served from the findings of the most recent lint
//!   (cached per URI by version in the main loop), with no parse or lint at all
//!   when the version matches; otherwise they fall back to an independent lint.

// `lsp_types::Uri` (a `fluent_uri` newtype) carries an internal `Cell` tag for
// its mutable-view mechanism, which trips `clippy::mutable_key_type` when a
// `Uri` is used as a map key. Our URIs are owned + parsed (never "taken"), and
// `Uri`'s `Hash`/`Eq` go through `as_str()`, so this is sound; `WorkspaceEdit`
// also forces `HashMap<Uri, _>` on us. Allow it module-wide.
#![allow(clippy::mutable_key_type)]

mod task_pool;

use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender, select};
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeConfiguration, DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument,
    Notification as NotificationTrait, PublishDiagnostics,
};
use lsp_types::request::{
    CodeActionRequest, DocumentHighlightRequest, DocumentSymbolRequest, Formatting, GotoDefinition,
    HoverRequest, PrepareRenameRequest, RangeFormatting, References, Rename,
    Request as RequestTrait,
};
use lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionProviderCapability, CodeActionResponse, Diagnostic as LspDiagnostic,
    DiagnosticSeverity, DidChangeConfigurationParams, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DocumentFormattingParams,
    DocumentHighlight, DocumentHighlightKind, DocumentHighlightParams,
    DocumentRangeFormattingParams, DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
    HoverProviderCapability, InitializeResult, Location, MarkupContent, MarkupKind, NumberOrString,
    OneOf, Position, PrepareRenameResponse, PublishDiagnosticsParams, Range, ReferenceParams,
    RenameOptions, RenameParams, ServerCapabilities, ServerInfo, SymbolKind as LspSymbolKind,
    TextDocumentPositionParams, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Uri,
    WorkspaceEdit,
};
use rowan::{NodeOrToken, SyntaxToken, TextRange, TextSize, TokenAtOffset};
use salsa::Database as _;
use serde::Deserialize;
use smol_str::SmolStr;

use crate::ast::{AssignmentExpr, AstNode as _, BinaryExpr, FunctionExpr};
use crate::config::{Config, FormatConfig, IndexConfig, LintConfig};
use crate::file_discovery::collect_r_files;
use crate::formatter::{FormatStyle, format_node, format_range, format_with_style};
use crate::incremental::{Analysis, IncrementalDatabase, SourceFile};
use crate::linter::{Diagnostic, Severity};
use crate::parser::{diff_edit, map_range_through_edit, parse};
use crate::rindex::build::{BuildOptions, build_index};
use crate::rindex::cache::{Cache, resolve_cache_root};
use crate::rindex::discover::{referenced_in_source, with_default_packages};
use crate::rindex::libpaths::LibrarySearch;
use crate::rindex::provider::{CompositeProvider, IndexedProvider, resolve_origin};
use crate::rindex::schema::{Formal, SymbolEntry, SymbolKind};
use crate::semantic::{BindingId, BindingKind, PackageOrigin, SemanticModel};
use crate::syntax::{NodePtr, RLanguage, SyntaxKind, SyntaxNode};
use crate::text::LineIndex;
use task_pool::{Spawner, TaskPool, read_pool_size};

type DynError = Box<dyn std::error::Error + Sync + Send>;

/// Run the language server on stdio until the client disconnects.
pub fn run() -> Result<(), DynError> {
    let (connection, io_threads) = Connection::stdio();

    let (id, params) = connection.initialize_start()?;
    let editor_settings = params
        .get("initializationOptions")
        .map(EditorSettings::from_client_value)
        .unwrap_or_default();
    let workspace_roots = workspace_roots_from_params(&params);
    let init_result = InitializeResult {
        capabilities: server_capabilities(),
        server_info: Some(ServerInfo {
            name: "arity".to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }),
    };
    connection.initialize_finish(id, serde_json::to_value(init_result)?)?;

    main_loop(connection, editor_settings, workspace_roots)?;
    io_threads.join()?;
    Ok(())
}

/// Extract the workspace roots from the `initialize` params: the
/// `workspaceFolders` array if present, else the legacy `rootUri`. Non-`file`
/// URIs are skipped. Drives the one-time workspace seed (see [`LintWorker`]).
fn workspace_roots_from_params(params: &serde_json::Value) -> Vec<PathBuf> {
    let from_uri = |s: &str| s.parse::<Uri>().ok().and_then(|u| uri::to_path(&u));
    let mut roots: Vec<PathBuf> = params
        .get("workspaceFolders")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|folder| folder.get("uri").and_then(|u| u.as_str()))
        .filter_map(from_uri)
        .collect();
    if roots.is_empty()
        && let Some(path) = params
            .get("rootUri")
            .and_then(|u| u.as_str())
            .and_then(from_uri)
    {
        roots.push(path);
    }
    roots
}

fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        document_formatting_provider: Some(OneOf::Left(true)),
        document_range_formatting_provider: Some(OneOf::Left(true)),
        code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        document_highlight_provider: Some(OneOf::Left(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        rename_provider: Some(OneOf::Right(RenameOptions {
            prepare_provider: Some(true),
            work_done_progress_options: Default::default(),
        })),
        ..Default::default()
    }
}

/// The main event loop: dispatch incoming JSON-RPC messages and lint results.
/// Owns the connection so that returning drops the sender and lets the writer
/// thread finish; joins the lint thread before returning.
fn main_loop(
    connection: Connection,
    editor_settings: EditorSettings,
    workspace_roots: Vec<PathBuf>,
) -> Result<(), DynError> {
    let (out_tx, out_rx) = crossbeam_channel::unbounded::<Outbound>();
    let (lint_tx, lint_rx) = crossbeam_channel::unbounded::<LintMsg>();
    let (read_tx, read_rx) = crossbeam_channel::unbounded::<ReadJob>();

    // The read pool serves latency-sensitive work (formatting, hover, the analyze
    // read-phase, code actions). Its `_workers` must outlive both `state` and the
    // lint thread; the drop order at the end of this function guarantees that.
    let read_pool = TaskPool::new("arity-lsp-read", read_pool_size());
    let lint_handle = spawn_lint_thread(lint_rx, read_rx, out_tx, read_pool.spawner());
    // `done_tx`/`done_rx` are created inside the lint thread (see
    // `spawn_lint_thread`) so the main loop never holds the read end.

    // Seed the explicit workspace file-set once, before any document traffic, so
    // cross-file queries see the whole workspace. The lint thread owns the db, so
    // the walk + upsert happen there (off the main loop).
    if !workspace_roots.is_empty() {
        let _ = lint_tx.send(LintMsg::SeedWorkspace {
            roots: workspace_roots,
        });
    }

    let mut state = GlobalState::new(
        connection.sender.clone(),
        lint_tx,
        read_tx,
        read_pool.spawner(),
        editor_settings,
    );

    loop {
        select! {
            recv(connection.receiver) -> msg => {
                let Ok(msg) = msg else { break };
                match msg {
                    Message::Request(req) => {
                        if connection.handle_shutdown(&req)? {
                            break;
                        }
                        state.on_request(req);
                    }
                    Message::Notification(not) => state.on_notification(not),
                    Message::Response(_) => {}
                }
            }
            recv(out_rx) -> ob => {
                let Ok(ob) = ob else { break };
                state.on_outbound(ob);
            }
        }
    }

    drop(state); // drops lint_tx → the lint thread's recv disconnects → it exits
    let _ = lint_handle.join();
    Ok(())
}

#[derive(Debug, Clone)]
struct Document {
    text: String,
    version: i32,
}

#[derive(Debug, Clone)]
struct ResolvedSettings {
    style: FormatStyle,
    lint: LintConfig,
    index: IndexConfig,
}

/// Formatter knobs the editor can push via `initializationOptions` (at startup)
/// or `workspace/didChangeConfiguration` (later). These are the *fallback*: a
/// discovered `arity.toml` is authoritative and ignores them entirely. Fields
/// are `Option` so an unset key leaves the built-in default in place.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct EditorSettings {
    line_width: Option<u32>,
    indent_width: Option<u32>,
}

impl EditorSettings {
    /// Extract our settings from a client-supplied JSON value. Accepts either
    /// the bare options object or a tree namespaced under a `"arity"` key (how
    /// `workspace/didChangeConfiguration` clients typically scope settings).
    /// Unknown keys are ignored, and a malformed value yields the defaults.
    fn from_client_value(value: &serde_json::Value) -> Self {
        let section = value
            .get("arity")
            .filter(|v| v.is_object())
            .unwrap_or(value);
        serde_json::from_value(section.clone()).unwrap_or_default()
    }

    /// The [`FormatStyle`] these settings imply, layered over the built-in
    /// defaults. Out-of-range values are rejected wholesale (falling back to
    /// defaults), reusing [`FormatConfig`]'s validation bounds — the LSP has no
    /// good channel to report a bad editor setting, so we ignore it.
    fn to_format_style(&self) -> FormatStyle {
        let mut config = FormatConfig::default();
        if let Some(width) = self.line_width {
            config.line_width = width;
        }
        if let Some(width) = self.indent_width {
            config.indent_width = width;
        }
        match config.validate(None) {
            Ok(()) => FormatStyle::from(&config),
            Err(_) => FormatStyle::default(),
        }
    }
}

/// Resolve the [`FormatStyle`] for a document: a discovered `arity.toml`
/// (`config_present`) wins outright; otherwise editor-pushed settings apply over
/// the built-in defaults.
fn resolve_format_style(
    config: &Config,
    config_present: bool,
    editor: &EditorSettings,
) -> FormatStyle {
    if config_present {
        FormatStyle::from(&config.format)
    } else {
        editor.to_format_style()
    }
}

/// A lint request handed to the dedicated lint thread.
struct LintRequest {
    uri: Uri,
    path: PathBuf,
    text: String,
    version: i32,
    lint_config: LintConfig,
    index_config: IndexConfig,
}

enum LintMsg {
    // Boxed: `LintRequest` is much larger than the other variant, so boxing keeps
    // the enum (and every channel slot) small.
    Request(Box<LintRequest>),
    /// Seed the explicit workspace file-set from the discovered roots (sent once
    /// at startup). Handled on the lint thread, the sole db writer.
    SeedWorkspace {
        roots: Vec<PathBuf>,
    },
}

/// A read-only request the lint thread services by cloning its salsa db and
/// running the work off-thread on the read pool. Each variant carries the live buffer
/// `text` and the client `sender` so the worker can reply directly; the lint
/// thread only adds the db snapshot. See [`run_read`].
enum ReadJob {
    Format {
        id: RequestId,
        path: PathBuf,
        text: String,
        style: FormatStyle,
        sender: Sender<Message>,
    },
    FormatRange {
        id: RequestId,
        path: PathBuf,
        text: String,
        range: Range,
        style: FormatStyle,
        sender: Sender<Message>,
    },
    Hover {
        id: RequestId,
        path: PathBuf,
        text: String,
        position: Position,
        sender: Sender<Message>,
    },
    Definition {
        id: RequestId,
        path: PathBuf,
        /// The current document's URI — an intra-file hit reports a `Location`
        /// back into it, so unlike the other jobs this one needs the URI too.
        uri: Uri,
        text: String,
        position: Position,
        sender: Sender<Message>,
    },
    References {
        id: RequestId,
        path: PathBuf,
        /// In-file reads report `Location`s back into this URI; cross-file reads
        /// carry their own.
        uri: Uri,
        text: String,
        position: Position,
        include_declaration: bool,
        sender: Sender<Message>,
    },
    Rename {
        id: RequestId,
        path: PathBuf,
        /// In-file edits land in this URI; cross-file edits carry their own.
        uri: Uri,
        text: String,
        /// The cursor's byte offset, already resolved on the main thread (via the
        /// `prepareRename` anchor when present, else the request position) so the
        /// anchor state never crosses the thread boundary.
        offset: usize,
        new_name: String,
        sender: Sender<Message>,
    },
}

/// Messages from the lint thread back to the main loop.
enum Outbound {
    /// Diagnostics for `uri` at `version`; published only if still current. The
    /// raw `findings` ride along so the main loop can cache them and serve
    /// quick-fix code actions without re-linting (see [`GlobalState::findings`]).
    Diagnostics {
        uri: Uri,
        version: i32,
        diags: Vec<LspDiagnostic>,
        findings: Arc<Vec<Diagnostic>>,
    },
    /// A background index build completed; re-lint every open document.
    RelintAll,
}

// ---------------------------------------------------------------------------
// Main-loop state
// ---------------------------------------------------------------------------

struct GlobalState {
    documents: HashMap<Uri, Document>,
    /// The most recent lint findings per document, tagged with the version they
    /// were computed against. `textDocument/codeAction` serves quick fixes from
    /// here (a pure lookup) when the cached version still matches the buffer,
    /// avoiding an independent re-lint; a stale or missing entry falls back to
    /// [`compute_code_actions`].
    findings: HashMap<Uri, (i32, Arc<Vec<Diagnostic>>)>,
    /// The most recent `prepareRename` anchor per document. Holds a [`NodePtr`]
    /// to the identifier's enclosing node plus the buffer it was taken against,
    /// so the follow-up `rename` re-locates the cursor even if the buffer changed
    /// since prepare (the "anchor that survives typing"). Cleared on rename/close.
    rename_anchors: HashMap<Uri, RenameAnchor>,
    config_cache: HashMap<PathBuf, ResolvedSettings>,
    /// Editor-pushed formatter defaults; the fallback when no `arity.toml` is
    /// found. Updated by `workspace/didChangeConfiguration`.
    editor_settings: EditorSettings,
    sender: Sender<Message>,
    lint_tx: Sender<LintMsg>,
    /// Channel to the lint thread for read-only jobs (formatting, hover). The
    /// lint thread owns the salsa db, so it mints a short-lived clone per job and
    /// runs the read off-thread against the cached parse. See [`run_read`].
    read_tx: Sender<ReadJob>,
    /// Submit-side handle onto the read pool, for serving `textDocument/codeAction`
    /// off the main loop (a pure lookup over cached findings, or an independent
    /// re-lint). Shared with the lint thread, which uses it for read jobs and the
    /// analyze read-phase.
    read_spawner: Spawner,
}

impl GlobalState {
    fn new(
        sender: Sender<Message>,
        lint_tx: Sender<LintMsg>,
        read_tx: Sender<ReadJob>,
        read_spawner: Spawner,
        editor_settings: EditorSettings,
    ) -> Self {
        Self {
            documents: HashMap::new(),
            findings: HashMap::new(),
            rename_anchors: HashMap::new(),
            config_cache: HashMap::new(),
            editor_settings,
            sender,
            lint_tx,
            read_tx,
            read_spawner,
        }
    }

    fn on_request(&mut self, req: Request) {
        match req.method.as_str() {
            Formatting::METHOD => self.on_formatting(req),
            RangeFormatting::METHOD => self.on_range_formatting(req),
            CodeActionRequest::METHOD => self.on_code_action(req),
            HoverRequest::METHOD => self.on_hover(req),
            GotoDefinition::METHOD => self.on_definition(req),
            References::METHOD => self.on_references(req),
            DocumentHighlightRequest::METHOD => self.on_document_highlight(req),
            DocumentSymbolRequest::METHOD => self.on_document_symbol(req),
            PrepareRenameRequest::METHOD => self.on_prepare_rename(req),
            Rename::METHOD => self.on_rename(req),
            _ => {
                let resp = Response::new_err(
                    req.id,
                    ErrorCode::MethodNotFound as i32,
                    format!("unhandled method: {}", req.method),
                );
                let _ = self.sender.send(Message::Response(resp));
            }
        }
    }

    fn on_formatting(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<DocumentFormattingParams>(Formatting::METHOD) else {
            self.respond_err(id, "invalid formatting params");
            return;
        };
        let uri = params.text_document.uri;
        let Some(text) = self.documents.get(&uri).map(|d| d.text.clone()) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let Ok(settings) = self.resolve_settings(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        self.dispatch_read(ReadJob::Format {
            id,
            path,
            text,
            style: settings.style,
            sender: self.sender.clone(),
        });
    }

    fn on_range_formatting(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<DocumentRangeFormattingParams>(RangeFormatting::METHOD)
        else {
            self.respond_err(id, "invalid range formatting params");
            return;
        };
        let uri = params.text_document.uri;
        let Some(text) = self.documents.get(&uri).map(|d| d.text.clone()) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let Ok(settings) = self.resolve_settings(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        self.dispatch_read(ReadJob::FormatRange {
            id,
            path,
            text,
            range: params.range,
            style: settings.style,
            sender: self.sender.clone(),
        });
    }

    fn on_code_action(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<CodeActionParams>(CodeActionRequest::METHOD) else {
            self.respond_err(id, "invalid code action params");
            return;
        };
        let uri = params.text_document.uri;
        let Some((text, version)) = self
            .documents
            .get(&uri)
            .map(|d| (d.text.clone(), d.version))
        else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let range = params.range;
        let sender = self.sender.clone();

        // Fast path: the last lint's findings are still current, so serving quick
        // fixes is a pure lookup — no re-parse, no re-lint. Their byte ranges
        // index `text`, which the version match proves is the linted source.
        if let Some((cached_version, findings)) = self.findings.get(&uri)
            && *cached_version == version
        {
            let findings = Arc::clone(findings);
            self.read_spawner.spawn(move || {
                let actions = code_actions_from_findings(&findings, &text, &uri, range);
                let _ = sender.send(Message::Response(Response::new_ok(id, actions)));
            });
            return;
        }

        // Fallback: no findings for this version yet (e.g. a fix requested before
        // the debounced lint caught up) — lint this buffer independently.
        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        let lint = self
            .resolve_settings(&uri)
            .map(|s| s.lint)
            .unwrap_or_default();
        self.read_spawner.spawn(move || {
            let actions = compute_code_actions(&text, &path, &lint, &uri, range);
            let _ = sender.send(Message::Response(Response::new_ok(id, actions)));
        });
    }

    fn on_hover(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<HoverParams>(HoverRequest::METHOD) else {
            self.respond_err(id, "invalid hover params");
            return;
        };
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some(text) = self.documents.get(&uri).map(|d| d.text.clone()) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        self.dispatch_read(ReadJob::Hover {
            id,
            path,
            text,
            position,
            sender: self.sender.clone(),
        });
    }

    /// `textDocument/definition`: jump to the definition of the name under the
    /// cursor. A read-only job, dispatched to the lint thread like hover; the
    /// resolution (intra-file binding, else a cross-file workspace def) runs on
    /// the read pool. See [`definition_via_db`].
    fn on_definition(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<GotoDefinitionParams>(GotoDefinition::METHOD) else {
            self.respond_err(id, "invalid definition params");
            return;
        };
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some(text) = self.documents.get(&uri).map(|d| d.text.clone()) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        self.dispatch_read(ReadJob::Definition {
            id,
            path,
            uri,
            text,
            position,
            sender: self.sender.clone(),
        });
    }

    /// `textDocument/references`: every read site of the name under the cursor. A
    /// read-only job dispatched to the lint thread like definition; resolution
    /// (intra-file reads of the local binding, plus cross-file reads of a
    /// top-level name) runs on the read pool. See [`references_via_db`].
    fn on_references(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<ReferenceParams>(References::METHOD) else {
            self.respond_err(id, "invalid references params");
            return;
        };
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let include_declaration = params.context.include_declaration;
        let Some(text) = self.documents.get(&uri).map(|d| d.text.clone()) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        self.dispatch_read(ReadJob::References {
            id,
            path,
            uri,
            text,
            position,
            include_declaration,
            sender: self.sender.clone(),
        });
    }

    /// `textDocument/documentHighlight`: the definition and reads of the local
    /// binding under the cursor, in the current file only — a degenerate same-file
    /// references query. Pure (no workspace snapshot needed), so it runs straight
    /// on the read pool like the cached code-action fast path. See
    /// [`compute_document_highlights`].
    fn on_document_highlight(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) =
            req.extract::<DocumentHighlightParams>(DocumentHighlightRequest::METHOD)
        else {
            self.respond_err(id, "invalid documentHighlight params");
            return;
        };
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some(text) = self.documents.get(&uri).map(|d| d.text.clone()) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let sender = self.sender.clone();
        self.read_spawner.spawn(move || {
            let line_index = LineIndex::new(&text);
            let offset = line_index.position_to_byte(position).min(text.len());
            let result = compute_document_highlights(&text, offset).map(|highlights| {
                highlights
                    .into_iter()
                    .map(|(range, kind)| DocumentHighlight {
                        range: text_range_to_lsp_range(&line_index, range),
                        kind: Some(kind),
                    })
                    .collect::<Vec<_>>()
            });
            let _ = sender.send(Message::Response(Response::new_ok(id, result)));
        });
    }

    /// `textDocument/documentSymbol`: the file's function and variable bindings
    /// as a hierarchical outline. Pure and single-file (no workspace lookup), so
    /// like document highlight it runs straight on the read pool rather than
    /// through the lint thread. See [`compute_document_symbols`].
    fn on_document_symbol(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<DocumentSymbolParams>(DocumentSymbolRequest::METHOD)
        else {
            self.respond_err(id, "invalid documentSymbol params");
            return;
        };
        let uri = params.text_document.uri;
        let Some(text) = self.documents.get(&uri).map(|d| d.text.clone()) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let sender = self.sender.clone();
        self.read_spawner.spawn(move || {
            let symbols = compute_document_symbols(&text);
            let response = DocumentSymbolResponse::Nested(symbols);
            let _ = sender.send(Message::Response(Response::new_ok(id, response)));
        });
    }

    /// `textDocument/prepareRename`: confirm the cursor sits on a renameable
    /// local identifier and return its range + placeholder. Computed
    /// synchronously (a single cheap parse) because the result anchors per-URI
    /// state on the main thread — these requests are deliberate and infrequent,
    /// unlike hover/format which offload to the read pool.
    fn on_prepare_rename(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) =
            req.extract::<TextDocumentPositionParams>(PrepareRenameRequest::METHOD)
        else {
            self.respond_err(id, "invalid prepareRename params");
            return;
        };
        let uri = params.text_document.uri;
        let Some(text) = self.documents.get(&uri).map(|d| d.text.clone()) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let line_index = LineIndex::new(&text);
        let offset = line_index.position_to_byte(params.position).min(text.len());
        match compute_prepare_rename(&text, offset) {
            Some(prepared) => {
                self.rename_anchors.insert(uri, prepared.anchor);
                let response = PrepareRenameResponse::RangeWithPlaceholder {
                    range: prepared.range,
                    placeholder: prepared.placeholder,
                };
                self.respond_ok(id, serde_json::to_value(response).unwrap_or_default());
            }
            None => {
                self.rename_anchors.remove(&uri);
                self.respond_ok(id, serde_json::Value::Null);
            }
        }
    }

    /// `textDocument/rename`: build a [`WorkspaceEdit`] renaming the binding under
    /// the cursor and every dependent read of it across the workspace. The cursor
    /// offset is resolved here on the main thread — preferring the stored
    /// `prepareRename` anchor (so the rename targets the same binding even if the
    /// buffer was edited since prepare), falling back to the request's position —
    /// so only a plain offset crosses to the read pool, where [`rename_via_db`]
    /// gathers the cross-file edits off a db snapshot.
    fn on_rename(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<RenameParams>(Rename::METHOD) else {
            self.respond_err(id, "invalid rename params");
            return;
        };
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = params.new_name;
        let Some(text) = self.documents.get(&uri).map(|d| d.text.clone()) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };

        let offset = self
            .rename_anchors
            .get(&uri)
            .and_then(|anchor| rename_cursor_offset(&text, anchor))
            .unwrap_or_else(|| {
                let line_index = LineIndex::new(&text);
                line_index.position_to_byte(position).min(text.len())
            });
        // A rename consumes its anchor; a fresh prepare precedes any next rename.
        self.rename_anchors.remove(&uri);

        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        self.dispatch_read(ReadJob::Rename {
            id,
            path,
            uri,
            text,
            offset,
            new_name,
            sender: self.sender.clone(),
        });
    }

    /// Hand a read-only job to the lint thread (db owner), which snapshots the db
    /// and runs it on the read pool. If that channel is gone (shutdown in flight),
    /// reply `null` so the client isn't left waiting.
    fn dispatch_read(&self, job: ReadJob) {
        if let Err(crossbeam_channel::SendError(job)) = self.read_tx.send(job) {
            let (id, sender) = match job {
                ReadJob::Format { id, sender, .. } => (id, sender),
                ReadJob::FormatRange { id, sender, .. } => (id, sender),
                ReadJob::Hover { id, sender, .. } => (id, sender),
                ReadJob::Definition { id, sender, .. } => (id, sender),
                ReadJob::References { id, sender, .. } => (id, sender),
                ReadJob::Rename { id, sender, .. } => (id, sender),
            };
            let _ = sender.send(Message::Response(Response::new_ok(
                id,
                serde_json::Value::Null,
            )));
        }
    }

    fn on_notification(&mut self, not: Notification) {
        match not.method.as_str() {
            DidOpenTextDocument::METHOD => {
                if let Ok(params) =
                    not.extract::<DidOpenTextDocumentParams>(DidOpenTextDocument::METHOD)
                {
                    let uri = params.text_document.uri;
                    self.documents.insert(
                        uri.clone(),
                        Document {
                            text: params.text_document.text,
                            version: params.text_document.version,
                        },
                    );
                    self.send_lint(uri);
                }
            }
            DidChangeTextDocument::METHOD => {
                if let Ok(mut params) =
                    not.extract::<DidChangeTextDocumentParams>(DidChangeTextDocument::METHOD)
                    && let Some(change) = params.content_changes.pop()
                {
                    let uri = params.text_document.uri;
                    self.documents.insert(
                        uri.clone(),
                        Document {
                            text: change.text,
                            version: params.text_document.version,
                        },
                    );
                    self.send_lint(uri);
                }
            }
            DidCloseTextDocument::METHOD => {
                if let Ok(params) =
                    not.extract::<DidCloseTextDocumentParams>(DidCloseTextDocument::METHOD)
                {
                    let uri = params.text_document.uri;
                    self.documents.remove(&uri);
                    self.findings.remove(&uri);
                    self.rename_anchors.remove(&uri);
                    // Tell the client to clear stale diagnostics.
                    self.publish(uri, Vec::new(), None);
                }
            }
            DidChangeConfiguration::METHOD => {
                if let Ok(params) =
                    not.extract::<DidChangeConfigurationParams>(DidChangeConfiguration::METHOD)
                {
                    let updated = EditorSettings::from_client_value(&params.settings);
                    if updated != self.editor_settings {
                        self.editor_settings = updated;
                        // Drop cached resolutions so the new fallback is picked
                        // up on the next pull. A discovered `arity.toml` still
                        // wins, so docs in a configured workspace are unaffected.
                        // Format requests re-resolve on demand; lint output does
                        // not depend on these knobs, so no re-lint is needed.
                        self.config_cache.clear();
                    }
                }
            }
            _ => {}
        }
    }

    fn on_outbound(&mut self, ob: Outbound) {
        match ob {
            Outbound::Diagnostics {
                uri,
                version,
                diags,
                findings,
            } => {
                if matches!(self.documents.get(&uri), Some(d) if d.version == version) {
                    self.findings.insert(uri.clone(), (version, findings));
                    self.publish(uri, diags, Some(version));
                }
            }
            Outbound::RelintAll => {
                let uris: Vec<Uri> = self.documents.keys().cloned().collect();
                for uri in uris {
                    self.send_lint(uri);
                }
            }
        }
    }

    /// Send a lint request for `uri`'s current buffer to the lint thread.
    fn send_lint(&mut self, uri: Uri) {
        let Some(doc) = self.documents.get(&uri) else {
            return;
        };
        let text = doc.text.clone();
        let version = doc.version;
        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        let (lint_config, index_config) = match self.resolve_settings(&uri) {
            Ok(s) => (s.lint, s.index),
            Err(_) => (LintConfig::default(), IndexConfig::default()),
        };
        let _ = self.lint_tx.send(LintMsg::Request(Box::new(LintRequest {
            uri,
            path,
            text,
            version,
            lint_config,
            index_config,
        })));
    }

    fn resolve_settings(&mut self, uri: &Uri) -> Result<ResolvedSettings, ConfigResolveError> {
        let path = uri::to_path(uri).ok_or(ConfigResolveError::NonFileUri)?;
        let anchor = path
            .parent()
            .ok_or(ConfigResolveError::NoParentDirectory)?
            .to_path_buf();

        if let Some(s) = self.config_cache.get(&anchor) {
            return Ok(s.clone());
        }

        let (config, source) = Config::resolve(None, false, &anchor)
            .map_err(|err| ConfigResolveError::Config(err.to_string()))?;
        let resolved = ResolvedSettings {
            style: resolve_format_style(&config, source.is_some(), &self.editor_settings),
            lint: config.lint,
            index: config.index,
        };
        self.config_cache.insert(anchor, resolved.clone());
        Ok(resolved)
    }

    fn publish(&self, uri: Uri, diagnostics: Vec<LspDiagnostic>, version: Option<i32>) {
        let params = PublishDiagnosticsParams {
            uri,
            diagnostics,
            version,
        };
        let not = Notification::new(PublishDiagnostics::METHOD.to_string(), params);
        let _ = self.sender.send(Message::Notification(not));
    }

    fn respond_ok(&self, id: RequestId, value: serde_json::Value) {
        let _ = self
            .sender
            .send(Message::Response(Response::new_ok(id, value)));
    }

    fn respond_err(&self, id: RequestId, message: &str) {
        let resp = Response::new_err(id, ErrorCode::InvalidParams as i32, message.to_string());
        let _ = self.sender.send(Message::Response(resp));
    }
}

// ---------------------------------------------------------------------------
// Lint thread
// ---------------------------------------------------------------------------

/// Spawn the dedicated lint thread that owns the persistent salsa database.
fn spawn_lint_thread(
    lint_rx: Receiver<LintMsg>,
    read_rx: Receiver<ReadJob>,
    out_tx: Sender<Outbound>,
    read_spawner: Spawner,
) -> JoinHandle<()> {
    let (build_tx, build_rx) = crossbeam_channel::unbounded::<IndexedProvider>();
    let (done_tx, done_rx) = crossbeam_channel::unbounded::<AnalyzeDone>();
    std::thread::Builder::new()
        .name("arity-lint".to_string())
        .spawn(move || {
            // The single-thread index pool isolates the one unbounded-duration
            // job (background package harvesting) from the read pool, so a long
            // build can never starve a latency-sensitive read. Owned by the
            // worker, so its thread lives exactly as long as the lint thread.
            let mut worker = LintWorker {
                db: IncrementalDatabase::default(),
                index_loaded: HashSet::new(),
                index_attempts: HashSet::new(),
                out_tx,
                build_tx,
                done_tx,
                inflight: None,
                pending: HashMap::new(),
                read_spawner,
                index_pool: TaskPool::new("arity-index", 1),
            };
            worker.run(&lint_rx, &read_rx, &build_rx, &done_rx);
        })
        .expect("spawn lint thread")
}

/// Signal from a finished read-phase ([`LintWorker::start`]) back to the lint
/// thread: the analyze for `uri`@`version` has completed (or unwound on
/// cancellation) and dropped its db clone, so the in-flight slot is free.
struct AnalyzeDone {
    uri: Uri,
    version: i32,
}

/// The single in-flight read-phase analyze, if any.
struct InflightAnalyze {
    uri: Uri,
    version: i32,
}

/// What [`LintWorker::try_dispatch`] should do given the in-flight analyze and
/// the pending queue. Pure decision (see [`decide`]) so it can be unit-tested.
#[derive(Debug, PartialEq, Eq)]
enum DispatchAction {
    /// Idle with nothing queued, or busy with no newer edit for the in-flight
    /// URI: leave the in-flight analyze running and wait for its `done`.
    Wait,
    /// The slot is free; start a fresh analyze for this URI.
    Start(Uri),
    /// A strictly-newer edit for the *in-flight* URI arrived; cancel the running
    /// analyze and start this URI. Only ever the in-flight URI — a different
    /// pending URI must never cancel the in-flight one (it would drop that
    /// file's diagnostics under `RelintAll`).
    SupersedeAndStart(Uri),
}

/// Decide the next dispatch action. `inflight` is the running analyze's
/// `(uri, version)`, if any; `pending` maps each queued URI to its latest
/// version. Cancel only on a strictly-newer edit of the *same* URI.
fn decide(inflight: Option<(&Uri, i32)>, pending: &HashMap<Uri, i32>) -> DispatchAction {
    match inflight {
        None => match pending.keys().next() {
            Some(uri) => DispatchAction::Start(uri.clone()),
            None => DispatchAction::Wait,
        },
        Some((uri, version)) => {
            if pending.get(uri).is_some_and(|&v| v > version) {
                DispatchAction::SupersedeAndStart(uri.clone())
            } else {
                DispatchAction::Wait
            }
        }
    }
}

struct LintWorker {
    db: IncrementalDatabase,
    /// Workspace anchors whose index cache has already been loaded into the salsa
    /// [`LibraryIndex`] singleton.
    index_loaded: HashSet<PathBuf>,
    /// Packages a background harvest has already been scheduled for this session
    /// — never retried, so a not-installed package doesn't loop.
    index_attempts: HashSet<SmolStr>,
    out_tx: Sender<Outbound>,
    /// A finished background harvest sends its freshly-loaded index here; the
    /// lint thread (sole writer) installs it into salsa at HIGH durability.
    build_tx: Sender<IndexedProvider>,
    /// Read-phase workers signal completion here so the lint thread can free the
    /// in-flight slot and dispatch the next pending lint.
    done_tx: Sender<AnalyzeDone>,
    /// The single in-flight read-phase analyze, if any. At most one runs at a
    /// time: the write-phase needs exclusive `&mut db`, and salsa cancellation is
    /// global, so a second concurrent analyze couldn't be cancelled selectively.
    inflight: Option<InflightAnalyze>,
    /// Coalesced lint queue: the latest pending request per URI. Persists across
    /// `select!` iterations (it used to be a per-iteration local).
    pending: HashMap<Uri, LintRequest>,
    /// Submit-side handle onto the read pool, shared with the main loop. Used for
    /// read jobs (formatting, hover) and the analyze read-phase.
    read_spawner: Spawner,
    /// Single-thread pool that isolates background package indexing — the one
    /// unbounded-duration job — from the read pool.
    index_pool: TaskPool,
}

impl LintWorker {
    fn run(
        &mut self,
        lint_rx: &Receiver<LintMsg>,
        read_rx: &Receiver<ReadJob>,
        build_rx: &Receiver<IndexedProvider>,
        done_rx: &Receiver<AnalyzeDone>,
    ) {
        loop {
            select! {
                recv(lint_rx) -> msg => {
                    let Ok(msg) = msg else { break };
                    // Coalesce: keep only the latest version per URI, so a fast
                    // typist's stale edits are dropped before they're ever linted.
                    // A `SeedWorkspace` is applied inline (it's the db writer).
                    self.handle_lint_msg(msg);
                    while let Ok(m) = lint_rx.try_recv() {
                        self.handle_lint_msg(m);
                    }
                    self.try_dispatch();
                }
                recv(done_rx) -> done => {
                    let Ok(done) = done else { continue };
                    // Free the slot only if this `done` is for the *current*
                    // in-flight analyze — a late `done` from a superseded one
                    // (different version) must not clear the new analyze.
                    if matches!(&self.inflight, Some(f) if f.uri == done.uri && f.version == done.version) {
                        self.inflight = None;
                    }
                    self.try_dispatch();
                }
                recv(read_rx) -> job => {
                    let Ok(job) = job else { continue };
                    // Mint a short-lived read-only snapshot and run the job off the
                    // lint thread. The clone is dropped inside `run_read`, so the
                    // next write isn't blocked once the read finishes (or a racing
                    // write trips `salsa::Cancelled`, handled by the fallback).
                    let snapshot = self.db.snapshot();
                    self.read_spawner.spawn(move || run_read(snapshot, job));
                }
                recv(build_rx) -> built => {
                    let Ok(indexed) = built else { continue };
                    // Sole writer installs the freshly-harvested index at HIGH
                    // durability, then re-lints every open document against it.
                    self.db.set_library_index(indexed);
                    let _ = self.out_tx.send(Outbound::RelintAll);
                }
            }
        }
    }

    /// Dispatch a lint-channel message: queue a request, or apply a workspace
    /// seed inline (the lint thread is the sole db writer).
    fn handle_lint_msg(&mut self, msg: LintMsg) {
        match msg {
            LintMsg::Request(req) => self.enqueue(*req),
            LintMsg::SeedWorkspace { roots } => self.seed_workspace(roots),
        }
    }

    /// Walk the workspace roots once and install the discovered `.R` files as the
    /// explicit [`Workspace`](crate::incremental::Workspace) file-set, unioned with
    /// anything already tracked. Pre-warms cross-file membership so later edits
    /// don't re-walk (see [`seed_workspace_for`](crate::linter::check::seed_workspace_for)).
    fn seed_workspace(&mut self, roots: Vec<PathBuf>) {
        let discovered = collect_r_files(&roots).unwrap_or_default();
        let mut files: Vec<SourceFile> = self
            .db
            .workspace()
            .map(|ws| ws.members(&self.db).to_vec())
            .unwrap_or_default();
        for path in discovered {
            if let Ok(text) = std::fs::read_to_string(&path) {
                files.push(self.db.upsert_file(&path, text));
            }
        }
        self.db.set_workspace_members(files, roots);
    }

    /// Add `req` to the pending queue, keeping the highest version per URI (guards
    /// against an out-of-order lower version clobbering a newer one).
    fn enqueue(&mut self, req: LintRequest) {
        match self.pending.get(&req.uri) {
            Some(existing) if existing.version >= req.version => {}
            _ => {
                self.pending.insert(req.uri.clone(), req);
            }
        }
    }

    /// Start lints until the slot is occupied or the queue is exhausted (see
    /// [`decide`]). Cancels the in-flight analyze only when superseded by a newer
    /// edit of the *same* URI. Loops because a [`start`](Self::start) that hits a
    /// parse error spawns no worker (and thus no `done`), so the next pending URI
    /// must be picked up here rather than stalling until the next event — this is
    /// what keeps a multi-URI `RelintAll` draining.
    fn try_dispatch(&mut self) {
        loop {
            let versions: HashMap<Uri, i32> = self
                .pending
                .iter()
                .map(|(uri, req)| (uri.clone(), req.version))
                .collect();
            let inflight = self.inflight.as_ref().map(|f| (&f.uri, f.version));
            let uri = match decide(inflight, &versions) {
                DispatchAction::Wait => return,
                DispatchAction::Start(uri) => uri,
                DispatchAction::SupersedeAndStart(uri) => {
                    // Explicit cancellation: the write-phase may be a no-op (an
                    // unchanged `upsert_file` doesn't bump the revision), so we
                    // can't rely on it to unwind the running analyze. Blocks until
                    // the old clone drops; safe — this thread holds no clone.
                    self.db.trigger_cancellation();
                    self.inflight = None;
                    uri
                }
            };
            let Some(req) = self.pending.remove(&uri) else {
                return;
            };
            // A spawned worker occupies the slot; stop. Otherwise (parse error /
            // bad config) the slot is still free, so loop to the next pending URI.
            if self.start(req) {
                return;
            }
        }
    }

    /// Run one lint: the write-phase (`&mut db`, on this thread) then the
    /// read-phase analyze on the read pool holding a db clone. Returning to
    /// `select!` right after spawning keeps reads responsive (problem 2) and lets
    /// a fresher edit cancel the analyze (problem 1).
    ///
    /// Returns `true` if a worker was spawned (the in-flight slot is now busy),
    /// `false` if the buffer couldn't be linted (no worker, slot still free).
    fn start(&mut self, req: LintRequest) -> bool {
        let anchor = req
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        self.ensure_index(&anchor, &req.index_config);

        // Write-phase: push the live buffer + sibling files into the persistent
        // db. Cheap — the parse/model are lazy salsa queries deferred to analyze.
        let active = self.db.upsert_file(&req.path, req.text.clone());
        // Ensure the active file's project is in the workspace file-set. Lazy:
        // only walks disk when the file isn't already a member (the initialize
        // seed covers the common case), so discovery leaves the keystroke path.
        let already_member = self
            .db
            .workspace()
            .is_some_and(|ws| ws.members(&self.db).contains(&active));
        if !already_member {
            crate::linter::check::seed_workspace_for(&mut self.db, &req.path, active);
        }
        let prepared = match crate::linter::check::prepare_document_in_project(
            &mut self.db,
            &req.path,
            active,
            &req.lint_config,
        ) {
            Ok(Some(prepared)) => prepared,
            // Parse errors (Ok(None)) or an unknown-rule config error (Err): clear
            // any stale diagnostics and run no worker. Leaves the slot free.
            Ok(None) | Err(_) => {
                self.publish_empty(&req);
                return false;
            }
        };

        // `auto_build` reads the buffer + the current salsa index and mutates
        // `index_attempts`, so it stays on the lint thread; it spawns its own
        // background build, whose result is installed back here on `build_rx`.
        if req.index_config.auto_build {
            self.maybe_build(&anchor, &req.index_config, &req.text);
        }

        // Read-phase on the read pool, holding a db clone. A superseding edit (or any
        // write) trips `salsa::Cancelled`, caught here so a cancelled analyze
        // publishes nothing; the main loop's version gate is the backstop.
        let snapshot = self.db.snapshot();
        let out_tx = self.out_tx.clone();
        let done_tx = self.done_tx.clone();
        let uri = req.uri.clone();
        let version = req.version;
        let text = req.text;
        self.inflight = Some(InflightAnalyze {
            uri: uri.clone(),
            version,
        });

        // The snapshot carries the salsa library index, so `analyze_prepared`
        // resolves undefined symbols through it; this provider is only the
        // fallback for rules that read static base-R facts (`is_base`).
        let fallback = CompositeProvider::base_only();
        self.read_spawner.spawn(move || {
            let result = salsa::Cancelled::catch(AssertUnwindSafe(|| {
                crate::linter::check::analyze_prepared(&snapshot, &prepared, &fallback)
            }));
            if let Ok(diagnostics) = result {
                let line_index = LineIndex::new(&text);
                let diags: Vec<LspDiagnostic> = diagnostics
                    .iter()
                    .map(|d| to_lsp_diagnostic(d, &line_index))
                    .collect();
                let _ = out_tx.send(Outbound::Diagnostics {
                    uri: uri.clone(),
                    version,
                    diags,
                    findings: Arc::new(diagnostics),
                });
            }
            // The clone MUST drop before we signal `done`: `trigger_cancellation`
            // / the next write-phase blocks until it's gone, so a premature `done`
            // could let the lint thread start a write that deadlocks on this clone.
            drop(snapshot);
            let _ = done_tx.send(AnalyzeDone { uri, version });
        });
        true
    }

    /// Publish empty diagnostics for `req` (clears any stale findings) without
    /// running a worker. Used when the buffer can't be linted (parse error / bad
    /// config), mirroring the old early-return that always sent diagnostics.
    fn publish_empty(&self, req: &LintRequest) {
        let _ = self.out_tx.send(Outbound::Diagnostics {
            uri: req.uri.clone(),
            version: req.version,
            diags: Vec::new(),
            findings: Arc::new(Vec::new()),
        });
    }

    /// Load the index cache for `anchor` into the salsa [`LibraryIndex`] the
    /// first time we see that workspace. Idempotent per anchor. Runs on the lint
    /// thread (sole writer); the HIGH-durability set means subsequent keystrokes
    /// don't revalidate the library subgraph.
    fn ensure_index(&mut self, anchor: &Path, cfg: &IndexConfig) {
        if self.index_loaded.contains(anchor) {
            return;
        }
        let indexed = match resolve_cache_root(None, cfg.cache_dir.as_deref()) {
            Ok(root) => IndexedProvider::from_cache(&Cache::new(root)),
            Err(_) => IndexedProvider::empty(),
        };
        self.db.set_library_index(indexed);
        self.index_loaded.insert(anchor.to_path_buf());
    }

    /// Spawn a background harvest for the document's unknown packages. On success
    /// the freshly-loaded index is sent back on `build_tx` for the lint thread to
    /// install. The "already indexed?" check reads the current salsa index.
    fn maybe_build(&mut self, anchor: &Path, cfg: &IndexConfig, source: &str) {
        let current = self.db.library_data();
        let empty = IndexedProvider::empty();
        let indexed = current.as_deref().unwrap_or(&empty);
        let to_build = packages_to_build(&mut self.index_attempts, indexed, source);
        if to_build.is_empty() {
            return;
        }
        let Ok(cache_root) = resolve_cache_root(None, cfg.cache_dir.as_deref()) else {
            return;
        };
        let cfg = cfg.clone();
        let anchor = anchor.to_path_buf();
        let build_tx = self.build_tx.clone();
        self.index_pool.spawn(move || {
            let now = now_unix_secs();
            let cache = Cache::new(cache_root);
            let search = LibrarySearch::discover(Some(&anchor), &cfg.library_paths);
            let report = build_index(
                &to_build,
                &cache,
                &search,
                BuildOptions {
                    help: cfg.help,
                    force: false,
                },
                now,
            );
            if report.newly_indexed().next().is_some() {
                let _ = build_tx.send(IndexedProvider::from_cache(&cache));
            }
        });
    }
}

/// Packages to harvest for `source`: the always-attached default packages plus
/// everything `source` references, minus what we already hold a *harvested*
/// index for and minus what we've already attempted this session. Marks the
/// returned packages as attempted so they aren't built twice.
///
/// The skip test is [`IndexedProvider::has_package`] (do we have the rich,
/// harvested data?), not mere name-resolvability: the default packages and the
/// bundled-CRAN packages resolve by name from static lists, but those carry no
/// help or formals, so they still need a real harvest for hover and signatures.
fn packages_to_build(
    attempts: &mut HashSet<SmolStr>,
    indexed: &IndexedProvider,
    source: &str,
) -> Vec<SmolStr> {
    with_default_packages(referenced_in_source(source))
        .into_iter()
        .filter(|pkg| !indexed.has_package(pkg) && attempts.insert(pkg.clone()))
        .collect()
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Read jobs (run on the read pool with a salsa db snapshot)
// ---------------------------------------------------------------------------

/// Service a read-only job against a db `snapshot`, replying to the client.
/// Runs on a read-pool worker; the `snapshot` is dropped on return so it never
/// blocks the lint thread's next write longer than the job itself.
fn run_read(snapshot: Analysis, job: ReadJob) {
    match job {
        ReadJob::Format {
            id,
            path,
            text,
            style,
            sender,
        } => {
            let result = format_edits_via_db(&snapshot, &path, &text, style);
            let _ = sender.send(Message::Response(Response::new_ok(id, result)));
        }
        ReadJob::FormatRange {
            id,
            path,
            text,
            range,
            style,
            sender,
        } => {
            let result = format_range_edits_via_db(&snapshot, &path, &text, range, style);
            let _ = sender.send(Message::Response(Response::new_ok(id, result)));
        }
        ReadJob::Hover {
            id,
            path,
            text,
            position,
            sender,
        } => {
            let result = hover_via_db(&snapshot, &path, &text, position);
            let _ = sender.send(Message::Response(Response::new_ok(id, result)));
        }
        ReadJob::Definition {
            id,
            path,
            uri,
            text,
            position,
            sender,
        } => {
            let result = definition_via_db(&snapshot, &path, &uri, &text, position);
            let _ = sender.send(Message::Response(Response::new_ok(id, result)));
        }
        ReadJob::References {
            id,
            path,
            uri,
            text,
            position,
            include_declaration,
            sender,
        } => {
            let result =
                references_via_db(&snapshot, &path, &uri, &text, position, include_declaration);
            let _ = sender.send(Message::Response(Response::new_ok(id, result)));
        }
        ReadJob::Rename {
            id,
            path,
            uri,
            text,
            offset,
            new_name,
            sender,
        } => {
            let result = rename_via_db(&snapshot, &path, &uri, &text, offset, &new_name);
            let _ = sender.send(Message::Response(Response::new_ok(id, result)));
        }
    }
}

/// Format `text` off the snapshot's cached parse when the db's tracked buffer
/// for `path` still matches it; otherwise re-parse. A write racing the read
/// trips [`salsa::Cancelled`], which also falls back to a fresh parse.
fn format_edits_via_db(
    snapshot: &Analysis,
    path: &Path,
    text: &str,
    style: FormatStyle,
) -> Option<Vec<TextEdit>> {
    let cached = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let file = snapshot.lookup_file(path)?;
        if snapshot.file_text(file) != text {
            // The tracked input lags the live buffer; the cached tree is stale.
            return None;
        }
        if !snapshot.parse_diagnostics(file).is_empty() {
            // Parse errors: the formatter refuses, like `compute_format_edits`.
            return Some(None);
        }
        let root = snapshot.parsed_tree(file);
        let formatted = format_node(&root, style, text.ends_with('\n')).ok();
        Some(formatted.map(|formatted| edits_for_formatted(text, formatted)))
    }));
    match cached {
        Ok(Some(edits)) => edits,
        // Cache miss (`Ok(None)`) or a racing write (`Err`): re-parse from text.
        Ok(None) | Err(_) => compute_format_edits(text, style),
    }
}

/// Range-format `text` off the snapshot's cached parse when the db's tracked
/// buffer for `path` still matches it; otherwise re-parse. Mirrors
/// [`format_edits_via_db`]'s cache/cancellation handling.
fn format_range_edits_via_db(
    snapshot: &Analysis,
    path: &Path,
    text: &str,
    range: Range,
    style: FormatStyle,
) -> Option<Vec<TextEdit>> {
    let cached = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let file = snapshot.lookup_file(path)?;
        if snapshot.file_text(file) != text {
            // The tracked input lags the live buffer; the cached tree is stale.
            return None;
        }
        if !snapshot.parse_diagnostics(file).is_empty() {
            // Parse errors: the formatter refuses, like the whole-document path.
            return Some(None);
        }
        let root = snapshot.parsed_tree(file);
        let line_index = LineIndex::new(text);
        let text_range = lsp_range_to_text_range(&line_index, range);
        let edits = match format_range(&root, text_range, style) {
            Ok(Some(formatted)) => Some(range_edits(&line_index, text, formatted)),
            Ok(None) => Some(Vec::new()),
            Err(_) => None,
        };
        Some(edits)
    }));
    match cached {
        Ok(Some(edits)) => edits,
        // Cache miss (`Ok(None)`) or a racing write (`Err`): re-parse from text.
        Ok(None) | Err(_) => compute_format_range_edits(text, range, style),
    }
}

/// Resolve hover off the snapshot's cached parse when the db's tracked buffer for
/// `path` still matches `text`; otherwise re-parse. Falls back on cancellation.
fn hover_via_db(snapshot: &Analysis, path: &Path, text: &str, position: Position) -> Option<Hover> {
    let line_index = LineIndex::new(text);
    let offset = line_index.position_to_byte(position).min(text.len());
    // Read the harvested index from the same snapshot, so hover sees exactly the
    // index the lint thread last installed. An empty index (none installed yet)
    // still resolves base-R + bundled names via the static layers.
    let index = snapshot.library_data().unwrap_or_default();
    let cached = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let file = snapshot.lookup_file(path)?;
        if snapshot.file_text(file) != text {
            return None;
        }
        let root = snapshot.parsed_tree(file);
        Some(hover_from_node(&root, &line_index, offset, &index))
    }));
    match cached {
        Ok(Some(hover)) => hover,
        Ok(None) | Err(_) => {
            let root = parse(text).cst;
            hover_from_node(&root, &line_index, offset, &index)
        }
    }
}

/// Resolve go-to-definition for the name at `position`. The current file is
/// always parsed from the live `text` (definition is a deliberate, infrequent
/// action, so the parse is cheap relative to the round-trip). An intra-file
/// binding wins and reports a `Location` back into `uri`. Otherwise a bare
/// top-level name falls back to the workspace index ([`Analysis::workspace_def_sites`]),
/// reporting the sibling file(s) it is defined in. Namespaced (`pkg::name`) and
/// base-R names have no in-tree location, so they resolve to nothing (hover still
/// documents them). Snapshot reads are wrapped in [`salsa::Cancelled::catch`].
fn definition_via_db(
    snapshot: &Analysis,
    path: &Path,
    uri: &Uri,
    text: &str,
    position: Position,
) -> Option<GotoDefinitionResponse> {
    let line_index = LineIndex::new(text);
    let offset = TextSize::new(line_index.position_to_byte(position).min(text.len()) as u32);
    let root = parse(text).cst;
    let model = SemanticModel::build(&root);

    // Intra-file: the cursor names a local binding (or sits on its definition).
    if let Some(def_range) = definition_local_range(&root, &model, offset) {
        let location = Location {
            uri: uri.clone(),
            range: text_range_to_lsp_range(&line_index, def_range),
        };
        return Some(GotoDefinitionResponse::Scalar(location));
    }

    // Cross-file: a bare top-level name defined in a sibling workspace file. A
    // namespaced name is a package export with no in-tree source location.
    let token = pick_name_token(&root, offset)?;
    if token.kind() != SyntaxKind::IDENT
        || matches!(
            symbol_query_at(&root, offset),
            Some(SymbolQuery::Namespaced { .. })
        )
    {
        return None;
    }
    let name = SmolStr::new(token.text());
    let locations = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        snapshot
            .workspace_def_sites(&name)
            .into_iter()
            // The current file is handled intra-file above; skip it so a stale
            // tracked copy never shadows the live buffer's own definition.
            .filter(|(def_path, _)| def_path != path)
            .filter_map(|(def_path, range)| {
                let file = snapshot.lookup_file(&def_path)?;
                let target_uri = uri::from_path(&def_path)?;
                let target_index = LineIndex::new(snapshot.file_text(file));
                Some(Location {
                    uri: target_uri,
                    range: text_range_to_lsp_range(&target_index, range),
                })
            })
            .collect::<Vec<_>>()
    }))
    .unwrap_or_default();

    match locations.len() {
        0 => None,
        1 => Some(GotoDefinitionResponse::Scalar(
            locations.into_iter().next()?,
        )),
        _ => Some(GotoDefinitionResponse::Array(locations)),
    }
}

/// Resolve `textDocument/references` against a db `snapshot`. The inverse of
/// [`definition_via_db`], in the same two phases. Intra-file: the cursor names a
/// local binding (or its definition), and every in-file read of it is reported as
/// a `Location` into `uri` (plus the definition when `include_declaration`). When
/// that binding is *file-scope* (a top-level name a sibling file can read), the
/// workspace read index ([`Analysis::workspace_read_sites`]) adds the cross-file
/// reads. Otherwise the cursor sits on a bare free read of a workspace name, and
/// every read of that name across the workspace is reported. Namespaced
/// (`pkg::name`) and base-R names have no in-tree reads to find. Snapshot reads
/// are wrapped in [`salsa::Cancelled::catch`].
fn references_via_db(
    snapshot: &Analysis,
    path: &Path,
    uri: &Uri,
    text: &str,
    position: Position,
    include_declaration: bool,
) -> Option<Vec<Location>> {
    let line_index = LineIndex::new(text);
    let offset = TextSize::new(line_index.position_to_byte(position).min(text.len()) as u32);
    let root = parse(text).cst;
    let model = SemanticModel::build(&root);

    // Intra-file: the cursor names a local binding (or sits on its definition).
    if let Some((target, occ)) = local_occurrences(&root, &model, offset) {
        let mut locations: Vec<Location> = occ
            .reads
            .iter()
            .map(|range| Location {
                uri: uri.clone(),
                range: text_range_to_lsp_range(&line_index, *range),
            })
            .collect();
        if include_declaration {
            locations.push(Location {
                uri: uri.clone(),
                range: text_range_to_lsp_range(&line_index, occ.def),
            });
        }
        // Cross-file: a top-level binding can be free-read from sibling files.
        // Nested locals are file-private, so they stay intra-file.
        if model.binding_is_file_scope(target.binding) {
            let cross = salsa::Cancelled::catch(AssertUnwindSafe(|| {
                snapshot
                    .workspace_read_sites(&target.name)
                    .into_iter()
                    // The current file's reads were collected intra-file above.
                    .filter(|(read_path, _)| read_path != path)
                    .filter_map(|(read_path, range)| location_in(snapshot, &read_path, range))
                    .collect::<Vec<_>>()
            }))
            .unwrap_or_default();
            locations.extend(cross);
        }
        return (!locations.is_empty()).then_some(locations);
    }

    // The cursor sits on a bare free read of a workspace name (no local binding).
    // A namespaced name is a package export with no in-tree reads to collect.
    let token = pick_name_token(&root, offset)?;
    if token.kind() != SyntaxKind::IDENT
        || matches!(
            symbol_query_at(&root, offset),
            Some(SymbolQuery::Namespaced { .. })
        )
    {
        return None;
    }
    let name = SmolStr::new(token.text());
    let locations = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        // Every read of the name across the workspace, including this file's own.
        let mut locs: Vec<Location> = snapshot
            .workspace_read_sites(&name)
            .into_iter()
            .filter_map(|(read_path, range)| location_in(snapshot, &read_path, range))
            .collect();
        if include_declaration {
            locs.extend(
                snapshot
                    .workspace_def_sites(&name)
                    .into_iter()
                    .filter_map(|(def_path, range)| location_in(snapshot, &def_path, range)),
            );
        }
        locs
    }))
    .unwrap_or_default();

    (!locations.is_empty()).then_some(locations)
}

/// A `Location` for `range` in the workspace file at `path`, mapping the byte
/// span through that file's *current* text. `None` if the file isn't tracked or
/// its path has no URI.
fn location_in(snapshot: &Analysis, path: &Path, range: TextRange) -> Option<Location> {
    let file = snapshot.lookup_file(path)?;
    let target_uri = uri::from_path(path)?;
    let target_index = LineIndex::new(snapshot.file_text(file));
    Some(Location {
        uri: target_uri,
        range: text_range_to_lsp_range(&target_index, range),
    })
}

/// The [`TextEdit`] rewriting `range` to `new_name` in the workspace file at
/// `path`, paired with that file's URI. The write mirror of [`location_in`]: the
/// byte span is mapped through the file's *current* text via its own line index.
/// `None` if the file isn't tracked or its path has no URI.
fn text_edit_in(
    snapshot: &Analysis,
    path: &Path,
    range: TextRange,
    new_name: &str,
) -> Option<(Uri, TextEdit)> {
    let file = snapshot.lookup_file(path)?;
    let target_uri = uri::from_path(path)?;
    let target_index = LineIndex::new(snapshot.file_text(file));
    Some((
        target_uri,
        TextEdit {
            range: text_range_to_lsp_range(&target_index, range),
            new_text: new_name.to_string(),
        },
    ))
}

/// Resolve `textDocument/rename` against a db `snapshot` — the write mirror of
/// [`references_via_db`], in the same two phases, emitting a multi-URI
/// [`WorkspaceEdit`] instead of `Location`s. Intra-file: the cursor names a local
/// binding (or its definition), and every in-file read plus the definition is
/// rewritten to `new_name` in `uri`. When that binding is *file-scope*, the
/// workspace read index ([`Analysis::workspace_read_sites`]) adds the cross-file
/// reads. Otherwise the cursor sits on a bare free read of a workspace name, and
/// every read *and* definition of it across the workspace is rewritten.
/// Namespaced (`pkg::name`) names and non-syntactic `new_name`s are declined.
/// Snapshot reads are wrapped in [`salsa::Cancelled::catch`].
fn rename_via_db(
    snapshot: &Analysis,
    path: &Path,
    uri: &Uri,
    text: &str,
    offset: usize,
    new_name: &str,
) -> Option<WorkspaceEdit> {
    if !is_syntactic_r_name(new_name) {
        return None;
    }
    let line_index = LineIndex::new(text);
    let off = TextSize::new(offset.min(text.len()) as u32);
    let root = parse(text).cst;
    let model = SemanticModel::build(&root);

    let mut changes: HashMap<Uri, Vec<TextEdit>> = HashMap::new();

    // Intra-file: the cursor names a local binding (or sits on its definition).
    if let Some(target) = resolve_local_target(&root, &model, off) {
        changes.insert(
            uri.clone(),
            rename_edits(&model, &target, new_name, &line_index),
        );
        // Cross-file: a top-level binding can be free-read from sibling files.
        // Nested locals are file-private, so they stay intra-file.
        if model.binding_is_file_scope(target.binding) {
            let cross = salsa::Cancelled::catch(AssertUnwindSafe(|| {
                snapshot
                    .workspace_read_sites(&target.name)
                    .into_iter()
                    // The current file's reads were rewritten intra-file above.
                    .filter(|(read_path, _)| read_path != path)
                    .filter_map(|(read_path, range)| {
                        text_edit_in(snapshot, &read_path, range, new_name)
                    })
                    .collect::<Vec<_>>()
            }))
            .unwrap_or_default();
            for (edit_uri, edit) in cross {
                changes.entry(edit_uri).or_default().push(edit);
            }
        }
        return finalize_rename(changes);
    }

    // The cursor sits on a bare free read of a workspace name (no local binding).
    // A namespaced name is a package export with no in-tree sites to rewrite.
    let token = pick_name_token(&root, off)?;
    if token.kind() != SyntaxKind::IDENT
        || matches!(
            symbol_query_at(&root, off),
            Some(SymbolQuery::Namespaced { .. })
        )
    {
        return None;
    }
    let name = SmolStr::new(token.text());
    let cross = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        // Every read and definition of the name across the workspace, this file's
        // own included — the read sites already cover the cursor's occurrence.
        snapshot
            .workspace_read_sites(&name)
            .into_iter()
            .chain(snapshot.workspace_def_sites(&name))
            .filter_map(|(site_path, range)| text_edit_in(snapshot, &site_path, range, new_name))
            .collect::<Vec<_>>()
    }))
    .unwrap_or_default();
    for (edit_uri, edit) in cross {
        changes.entry(edit_uri).or_default().push(edit);
    }
    finalize_rename(changes)
}

/// Sort and dedup each file's edits, dropping empties, and wrap them in a
/// [`WorkspaceEdit`]. `None` when nothing is left to rewrite.
fn finalize_rename(mut changes: HashMap<Uri, Vec<TextEdit>>) -> Option<WorkspaceEdit> {
    changes.retain(|_, edits| {
        edits.sort_by_key(|a| (a.range.start, a.range.end));
        edits.dedup();
        !edits.is_empty()
    });
    (!changes.is_empty()).then(|| WorkspaceEdit {
        changes: Some(changes),
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// Pure compute helpers (unit-testable; no IO beyond the in-memory `text`)
// ---------------------------------------------------------------------------

/// Build quick-fix code actions for the fixes whose diagnostics overlap
/// `range`. Pure (no IO beyond the in-memory `text`) so it can be unit-tested.
pub fn compute_code_actions(
    text: &str,
    path: &std::path::Path,
    lint: &LintConfig,
    uri: &Uri,
    range: Range,
) -> CodeActionResponse {
    let diagnostics = crate::linter::check_document(path, text, lint).unwrap_or_default();
    code_actions_from_findings(&diagnostics, text, uri, range)
}

/// Build quick-fix code actions from already-computed lint findings, for the
/// fixes whose diagnostics overlap `range`. `text` must be the source the
/// `findings` were produced against (their ranges are byte offsets into it), so
/// the LSP only serves cached findings when the buffer version still matches.
fn code_actions_from_findings(
    findings: &[Diagnostic],
    text: &str,
    uri: &Uri,
    range: Range,
) -> CodeActionResponse {
    let line_index = LineIndex::new(text);

    findings
        .iter()
        .filter_map(|d| {
            let fix = d.fix.as_ref()?;
            let diag_range = Range {
                start: line_index.byte_to_position(u32::from(d.range.start()) as usize),
                end: line_index.byte_to_position(u32::from(d.range.end()) as usize),
            };
            if !ranges_overlap(diag_range, range) {
                return None;
            }
            let edit = TextEdit {
                range: Range {
                    start: line_index.byte_to_position(fix.start),
                    end: line_index.byte_to_position(fix.end),
                },
                new_text: fix.content.clone(),
            };
            let mut changes = HashMap::new();
            changes.insert(uri.clone(), vec![edit]);
            Some(CodeActionOrCommand::CodeAction(CodeAction {
                title: fix.description.clone(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![to_lsp_diagnostic(d, &line_index)]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }))
        })
        .collect()
}

/// Inclusive overlap test for two LSP ranges (a zero-width cursor touching a
/// diagnostic's edge counts as overlapping, so the quick-fix still shows up).
fn ranges_overlap(a: Range, b: Range) -> bool {
    !(position_lt(a.end, b.start) || position_lt(b.end, a.start))
}

fn position_lt(a: Position, b: Position) -> bool {
    (a.line, a.character) < (b.line, b.character)
}

#[derive(Debug)]
enum ConfigResolveError {
    NonFileUri,
    NoParentDirectory,
    Config(String),
}

impl std::fmt::Display for ConfigResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonFileUri => write!(f, "URI is not a file:// URI"),
            Self::NoParentDirectory => write!(f, "file has no parent directory"),
            Self::Config(msg) => f.write_str(msg),
        }
    }
}

/// Compute the LSP `TextEdit`s to format `text` with `style`, re-parsing it.
///
/// Returns `None` when the formatter rejects the input (e.g. parse error).
/// An empty `Vec` means the document is already formatted.
pub fn compute_format_edits(text: &str, style: FormatStyle) -> Option<Vec<TextEdit>> {
    let formatted = format_with_style(text, style).ok()?;
    Some(edits_for_formatted(text, formatted))
}

/// Compute the LSP `TextEdit`s to format the selection `range` of `text`,
/// re-parsing it.
///
/// Returns `None` when the formatter rejects the input (e.g. parse error). An
/// empty `Vec` means the selected region is already formatted or covers no
/// statement.
pub fn compute_format_range_edits(
    text: &str,
    range: Range,
    style: FormatStyle,
) -> Option<Vec<TextEdit>> {
    let parsed = parse(text);
    if !parsed.diagnostics.is_empty() {
        return None;
    }
    let line_index = LineIndex::new(text);
    let text_range = lsp_range_to_text_range(&line_index, range);
    match format_range(&parsed.cst, text_range, style).ok()? {
        Some(formatted) => Some(range_edits(&line_index, text, formatted)),
        None => Some(Vec::new()),
    }
}

/// Convert a byte `TextRange` to an LSP `Range` via `line_index` (built over the
/// text the range indexes).
fn text_range_to_lsp_range(line_index: &LineIndex, range: TextRange) -> Range {
    Range {
        start: line_index.byte_to_position(u32::from(range.start()) as usize),
        end: line_index.byte_to_position(u32::from(range.end()) as usize),
    }
}

/// Convert an LSP `Range` to a byte `TextRange`. `position_to_byte` already
/// clamps to the text length; we only ensure `start <= end`.
fn lsp_range_to_text_range(line_index: &LineIndex, range: Range) -> TextRange {
    let start = line_index.position_to_byte(range.start);
    let end = line_index.position_to_byte(range.end);
    TextRange::new(
        TextSize::new(start as u32),
        TextSize::new(start.max(end) as u32),
    )
}

/// Turn a [`RangeFormatted`] region into the LSP edit list, dropping the edit
/// when it would not change the buffer.
fn range_edits(
    line_index: &LineIndex,
    text: &str,
    formatted: crate::formatter::RangeFormatted,
) -> Vec<TextEdit> {
    let start = usize::from(formatted.range.start());
    let end = usize::from(formatted.range.end());
    if text.get(start..end) == Some(formatted.text.as_str()) {
        return Vec::new();
    }
    vec![TextEdit {
        range: Range {
            start: line_index.byte_to_position(start),
            end: line_index.byte_to_position(end),
        },
        new_text: formatted.text,
    }]
}

/// The whole-document edit replacing `text` with its formatted form (empty when
/// already formatted). The single source of the edit geometry shared by the
/// re-parse path ([`compute_format_edits`]) and the cached-tree path.
fn edits_for_formatted(text: &str, formatted: String) -> Vec<TextEdit> {
    if formatted == text {
        return Vec::new();
    }
    let line_index = LineIndex::new(text);
    let end = line_index.byte_to_position(text.len());
    vec![TextEdit {
        range: Range {
            start: Position::new(0, 0),
            end,
        },
        new_text: formatted,
    }]
}

fn to_lsp_diagnostic(d: &Diagnostic, idx: &LineIndex) -> LspDiagnostic {
    let start = idx.byte_to_position(u32::from(d.range.start()) as usize);
    let end = idx.byte_to_position(u32::from(d.range.end()) as usize);
    let severity = match d.severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Info => DiagnosticSeverity::INFORMATION,
        Severity::Hint => DiagnosticSeverity::HINT,
    };
    LspDiagnostic {
        range: Range { start, end },
        severity: Some(severity),
        code: Some(NumberOrString::String(d.rule.to_string())),
        source: Some("arity".to_string()),
        message: d.message.body.clone(),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Hover
// ---------------------------------------------------------------------------

/// The symbol referenced at a cursor position: either a namespaced access
/// (`pkg::name`) whose package is known directly, or a bare name whose package
/// must be resolved against the attached packages.
enum SymbolQuery {
    Namespaced {
        package: SmolStr,
        name: SmolStr,
        range: TextRange,
    },
    Bare {
        name: SmolStr,
        range: TextRange,
    },
}

/// A cross-edit-stable anchor for an in-flight rename: a [`NodePtr`] to the
/// renamed identifier's enclosing node, the cursor's offset *within* that node,
/// the name, and the buffer the handle was taken against. Opaque to callers —
/// produced by [`compute_prepare_rename`] and consumed by
/// [`compute_rename_with_anchor`].
#[derive(Debug, Clone)]
pub struct RenameAnchor {
    node_ptr: NodePtr,
    offset_in_node: u32,
    text: String,
}

/// The result of [`compute_prepare_rename`]: the editable range + placeholder the
/// LSP returns, plus the [`RenameAnchor`] the server stashes for the follow-up
/// `rename`.
#[derive(Debug, Clone)]
pub struct PreparedRename {
    pub range: Range,
    pub placeholder: String,
    pub anchor: RenameAnchor,
}

/// The binding the cursor names, plus the token range and name.
struct LocalTarget {
    binding: BindingId,
    range: TextRange,
    name: SmolStr,
}

/// Resolve the cursor to the *local* binding it names, whether the cursor is on a
/// read site or the definition itself. Returns `None` for a non-identifier, or a
/// name that resolves to no local binding (a package export, a global, an
/// undefined name) — those are out of scope for intra-file rename and the
/// intra-file branch of go-to-definition (which falls back to the workspace
/// index for them).
fn resolve_local_target(
    root: &SyntaxNode,
    model: &SemanticModel,
    offset: TextSize,
) -> Option<LocalTarget> {
    let token = pick_name_token(root, offset)?;
    if token.kind() != SyntaxKind::IDENT {
        return None;
    }
    let range = token.text_range();
    let name = SmolStr::new(token.text());
    if let Some(ident) = model.idents().iter().find(|i| i.range == range) {
        let binding = model.resolve_local(ident)?;
        return Some(LocalTarget {
            binding,
            range,
            name,
        });
    }
    let idx = model.bindings().iter().position(|b| b.def_range == range)?;
    Some(LocalTarget {
        binding: BindingId::from_index(idx),
        range,
        name,
    })
}

/// `textDocument/prepareRename`: validate the cursor is on a renameable local and
/// return its range + placeholder, plus the cross-edit [`RenameAnchor`]. Pure
/// (parses `text` itself) so it is unit-testable. Refuses on parse errors so a
/// prepared rename never resolves against a malformed tree.
pub fn compute_prepare_rename(text: &str, offset: usize) -> Option<PreparedRename> {
    let parsed = parse(text);
    if !parsed.diagnostics.is_empty() {
        return None;
    }
    let root = parsed.cst;
    let model = SemanticModel::build(&root);
    let off = TextSize::new(offset.min(text.len()) as u32);
    let target = resolve_local_target(&root, &model, off)?;
    let token = pick_name_token(&root, off)?;
    let node = token.parent()?;
    let offset_in_node = u32::from(target.range.start()) - u32::from(node.text_range().start());
    let line_index = LineIndex::new(text);
    Some(PreparedRename {
        range: Range {
            start: line_index.byte_to_position(usize::from(target.range.start())),
            end: line_index.byte_to_position(usize::from(target.range.end())),
        },
        placeholder: target.name.to_string(),
        anchor: RenameAnchor {
            node_ptr: NodePtr::from_node(&node),
            offset_in_node,
            text: text.to_string(),
        },
    })
}

/// `textDocument/rename` from a cursor offset: the text edits that rename the
/// binding under the cursor and all its in-file reads to `new_name`. Pure and
/// unit-testable. Returns `None` when `new_name` isn't a syntactic R identifier,
/// the file has parse errors, or the cursor names no renameable local.
pub fn compute_rename(text: &str, offset: usize, new_name: &str) -> Option<Vec<TextEdit>> {
    if !is_syntactic_r_name(new_name) {
        return None;
    }
    let parsed = parse(text);
    if !parsed.diagnostics.is_empty() {
        return None;
    }
    let root = parsed.cst;
    let model = SemanticModel::build(&root);
    let off = TextSize::new(offset.min(text.len()) as u32);
    let target = resolve_local_target(&root, &model, off)?;
    let line_index = LineIndex::new(text);
    let edits = rename_edits(&model, &target, new_name, &line_index);
    (!edits.is_empty()).then_some(edits)
}

/// The byte span of the definition of the local binding under the cursor, if the
/// name resolves to an in-file binding. Pure (parses `text` itself) and
/// unit-testable; the intra-file half of go-to-definition. Best-effort, with no
/// clean-parse gate (jumping is always safe). Returns `None` for a
/// non-identifier or a name that names no local binding — a package export, a
/// global, or a cross-file name (the last is resolved against the workspace index
/// by [`definition_via_db`], which this helper does not see).
pub fn compute_definition(text: &str, offset: usize) -> Option<TextRange> {
    let root = parse(text).cst;
    let model = SemanticModel::build(&root);
    let off = TextSize::new(offset.min(text.len()) as u32);
    definition_local_range(&root, &model, off)
}

/// The in-file read spans of the local binding under the cursor, plus its
/// definition span when `include_declaration`. Pure (parses `text` itself) and
/// unit-testable: the intra-file core of go-to-references. The cross-file reads
/// of a top-level name are added by [`references_via_db`], which this helper does
/// not see (mirroring how [`compute_definition`] handles only the intra-file
/// half). `None` for a non-identifier or a name that names no local binding.
pub fn compute_references(
    text: &str,
    offset: usize,
    include_declaration: bool,
) -> Option<Vec<TextRange>> {
    let root = parse(text).cst;
    let model = SemanticModel::build(&root);
    let off = TextSize::new(offset.min(text.len()) as u32);
    let (_, occ) = local_occurrences(&root, &model, off)?;
    let mut ranges = occ.reads;
    if include_declaration {
        ranges.push(occ.def);
    }
    ranges.sort_by_key(|range| range.start());
    ranges.dedup();
    Some(ranges)
}

/// The document highlights for the local binding under the cursor: its definition
/// as [`DocumentHighlightKind::WRITE`] and each in-file read as
/// [`DocumentHighlightKind::READ`], sorted by position. Pure and unit-testable;
/// always same-file (no workspace lookup). `None` when the cursor names no local
/// binding.
pub fn compute_document_highlights(
    text: &str,
    offset: usize,
) -> Option<Vec<(TextRange, DocumentHighlightKind)>> {
    let root = parse(text).cst;
    let model = SemanticModel::build(&root);
    let off = TextSize::new(offset.min(text.len()) as u32);
    let (_, occ) = local_occurrences(&root, &model, off)?;
    let mut highlights: Vec<(TextRange, DocumentHighlightKind)> =
        Vec::with_capacity(occ.reads.len() + 1);
    highlights.push((occ.def, DocumentHighlightKind::WRITE));
    highlights.extend(
        occ.reads
            .into_iter()
            .map(|range| (range, DocumentHighlightKind::READ)),
    );
    highlights.sort_by_key(|(range, _)| range.start());
    Some(highlights)
}

/// The document-symbol outline for `text`: every function and variable binding,
/// nested to mirror the source. Pure (parses `text` itself) and unit-testable;
/// single-file, so it never consults the workspace.
///
/// The set of names is authoritative from the [`SemanticModel`] — the file-scope
/// `Local`/`Implicit` predicate behind [`crate::project::file_exports`], lifted to
/// *every* scope so nested locals are included; parameters and `for`-vars are
/// deliberately excluded. The CST then supplies the tree shape and each symbol's
/// spans. Best-effort, with no clean-parse gate (an outline of partial input is
/// still useful).
pub fn compute_document_symbols(text: &str) -> Vec<DocumentSymbol> {
    let root = parse(text).cst;
    let model = SemanticModel::build(&root);
    // Name keyed by the defining identifier's span: an assignment is a symbol iff
    // its target token range is a key here. Using the model's name (not the raw
    // token text) yields the unquoted form for backtick/string targets.
    let bindings: HashMap<TextRange, SmolStr> = model
        .bindings()
        .iter()
        .filter(|b| matches!(b.kind, BindingKind::Local | BindingKind::Implicit))
        .map(|b| (b.def_range, b.name.clone()))
        .collect();
    let line_index = LineIndex::new(text);
    let mut symbols = Vec::new();
    collect_document_symbols(&root, &bindings, &line_index, &mut symbols);
    symbols
}

/// Walk `node`'s child nodes, emitting a [`DocumentSymbol`] for each assignment
/// whose target is a known binding (recursing into its value for nested symbols)
/// and descending through every other node. Descending into non-binding nodes is
/// what lets a binding nested in an `if`/`for`/`{}` (none of which introduce a
/// symbol of their own) surface at the right level instead of being dropped.
fn collect_document_symbols(
    node: &SyntaxNode,
    bindings: &HashMap<TextRange, SmolStr>,
    line_index: &LineIndex,
    out: &mut Vec<DocumentSymbol>,
) {
    for child in node.children() {
        match document_symbol_for(&child, bindings, line_index) {
            Some(symbol) => out.push(symbol),
            None => collect_document_symbols(&child, bindings, line_index, out),
        }
    }
}

/// Build the [`DocumentSymbol`] for `node` when it is an assignment binding a
/// known name, else `None`. The full range is the whole assignment statement; the
/// selection range is the defining identifier; the kind is `FUNCTION` when the
/// value is a function/lambda, else `VARIABLE`. Children are the symbols nested in
/// the value side.
#[expect(deprecated, reason = "DocumentSymbol::deprecated is a required field")]
fn document_symbol_for(
    node: &SyntaxNode,
    bindings: &HashMap<TextRange, SmolStr>,
    line_index: &LineIndex,
) -> Option<DocumentSymbol> {
    let assign = AssignmentExpr::cast(node.clone())?;
    let name_token = assign.target_name_token()?;
    let name = bindings.get(&name_token.text_range())?;
    let value = assign.value_element();
    let is_function =
        matches!(&value, Some(NodeOrToken::Node(n)) if FunctionExpr::can_cast(n.kind()));

    // Nested bindings live in the value side (a function body, or any expression
    // that itself contains assignments). The target side binds no further names.
    let mut children = Vec::new();
    if let Some(NodeOrToken::Node(value_node)) = &value {
        collect_document_symbols(value_node, bindings, line_index, &mut children);
    }

    Some(DocumentSymbol {
        name: name.to_string(),
        detail: None,
        kind: if is_function {
            LspSymbolKind::FUNCTION
        } else {
            LspSymbolKind::VARIABLE
        },
        tags: None,
        deprecated: None,
        range: text_range_to_lsp_range(line_index, node.text_range()),
        selection_range: text_range_to_lsp_range(line_index, name_token.text_range()),
        children: (!children.is_empty()).then_some(children),
    })
}

/// The def span the cursor's local binding resolves to, off an already-parsed CST
/// and model. The shared core of [`compute_definition`] and the intra-file branch
/// of [`definition_via_db`].
fn definition_local_range(
    root: &SyntaxNode,
    model: &SemanticModel,
    offset: TextSize,
) -> Option<TextRange> {
    let target = resolve_local_target(root, model, offset)?;
    Some(model.binding(target.binding).def_range)
}

/// The definition span and every in-file read span of the local binding under the
/// cursor, sorted and deduped. The shared intra-file core of find-references and
/// document highlight: [`resolve_local_target`] picks the binding, then the
/// `idents()` reads resolving to it are collected (the read-gathering half of
/// [`rename_edits`]). `None` when the cursor names no local binding.
struct LocalOccurrences {
    def: TextRange,
    reads: Vec<TextRange>,
}

fn local_occurrences(
    root: &SyntaxNode,
    model: &SemanticModel,
    offset: TextSize,
) -> Option<(LocalTarget, LocalOccurrences)> {
    let target = resolve_local_target(root, model, offset)?;
    let mut reads: Vec<TextRange> = model
        .idents()
        .iter()
        .filter(|ident| {
            ident.name == target.name && model.resolve_local(ident) == Some(target.binding)
        })
        .map(|ident| ident.range)
        .collect();
    reads.sort_by_key(|range| range.start());
    reads.dedup();
    let def = model.binding(target.binding).def_range;
    Some((target, LocalOccurrences { def, reads }))
}

/// `textDocument/rename` driven by a [`RenameAnchor`] instead of a fresh
/// position: re-locate the cursor in `current_text` via the anchor (mapping its
/// node range across any edit since prepare), then rename. This mirrors
/// [`Analysis::resolve_ptr`](crate::incremental::Analysis::resolve_ptr) but
/// resolves against the live buffer, which is authoritative for the in-flight
/// edit. Returns `None` if the anchor's node was edited away (caller falls back
/// to the request position).
pub fn compute_rename_with_anchor(
    current_text: &str,
    anchor: &RenameAnchor,
    new_name: &str,
) -> Option<Vec<TextEdit>> {
    let offset = rename_cursor_offset(current_text, anchor)?;
    compute_rename(current_text, offset, new_name)
}

/// Re-derive the cursor's byte offset in `current_text` from a [`RenameAnchor`]:
/// resolve the anchor's node (directly when the text is unchanged, else by
/// mapping its range through the edit) and add the stored intra-node offset.
fn rename_cursor_offset(current_text: &str, anchor: &RenameAnchor) -> Option<usize> {
    let root = parse(current_text).cst;
    let node = if current_text == anchor.text {
        anchor.node_ptr.try_to_node(&root)?
    } else {
        let edit = diff_edit(&anchor.text, current_text);
        let mapped = map_range_through_edit(anchor.node_ptr.text_range(), &edit)?;
        anchor.node_ptr.with_range(mapped).try_to_node(&root)?
    };
    Some(usize::from(node.text_range().start()) + anchor.offset_in_node as usize)
}

/// The text edits renaming `target`'s definition and every in-file read of it.
fn rename_edits(
    model: &SemanticModel,
    target: &LocalTarget,
    new_name: &str,
    line_index: &LineIndex,
) -> Vec<TextEdit> {
    let mut ranges: Vec<TextRange> = vec![model.binding(target.binding).def_range];
    for ident in model.idents() {
        if ident.name == target.name && model.resolve_local(ident) == Some(target.binding) {
            ranges.push(ident.range);
        }
    }
    ranges.sort_by_key(|range| range.start());
    ranges.dedup();
    ranges
        .into_iter()
        .map(|range| TextEdit {
            range: Range {
                start: line_index.byte_to_position(usize::from(range.start())),
                end: line_index.byte_to_position(usize::from(range.end())),
            },
            new_text: new_name.to_string(),
        })
        .collect()
}

/// Whether `name` is a syntactic R identifier usable without backtick-quoting:
/// starts with a letter or `.` (and a leading `.` is not followed by a digit),
/// contains only letters, digits, `.`, and `_`, and isn't a reserved word.
/// Backtick-quoted non-syntactic names are out of scope (the rename withholds).
fn is_syntactic_r_name(name: &str) -> bool {
    let Some(first) = name.chars().next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '.') {
        return false;
    }
    if first == '.' && matches!(name.as_bytes().get(1), Some(b) if b.is_ascii_digit()) {
        return false;
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
    {
        return false;
    }
    !is_reserved_word(name)
}

fn is_reserved_word(name: &str) -> bool {
    matches!(
        name,
        "if" | "else"
            | "repeat"
            | "while"
            | "function"
            | "for"
            | "in"
            | "next"
            | "break"
            | "TRUE"
            | "FALSE"
            | "NULL"
            | "Inf"
            | "NaN"
            | "NA"
            | "NA_integer_"
            | "NA_real_"
            | "NA_character_"
            | "NA_complex_"
    )
}

/// Build hover contents for the symbol at byte `offset`, if it resolves to an
/// indexed package export. Pure (parses `text` itself) so it is unit-testable.
pub fn compute_hover(text: &str, offset: usize, indexed: &IndexedProvider) -> Option<Hover> {
    let root = parse(text).cst;
    let line_index = LineIndex::new(text);
    hover_from_node(&root, &line_index, offset.min(text.len()), indexed)
}

/// Build hover contents off an already-parsed CST (and a matching line index),
/// without re-parsing. The LSP read path uses this against the cached parse tree
/// in its salsa database; [`compute_hover`] is the parse-from-text wrapper.
fn hover_from_node(
    root: &SyntaxNode,
    line_index: &LineIndex,
    offset: usize,
    indexed: &IndexedProvider,
) -> Option<Hover> {
    let offset = TextSize::new(offset as u32);
    let query = symbol_query_at(root, offset)?;
    let (package, entry, range) = resolve_query(query, root, indexed)?;

    let lsp_range = Range {
        start: line_index.byte_to_position(u32::from(range.start()) as usize),
        end: line_index.byte_to_position(u32::from(range.end()) as usize),
    };
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: render_hover_markdown(&package, entry),
        }),
        range: Some(lsp_range),
    })
}

/// Classify the name token under the cursor, distinguishing `pkg::name` from a
/// bare reference. Returns `None` when the cursor isn't on a name.
fn symbol_query_at(root: &SyntaxNode, offset: TextSize) -> Option<SymbolQuery> {
    let token = pick_name_token(root, offset)?;
    for ancestor in token.parent_ancestors() {
        if ancestor.kind() == SyntaxKind::BINARY_EXPR
            && let Some(access) = BinaryExpr::cast(ancestor).and_then(|b| b.namespace_access())
            && access.name_token == token
        {
            return Some(SymbolQuery::Namespaced {
                package: access.package,
                name: access.name,
                range: token.text_range(),
            });
        }
    }
    Some(SymbolQuery::Bare {
        name: SmolStr::new(token.text()),
        range: token.text_range(),
    })
}

/// The `IDENT`/`USER_OP` token at `offset`, preferring the right side when the
/// cursor sits exactly between two tokens.
fn pick_name_token(root: &SyntaxNode, offset: TextSize) -> Option<SyntaxToken<RLanguage>> {
    let is_name = |k: SyntaxKind| matches!(k, SyntaxKind::IDENT | SyntaxKind::USER_OP);
    match root.token_at_offset(offset) {
        TokenAtOffset::None => None,
        TokenAtOffset::Single(t) => is_name(t.kind()).then_some(t),
        TokenAtOffset::Between(left, right) => {
            if is_name(right.kind()) {
                Some(right)
            } else if is_name(left.kind()) {
                Some(left)
            } else {
                None
            }
        }
    }
}

/// Resolve a [`SymbolQuery`] to the indexed entry that documents it.
fn resolve_query<'p>(
    query: SymbolQuery,
    root: &SyntaxNode,
    indexed: &'p IndexedProvider,
) -> Option<(SmolStr, &'p SymbolEntry, TextRange)> {
    match query {
        SymbolQuery::Namespaced {
            package,
            name,
            range,
        } => {
            let entry = indexed.lookup(&package, &name)?;
            Some((package, entry, range))
        }
        SymbolQuery::Bare { name, range } => {
            let model = SemanticModel::build(root);
            let package = match resolve_origin(indexed, &name, model.loaded_packages()) {
                PackageOrigin::Resolved(p) => p,
                // The last attacher masks the rest under R's lookup rules.
                PackageOrigin::Ambiguous(mut v) => v.pop()?,
                PackageOrigin::Unknown => return None,
            };
            let entry = indexed.lookup(&package, &name)?;
            Some((package, entry, range))
        }
    }
}

/// Render a symbol's signature + help into hover markdown.
fn render_hover_markdown(package: &str, entry: &SymbolEntry) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();

    // Signature: the `\usage` block if present, else a formals-derived call.
    let usage = entry.help.as_ref().and_then(|h| h.usage.as_deref());
    let signature = usage.map(str::to_string).or_else(|| {
        entry.formals.as_ref().map(|formals| {
            let args = formals
                .iter()
                .map(format_formal)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({})", entry.name, args)
        })
    });
    if let Some(signature) = signature {
        let _ = write!(out, "```r\n{signature}\n```\n");
    }

    let kind = match entry.kind {
        SymbolKind::Function => "function",
        SymbolKind::Data => "data",
        SymbolKind::Other => "object",
    };
    let _ = write!(out, "`{package}::{}` · {kind}", entry.name);

    if let Some(help) = &entry.help {
        if let Some(title) = &help.title {
            let _ = write!(out, "\n\n**{title}**");
        }
        if let Some(description) = &help.description {
            let _ = write!(out, "\n\n{description}");
        }
        if !help.arguments.is_empty() {
            out.push_str("\n\n**Arguments**\n");
            for arg in &help.arguments {
                let _ = write!(out, "\n- `{}` — {}", arg.name, arg.description);
            }
        }
    }
    out
}

fn format_formal(formal: &Formal) -> String {
    match &formal.default {
        Some(default) => format!("{} = {}", formal.name, default),
        None => formal.name.to_string(),
    }
}

// ---------------------------------------------------------------------------
// file: URI ↔ path
// ---------------------------------------------------------------------------

/// `lsp-types`' `Uri` (a `fluent_uri` newtype) has no file-path conveniences, so
/// we provide our own. Decoding rides on `fluent_uri` via `Deref`; encoding uses
/// a small percent-encoder over a fixed safe set.
mod uri {
    use std::path::Path;
    use std::path::PathBuf;
    use std::str::FromStr;

    use lsp_types::Uri;

    /// Convert a `file:` URI to a filesystem path, or `None` if it isn't a file
    /// URI or has no scheme.
    pub fn to_path(uri: &Uri) -> Option<PathBuf> {
        let scheme = uri.scheme()?;
        if !scheme.as_str().eq_ignore_ascii_case("file") {
            return None;
        }
        let decoded = uri
            .path()
            .as_estr()
            .decode()
            .into_string_lossy()
            .into_owned();
        Some(from_uri_path(&decoded))
    }

    #[cfg(windows)]
    fn from_uri_path(p: &str) -> PathBuf {
        // "/C:/Users/x" → "C:\Users\x"
        PathBuf::from(p.strip_prefix('/').unwrap_or(p).replace('/', "\\"))
    }

    #[cfg(not(windows))]
    fn from_uri_path(p: &str) -> PathBuf {
        PathBuf::from(p)
    }

    /// Convert a filesystem path to a `file:` URI. Used by go-to-definition to
    /// name a cross-file target (the client always supplies URIs in real traffic,
    /// so request handling never needs this) and by the tests.
    pub fn from_path(path: &Path) -> Option<Uri> {
        let s = path.to_str()?;
        let mut out = String::from("file://");
        encode_into(&to_uri_path(s), &mut out);
        Uri::from_str(&out).ok()
    }

    #[cfg(windows)]
    fn to_uri_path(s: &str) -> String {
        // "C:\Users\x" → "/C:/Users/x" (the URI path needs a leading slash)
        format!("/{}", s.replace('\\', "/"))
    }

    #[cfg(not(windows))]
    fn to_uri_path(s: &str) -> String {
        s.to_string()
    }

    /// Percent-encode `s`, leaving the unreserved set plus `/` and `:` (drive
    /// letters) intact.
    fn encode_into(s: &str, out: &mut String) {
        for &b in s.as_bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/' | b':') {
                out.push(b as char);
            } else {
                out.push('%');
                out.push(hex(b >> 4));
                out.push(hex(b & 0x0f));
            }
        }
    }

    fn hex(n: u8) -> char {
        char::from(if n < 10 { b'0' + n } else { b'A' + (n - 10) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    fn full_line_0() -> Range {
        Range {
            start: pos(0, 0),
            end: pos(0, 100),
        }
    }

    /// An absolute path valid on the host platform (the URI conversion needs an
    /// absolute path, and `/tmp/t.R` is not absolute on Windows).
    fn test_path() -> &'static Path {
        if cfg!(windows) {
            Path::new(r"C:\tmp\t.R")
        } else {
            Path::new("/tmp/t.R")
        }
    }

    fn test_uri() -> Uri {
        uri::from_path(test_path()).expect("valid file uri")
    }

    // --- scheduler: decide() ----------------------------------------------

    fn uri_named(name: &str) -> Uri {
        let path = if cfg!(windows) {
            PathBuf::from(format!(r"C:\tmp\{name}"))
        } else {
            PathBuf::from(format!("/tmp/{name}"))
        };
        uri::from_path(&path).expect("valid file uri")
    }

    #[test]
    fn decide_idle_starts_a_pending_uri() {
        let a = uri_named("a.R");
        let pending = HashMap::from([(a.clone(), 1)]);
        assert_eq!(decide(None, &pending), DispatchAction::Start(a));
    }

    #[test]
    fn decide_idle_empty_queue_waits() {
        let pending: HashMap<Uri, i32> = HashMap::new();
        assert_eq!(decide(None, &pending), DispatchAction::Wait);
    }

    #[test]
    fn decide_supersedes_same_uri_newer_version() {
        let a = uri_named("a.R");
        let pending = HashMap::from([(a.clone(), 2)]);
        assert_eq!(
            decide(Some((&a, 1)), &pending),
            DispatchAction::SupersedeAndStart(a)
        );
    }

    #[test]
    fn decide_waits_when_pending_same_uri_not_newer() {
        // A duplicate / same-version request for the in-flight URI must not
        // restart it.
        let a = uri_named("a.R");
        let pending = HashMap::from([(a.clone(), 1)]);
        assert_eq!(decide(Some((&a, 1)), &pending), DispatchAction::Wait);
    }

    #[test]
    fn decide_never_cancels_a_different_uri() {
        // The core RelintAll guard: with A in flight and only *other* URIs
        // queued, we wait for A's `done` — we never cancel A to start B/C, which
        // would silently drop A's diagnostics.
        let a = uri_named("a.R");
        let pending = HashMap::from([(uri_named("b.R"), 5), (uri_named("c.R"), 9)]);
        assert_eq!(decide(Some((&a, 1)), &pending), DispatchAction::Wait);
    }

    #[test]
    fn decide_relint_all_drains_one_uri_at_a_time() {
        // Simulate a multi-URI RelintAll: each file is dispatched only once the
        // slot is free, and `decide` never returns SupersedeAndStart for a URI
        // other than the in-flight one.
        let (a, b, c) = (uri_named("a.R"), uri_named("b.R"), uri_named("c.R"));
        let mut pending = HashMap::from([(a.clone(), 1), (b.clone(), 1), (c.clone(), 1)]);

        // Idle: start some URI.
        let DispatchAction::Start(first) = decide(None, &pending) else {
            panic!("expected Start");
        };
        assert!(pending.contains_key(&first));
        pending.remove(&first);

        // Busy with `first`, two others still queued → wait, never supersede.
        let action = decide(Some((&first, 1)), &pending);
        assert_eq!(action, DispatchAction::Wait);

        // first's `done` frees the slot; the next URI starts. Repeat to drain.
        let mut started = vec![first];
        while !pending.is_empty() {
            let DispatchAction::Start(next) = decide(None, &pending) else {
                panic!("expected Start");
            };
            pending.remove(&next);
            started.push(next);
        }
        started.sort_by_key(|u| u.as_str().to_string());
        assert_eq!(started, {
            let mut all = vec![a, b, c];
            all.sort_by_key(|u| u.as_str().to_string());
            all
        });
    }

    // --- editor settings --------------------------------------------------

    #[test]
    fn editor_settings_parse_bare_camel_case_object() {
        let value = serde_json::json!({ "lineWidth": 100, "indentWidth": 4 });
        let settings = EditorSettings::from_client_value(&value);
        assert_eq!(settings.line_width, Some(100));
        assert_eq!(settings.indent_width, Some(4));
    }

    #[test]
    fn editor_settings_parse_namespaced_under_arity() {
        // didChangeConfiguration clients push their whole settings tree; ours is
        // scoped under "arity" and sibling keys are ignored.
        let value = serde_json::json!({
            "arity": { "lineWidth": 120 },
            "editor": { "tabSize": 8 },
        });
        let settings = EditorSettings::from_client_value(&value);
        assert_eq!(settings.line_width, Some(120));
        assert_eq!(settings.indent_width, None);
    }

    #[test]
    fn editor_settings_ignore_unknown_and_malformed() {
        let unknown = serde_json::json!({ "bogus": true });
        assert_eq!(
            EditorSettings::from_client_value(&unknown),
            EditorSettings::default()
        );
        let malformed = serde_json::json!("not an object");
        assert_eq!(
            EditorSettings::from_client_value(&malformed),
            EditorSettings::default()
        );
    }

    #[test]
    fn editor_settings_to_style_layers_over_defaults() {
        let settings = EditorSettings {
            line_width: Some(100),
            indent_width: None,
        };
        let style = settings.to_format_style();
        assert_eq!(style.line_width, 100);
        // Unset field keeps the built-in default.
        assert_eq!(style.indent_width, FormatStyle::default().indent_width);
    }

    #[test]
    fn editor_settings_out_of_range_fall_back_to_defaults() {
        // 0 is below the valid width floor; the whole layer is discarded.
        let settings = EditorSettings {
            line_width: Some(0),
            indent_width: Some(4),
        };
        assert_eq!(settings.to_format_style(), FormatStyle::default());
    }

    #[test]
    fn config_file_wins_over_editor_settings() {
        let mut config = Config::default();
        config.format.line_width = 70;
        let editor = EditorSettings {
            line_width: Some(120),
            indent_width: Some(8),
        };
        // arity.toml present → editor settings ignored entirely.
        let style = resolve_format_style(&config, true, &editor);
        assert_eq!(style.line_width, 70);
        assert_eq!(style.indent_width, FormatStyle::default().indent_width);
        // No config file → editor settings apply over defaults.
        let fallback = resolve_format_style(&Config::default(), false, &editor);
        assert_eq!(fallback.line_width, 120);
        assert_eq!(fallback.indent_width, 8);
    }

    #[test]
    fn uri_path_round_trips() {
        let uri = test_uri();
        assert_eq!(uri::to_path(&uri).as_deref(), Some(test_path()));
    }

    #[test]
    fn code_action_offers_quickfix_for_diagnostic_in_range() {
        let src = "if (x = 1) print(x)\n";
        let actions = compute_code_actions(
            src,
            test_path(),
            &LintConfig::default(),
            &test_uri(),
            full_line_0(),
        );

        let CodeActionOrCommand::CodeAction(action) = actions
            .iter()
            .find(|a| matches!(a, CodeActionOrCommand::CodeAction(a) if a.title.contains("==")))
            .expect("an `=` → `==` quick-fix")
        else {
            unreachable!()
        };
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
        let changes = action
            .edit
            .as_ref()
            .and_then(|e| e.changes.as_ref())
            .expect("workspace edit with changes");
        let edits = changes.get(&test_uri()).expect("edits for our uri");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "==");
        // The edit targets the `=` token on line 0.
        assert_eq!(edits[0].range.start.line, 0);
    }

    #[test]
    fn code_action_empty_when_range_misses_diagnostics() {
        let src = "if (x = 1) print(x)\n";
        let far = Range {
            start: pos(5, 0),
            end: pos(5, 0),
        };
        let actions =
            compute_code_actions(src, test_path(), &LintConfig::default(), &test_uri(), far);
        assert!(actions.is_empty(), "expected no actions, got {actions:?}");
    }

    fn indexed_dplyr() -> IndexedProvider {
        use crate::rindex::schema::{PackageIndex, SCHEMA_VERSION, SymbolEntry, SymbolKind};
        let idx = PackageIndex {
            schema_version: SCHEMA_VERSION,
            package: "dplyr".into(),
            version: "1.0".into(),
            lib_path: "/lib".into(),
            r_version: None,
            harvested_at: 0,
            symbols: vec![SymbolEntry {
                name: "across".into(),
                kind: SymbolKind::Function,
                exported: true,
                formals: None,
                help: None,
            }],
        };
        IndexedProvider::from_indices([idx])
    }

    #[test]
    fn packages_to_build_covers_defaults_and_unharvested_deps() {
        let mut attempts = HashSet::new();
        let indexed = indexed_dplyr();
        // dplyr is already harvested (skipped). The default packages and any
        // referenced-but-unharvested package (stats is a default; notarealpkg
        // is neither default nor harvested) still need a build for rich data.
        let src = "library(dplyr)\nlibrary(stats)\nlibrary(notarealpkg)\n";
        let first = packages_to_build(&mut attempts, &indexed, src);
        assert!(
            !first.contains(&SmolStr::new("dplyr")),
            "harvested dep skipped"
        );
        for default in crate::semantic::symbols::default_packages() {
            assert!(
                first.contains(&SmolStr::new(*default)),
                "default package {default} should be built, got {first:?}"
            );
        }
        assert!(first.contains(&SmolStr::new("notarealpkg")));
        // A second pass returns nothing — every package was already attempted.
        let second = packages_to_build(&mut attempts, &indexed, src);
        assert!(second.is_empty(), "expected no re-attempt, got {second:?}");
    }

    // --- hover ------------------------------------------------------------

    /// dplyr with one richly-documented export (`across`).
    fn documented_dplyr() -> IndexedProvider {
        use crate::rindex::schema::{Formal, HelpArg, HelpDoc, PackageIndex, SCHEMA_VERSION};
        let idx = PackageIndex {
            schema_version: SCHEMA_VERSION,
            package: "dplyr".into(),
            version: "1.0".into(),
            lib_path: "/lib".into(),
            r_version: None,
            harvested_at: 0,
            symbols: vec![SymbolEntry {
                name: "across".into(),
                kind: SymbolKind::Function,
                exported: true,
                formals: Some(vec![
                    Formal {
                        name: ".cols".into(),
                        default: Some("everything()".into()),
                    },
                    Formal {
                        name: ".fns".into(),
                        default: None,
                    },
                ]),
                help: Some(HelpDoc {
                    title: Some("Apply a function across columns".into()),
                    description: Some("Apply one or more functions to a set of columns.".into()),
                    usage: Some("across(.cols, .fns)".into()),
                    arguments: vec![HelpArg {
                        name: ".cols".into(),
                        description: "Columns to transform.".into(),
                    }],
                }),
            }],
        };
        IndexedProvider::from_indices([idx])
    }

    /// Byte offset of the first occurrence of `needle` in `src`.
    fn offset_of(src: &str, needle: &str) -> usize {
        src.find(needle).expect("needle present") + 1
    }

    fn hover_markdown(src: &str, needle: &str, indexed: &IndexedProvider) -> Option<String> {
        compute_hover(src, offset_of(src, needle), indexed).map(|h| match h.contents {
            HoverContents::Markup(m) => m.value,
            other => panic!("expected markup, got {other:?}"),
        })
    }

    #[test]
    fn hover_resolves_bare_name_via_attached_package() {
        let provider = documented_dplyr();
        let src = "library(dplyr)\nacross(a, mean)\n";
        let md = hover_markdown(src, "across(a", &provider).expect("hover for across");
        assert!(md.contains("across(.cols, .fns)"), "signature: {md}");
        assert!(md.contains("dplyr::across"), "origin: {md}");
        assert!(
            md.contains("Apply a function across columns"),
            "title: {md}"
        );
        assert!(md.contains("`.cols`"), "arguments: {md}");
    }

    #[test]
    fn hover_resolves_base_r_bare_name() {
        // Regression: base-R symbols resolve to package `base` via the static
        // name list, but hover also needs the harvested rich entry. Once `base`
        // is harvested, a bare `as.matrix` (no `library()`) hovers.
        use crate::rindex::schema::{HelpDoc, PackageIndex, SCHEMA_VERSION};
        let idx = PackageIndex {
            schema_version: SCHEMA_VERSION,
            package: "base".into(),
            version: "4.5.3".into(),
            lib_path: "/lib".into(),
            r_version: None,
            harvested_at: 0,
            symbols: vec![SymbolEntry {
                name: "as.matrix".into(),
                kind: SymbolKind::Function,
                exported: true,
                formals: None,
                help: Some(HelpDoc {
                    title: Some("Matrices".into()),
                    description: None,
                    usage: Some("as.matrix(x, ...)".into()),
                    arguments: vec![],
                }),
            }],
        };
        let provider = IndexedProvider::from_indices([idx]);
        let src = "x <- cbind(1:5, 6:10)\nas.matrix(x)\n";
        let md = hover_markdown(src, "as.matrix(x)", &provider).expect("hover for as.matrix");
        assert!(md.contains("as.matrix(x, ...)"), "signature: {md}");
        assert!(md.contains("base::as.matrix"), "origin: {md}");
        assert!(md.contains("Matrices"), "title: {md}");
    }

    #[test]
    fn hover_resolves_namespaced_without_library() {
        let provider = documented_dplyr();
        // No `library(dplyr)`: the `pkg::name` form resolves directly.
        let src = "dplyr::across(a)\n";
        let md = hover_markdown(src, "across", &provider).expect("hover for dplyr::across");
        assert!(md.contains("dplyr::across"));
    }

    #[test]
    fn hover_none_for_unknown_and_non_name() {
        let provider = documented_dplyr();
        // `bogus` is not indexed by any attached package.
        assert!(compute_hover("bogus()\n", 1, &provider).is_none());
        // Cursor on whitespace yields nothing.
        let src = "across (a)\n";
        assert!(compute_hover(src, offset_of(src, " (a"), &provider).is_none());
    }

    // --- db read path -----------------------------------------------------

    /// The cached-tree format path matches the re-parse path when the db's
    /// tracked buffer is the live text, and falls back (still correctly) when the
    /// db lags the buffer or has never seen the path.
    #[test]
    fn format_via_db_matches_compute_and_falls_back() {
        use crate::incremental::IncrementalDatabase;
        let style = FormatStyle::default();
        let path = test_path();
        let buffer = "x<-f(1 )\n";
        let expected = compute_format_edits(buffer, style);
        assert!(
            matches!(&expected, Some(edits) if !edits.is_empty()),
            "fixture must require reformatting"
        );

        // Cache hit: tracked text == buffer → format off the cached tree.
        let mut db = IncrementalDatabase::default();
        db.upsert_file(path, buffer.to_string());
        let snapshot = db.snapshot();
        assert_eq!(
            format_edits_via_db(&snapshot, path, buffer, style),
            expected,
            "cached-tree format must match the re-parse path"
        );

        // Stale db (tracked text lags the buffer) → fall back to a fresh parse.
        let mut stale = IncrementalDatabase::default();
        stale.upsert_file(path, "y <- 1\n".to_string());
        assert_eq!(
            format_edits_via_db(&stale.snapshot(), path, buffer, style),
            expected,
            "version skew must fall back to the buffer text"
        );

        // Untracked path → fall back as well.
        let empty = IncrementalDatabase::default();
        assert_eq!(
            format_edits_via_db(&empty.snapshot(), path, buffer, style),
            expected,
            "untracked path must fall back to the buffer text"
        );
    }

    #[test]
    fn hover_via_db_matches_compute() {
        use crate::incremental::IncrementalDatabase;
        let path = test_path();
        let src = "library(dplyr)\nacross(a, mean)\n";
        // Cursor on `across` (line 1, character 0).
        let position = pos(1, 0);

        // Hover reads the index from the snapshot, so it must be installed first.
        let mut db = IncrementalDatabase::default();
        db.set_library_index(documented_dplyr());
        db.upsert_file(path, src.to_string());
        let hover =
            hover_via_db(&db.snapshot(), path, src, position).expect("hover for across via db");
        let md = match hover.contents {
            HoverContents::Markup(m) => m.value,
            other => panic!("expected markup, got {other:?}"),
        };
        assert!(md.contains("dplyr::across"), "origin: {md}");

        // Untracked path still resolves, via the fresh-parse fallback.
        let mut empty = IncrementalDatabase::default();
        empty.set_library_index(documented_dplyr());
        assert!(
            hover_via_db(&empty.snapshot(), path, src, position).is_some(),
            "fallback hover should resolve too"
        );
    }

    #[test]
    fn workspace_roots_parses_folders_then_root_uri() {
        let uri = test_uri();
        let want = vec![test_path().to_path_buf()];

        // `workspaceFolders` is used when present.
        let params = serde_json::json!({
            "workspaceFolders": [{ "uri": uri.as_str(), "name": "w" }],
        });
        assert_eq!(workspace_roots_from_params(&params), want);

        // Falls back to the legacy `rootUri` when no folders are given.
        let params = serde_json::json!({ "rootUri": uri.as_str() });
        assert_eq!(workspace_roots_from_params(&params), want);

        // Neither present → no roots (a single file opened outside a workspace).
        assert!(workspace_roots_from_params(&serde_json::json!({})).is_empty());
    }

    // --- cross-file rename (rename_via_db) --------------------------------

    /// A workspace root valid on the host (absolute, so URI conversion works).
    fn ws_root() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\s")
        } else {
            PathBuf::from("/s")
        }
    }

    fn ws_path(name: &str) -> PathBuf {
        ws_root().join(name)
    }

    /// A two-file workspace (`a.R`, `b.R`) seeded as members, snapshotted for a
    /// read job. The names are flat scripts; the workspace index keys on name, so
    /// this is enough to exercise cross-file rewriting.
    fn rename_workspace(a_src: &str, b_src: &str) -> Analysis {
        let mut db = IncrementalDatabase::default();
        let a = db.upsert_file(&ws_path("a.R"), a_src.to_string());
        let b = db.upsert_file(&ws_path("b.R"), b_src.to_string());
        db.set_workspace_members(vec![a, b], vec![ws_root()]);
        db.snapshot()
    }

    #[test]
    fn rename_via_db_rewrites_a_definition_and_its_cross_file_reads() {
        // a.R defines `foo`; b.R reads it. Renaming from the definition must edit
        // both files in one WorkspaceEdit.
        let a_src = "foo <- function() 1\n";
        let b_src = "bar <- function() foo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let offset = a_src.find("foo").unwrap();

        let edit = rename_via_db(&snapshot, &ws_path("a.R"), &uri_a, a_src, offset, "renamed")
            .expect("rename is available on a file-scope definition");
        let changes = edit.changes.expect("changes present");

        let a_edits = changes
            .get(&uri_a)
            .expect("the definition in a.R is edited");
        assert_eq!(a_edits.len(), 1);
        assert_eq!(a_edits[0].new_text, "renamed");

        let b_edits = changes
            .get(&uri_b)
            .expect("the cross-file read in b.R is edited");
        assert_eq!(b_edits.len(), 1);
        assert_eq!(b_edits[0].new_text, "renamed");
    }

    #[test]
    fn rename_via_db_from_a_cross_file_read_rewrites_the_definition() {
        // Cursor on the `foo()` read in b.R, which binds to no local: rename rides
        // the workspace def + read indices, touching both files.
        let a_src = "foo <- function() 1\n";
        let b_src = "bar <- function() foo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let offset = b_src.find("foo").unwrap();

        let edit = rename_via_db(&snapshot, &ws_path("b.R"), &uri_b, b_src, offset, "renamed")
            .expect("rename is available on a workspace free read");
        let changes = edit.changes.expect("changes present");

        assert!(
            changes.contains_key(&uri_a),
            "the definition in a.R is edited"
        );
        assert!(changes.contains_key(&uri_b), "the read in b.R is edited");
    }

    #[test]
    fn rename_via_db_keeps_a_nested_local_intra_file() {
        // `x` is a local inside f's body, not a file-scope binding, so a same-named
        // free read in another file is unrelated and must not be touched.
        let a_src = "f <- function() {\n  x <- 1\n  x + 1\n}\n";
        let b_src = "g <- function() x\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let offset = a_src.find("x").unwrap();

        let edit = rename_via_db(&snapshot, &ws_path("a.R"), &uri_a, a_src, offset, "y")
            .expect("rename is available on the local");
        let changes = edit.changes.expect("changes present");

        assert_eq!(changes.len(), 1, "only a.R is touched");
        let a_edits = changes.get(&uri_a).expect("the local def and read in a.R");
        assert_eq!(a_edits.len(), 2, "definition plus the one read");
        assert!(
            !changes.contains_key(&uri_b),
            "the sibling free read is unrelated"
        );
    }

    #[test]
    fn rename_via_db_declines_a_non_syntactic_new_name() {
        let a_src = "foo <- function() 1\n";
        let b_src = "bar <- function() foo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let offset = a_src.find("foo").unwrap();

        assert!(
            rename_via_db(
                &snapshot,
                &ws_path("a.R"),
                &uri_a,
                a_src,
                offset,
                "new name"
            )
            .is_none(),
            "a non-syntactic new name is withheld (backtick-quoting is out of scope)"
        );
    }
}
