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
//! `&db` only) that runs on a rayon worker holding a short-lived db clone. The
//! lint thread returns to its `select!` right after the write-phase, so a long
//! analyze no longer blocks queued reads.
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
//!   mints a short-lived db clone and runs the job on rayon ([`run_read`]),
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
use lsp_types::request::{CodeActionRequest, Formatting, HoverRequest, Request as RequestTrait};
use lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionProviderCapability, CodeActionResponse, Diagnostic as LspDiagnostic,
    DiagnosticSeverity, DidChangeConfigurationParams, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DocumentFormattingParams, Hover,
    HoverContents, HoverParams, HoverProviderCapability, InitializeResult, MarkupContent,
    MarkupKind, NumberOrString, OneOf, Position, PublishDiagnosticsParams, Range,
    ServerCapabilities, ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit,
    Uri, WorkspaceEdit,
};
use rowan::{SyntaxToken, TextRange, TextSize, TokenAtOffset};
use salsa::Database as _;
use serde::Deserialize;
use smol_str::SmolStr;

use crate::ast::{AstNode as _, BinaryExpr};
use crate::config::{Config, FormatConfig, IndexConfig, LintConfig};
use crate::formatter::{FormatStyle, format_node, format_with_style};
use crate::incremental::IncrementalDatabase;
use crate::linter::{Diagnostic, Severity};
use crate::parser::parse;
use crate::rindex::build::{BuildOptions, build_index};
use crate::rindex::cache::{Cache, resolve_cache_root};
use crate::rindex::discover::referenced_in_source;
use crate::rindex::libpaths::LibrarySearch;
use crate::rindex::provider::{CompositeProvider, IndexedProvider};
use crate::rindex::schema::{Formal, SymbolEntry, SymbolKind};
use crate::semantic::{PackageOrigin, SemanticModel, SymbolProvider as _};
use crate::syntax::{RLanguage, SyntaxKind, SyntaxNode};
use crate::text::LineIndex;

type DynError = Box<dyn std::error::Error + Sync + Send>;

/// Run the language server on stdio until the client disconnects.
pub fn run() -> Result<(), DynError> {
    let (connection, io_threads) = Connection::stdio();

    let (id, params) = connection.initialize_start()?;
    let editor_settings = params
        .get("initializationOptions")
        .map(EditorSettings::from_client_value)
        .unwrap_or_default();
    let init_result = InitializeResult {
        capabilities: server_capabilities(),
        server_info: Some(ServerInfo {
            name: "ravel".to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }),
    };
    connection.initialize_finish(id, serde_json::to_value(init_result)?)?;

    main_loop(connection, editor_settings)?;
    io_threads.join()?;
    Ok(())
}

fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        document_formatting_provider: Some(OneOf::Left(true)),
        code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        ..Default::default()
    }
}

/// The main event loop: dispatch incoming JSON-RPC messages and lint results.
/// Owns the connection so that returning drops the sender and lets the writer
/// thread finish; joins the lint thread before returning.
fn main_loop(connection: Connection, editor_settings: EditorSettings) -> Result<(), DynError> {
    let (out_tx, out_rx) = crossbeam_channel::unbounded::<Outbound>();
    let (lint_tx, lint_rx) = crossbeam_channel::unbounded::<LintMsg>();
    let (read_tx, read_rx) = crossbeam_channel::unbounded::<ReadJob>();
    let lint_handle = spawn_lint_thread(lint_rx, read_rx, out_tx);
    // `done_tx`/`done_rx` are created inside the lint thread (see
    // `spawn_lint_thread`) so the main loop never holds the read end.

    let mut state = GlobalState::new(connection.sender.clone(), lint_tx, read_tx, editor_settings);

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
/// discovered `ravel.toml` is authoritative and ignores them entirely. Fields
/// are `Option` so an unset key leaves the built-in default in place.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct EditorSettings {
    line_width: Option<u32>,
    indent_width: Option<u32>,
}

impl EditorSettings {
    /// Extract our settings from a client-supplied JSON value. Accepts either
    /// the bare options object or a tree namespaced under a `"ravel"` key (how
    /// `workspace/didChangeConfiguration` clients typically scope settings).
    /// Unknown keys are ignored, and a malformed value yields the defaults.
    fn from_client_value(value: &serde_json::Value) -> Self {
        let section = value
            .get("ravel")
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

/// Resolve the [`FormatStyle`] for a document: a discovered `ravel.toml`
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
    Request(LintRequest),
}

/// A read-only request the lint thread services by cloning its salsa db and
/// running the work off-thread on `rayon`. Each variant carries the live buffer
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
    Hover {
        id: RequestId,
        path: PathBuf,
        text: String,
        position: Position,
        provider: Arc<CompositeProvider>,
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
    /// The symbol provider changed (cache loaded or a background build finished);
    /// the main loop caches it for hover.
    ProviderUpdated(Arc<CompositeProvider>),
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
    config_cache: HashMap<PathBuf, ResolvedSettings>,
    /// The current symbol provider, used for hover. Updated by the lint thread
    /// via [`Outbound::ProviderUpdated`]; starts base-R-only.
    provider: Arc<CompositeProvider>,
    /// Editor-pushed formatter defaults; the fallback when no `ravel.toml` is
    /// found. Updated by `workspace/didChangeConfiguration`.
    editor_settings: EditorSettings,
    sender: Sender<Message>,
    lint_tx: Sender<LintMsg>,
    /// Channel to the lint thread for read-only jobs (formatting, hover). The
    /// lint thread owns the salsa db, so it mints a short-lived clone per job and
    /// runs the read off-thread against the cached parse. See [`run_read`].
    read_tx: Sender<ReadJob>,
}

impl GlobalState {
    fn new(
        sender: Sender<Message>,
        lint_tx: Sender<LintMsg>,
        read_tx: Sender<ReadJob>,
        editor_settings: EditorSettings,
    ) -> Self {
        Self {
            documents: HashMap::new(),
            findings: HashMap::new(),
            config_cache: HashMap::new(),
            provider: Arc::new(CompositeProvider::base_only()),
            editor_settings,
            sender,
            lint_tx,
            read_tx,
        }
    }

    fn on_request(&mut self, req: Request) {
        match req.method.as_str() {
            Formatting::METHOD => self.on_formatting(req),
            CodeActionRequest::METHOD => self.on_code_action(req),
            HoverRequest::METHOD => self.on_hover(req),
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
            rayon::spawn(move || {
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
        rayon::spawn(move || {
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
            provider: Arc::clone(&self.provider),
            sender: self.sender.clone(),
        });
    }

    /// Hand a read-only job to the lint thread (db owner), which snapshots the db
    /// and runs it on `rayon`. If that channel is gone (shutdown in flight), reply
    /// `null` so the client isn't left waiting.
    fn dispatch_read(&self, job: ReadJob) {
        if let Err(crossbeam_channel::SendError(job)) = self.read_tx.send(job) {
            let (id, sender) = match job {
                ReadJob::Format { id, sender, .. } => (id, sender),
                ReadJob::Hover { id, sender, .. } => (id, sender),
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
                        // up on the next pull. A discovered `ravel.toml` still
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
            Outbound::ProviderUpdated(provider) => self.provider = provider,
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
        let _ = self.lint_tx.send(LintMsg::Request(LintRequest {
            uri,
            path,
            text,
            version,
            lint_config,
            index_config,
        }));
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
) -> JoinHandle<()> {
    let (build_tx, build_rx) = crossbeam_channel::unbounded::<Arc<CompositeProvider>>();
    let (done_tx, done_rx) = crossbeam_channel::unbounded::<AnalyzeDone>();
    std::thread::Builder::new()
        .name("ravel-lint".to_string())
        .spawn(move || {
            let mut worker = LintWorker {
                db: IncrementalDatabase::default(),
                index: Arc::new(CompositeProvider::base_only()),
                index_loaded: HashSet::new(),
                index_attempts: HashSet::new(),
                out_tx,
                build_tx,
                done_tx,
                inflight: None,
                pending: HashMap::new(),
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
    /// The symbol provider used for linting. Starts base-R-only and is replaced
    /// once the index cache is loaded (and again after a background build).
    index: Arc<CompositeProvider>,
    /// Workspace anchors whose index cache has already been loaded into `index`.
    index_loaded: HashSet<PathBuf>,
    /// Packages a background harvest has already been scheduled for this session
    /// — never retried, so a not-installed package doesn't loop.
    index_attempts: HashSet<SmolStr>,
    out_tx: Sender<Outbound>,
    build_tx: Sender<Arc<CompositeProvider>>,
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
}

impl LintWorker {
    fn run(
        &mut self,
        lint_rx: &Receiver<LintMsg>,
        read_rx: &Receiver<ReadJob>,
        build_rx: &Receiver<Arc<CompositeProvider>>,
        done_rx: &Receiver<AnalyzeDone>,
    ) {
        loop {
            select! {
                recv(lint_rx) -> msg => {
                    let Ok(LintMsg::Request(req)) = msg else { break };
                    // Coalesce: keep only the latest version per URI, so a fast
                    // typist's stale edits are dropped before they're ever linted.
                    self.enqueue(req);
                    while let Ok(LintMsg::Request(r)) = lint_rx.try_recv() {
                        self.enqueue(r);
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
                    let snapshot = self.db.clone();
                    rayon::spawn(move || run_read(snapshot, job));
                }
                recv(build_rx) -> built => {
                    let Ok(provider) = built else { continue };
                    self.index = Arc::clone(&provider);
                    let _ = self.out_tx.send(Outbound::ProviderUpdated(provider));
                    let _ = self.out_tx.send(Outbound::RelintAll);
                }
            }
        }
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
    /// read-phase analyze on a `rayon` worker holding a db clone. Returning to
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

        let provider = self.ensure_index(&anchor, &req.index_config);

        // Write-phase: push the live buffer + sibling files into the persistent
        // db. Cheap — the parse/model are lazy salsa queries deferred to analyze.
        let active = self.db.upsert_file(&req.path, req.text.clone());
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

        // `auto_build` reads the buffer + provider and mutates `index_attempts`,
        // so it stays on the lint thread; it spawns its own background build.
        if req.index_config.auto_build {
            self.maybe_build(&anchor, &req.index_config, &req.text, &provider);
        }

        // Read-phase on rayon, holding a db clone. A superseding edit (or any
        // write) trips `salsa::Cancelled`, caught here so a cancelled analyze
        // publishes nothing; the main loop's version gate is the backstop.
        let snapshot = self.db.clone();
        let out_tx = self.out_tx.clone();
        let done_tx = self.done_tx.clone();
        let uri = req.uri.clone();
        let version = req.version;
        let text = req.text;
        self.inflight = Some(InflightAnalyze {
            uri: uri.clone(),
            version,
        });

        rayon::spawn(move || {
            let result = salsa::Cancelled::catch(AssertUnwindSafe(|| {
                crate::linter::check::analyze_prepared(&snapshot, &prepared, &*provider)
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

    /// Load the index cache for `anchor` into the provider the first time we see
    /// that workspace; return the current provider either way.
    fn ensure_index(&mut self, anchor: &Path, cfg: &IndexConfig) -> Arc<CompositeProvider> {
        if self.index_loaded.contains(anchor) {
            return Arc::clone(&self.index);
        }
        let provider = match resolve_cache_root(None, cfg.cache_dir.as_deref()) {
            Ok(root) => {
                let cache = Cache::new(root);
                Arc::new(CompositeProvider::with_index(IndexedProvider::from_cache(
                    &cache,
                )))
            }
            Err(_) => Arc::new(CompositeProvider::base_only()),
        };
        self.index = Arc::clone(&provider);
        self.index_loaded.insert(anchor.to_path_buf());
        let _ = self
            .out_tx
            .send(Outbound::ProviderUpdated(Arc::clone(&provider)));
        provider
    }

    /// Spawn a background harvest for the document's unknown packages. On
    /// success the new provider is sent back on `build_tx`.
    fn maybe_build(
        &mut self,
        anchor: &Path,
        cfg: &IndexConfig,
        source: &str,
        provider: &CompositeProvider,
    ) {
        let to_build = packages_to_build(&mut self.index_attempts, provider, source);
        if to_build.is_empty() {
            return;
        }
        let Ok(cache_root) = resolve_cache_root(None, cfg.cache_dir.as_deref()) else {
            return;
        };
        let cfg = cfg.clone();
        let anchor = anchor.to_path_buf();
        let build_tx = self.build_tx.clone();
        rayon::spawn(move || {
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
                let provider = Arc::new(CompositeProvider::with_index(
                    IndexedProvider::from_cache(&cache),
                ));
                let _ = build_tx.send(provider);
            }
        });
    }
}

/// Packages referenced in `source` that the current `provider` can't resolve and
/// that we haven't already attempted this session. Marks the returned packages
/// as attempted so they aren't built twice.
fn packages_to_build(
    attempts: &mut HashSet<SmolStr>,
    provider: &CompositeProvider,
    source: &str,
) -> Vec<SmolStr> {
    referenced_in_source(source)
        .into_iter()
        .filter(|pkg| !provider.package_indexed(pkg) && attempts.insert(pkg.clone()))
        .collect()
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Read jobs (run on `rayon` with a salsa db snapshot)
// ---------------------------------------------------------------------------

/// Service a read-only job against a db `snapshot`, replying to the client.
/// Runs on a `rayon` worker; the `snapshot` is dropped on return so it never
/// blocks the lint thread's next write longer than the job itself.
fn run_read(snapshot: IncrementalDatabase, job: ReadJob) {
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
        ReadJob::Hover {
            id,
            path,
            text,
            position,
            provider,
            sender,
        } => {
            let result = hover_via_db(&snapshot, &path, &text, position, &provider);
            let _ = sender.send(Message::Response(Response::new_ok(id, result)));
        }
    }
}

/// Format `text` off the snapshot's cached parse when the db's tracked buffer
/// for `path` still matches it; otherwise re-parse. A write racing the read
/// trips [`salsa::Cancelled`], which also falls back to a fresh parse.
fn format_edits_via_db(
    snapshot: &IncrementalDatabase,
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

/// Resolve hover off the snapshot's cached parse when the db's tracked buffer for
/// `path` still matches `text`; otherwise re-parse. Falls back on cancellation.
fn hover_via_db(
    snapshot: &IncrementalDatabase,
    path: &Path,
    text: &str,
    position: Position,
    provider: &CompositeProvider,
) -> Option<Hover> {
    let line_index = LineIndex::new(text);
    let offset = line_index.position_to_byte(position).min(text.len());
    let cached = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let file = snapshot.lookup_file(path)?;
        if snapshot.file_text(file) != text {
            return None;
        }
        let root = snapshot.parsed_tree(file);
        Some(hover_from_node(&root, &line_index, offset, provider))
    }));
    match cached {
        Ok(Some(hover)) => hover,
        Ok(None) | Err(_) => {
            let root = parse(text).cst;
            hover_from_node(&root, &line_index, offset, provider)
        }
    }
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
        source: Some("ravel".to_string()),
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

/// Build hover contents for the symbol at byte `offset`, if it resolves to an
/// indexed package export. Pure (parses `text` itself) so it is unit-testable.
pub fn compute_hover(text: &str, offset: usize, provider: &CompositeProvider) -> Option<Hover> {
    let root = parse(text).cst;
    let line_index = LineIndex::new(text);
    hover_from_node(&root, &line_index, offset.min(text.len()), provider)
}

/// Build hover contents off an already-parsed CST (and a matching line index),
/// without re-parsing. The LSP read path uses this against the cached parse tree
/// in its salsa database; [`compute_hover`] is the parse-from-text wrapper.
fn hover_from_node(
    root: &SyntaxNode,
    line_index: &LineIndex,
    offset: usize,
    provider: &CompositeProvider,
) -> Option<Hover> {
    let offset = TextSize::new(offset as u32);
    let query = symbol_query_at(root, offset)?;
    let (package, entry, range) = resolve_query(query, root, provider)?;

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
    provider: &'p CompositeProvider,
) -> Option<(SmolStr, &'p SymbolEntry, TextRange)> {
    match query {
        SymbolQuery::Namespaced {
            package,
            name,
            range,
        } => {
            let entry = provider.indexed().lookup(&package, &name)?;
            Some((package, entry, range))
        }
        SymbolQuery::Bare { name, range } => {
            let model = SemanticModel::build(root);
            let package = match provider.origin(&name, model.loaded_packages()) {
                PackageOrigin::Resolved(p) => p,
                // The last attacher masks the rest under R's lookup rules.
                PackageOrigin::Ambiguous(mut v) => v.pop()?,
                PackageOrigin::Unknown => return None,
            };
            let entry = provider.indexed().lookup(&package, &name)?;
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
    #[cfg(test)]
    use std::path::Path;
    use std::path::PathBuf;
    #[cfg(test)]
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

    /// Convert a filesystem path to a `file:` URI. Currently only the tests build
    /// URIs from paths (the client always supplies URIs in real traffic).
    #[cfg(test)]
    pub fn from_path(path: &Path) -> Option<Uri> {
        let s = path.to_str()?;
        let mut out = String::from("file://");
        encode_into(&to_uri_path(s), &mut out);
        Uri::from_str(&out).ok()
    }

    #[cfg(all(test, windows))]
    fn to_uri_path(s: &str) -> String {
        // "C:\Users\x" → "/C:/Users/x" (the URI path needs a leading slash)
        format!("/{}", s.replace('\\', "/"))
    }

    #[cfg(all(test, not(windows)))]
    fn to_uri_path(s: &str) -> String {
        s.to_string()
    }

    /// Percent-encode `s`, leaving the unreserved set plus `/` and `:` (drive
    /// letters) intact.
    #[cfg(test)]
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

    #[cfg(test)]
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
    fn editor_settings_parse_namespaced_under_ravel() {
        // didChangeConfiguration clients push their whole settings tree; ours is
        // scoped under "ravel" and sibling keys are ignored.
        let value = serde_json::json!({
            "ravel": { "lineWidth": 120 },
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
        // ravel.toml present → editor settings ignored entirely.
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

    fn indexed_dplyr() -> CompositeProvider {
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
        CompositeProvider::with_index(IndexedProvider::from_indices([idx]))
    }

    #[test]
    fn packages_to_build_skips_indexed_and_dedups_attempts() {
        let mut attempts = HashSet::new();
        let provider = indexed_dplyr();
        // dplyr is indexed (skipped); a default package (stats) is "indexed" too;
        // only tidyr needs a build.
        let src = "library(dplyr)\nlibrary(stats)\nlibrary(tidyr)\n";
        let first = packages_to_build(&mut attempts, &provider, src);
        assert_eq!(first, vec![SmolStr::new("tidyr")]);
        // A second pass returns nothing — tidyr was already attempted.
        let second = packages_to_build(&mut attempts, &provider, src);
        assert!(second.is_empty(), "expected no re-attempt, got {second:?}");
    }

    // --- hover ------------------------------------------------------------

    /// dplyr with one richly-documented export (`across`).
    fn documented_dplyr() -> CompositeProvider {
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
        CompositeProvider::with_index(IndexedProvider::from_indices([idx]))
    }

    /// Byte offset of the first occurrence of `needle` in `src`.
    fn offset_of(src: &str, needle: &str) -> usize {
        src.find(needle).expect("needle present") + 1
    }

    fn hover_markdown(src: &str, needle: &str, provider: &CompositeProvider) -> Option<String> {
        compute_hover(src, offset_of(src, needle), provider).map(|h| match h.contents {
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
        let snapshot = db.clone();
        assert_eq!(
            format_edits_via_db(&snapshot, path, buffer, style),
            expected,
            "cached-tree format must match the re-parse path"
        );

        // Stale db (tracked text lags the buffer) → fall back to a fresh parse.
        let mut stale = IncrementalDatabase::default();
        stale.upsert_file(path, "y <- 1\n".to_string());
        assert_eq!(
            format_edits_via_db(&stale.clone(), path, buffer, style),
            expected,
            "version skew must fall back to the buffer text"
        );

        // Untracked path → fall back as well.
        let empty = IncrementalDatabase::default();
        assert_eq!(
            format_edits_via_db(&empty, path, buffer, style),
            expected,
            "untracked path must fall back to the buffer text"
        );
    }

    #[test]
    fn hover_via_db_matches_compute() {
        use crate::incremental::IncrementalDatabase;
        let provider = documented_dplyr();
        let path = test_path();
        let src = "library(dplyr)\nacross(a, mean)\n";
        // Cursor on `across` (line 1, character 0).
        let position = pos(1, 0);

        let mut db = IncrementalDatabase::default();
        db.upsert_file(path, src.to_string());
        let hover = hover_via_db(&db.clone(), path, src, position, &provider)
            .expect("hover for across via db");
        let md = match hover.contents {
            HoverContents::Markup(m) => m.value,
            other => panic!("expected markup, got {other:?}"),
        };
        assert!(md.contains("dplyr::across"), "origin: {md}");

        // Untracked path still resolves, via the fresh-parse fallback.
        let empty = IncrementalDatabase::default();
        assert!(
            hover_via_db(&empty, path, src, position, &provider).is_some(),
            "fallback hover should resolve too"
        );
    }
}
