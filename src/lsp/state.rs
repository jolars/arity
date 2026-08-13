use super::*;

/// Which grammar an open document is written in.
///
/// The server serves **two** languages — R and the DCF of a `DESCRIPTION` — and
/// nearly every request is R-only. Answering an R-grammar request for a
/// `DESCRIPTION` ranges from useless (folding) to destructive: formatting would
/// hand the client the DCF reflowed as R and rewrite the file. So the kind is
/// decided once, at `didOpen`, and every buffer lookup states which grammar it
/// expects (see [`GlobalState::r_doc_snapshot`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentKind {
    /// R source. The default: anything not recognized as another grammar.
    R,
    /// A package `DESCRIPTION`, in DCF.
    Description,
}

impl DocumentKind {
    /// Classify from the URI, with the client's `languageId` as a fallback.
    ///
    /// **The file name wins.** `editors/code` already registers `NAMESPACE`
    /// under language `r`, so a client sending `languageId: "r"` for a
    /// `DESCRIPTION` is entirely plausible — and trusting it would format DCF
    /// as R. The last path segment is read off the URI rather than a converted
    /// path because [`uri::to_path`] gives up on non-`file` schemes, and a
    /// `git:`-scheme diff of a `DESCRIPTION` must not route as R either.
    pub(crate) fn from_uri(uri: &Uri, language_id: Option<&str>) -> Self {
        let name = uri.path().as_str().rsplit('/').next().unwrap_or_default();
        if name == DESCRIPTION_FILE_NAME {
            return Self::Description;
        }
        match language_id {
            // DCF is the Debian `control` grammar, so a client may have picked
            // up any of these ids for the file from another extension.
            Some("r-description" | "dcf" | "debian-control") => Self::Description,
            _ => Self::R,
        }
    }

    /// Classify from a filesystem path, for the read jobs — which carry the
    /// derived [`PathBuf`], never the URI. Agrees with
    /// [`from_uri`](Self::from_uri), including for the synthesized path an
    /// `untitled:` buffer gets (see
    /// [`placeholder_file_name`](Self::placeholder_file_name)).
    pub(crate) fn from_path(path: &Path) -> Self {
        match path.file_name().and_then(|n| n.to_str()) {
            Some(DESCRIPTION_FILE_NAME) => Self::Description,
            _ => Self::R,
        }
    }

    /// The file name a document of this kind gets when its URI has no path —
    /// an `untitled:` buffer. Keeps the synthesized path and the kind agreeing,
    /// so anything downstream that re-derives the grammar from the path reaches
    /// the same answer.
    pub(crate) fn placeholder_file_name(self) -> &'static str {
        match self {
            Self::R => "untitled.R",
            Self::Description => DESCRIPTION_FILE_NAME,
        }
    }
}

/// The one spelling of the file name, shared by both [`DocumentKind`]
/// constructors so they cannot drift apart.
const DESCRIPTION_FILE_NAME: &str = "DESCRIPTION";

/// An open document: the live buffer plus the version the client last sent.
///
/// The buffer is shared — reads and lints clone the `Arc`, never the text — and
/// is **immutable once shared**: [`Document::apply_edit`] mutates only a
/// uniquely-owned buffer, so an in-flight read observes exactly the bytes of
/// the version it was dispatched at. `version` sits outside the `Arc` because
/// the staleness gate compares it, not the contents.
#[derive(Debug, Clone)]
pub(crate) struct Document {
    buffer: Arc<TextBuffer>,
    version: i32,
    /// Decided once at `didOpen`: a document's grammar cannot change under it.
    kind: DocumentKind,
}

impl Document {
    fn new(text: String, version: i32, kind: DocumentKind) -> Self {
        Self {
            buffer: Arc::new(TextBuffer::new(text)),
            version,
            kind,
        }
    }

    /// Splice `range` -> `insert` into the buffer, patching its line index.
    ///
    /// `Arc::make_mut` copies only while the buffer is still shared, so a
    /// `didChange` batch pays at most one copy: the first change unshares it and
    /// the rest splice in place (the main loop is the only writer).
    fn apply_edit(&mut self, range: std::ops::Range<usize>, insert: &str) {
        Arc::make_mut(&mut self.buffer).apply_edit(range, insert);
    }
}

/// A `textDocument/diagnostic` request awaiting fresh findings: the buffer had no
/// current cached findings when the pull arrived, so the request is parked here
/// (keyed by URI) until the lint thread reports back. See
/// [`GlobalState::on_document_diagnostic`]. `previous_result_id` is the client's
/// last-seen id, kept so the cold path can still answer `Unchanged` when the
/// fresh findings hash to the same id (e.g. an unrelated file re-linted by a
/// cross-file `RelintAll`).
pub(crate) struct PendingPull {
    id: RequestId,
    previous_result_id: Option<String>,
}

/// An internal, already-decided document diagnostic report ready to serialize.
pub(crate) enum DiagnosticReport {
    /// A full set of items with an optional `resultId`.
    Full(Vec<LspDiagnostic>, Option<String>),
    /// The previously delivered report (with this `resultId`) is still accurate.
    Unchanged(String),
}

/// Which kind of report to return for a pull. `Unchanged` is only valid when the
/// client supplied a `previousResultId` that equals the current one — a server
/// "can only return `unchanged` if result ids are provided" (LSP 3.17).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DiagnosticReportKind {
    Full,
    Unchanged,
}

/// Decide a pull's report kind: `Unchanged` iff both ids are present and equal.
pub(crate) fn report_kind(
    previous_result_id: Option<&str>,
    current_id: Option<&str>,
) -> DiagnosticReportKind {
    match (previous_result_id, current_id) {
        (Some(prev), Some(cur)) if prev == cur => DiagnosticReportKind::Unchanged,
        _ => DiagnosticReportKind::Full,
    }
}

/// Content-derived `resultId`: identical findings hash to an identical id, so a
/// re-lint that changes nothing (e.g. a cross-file [`Outbound::RelintAll`]) lets
/// `Unchanged` fire instead of re-sending a full report. Findings fully determine
/// the delivered report at a fixed buffer version, so hashing them is equivalent
/// to hashing the rendered items but far cheaper. Only session-scoped stability
/// is required (the client compares ids within a live session), so a plain
/// [`DefaultHasher`](std::collections::hash_map::DefaultHasher) over the already
/// `Serialize`d findings is enough and pulls in no new dependency.
pub(crate) fn content_result_id(findings: &[Diagnostic]) -> String {
    use std::hash::{Hash, Hasher};
    let json = serde_json::to_vec(findings).unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    json.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Messages from the lint thread back to the main loop.
pub(crate) enum Outbound {
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
    /// A work-done progress update from a background job (`build_index` or the
    /// sidecar fetch). The main loop forwards it to the client as `$/progress`
    /// (creating the token via `window/workDoneProgress/create` on the first
    /// `Begin`), gated on the client's `window.workDoneProgress` capability. See
    /// [`GlobalState::on_progress`] and [`ProgressReporter`].
    Progress {
        token: String,
        work: WorkDoneProgress,
    },
    /// A finished read-pool reply, routed back through the main loop so it can
    /// be gated on cancellation and document version before reaching the client.
    /// The read pool cannot see either (the live-request set and current buffer
    /// versions live on the main loop), so the worker builds the success
    /// response and the loop decides whether to deliver it, drop it (canceled),
    /// or replace it with `ContentModified` (superseded). See
    /// [`GlobalState::on_read_reply`].
    ReadReply(Response),
}

/// A read dispatched to the pool and not yet answered. Tracked in
/// [`GlobalState::live_reads`] so a `$/cancelRequest` can short-circuit it and a
/// superseding edit can invalidate its result.
struct InflightRead {
    /// The `(uri, version)` the read was dispatched against, for stale-read
    /// gating. `None` for reads not tied to a single document version (workspace
    /// symbol, completion resolve, and the hierarchy calls re-derived from a
    /// round-tripped item) — those are cancelable but never `ContentModified`.
    doc: Option<(Uri, i32)>,
}

/// `RequestCancelled`: the client asked to cancel this request. Mirrors
/// `lsp_types::error_codes::REQUEST_CANCELLED` (which is `i64`; `Response::new_err`
/// wants `i32`).
const REQUEST_CANCELLED: i32 = -32800;
/// `ContentModified`: the document changed under a read, so its result is stale
/// and the client should re-request. Mirrors `error_codes::CONTENT_MODIFIED`.
const CONTENT_MODIFIED: i32 = -32801;

pub(crate) struct GlobalState {
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
    /// True when the client supports the pull diagnostic model: we suppress push
    /// (no `publishDiagnostics`) and answer `textDocument/diagnostic` instead.
    pull_mode: bool,
    /// True when the client advertised `window.workDoneProgress`. Gates all
    /// server-initiated progress: without it we emit no `window/workDoneProgress/
    /// create` and no `$/progress` (see [`on_progress`](Self::on_progress)).
    work_done_progress: bool,
    /// The position encoding negotiated at `initialize` (see
    /// [`negotiate_position_encoding`](super::server::negotiate_position_encoding)).
    /// Threaded into every byte-offset ↔ LSP-position conversion; a session
    /// constant.
    position_encoding: PositionEncoding,
    /// Pull requests parked while the lint thread computes fresh findings, keyed
    /// by URI. Drained on the next `Outbound::Diagnostics` for that URI (or on
    /// close, with an empty report, so a request never hangs).
    pending_pull: HashMap<Uri, Vec<PendingPull>>,
    /// The opaque `resultId` of the report most recently delivered per URI. It is
    /// a content hash of that lint's findings ([`content_result_id`]), so an
    /// unrelated file re-linted by a cross-file change keeps its id and a re-pull
    /// gets `unchanged` — which is only returned when this matches the client's
    /// `previousResultId`.
    report_ids: HashMap<Uri, String>,
    /// Monotonic id source for server→client requests (`workspace/diagnostic/
    /// refresh`); the client's responses are ignored by the main loop.
    next_req_id: i32,
    config_cache: HashMap<PathBuf, ResolvedSettings>,
    /// Editor-pushed formatter defaults; the fallback when no `arity.toml` is
    /// found. Updated by `workspace/didChangeConfiguration`.
    editor_settings: EditorSettings,
    /// Reads dispatched to the pool and not yet answered, keyed by request id.
    /// An entry is present exactly while the read is in flight and is removed
    /// once — by whichever of `$/cancelRequest` or the read's reply fires first
    /// on this (single) thread. The loser finds the id absent and no-ops, so a
    /// request is answered exactly once. See [`register_read`](Self::register_read),
    /// [`on_read_reply`](Self::on_read_reply), and the `Cancel` arm of
    /// [`on_notification`](Self::on_notification).
    live_reads: HashMap<RequestId, InflightRead>,
    sender: Sender<Message>,
    /// Clone of the main loop's outbound channel. Read handlers that spawn
    /// directly on the read pool (code actions, symbols, folding, …) route their
    /// replies back through here as [`Outbound::ReadReply`] so the loop can gate
    /// them, exactly like the [`ReadJob`] path does via the lint thread.
    out_tx: Sender<Outbound>,
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
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        sender: Sender<Message>,
        out_tx: Sender<Outbound>,
        lint_tx: Sender<LintMsg>,
        read_tx: Sender<ReadJob>,
        read_spawner: Spawner,
        editor_settings: EditorSettings,
        pull_mode: bool,
        work_done_progress: bool,
        position_encoding: PositionEncoding,
    ) -> Self {
        Self {
            documents: HashMap::new(),
            findings: HashMap::new(),
            rename_anchors: HashMap::new(),
            pull_mode,
            work_done_progress,
            position_encoding,
            pending_pull: HashMap::new(),
            report_ids: HashMap::new(),
            next_req_id: 0,
            config_cache: HashMap::new(),
            editor_settings,
            live_reads: HashMap::new(),
            sender,
            out_tx,
            lint_tx,
            read_tx,
            read_spawner,
        }
    }

    /// Record that the read answering `id` is in flight, so `$/cancelRequest`
    /// can short-circuit it and a superseding edit can invalidate it. Pass the
    /// `(uri, version)` the read reads for stale-read gating, or `None` for a
    /// read not tied to one document version (cancelable but never stale).
    fn register_read(&mut self, id: RequestId, doc: Option<(Uri, i32)>) {
        self.live_reads.insert(id, InflightRead { doc });
    }

    pub(crate) fn on_request(&mut self, req: Request) {
        match req.method.as_str() {
            Formatting::METHOD => self.on_formatting(req),
            RangeFormatting::METHOD => self.on_range_formatting(req),
            CodeActionRequest::METHOD => self.on_code_action(req),
            DocumentDiagnosticRequest::METHOD => self.on_document_diagnostic(req),
            HoverRequest::METHOD => self.on_hover(req),
            SignatureHelpRequest::METHOD => self.on_signature_help(req),
            Completion::METHOD => self.on_completion(req),
            ResolveCompletionItem::METHOD => self.on_resolve_completion(req),
            GotoDefinition::METHOD => self.on_definition(req),
            References::METHOD => self.on_references(req),
            DocumentHighlightRequest::METHOD => self.on_document_highlight(req),
            DocumentSymbolRequest::METHOD => self.on_document_symbol(req),
            FoldingRangeRequest::METHOD => self.on_folding_range(req),
            SelectionRangeRequest::METHOD => self.on_selection_range(req),
            DocumentColor::METHOD => self.on_document_color(req),
            ColorPresentationRequest::METHOD => self.on_color_presentation(req),
            DocumentLinkRequest::METHOD => self.on_document_link(req),
            SemanticTokensFullRequest::METHOD => self.on_semantic_tokens(req),
            PrepareRenameRequest::METHOD => self.on_prepare_rename(req),
            Rename::METHOD => self.on_rename(req),
            WillRenameFiles::METHOD => self.on_will_rename_files(req),
            WorkspaceSymbolRequest::METHOD => self.on_workspace_symbol(req),
            CallHierarchyPrepare::METHOD => self.on_prepare_call_hierarchy(req),
            CallHierarchyIncomingCalls::METHOD => self.on_incoming_calls(req),
            CallHierarchyOutgoingCalls::METHOD => self.on_outgoing_calls(req),
            TypeHierarchyPrepare::METHOD => self.on_prepare_type_hierarchy(req),
            TypeHierarchySupertypes::METHOD => self.on_supertypes(req),
            TypeHierarchySubtypes::METHOD => self.on_subtypes(req),
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
        let Some((buffer, version)) = self.r_doc_snapshot(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let Ok(settings) = self.resolve_settings(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        self.register_read(id.clone(), Some((uri, version)));
        self.dispatch_read(ReadJob::Format {
            id,
            path,
            buffer,
            style: settings.style,
            out: self.out_tx.clone(),
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
        let Some((buffer, version)) = self.r_doc_snapshot(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let Ok(settings) = self.resolve_settings(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        self.register_read(id.clone(), Some((uri, version)));
        self.dispatch_read(ReadJob::FormatRange {
            id,
            path,
            buffer,
            range: params.range,
            style: settings.style,
            out: self.out_tx.clone(),
        });
    }

    fn on_code_action(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<CodeActionParams>(CodeActionRequest::METHOD) else {
            self.respond_err(id, "invalid code action params");
            return;
        };
        let uri = params.text_document.uri;
        let Some((buffer, version)) = self.r_doc_snapshot(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let range = params.range;
        let out = self.out_tx.clone();
        let encoding = self.position_encoding;
        self.register_read(id.clone(), Some((uri.clone(), version)));

        // Fast path: the last lint's findings are still current, so serving quick
        // fixes is a pure lookup — no re-parse, no re-lint. Their byte ranges
        // index `text`, which the version match proves is the linted source.
        if let Some((cached_version, findings)) = self.findings.get(&uri)
            && *cached_version == version
        {
            let findings = Arc::clone(findings);
            self.read_spawner.spawn(move || {
                let actions = code_actions_from_findings(&findings, &buffer, &uri, range, encoding);
                let _ = out.send(Outbound::ReadReply(Response::new_ok(id, actions)));
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
            let actions = compute_code_actions(buffer.text(), &path, &lint, &uri, range, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, actions)));
        });
    }

    /// `textDocument/diagnostic`: the pull counterpart of pushed diagnostics.
    /// Serves the most recent lint's findings (cached per URI by version, like the
    /// code-action fast path) when they're current; otherwise parks the request in
    /// [`pending_pull`](Self::pending_pull) and triggers a lint, answering once the
    /// findings arrive in [`on_outbound`](Self::on_outbound). Returns `unchanged`
    /// when the client's `previousResultId` still matches the cached report id.
    fn on_document_diagnostic(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) =
            req.extract::<DocumentDiagnosticParams>(DocumentDiagnosticRequest::METHOD)
        else {
            self.respond_err(id, "invalid diagnostic params");
            return;
        };
        let uri = params.text_document.uri;
        // Both grammars publish diagnostics, and the pull path only shuttles
        // whatever the lint thread decided — so it does not branch on the kind.
        let Some((buffer, version, _kind)) = self.doc_snapshot_any(&uri) else {
            // Unknown document (never opened, or already closed): an empty report.
            self.respond_diagnostic(id, DiagnosticReport::Full(Vec::new(), None));
            return;
        };

        // Warm path: the cached findings still describe the live buffer, so the
        // report is a pure lookup (their byte ranges index `text`).
        if matches!(self.findings.get(&uri), Some((v, _)) if *v == version) {
            let result_id = self.report_ids.get(&uri).cloned();
            let report =
                match report_kind(params.previous_result_id.as_deref(), result_id.as_deref()) {
                    DiagnosticReportKind::Unchanged => DiagnosticReport::Unchanged(
                        result_id.expect("unchanged implies a known result id"),
                    ),
                    DiagnosticReportKind::Full => {
                        let (_, findings) = self.findings.get(&uri).expect("present above");
                        let items = findings_to_items(findings, &buffer, self.position_encoding);
                        DiagnosticReport::Full(items, result_id)
                    }
                };
            self.respond_diagnostic(id, report);
            return;
        }

        // Cold path: no current findings yet (a pull before the lint caught up).
        // Park the request and lint; `on_outbound` answers it with fresh results,
        // comparing this `previous_result_id` so an unchanged file still resolves
        // to `Unchanged`.
        self.pending_pull
            .entry(uri.clone())
            .or_default()
            .push(PendingPull {
                id,
                previous_result_id: params.previous_result_id,
            });
        self.send_lint(uri, Vec::new());
    }

    fn on_hover(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<HoverParams>(HoverRequest::METHOD) else {
            self.respond_err(id, "invalid hover params");
            return;
        };
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        // Both grammars hover; `hover_via_db` picks the resolver off the path.
        let Some((buffer, version, kind)) = self.doc_snapshot_any(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let path =
            uri::to_path(&uri).unwrap_or_else(|| PathBuf::from(kind.placeholder_file_name()));
        self.register_read(id.clone(), Some((uri, version)));
        self.dispatch_read(ReadJob::Hover {
            id,
            path,
            buffer,
            position,
            out: self.out_tx.clone(),
        });
    }

    /// `textDocument/signatureHelp`: inside a call, show the callee's signature
    /// and highlight the active parameter. A read-only job dispatched like hover;
    /// resolution + active-parameter tracking run on the read pool. See
    /// [`signature_help_via_db`].
    fn on_signature_help(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<SignatureHelpParams>(SignatureHelpRequest::METHOD)
        else {
            self.respond_err(id, "invalid signatureHelp params");
            return;
        };
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some((buffer, version)) = self.r_doc_snapshot(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        self.register_read(id.clone(), Some((uri, version)));
        self.dispatch_read(ReadJob::SignatureHelp {
            id,
            path,
            buffer,
            position,
            out: self.out_tx.clone(),
        });
    }

    /// `textDocument/completion`: scope-aware names + `pkg::` members. A
    /// read-only job dispatched like hover; items carry only labels until
    /// `completionItem/resolve` attaches docs.
    fn on_completion(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<CompletionParams>(Completion::METHOD) else {
            self.respond_err(id, "invalid completion params");
            return;
        };
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        // Both grammars complete; `completion_via_db` picks the candidate pool
        // off the path.
        let Some((buffer, version, kind)) = self.doc_snapshot_any(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let path =
            uri::to_path(&uri).unwrap_or_else(|| PathBuf::from(kind.placeholder_file_name()));
        self.register_read(id.clone(), Some((uri, version)));
        self.dispatch_read(ReadJob::Completion {
            id,
            path,
            buffer,
            position,
            out: self.out_tx.clone(),
        });
    }

    /// `completionItem/resolve`: lazily attach docs/signature to a completion
    /// item, using the identity stashed in its `data`. Needs the index, so it
    /// runs as a read-only job (no document lookup).
    fn on_resolve_completion(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, item)) = req.extract::<CompletionItem>(ResolveCompletionItem::METHOD) else {
            self.respond_err(id, "invalid completionItem/resolve params");
            return;
        };
        self.register_read(id.clone(), None);
        self.dispatch_read(ReadJob::ResolveCompletion {
            id,
            item: Box::new(item),
            out: self.out_tx.clone(),
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
        let Some((buffer, version)) = self.r_doc_snapshot(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        self.register_read(id.clone(), Some((uri.clone(), version)));
        self.dispatch_read(ReadJob::Definition {
            id,
            path,
            uri,
            buffer,
            position,
            out: self.out_tx.clone(),
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
        let Some((buffer, version)) = self.r_doc_snapshot(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        self.register_read(id.clone(), Some((uri.clone(), version)));
        self.dispatch_read(ReadJob::References {
            id,
            path,
            uri,
            buffer,
            position,
            include_declaration,
            out: self.out_tx.clone(),
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
        let Some((buffer, version)) = self.r_doc_snapshot(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        self.register_read(id.clone(), Some((uri, version)));
        let out = self.out_tx.clone();
        let encoding = self.position_encoding;
        self.read_spawner.spawn(move || {
            let line_index = buffer.line_index();
            let offset = line_index
                .position_to_byte(position, encoding)
                .min(buffer.len());
            let result = compute_document_highlights(buffer.text(), offset).map(|highlights| {
                highlights
                    .into_iter()
                    .map(|(range, kind)| DocumentHighlight {
                        range: text_range_to_lsp_range(line_index, range, encoding),
                        kind: Some(kind),
                    })
                    .collect::<Vec<_>>()
            });
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
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
        let Some((buffer, version)) = self.r_doc_snapshot(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        self.register_read(id.clone(), Some((uri, version)));
        let out = self.out_tx.clone();
        let encoding = self.position_encoding;
        self.read_spawner.spawn(move || {
            let symbols = compute_document_symbols_in(&buffer, encoding);
            let response = DocumentSymbolResponse::Nested(symbols);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, response)));
        });
    }

    /// `textDocument/foldingRange`: foldable regions (brace blocks, multi-line
    /// argument/parameter lists, parenthesized expressions, and comment runs).
    /// A pure CST walk with no semantic model, so it runs straight on the read
    /// pool like `on_document_symbol`.
    fn on_folding_range(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<FoldingRangeParams>(FoldingRangeRequest::METHOD) else {
            self.respond_err(id, "invalid foldingRange params");
            return;
        };
        let uri = params.text_document.uri;
        let Some((buffer, version)) = self.r_doc_snapshot(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        self.register_read(id.clone(), Some((uri, version)));
        let out = self.out_tx.clone();
        self.read_spawner.spawn(move || {
            let ranges = compute_folding_ranges_in(&buffer);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, ranges)));
        });
    }

    /// `textDocument/selectionRange`: "smart selection" chains that expand from
    /// each cursor position outward through the enclosing CST nodes. A pure
    /// single-file CST walk with no semantic model, so it runs straight on the
    /// read pool like folding range. See [`compute_selection_ranges`].
    fn on_selection_range(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<SelectionRangeParams>(SelectionRangeRequest::METHOD)
        else {
            self.respond_err(id, "invalid selectionRange params");
            return;
        };
        let uri = params.text_document.uri;
        let Some((buffer, version)) = self.r_doc_snapshot(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let positions = params.positions;
        self.register_read(id.clone(), Some((uri, version)));
        let out = self.out_tx.clone();
        let encoding = self.position_encoding;
        self.read_spawner.spawn(move || {
            let ranges = compute_selection_ranges_in(&buffer, &positions, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, ranges)));
        });
    }

    /// `textDocument/documentLink`: string literals that name existing files,
    /// made clickable. Relative spellings resolve against the document's own
    /// directory. A pure CST walk plus per-literal `stat` (no semantic model or
    /// workspace snapshot), so it runs straight on the read pool like folding.
    /// See [`compute_document_links`].
    fn on_document_link(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<DocumentLinkParams>(DocumentLinkRequest::METHOD) else {
            self.respond_err(id, "invalid documentLink params");
            return;
        };
        let uri = params.text_document.uri;
        let Some((buffer, version)) = self.r_doc_snapshot(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let base_dir = uri::to_path(&uri).and_then(|p| p.parent().map(Path::to_path_buf));
        let size_limit = self.editor_settings.link_file_size_limit();
        self.register_read(id.clone(), Some((uri, version)));
        let out = self.out_tx.clone();
        let encoding = self.position_encoding;
        self.read_spawner.spawn(move || {
            let links =
                compute_document_links_in(&buffer, base_dir.as_deref(), size_limit, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, links)));
        });
    }

    /// `textDocument/documentColor`: inline color swatches for string literals
    /// that spell a hex code or a named `grDevices` color. A pure single-file CST
    /// walk (no semantic model or workspace snapshot), so it runs straight on the
    /// read pool like folding. See [`compute_document_colors`].
    fn on_document_color(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<DocumentColorParams>(DocumentColor::METHOD) else {
            self.respond_err(id, "invalid documentColor params");
            return;
        };
        let uri = params.text_document.uri;
        let Some((buffer, version)) = self.r_doc_snapshot(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        self.register_read(id.clone(), Some((uri, version)));
        let out = self.out_tx.clone();
        let encoding = self.position_encoding;
        self.read_spawner.spawn(move || {
            let colors = compute_document_colors_in(&buffer, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, colors)));
        });
    }

    /// `textDocument/colorPresentation`: the hex spelling(s) the picker offers for
    /// a chosen color, with an edit that rewrites the literal in place. Pure text
    /// work on the read pool. See [`compute_color_presentations`].
    fn on_color_presentation(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) =
            req.extract::<ColorPresentationParams>(ColorPresentationRequest::METHOD)
        else {
            self.respond_err(id, "invalid colorPresentation params");
            return;
        };
        let uri = params.text_document.uri;
        let Some(buffer) = self.r_doc_snapshot(&uri).map(|(buffer, _)| buffer) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let (color, range) = (params.color, params.range);
        self.register_read(id.clone(), None);
        let out = self.out_tx.clone();
        let encoding = self.position_encoding;
        self.read_spawner.spawn(move || {
            let presentations = compute_color_presentations_in(&buffer, &color, range, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, presentations)));
        });
    }

    /// `textDocument/semanticTokens/full`: scope-aware highlighting for the whole
    /// document. A pure single-file CST walk (no workspace lookup), so like
    /// document symbol and folding range it runs straight on the read pool rather
    /// than through the lint thread. See [`compute_semantic_tokens`].
    fn on_semantic_tokens(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) =
            req.extract::<SemanticTokensParams>(SemanticTokensFullRequest::METHOD)
        else {
            self.respond_err(id, "invalid semanticTokens params");
            return;
        };
        let uri = params.text_document.uri;
        let Some((buffer, version)) = self.r_doc_snapshot(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        self.register_read(id.clone(), Some((uri, version)));
        let out = self.out_tx.clone();
        let encoding = self.position_encoding;
        self.read_spawner.spawn(move || {
            let tokens = compute_semantic_tokens_in(&buffer, encoding);
            let result = SemanticTokensResult::Tokens(tokens);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
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
        let Some(buffer) = self.r_doc_snapshot(&uri).map(|(buffer, _)| buffer) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let encoding = self.position_encoding;
        let line_index = buffer.line_index();
        let offset = line_index
            .position_to_byte(params.position, encoding)
            .min(buffer.len());
        match compute_prepare_rename_in(&buffer, offset, encoding) {
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
        let Some((buffer, version)) = self.r_doc_snapshot(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };

        let encoding = self.position_encoding;
        let offset = self
            .rename_anchors
            .get(&uri)
            .and_then(|anchor| rename_cursor_offset(buffer.text(), anchor))
            .unwrap_or_else(|| {
                let line_index = buffer.line_index();
                line_index
                    .position_to_byte(position, encoding)
                    .min(buffer.len())
            });
        // A rename consumes its anchor; a fresh prepare precedes any next rename.
        self.rename_anchors.remove(&uri);

        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        self.register_read(id.clone(), Some((uri.clone(), version)));
        self.dispatch_read(ReadJob::Rename {
            id,
            path,
            uri,
            buffer,
            offset,
            new_name,
            out: self.out_tx.clone(),
        });
    }

    /// `workspace/willRenameFiles`: build a [`WorkspaceEdit`] that rewrites
    /// `source("old")` literals in dependents to the renamed targets, so a file
    /// move keeps cross-file `source()` references resolving. The editor applies
    /// it atomically with the rename. Runs off a db snapshot on the read pool,
    /// like [`on_rename`](Self::on_rename).
    fn on_will_rename_files(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<RenameFilesParams>(WillRenameFiles::METHOD) else {
            self.respond_err(id, "invalid willRenameFiles params");
            return;
        };
        let renames = file_renames_to_paths(&params);
        if renames.is_empty() {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        }
        self.register_read(id.clone(), None);
        self.dispatch_read(ReadJob::WillRenameFiles {
            id,
            renames,
            out: self.out_tx.clone(),
        });
    }

    /// `workspace/symbol`: fuzzy name search over the workspace's top-level
    /// definitions. A read-only job dispatched to the lint thread like
    /// definition/references; the query runs on the read pool against a db
    /// snapshot. Unlike the position-based requests it isn't tied to an open
    /// buffer, so it does no document lookup. See [`workspace_symbols_via_db`].
    fn on_workspace_symbol(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) = req.extract::<WorkspaceSymbolParams>(WorkspaceSymbolRequest::METHOD)
        else {
            self.respond_err(id, "invalid workspaceSymbol params");
            return;
        };
        self.register_read(id.clone(), None);
        self.dispatch_read(ReadJob::WorkspaceSymbol {
            id,
            query: params.query,
            out: self.out_tx.clone(),
        });
    }

    /// `textDocument/prepareCallHierarchy`: resolve the cursor to the top-level
    /// function(s) it names, returning items the client round-trips back to
    /// incoming/outgoing. A read-only job dispatched like definition; the live
    /// buffer is parsed on the read pool. See [`prepare_call_hierarchy_via_db`].
    fn on_prepare_call_hierarchy(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) =
            req.extract::<CallHierarchyPrepareParams>(CallHierarchyPrepare::METHOD)
        else {
            self.respond_err(id, "invalid prepareCallHierarchy params");
            return;
        };
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some((buffer, version)) = self.r_doc_snapshot(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        self.register_read(id.clone(), Some((uri.clone(), version)));
        self.dispatch_read(ReadJob::PrepareCallHierarchy {
            id,
            path,
            uri,
            buffer,
            position,
            out: self.out_tx.clone(),
        });
    }

    /// `callHierarchy/incomingCalls`: the top-level functions that call the item's
    /// function. The item carries its own identity (`uri` + `name`), so unlike the
    /// position-based requests this does no document lookup and works off the db
    /// snapshot, like `workspace/symbol`. See [`incoming_calls_via_db`].
    fn on_incoming_calls(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) =
            req.extract::<CallHierarchyIncomingCallsParams>(CallHierarchyIncomingCalls::METHOD)
        else {
            self.respond_err(id, "invalid incomingCalls params");
            return;
        };
        self.register_read(id.clone(), None);
        self.dispatch_read(ReadJob::IncomingCalls {
            id,
            item: Box::new(params.item),
            out: self.out_tx.clone(),
        });
    }

    /// `callHierarchy/outgoingCalls`: the top-level functions the item's function
    /// calls. Like incoming, served off the db snapshot from the item's identity.
    /// See [`outgoing_calls_via_db`].
    fn on_outgoing_calls(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) =
            req.extract::<CallHierarchyOutgoingCallsParams>(CallHierarchyOutgoingCalls::METHOD)
        else {
            self.respond_err(id, "invalid outgoingCalls params");
            return;
        };
        self.register_read(id.clone(), None);
        self.dispatch_read(ReadJob::OutgoingCalls {
            id,
            item: Box::new(params.item),
            out: self.out_tx.clone(),
        });
    }

    /// `textDocument/prepareTypeHierarchy`: resolve the cursor to the class(es)
    /// it names, returning items the client round-trips back to
    /// supertypes/subtypes. Dispatched like prepare-call-hierarchy; the live
    /// buffer is parsed on the read pool. See [`prepare_type_hierarchy_via_db`].
    fn on_prepare_type_hierarchy(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) =
            req.extract::<TypeHierarchyPrepareParams>(TypeHierarchyPrepare::METHOD)
        else {
            self.respond_err(id, "invalid prepareTypeHierarchy params");
            return;
        };
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some((buffer, version)) = self.r_doc_snapshot(&uri) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        self.register_read(id.clone(), Some((uri.clone(), version)));
        self.dispatch_read(ReadJob::PrepareTypeHierarchy {
            id,
            path,
            uri,
            buffer,
            position,
            out: self.out_tx.clone(),
        });
    }

    /// `typeHierarchy/supertypes`: the declared parent classes of the item's
    /// class. The item carries its own identity (`name`), so this does no
    /// document lookup and works off the db snapshot. See [`supertypes_via_db`].
    fn on_supertypes(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) =
            req.extract::<TypeHierarchySupertypesParams>(TypeHierarchySupertypes::METHOD)
        else {
            self.respond_err(id, "invalid supertypes params");
            return;
        };
        self.register_read(id.clone(), None);
        self.dispatch_read(ReadJob::Supertypes {
            id,
            item: Box::new(params.item),
            out: self.out_tx.clone(),
        });
    }

    /// `typeHierarchy/subtypes`: the classes that declare the item's class a
    /// supertype. Like supertypes, served off the db snapshot from the item's
    /// identity. See [`subtypes_via_db`].
    fn on_subtypes(&mut self, req: Request) {
        let id = req.id.clone();
        let Ok((_, params)) =
            req.extract::<TypeHierarchySubtypesParams>(TypeHierarchySubtypes::METHOD)
        else {
            self.respond_err(id, "invalid subtypes params");
            return;
        };
        self.register_read(id.clone(), None);
        self.dispatch_read(ReadJob::Subtypes {
            id,
            item: Box::new(params.item),
            out: self.out_tx.clone(),
        });
    }

    /// Hand a read-only job to the lint thread (db owner), which snapshots the db
    /// and runs it on the read pool. If that channel is gone (shutdown in flight),
    /// reply `null` so the client isn't left waiting.
    fn dispatch_read(&self, job: ReadJob) {
        if let Err(crossbeam_channel::SendError(job)) = self.read_tx.send(job) {
            let (id, out) = match job {
                ReadJob::Format { id, out, .. } => (id, out),
                ReadJob::FormatRange { id, out, .. } => (id, out),
                ReadJob::Hover { id, out, .. } => (id, out),
                ReadJob::Completion { id, out, .. } => (id, out),
                ReadJob::SignatureHelp { id, out, .. } => (id, out),
                ReadJob::ResolveCompletion { id, out, .. } => (id, out),
                ReadJob::Definition { id, out, .. } => (id, out),
                ReadJob::References { id, out, .. } => (id, out),
                ReadJob::Rename { id, out, .. } => (id, out),
                ReadJob::WillRenameFiles { id, out, .. } => (id, out),
                ReadJob::WorkspaceSymbol { id, out, .. } => (id, out),
                ReadJob::PrepareCallHierarchy { id, out, .. } => (id, out),
                ReadJob::IncomingCalls { id, out, .. } => (id, out),
                ReadJob::OutgoingCalls { id, out, .. } => (id, out),
                ReadJob::PrepareTypeHierarchy { id, out, .. } => (id, out),
                ReadJob::Supertypes { id, out, .. } => (id, out),
                ReadJob::Subtypes { id, out, .. } => (id, out),
            };
            let _ = out.send(Outbound::ReadReply(Response::new_ok(
                id,
                serde_json::Value::Null,
            )));
        }
    }

    pub(crate) fn on_notification(&mut self, not: Notification) {
        match not.method.as_str() {
            DidOpenTextDocument::METHOD => {
                if let Ok(params) =
                    not.extract::<DidOpenTextDocumentParams>(DidOpenTextDocument::METHOD)
                {
                    let uri = params.text_document.uri;
                    let kind =
                        DocumentKind::from_uri(&uri, Some(&params.text_document.language_id));
                    self.documents.insert(
                        uri.clone(),
                        Document::new(
                            params.text_document.text,
                            params.text_document.version,
                            kind,
                        ),
                    );
                    self.send_lint(uri, Vec::new());
                }
            }
            DidChangeTextDocument::METHOD => {
                if let Ok(params) =
                    not.extract::<DidChangeTextDocumentParams>(DidChangeTextDocument::METHOD)
                {
                    let uri = params.text_document.uri;
                    let version = params.text_document.version;
                    let encoding = self.position_encoding;
                    // The client sends incremental (ranged) changes now that we
                    // advertise `TextDocumentSyncKind::INCREMENTAL`; apply them to
                    // the stored buffer in order. A `range: None` change is a
                    // full-document replacement (still valid) and reseeds the text.
                    //
                    // Alongside the splice, record each ranged change as a byte
                    // `Edit` (Stage B): the precise sequence transforming the prior
                    // buffer into the new one, threaded to `parsed_document` for a
                    // multi-edit reparse. A full replacement can't be expressed as a
                    // tight edit against the last-parsed base, so it clears the
                    // sequence — `edits` empty means "fall back to `diff_edit`".
                    let mut applied = false;
                    let mut edits: Vec<Edit> = Vec::new();
                    let mut precise = true;
                    for change in params.content_changes {
                        match change.range {
                            None => {
                                // A full replacement reseeds the text, never the
                                // grammar: the file the client has open is the
                                // same file it opened.
                                let kind = self
                                    .documents
                                    .get(&uri)
                                    .map_or(DocumentKind::R, |doc| doc.kind);
                                self.documents
                                    .insert(uri.clone(), Document::new(change.text, version, kind));
                                applied = true;
                                precise = false;
                                edits.clear();
                            }
                            Some(range) => {
                                // A ranged change needs an existing buffer to
                                // splice into; without one (a change before open)
                                // there is nothing to edit, so skip it.
                                if let Some(doc) = self.documents.get_mut(&uri) {
                                    // Each range is against the text its
                                    // predecessors in this batch left, and the
                                    // buffer's index is patched in step with
                                    // them — so resolve against it directly
                                    // rather than re-scanning per change.
                                    let index = doc.buffer.line_index();
                                    let start = index.position_to_byte(range.start, encoding);
                                    let end = index.position_to_byte(range.end, encoding);
                                    let insert = change.text;
                                    // Resolve and record the byte range *before*
                                    // editing: the rename anchor replays these
                                    // against the pre-edit buffer.
                                    doc.apply_edit(start..end, &insert);
                                    applied = true;
                                    if precise {
                                        edits.push(Edit {
                                            range: start..end,
                                            insert,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    if applied && let Some(doc) = self.documents.get_mut(&uri) {
                        doc.version = version;
                    }
                    if applied {
                        // Thread this batch into any in-flight rename anchor so a
                        // later `rename` re-anchors precisely across these edits
                        // (see `rename_cursor_offset`). `precise` is false for a
                        // full-document replacement, which invalidates the slice.
                        if let Some(anchor) = self.rename_anchors.get_mut(&uri) {
                            anchor.record_edits(&edits, precise);
                        }
                        self.send_lint(uri, edits);
                    }
                }
            }
            DidCloseTextDocument::METHOD => {
                if let Ok(params) =
                    not.extract::<DidCloseTextDocumentParams>(DidCloseTextDocument::METHOD)
                {
                    let uri = params.text_document.uri;
                    let closed = self.documents.remove(&uri);
                    self.findings.remove(&uri);
                    self.report_ids.remove(&uri);
                    self.rename_anchors.remove(&uri);
                    // A closed `DESCRIPTION` buffer stops being authoritative:
                    // put the on-disk facts back, or an unsaved dependency list
                    // outlives the editor session and keeps gating every R
                    // diagnostic in the package. This is exactly what a watched
                    // on-disk edit already does, so reuse that path rather than
                    // inventing a second message for it.
                    if let Some(doc) = &closed
                        && doc.kind == DocumentKind::Description
                        && let Some(path) = uri::to_path(&uri)
                    {
                        let _ = self.lint_tx.send(LintMsg::WatchedFiles {
                            batch: WatchedFilesBatch {
                                meta_changed: vec![(path, WatchedKind::Description)],
                                ..Default::default()
                            },
                        });
                    }
                    // Resolve any parked pulls with an empty report so they don't
                    // hang now that the buffer is gone.
                    for PendingPull { id, .. } in self.pending_pull.remove(&uri).unwrap_or_default()
                    {
                        self.respond_diagnostic(id, DiagnosticReport::Full(Vec::new(), None));
                    }
                    if !self.pull_mode {
                        // Tell the client to clear stale diagnostics.
                        self.publish(uri, Vec::new(), None);
                    }
                }
            }
            DidRenameFiles::METHOD => {
                if let Ok(params) = not.extract::<RenameFilesParams>(DidRenameFiles::METHOD) {
                    let renames = file_renames_to_paths(&params);
                    if !renames.is_empty() {
                        let _ = self.lint_tx.send(LintMsg::RenameFiles { renames });
                    }
                }
            }
            DidChangeWatchedFiles::METHOD => {
                if let Ok(params) =
                    not.extract::<DidChangeWatchedFilesParams>(DidChangeWatchedFiles::METHOD)
                {
                    self.on_watched_files_changed(params);
                }
            }
            DidChangeWorkspaceFolders::METHOD => {
                if let Ok(params) = not
                    .extract::<DidChangeWorkspaceFoldersParams>(DidChangeWorkspaceFolders::METHOD)
                {
                    self.on_workspace_folders_changed(params);
                }
            }
            Cancel::METHOD => {
                if let Ok(params) = not.extract::<CancelParams>(Cancel::METHOD) {
                    let id = match params.id {
                        NumberOrString::Number(n) => RequestId::from(n),
                        NumberOrString::String(s) => RequestId::from(s),
                    };
                    // Only in-flight reads are tracked. An absent id was already
                    // answered (or was synchronous), so canceling it is a no-op —
                    // and must stay silent, or we'd double-respond.
                    if self.live_reads.remove(&id).is_some() {
                        self.respond_cancelled(id);
                    }
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

    pub(crate) fn on_outbound(&mut self, ob: Outbound) {
        match ob {
            Outbound::Diagnostics {
                uri,
                version,
                diags,
                findings,
            } => {
                // Stale results (a newer edit superseded this lint) are dropped:
                // the newer version's lint will produce its own `Outbound`.
                if !matches!(self.documents.get(&uri), Some(d) if d.version == version) {
                    return;
                }
                // Cache findings (code actions still need them) and derive the
                // report id from their content, so an unchanged file keeps its id.
                let result_id = content_result_id(&findings);
                self.findings.insert(uri.clone(), (version, findings));
                self.report_ids.insert(uri.clone(), result_id.clone());

                // Answer any parked pulls for this URI; otherwise deliver via the
                // active channel (pull clients re-pull on their own cadence). A
                // parked pull whose `previous_result_id` still matches gets
                // `Unchanged` (the cross-file re-lint case).
                let pending = self.pending_pull.remove(&uri).unwrap_or_default();
                for PendingPull {
                    id,
                    previous_result_id,
                } in pending
                {
                    let report = match report_kind(previous_result_id.as_deref(), Some(&result_id))
                    {
                        DiagnosticReportKind::Unchanged => {
                            DiagnosticReport::Unchanged(result_id.clone())
                        }
                        DiagnosticReportKind::Full => {
                            DiagnosticReport::Full(diags.clone(), Some(result_id.clone()))
                        }
                    };
                    self.respond_diagnostic(id, report);
                }
                if !self.pull_mode {
                    self.publish(uri, diags, Some(version));
                }
            }
            Outbound::ReadReply(response) => self.on_read_reply(response),
            Outbound::RelintAll => self.request_relint_all(),
            Outbound::Progress { token, work } => self.on_progress(token, work),
        }
    }

    /// Forward a background job's work-done progress to the client as `$/progress`,
    /// gated on the client capability. On the first update (`Begin`) the token is
    /// created via a fire-and-forget `window/workDoneProgress/create` request
    /// (like [`send_workspace_refresh`](Self::send_workspace_refresh); the client's
    /// response is ignored by the main loop). Mirrors [`publish`](Self::publish)
    /// for the notification itself. A no-op when the client didn't advertise
    /// `window.workDoneProgress`.
    fn on_progress(&mut self, token: String, work: WorkDoneProgress) {
        if !self.work_done_progress {
            return;
        }
        if matches!(work, WorkDoneProgress::Begin(_)) {
            self.next_req_id += 1;
            let req = Request::new(
                RequestId::from(self.next_req_id),
                WorkDoneProgressCreate::METHOD.to_string(),
                WorkDoneProgressCreateParams {
                    token: ProgressToken::String(token.clone()),
                },
            );
            let _ = self.sender.send(Message::Request(req));
        }
        let params = ProgressParams {
            token: ProgressToken::String(token),
            value: ProgressParamsValue::WorkDone(work),
        };
        let not = Notification::new(Progress::METHOD.to_string(), params);
        let _ = self.sender.send(Message::Notification(not));
    }

    /// Re-lint every open document because cross-file context changed without a
    /// document edit (a fresh index, a sibling, a config or metadata change on
    /// disk). Pull clients are asked to re-request (after invalidating their cached
    /// reports); push clients get a fresh lint per buffer.
    fn request_relint_all(&mut self) {
        let uris: Vec<Uri> = self.documents.keys().cloned().collect();
        if self.pull_mode {
            for uri in &uris {
                self.findings.remove(uri);
                self.report_ids.remove(uri);
            }
            self.send_workspace_refresh();
        } else {
            for uri in uris {
                self.send_lint(uri, Vec::new());
            }
        }
    }

    /// Handle `workspace/didChangeWatchedFiles`: an on-disk change to a config,
    /// package-metadata, or `.R` file outside the editor. An `arity.toml` edit is
    /// resolved here (drop the config cache, re-lint); the rest is db work, routed
    /// to the lint thread (the sole writer). See [`classify_watched_files`].
    fn on_watched_files_changed(&mut self, params: DidChangeWatchedFilesParams) {
        let WatchedClassification {
            batch,
            config_changed,
        } = classify_watched_files(&params, |uri| self.documents.contains_key(uri));
        if config_changed {
            // A committed `arity.toml` moved; drop cached resolutions so the next
            // lint/format re-reads it, then re-lint every open document.
            self.config_cache.clear();
            self.request_relint_all();
        }
        if !batch.is_empty() {
            let _ = self.lint_tx.send(LintMsg::WatchedFiles { batch });
        }
    }

    /// Handle `workspace/didChangeWorkspaceFolders`: seed newly-added folders as
    /// workspace members (the seed unions with the existing set). Removed folders
    /// are left in place for now — dropping their members is a follow-up.
    fn on_workspace_folders_changed(&mut self, params: DidChangeWorkspaceFoldersParams) {
        let added: Vec<PathBuf> = params
            .event
            .added
            .iter()
            .filter_map(|folder| uri::to_path(&folder.uri))
            .collect();
        if !added.is_empty() {
            let _ = self.lint_tx.send(LintMsg::SeedWorkspace { roots: added });
        }
    }

    /// Register on-disk file watchers with the client via dynamic
    /// `client/registerCapability`. Called once at startup when the client supports
    /// dynamic registration for `workspace/didChangeWatchedFiles`; the client's
    /// response is ignored by the main loop. Watches R sources (which drive
    /// membership) plus the config and package-metadata files that shape cross-file
    /// analysis (see [`WATCHED_GLOBS`]).
    pub(crate) fn register_file_watchers(&mut self) {
        let watchers = WATCHED_GLOBS
            .iter()
            .map(|glob| FileSystemWatcher {
                glob_pattern: GlobPattern::String((*glob).to_string()),
                kind: None, // default: create | change | delete
            })
            .collect();
        let registration = Registration {
            id: "arity-watched-files".to_string(),
            method: DidChangeWatchedFiles::METHOD.to_string(),
            register_options: serde_json::to_value(DidChangeWatchedFilesRegistrationOptions {
                watchers,
            })
            .ok(),
        };
        self.next_req_id += 1;
        let req = Request::new(
            RequestId::from(self.next_req_id),
            RegisterCapability::METHOD.to_string(),
            RegistrationParams {
                registrations: vec![registration],
            },
        );
        let _ = self.sender.send(Message::Request(req));
    }

    /// Send a lint request for `uri`'s current buffer to the lint thread. `edits`
    /// are the precise per-change edits transforming the previously sent buffer
    /// into the current one (Stage B); pass an empty vec when no precise sequence
    /// is available (a first send, a full replacement, or a non-edit trigger),
    /// which makes `parsed_document` fall back to the whole-text `diff_edit`.
    fn send_lint(&mut self, uri: Uri, edits: Vec<Edit>) {
        let Some(doc) = self.documents.get(&uri) else {
            return;
        };
        let buffer = Arc::clone(&doc.buffer);
        let version = doc.version;
        let kind = doc.kind;
        let path =
            uri::to_path(&uri).unwrap_or_else(|| PathBuf::from(kind.placeholder_file_name()));
        let (lint_config, index_config) = match self.resolve_settings(&uri) {
            Ok(s) => (s.lint, s.index),
            Err(_) => (LintConfig::default(), IndexConfig::default()),
        };
        let _ = self.lint_tx.send(LintMsg::Request(Box::new(LintRequest {
            uri,
            path,
            buffer,
            edits,
            version,
            kind,
            lint_config,
            index_config,
        })));
    }

    /// The live buffer and version for `uri`, **only if it is an R document**.
    /// Reads clone the `Arc`, never the text.
    ///
    /// Every R-grammar request goes through this, and there is deliberately no
    /// un-annotated way to get a buffer: a `DESCRIPTION` answers `None` here, so
    /// each handler's existing "not open" arm already declines correctly, and a
    /// handler added later cannot silently inherit the wrong grammar. Use
    /// [`doc_snapshot_any`](Self::doc_snapshot_any) for the few requests that
    /// serve both.
    fn r_doc_snapshot(&self, uri: &Uri) -> Option<(Arc<TextBuffer>, i32)> {
        let doc = self.documents.get(uri)?;
        match doc.kind {
            DocumentKind::R => Some((Arc::clone(&doc.buffer), doc.version)),
            DocumentKind::Description => None,
        }
    }

    /// The live buffer, version, and grammar for `uri` — for the requests that
    /// serve both languages and branch on the kind themselves.
    fn doc_snapshot_any(&self, uri: &Uri) -> Option<(Arc<TextBuffer>, i32, DocumentKind)> {
        self.documents
            .get(uri)
            .map(|d| (Arc::clone(&d.buffer), d.version, d.kind))
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
        let style = resolve_format_style(&config, source.is_some(), &self.editor_settings);
        let mut index = config.index;
        // Network egress is a per-user/per-machine consent decision, so the sidecar
        // URL comes from the environment, never the shared, committed arity.toml.
        // Absent or empty → no fetching (arity stays offline).
        index.remote_url = std::env::var("ARITY_REMOTE_URL")
            .ok()
            .filter(|s| !s.is_empty());
        let resolved = ResolvedSettings {
            style,
            lint: config.lint,
            index,
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

    /// Respond to a `textDocument/diagnostic` request with a decided report.
    fn respond_diagnostic(&self, id: RequestId, report: DiagnosticReport) {
        let result: DocumentDiagnosticReportResult = match report {
            DiagnosticReport::Full(items, result_id) => {
                DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                    related_documents: None,
                    full_document_diagnostic_report: FullDocumentDiagnosticReport {
                        result_id,
                        items,
                    },
                })
                .into()
            }
            DiagnosticReport::Unchanged(result_id) => {
                DocumentDiagnosticReport::Unchanged(RelatedUnchangedDocumentDiagnosticReport {
                    related_documents: None,
                    unchanged_document_diagnostic_report: UnchangedDocumentDiagnosticReport {
                        result_id,
                    },
                })
                .into()
            }
        };
        match serde_json::to_value(result) {
            Ok(value) => self.respond_ok(id, value),
            Err(_) => self.respond_err(id, "failed to serialize diagnostic report"),
        }
    }

    /// Ask pull clients to re-request diagnostics (server→client request). Sent
    /// when cross-file context changed without a document edit (a fresh index or
    /// sibling); the client's response is ignored by the main loop.
    fn send_workspace_refresh(&mut self) {
        self.next_req_id += 1;
        let req = Request::new(
            RequestId::from(self.next_req_id),
            WorkspaceDiagnosticRefresh::METHOD.to_string(),
            serde_json::Value::Null,
        );
        let _ = self.sender.send(Message::Request(req));
    }

    /// Deliver a finished read-pool reply, gating it on the same single thread
    /// that owns the live-request set and buffer versions. A reply whose id is no
    /// longer live was canceled (its `RequestCancelled` already went out) and is
    /// dropped. A reply whose document advanced past the version it read is stale,
    /// so we answer `ContentModified` instead and let the client re-request.
    fn on_read_reply(&mut self, response: Response) {
        let Some(inflight) = self.live_reads.remove(&response.id) else {
            // Already answered — canceled, or a duplicate; drop it silently.
            return;
        };
        if let Some((uri, version)) = inflight.doc
            && !matches!(self.documents.get(&uri), Some(d) if d.version == version)
        {
            self.respond_content_modified(response.id);
            return;
        }
        let _ = self.sender.send(Message::Response(response));
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

    /// Answer a canceled request with `RequestCancelled` (-32800).
    fn respond_cancelled(&self, id: RequestId) {
        let resp = Response::new_err(id, REQUEST_CANCELLED, "request cancelled".to_string());
        let _ = self.sender.send(Message::Response(resp));
    }

    /// Answer a superseded read with `ContentModified` (-32801) so the client
    /// re-requests against the current buffer.
    fn respond_content_modified(&self, id: RequestId) {
        let resp = Response::new_err(id, CONTENT_MODIFIED, "content modified".to_string());
        let _ = self.sender.send(Message::Response(resp));
    }
}

/// A minimal [`Diagnostic`] for tests in this module.
#[cfg(test)]
fn sample_diagnostic(rule: &'static str, start: u32, end: u32) -> Diagnostic {
    Diagnostic {
        rule,
        severity: Severity::Warning,
        path: std::path::PathBuf::from("f.R"),
        range: TextRange::new(TextSize::from(start), TextSize::from(end)),
        message: crate::linter::ViolationData::new(rule, "body"),
        fix: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_result_id_stable_and_distinct() {
        let a = vec![sample_diagnostic("rule-a", 0, 4)];
        let a_again = vec![sample_diagnostic("rule-a", 0, 4)];
        // Identical findings hash to an identical id (so `Unchanged` can fire).
        assert_eq!(content_result_id(&a), content_result_id(&a_again));

        // A different rule, range, or count changes the id.
        assert_ne!(
            content_result_id(&a),
            content_result_id(&[sample_diagnostic("rule-b", 0, 4)])
        );
        assert_ne!(
            content_result_id(&a),
            content_result_id(&[sample_diagnostic("rule-a", 1, 4)])
        );
        assert_ne!(content_result_id(&a), content_result_id(&[]));
    }

    #[test]
    fn report_kind_unchanged_only_when_ids_match() {
        // Both present and equal → the client's report is still accurate.
        assert_eq!(
            report_kind(Some("3"), Some("3")),
            DiagnosticReportKind::Unchanged
        );
        // Different ids → a fresh full report.
        assert_eq!(
            report_kind(Some("2"), Some("3")),
            DiagnosticReportKind::Full
        );
        // No previousResultId (first pull) → full, even if we have a current id.
        assert_eq!(report_kind(None, Some("3")), DiagnosticReportKind::Full);
        // No current id (nothing cached yet) → full, never unchanged.
        assert_eq!(report_kind(Some("3"), None), DiagnosticReportKind::Full);
        assert_eq!(report_kind(None, None), DiagnosticReportKind::Full);
    }
}

/// Deterministic, timing-free coverage of the request-cancellation and
/// stale-read gate. The protocol tests (`tests/lsp_protocol.rs`) drive the same
/// behavior over the real loop but depend on message ordering; here we call the
/// state machine directly, so the outcomes are exact.
#[cfg(test)]
mod cancellation_gate {
    use super::*;
    use crossbeam_channel::Receiver;
    use std::time::Duration;

    /// Holds the channel read-ends and the read pool alive for a test's
    /// lifetime, so senders inside [`GlobalState`] never see a closed channel.
    struct Rig {
        client_rx: Receiver<Message>,
        out_rx: Receiver<Outbound>,
        lint_rx: Receiver<LintMsg>,
        read_rx: Receiver<ReadJob>,
        _pool: TaskPool,
    }

    impl Rig {
        /// The next message queued for the lint thread, or `None` on a short poll.
        fn try_lint_msg(&self) -> Option<LintMsg> {
            self.lint_rx.recv_timeout(Duration::from_millis(200)).ok()
        }

        /// Whether the server queued no read work of either shape. Most handlers
        /// dispatch a [`ReadJob`]; `on_code_action` instead spawns on the read
        /// pool directly and answers over `out_tx`, so both count. A guarded
        /// handler must decline *before* spending a read slot, not after.
        fn no_read_work(&self) -> bool {
            self.read_rx.try_recv().is_err()
                && self
                    .out_rx
                    .recv_timeout(Duration::from_millis(200))
                    .is_err()
        }

        /// The next message the server sent to the client, or `None` if it sent
        /// nothing (a short poll — the loop is synchronous in these tests).
        fn try_response(&self) -> Option<Response> {
            match self.try_message() {
                Some(Message::Response(r)) => Some(r),
                Some(other) => panic!("expected a response, got {other:?}"),
                None => None,
            }
        }

        /// The next raw message the server sent to the client, or `None` on a
        /// short poll (the loop is synchronous in these tests).
        fn try_message(&self) -> Option<Message> {
            self.client_rx.recv_timeout(Duration::from_millis(200)).ok()
        }
    }

    fn test_state() -> (GlobalState, Rig) {
        test_state_full(false, false)
    }

    fn test_state_with(work_done_progress: bool) -> (GlobalState, Rig) {
        test_state_full(false, work_done_progress)
    }

    fn test_state_pull() -> (GlobalState, Rig) {
        test_state_full(true, false)
    }

    fn test_state_full(pull_mode: bool, work_done_progress: bool) -> (GlobalState, Rig) {
        let (sender, client_rx) = crossbeam_channel::unbounded::<Message>();
        let (out_tx, out_rx) = crossbeam_channel::unbounded::<Outbound>();
        let (lint_tx, lint_rx) = crossbeam_channel::unbounded::<LintMsg>();
        let (read_tx, read_rx) = crossbeam_channel::unbounded::<ReadJob>();
        let pool = TaskPool::new("test-read", 1);
        let state = GlobalState::new(
            sender,
            out_tx,
            lint_tx,
            read_tx,
            pool.spawner(),
            EditorSettings::default(),
            pull_mode,
            work_done_progress,
            PositionEncoding::Utf16,
        );
        let rig = Rig {
            client_rx,
            out_rx,
            lint_rx,
            read_rx,
            _pool: pool,
        };
        (state, rig)
    }

    fn progress_begin() -> WorkDoneProgress {
        WorkDoneProgress::Begin(WorkDoneProgressBegin {
            title: "Indexing R packages".to_string(),
            cancellable: Some(false),
            message: None,
            percentage: None,
        })
    }

    fn cancel(id: i32) -> Notification {
        Notification::new(
            Cancel::METHOD.to_string(),
            CancelParams {
                id: NumberOrString::Number(id),
            },
        )
    }

    fn doc_uri() -> Uri {
        uri::from_path(Path::new(if cfg!(windows) {
            r"C:\tmp\t.R"
        } else {
            "/tmp/t.R"
        }))
        .expect("valid file uri")
    }

    #[test]
    fn cancel_short_circuits_live_read_with_request_cancelled() {
        let (mut state, rig) = test_state();
        let id = RequestId::from(7);
        state.register_read(id.clone(), None);

        state.on_notification(cancel(7));

        assert!(
            !state.live_reads.contains_key(&id),
            "canceling clears the live entry"
        );
        let resp = rig.try_response().expect("cancel sends a response");
        assert_eq!(resp.id, id);
        assert_eq!(
            resp.response_result.expect_err("cancel errors").code,
            REQUEST_CANCELLED
        );
    }

    #[test]
    fn cancel_of_untracked_id_is_a_silent_noop() {
        let (mut state, rig) = test_state();
        // Never registered: an id that already completed, or was answered
        // synchronously. Canceling it must not emit a (double) response.
        state.on_notification(cancel(99));
        assert!(
            rig.try_response().is_none(),
            "no response for an unknown id"
        );
    }

    #[test]
    fn reply_superseded_by_a_newer_edit_becomes_content_modified() {
        let (mut state, rig) = test_state();
        let uri = doc_uri();
        let id = RequestId::from(1);
        // The read was dispatched against version 1...
        state.register_read(id.clone(), Some((uri.clone(), 1)));
        // ...but the buffer has since advanced to version 2.
        state.documents.insert(
            uri,
            Document::new("y <- 2\n".to_string(), 2, DocumentKind::R),
        );

        state.on_read_reply(Response::new_ok(id.clone(), serde_json::Value::Null));

        let resp = rig.try_response().expect("a gated reply is still answered");
        assert_eq!(resp.id, id);
        assert_eq!(
            resp.response_result.expect_err("stale errors").code,
            CONTENT_MODIFIED
        );
        assert!(!state.live_reads.contains_key(&id));
    }

    #[test]
    fn fresh_reply_is_delivered_unchanged() {
        let (mut state, rig) = test_state();
        let uri = doc_uri();
        let id = RequestId::from(1);
        state.register_read(id.clone(), Some((uri.clone(), 1)));
        state.documents.insert(
            uri,
            Document::new("x <- 1\n".to_string(), 1, DocumentKind::R),
        );

        state.on_read_reply(Response::new_ok(id.clone(), serde_json::json!("ok")));

        let resp = rig.try_response().expect("fresh reply delivered");
        assert_eq!(resp.id, id);
        assert_eq!(
            resp.response_result.expect("fresh reply is ok"),
            serde_json::json!("ok")
        );
        assert!(!state.live_reads.contains_key(&id));
    }

    #[test]
    fn a_reply_arriving_after_cancel_is_dropped() {
        let (mut state, rig) = test_state();
        let id = RequestId::from(3);
        state.register_read(id.clone(), None);

        // Cancel wins the race: it removes the entry and answers RequestCancelled.
        state.on_notification(cancel(3));
        let cancelled = rig.try_response().expect("cancel response");
        assert_eq!(
            cancelled.response_result.expect_err("cancel errors").code,
            REQUEST_CANCELLED
        );

        // The read's own reply then lands late — it must be dropped, not sent as a
        // second (protocol-illegal) response to the same id.
        state.on_read_reply(Response::new_ok(id, serde_json::Value::Null));
        assert!(
            rig.try_response().is_none(),
            "the late reply must not double-respond"
        );
    }

    #[test]
    fn progress_begin_creates_token_then_notifies() {
        let (mut state, rig) = test_state_with(true);
        state.on_outbound(Outbound::Progress {
            token: "arity/progress/0".to_string(),
            work: progress_begin(),
        });
        // First the server→client create request, then the `$/progress` begin.
        let Some(Message::Request(req)) = rig.try_message() else {
            panic!("expected a workDoneProgress/create request first");
        };
        assert_eq!(req.method, WorkDoneProgressCreate::METHOD);
        let Some(Message::Notification(not)) = rig.try_message() else {
            panic!("expected a $/progress notification after create");
        };
        assert_eq!(not.method, Progress::METHOD);
        assert!(rig.try_message().is_none());
    }

    #[test]
    fn progress_report_notifies_without_create() {
        let (mut state, rig) = test_state_with(true);
        state.on_outbound(Outbound::Progress {
            token: "arity/progress/0".to_string(),
            work: WorkDoneProgress::Report(WorkDoneProgressReport {
                cancellable: Some(false),
                message: Some("magrittr".to_string()),
                percentage: Some(50),
            }),
        });
        // A non-Begin update is a bare notification — no create request.
        let Some(Message::Notification(not)) = rig.try_message() else {
            panic!("expected a $/progress notification");
        };
        assert_eq!(not.method, Progress::METHOD);
        assert!(rig.try_message().is_none());
    }

    #[test]
    fn progress_suppressed_without_client_capability() {
        // The client never advertised `window.workDoneProgress`, so no create
        // request and no `$/progress` may be sent.
        let (mut state, rig) = test_state_with(false);
        state.on_outbound(Outbound::Progress {
            token: "arity/progress/0".to_string(),
            work: progress_begin(),
        });
        assert!(
            rig.try_message().is_none(),
            "progress must be silent without the client capability"
        );
    }

    /// Drive a parked pull through `on_outbound` with the given findings and the
    /// client's `previous_result_id`, returning the report `Value` sent back.
    fn drive_parked_pull(
        pull_mode: bool,
        previous_result_id: Option<String>,
        findings: Vec<Diagnostic>,
    ) -> (serde_json::Value, String) {
        let (mut state, rig) = if pull_mode {
            test_state_pull()
        } else {
            test_state()
        };
        let uri = doc_uri();
        let version = 1;
        state.documents.insert(
            uri.clone(),
            Document::new("x <- 1\n".to_string(), version, DocumentKind::R),
        );
        let findings = Arc::new(findings);
        let result_id = content_result_id(&findings);
        let req_id = RequestId::from(42);
        state
            .pending_pull
            .entry(uri.clone())
            .or_default()
            .push(PendingPull {
                id: req_id.clone(),
                previous_result_id,
            });

        state.on_outbound(Outbound::Diagnostics {
            uri,
            version,
            diags: Vec::new(),
            findings,
        });

        let resp = rig.try_response().expect("parked pull is answered");
        assert_eq!(resp.id, req_id);
        (resp.response_result.expect("ok report"), result_id)
    }

    #[test]
    fn cold_path_returns_unchanged_when_findings_hash_matches() {
        // The client already holds the id for these exact findings (e.g. an
        // unrelated file re-linted by a cross-file `RelintAll`): answer Unchanged.
        let findings = vec![sample_diagnostic("rule-a", 0, 1)];
        let id = content_result_id(&findings);
        let (report, result_id) = drive_parked_pull(true, Some(id.clone()), findings);
        assert_eq!(report["kind"], "unchanged");
        assert_eq!(report["resultId"], result_id);
        assert_eq!(result_id, id);
    }

    #[test]
    fn cold_path_returns_full_when_previous_id_differs() {
        // The client's cached id is stale (findings actually changed): send Full.
        let findings = vec![sample_diagnostic("rule-a", 0, 1)];
        let (report, result_id) = drive_parked_pull(true, Some("stale".to_string()), findings);
        assert_eq!(report["kind"], "full");
        assert_eq!(report["resultId"], result_id);
        assert!(report["items"].is_array());
    }

    #[test]
    fn cold_path_returns_full_on_first_pull() {
        // No `previous_result_id` at all (a first pull): always Full.
        let findings = vec![sample_diagnostic("rule-a", 0, 1)];
        let (report, _) = drive_parked_pull(true, None, findings);
        assert_eq!(report["kind"], "full");
    }

    use lsp_types::{TextDocumentIdentifier, TextDocumentItem};
    use serde_json::json;

    fn description_uri() -> Uri {
        uri_named("DESCRIPTION")
    }

    fn did_open(uri: &Uri, text: &str, language_id: &str) -> Notification {
        Notification::new(
            DidOpenTextDocument::METHOD.to_string(),
            DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri.clone(),
                    language_id: language_id.to_string(),
                    version: 1,
                    text: text.to_string(),
                },
            },
        )
    }

    #[test]
    fn document_kind_prefers_the_path_over_the_language_id() {
        // A client that registered `DESCRIPTION` under the R language (as
        // `editors/code` already does for `NAMESPACE`) must not get DCF parsed
        // as R. The file name is the authority.
        assert_eq!(
            DocumentKind::from_uri(&description_uri(), Some("r")),
            DocumentKind::Description
        );
        // ...and the converse: a mislabeled R file is still R.
        assert_eq!(
            DocumentKind::from_uri(&test_uri(), Some("plaintext")),
            DocumentKind::R
        );
        // With no file name to go on, the language id is all there is.
        assert_eq!(
            DocumentKind::from_uri(&test_uri(), Some("r-description")),
            DocumentKind::Description
        );
    }

    #[test]
    fn a_description_lint_request_carries_its_grammar() {
        let (mut state, rig) = test_state();
        let uri = description_uri();
        state.on_notification(did_open(&uri, "Package: testpkg\n", "r"));

        match rig.try_lint_msg() {
            Some(LintMsg::Request(req)) => assert_eq!(req.kind, DocumentKind::Description),
            other => panic!("expected a lint request, got {:?}", other.is_some()),
        }
    }

    /// Every R-grammar request, asked of a `DESCRIPTION`.
    ///
    /// `textDocument/formatting` is the one that matters most: the R formatter
    /// happily rewrites `Package: testpkg` to `Package:testpkg`, so answering it
    /// here would corrupt the user's file. The rest range from useless to
    /// misleading. Each must decline *without* spending a read slot.
    #[test]
    fn r_only_requests_return_null_for_a_description() {
        let uri = description_uri();
        let position = json!({ "line": 0, "character": 3 });
        let doc = json!({ "uri": uri.as_str() });
        let cases: Vec<(&str, serde_json::Value)> = vec![
            (
                "textDocument/formatting",
                json!({ "textDocument": doc, "options": { "tabSize": 2, "insertSpaces": true } }),
            ),
            (
                "textDocument/rangeFormatting",
                json!({
                    "textDocument": doc,
                    "range": { "start": { "line": 0, "character": 0 }, "end": position },
                    "options": { "tabSize": 2, "insertSpaces": true }
                }),
            ),
            (
                "textDocument/codeAction",
                json!({
                    "textDocument": doc,
                    "range": { "start": { "line": 0, "character": 0 }, "end": position },
                    "context": { "diagnostics": [] }
                }),
            ),
            (
                "textDocument/signatureHelp",
                json!({ "textDocument": doc, "position": position }),
            ),
            (
                "textDocument/definition",
                json!({ "textDocument": doc, "position": position }),
            ),
            (
                "textDocument/references",
                json!({
                    "textDocument": doc, "position": position,
                    "context": { "includeDeclaration": true }
                }),
            ),
            (
                "textDocument/documentHighlight",
                json!({ "textDocument": doc, "position": position }),
            ),
            (
                "textDocument/documentSymbol",
                json!({ "textDocument": doc }),
            ),
            ("textDocument/foldingRange", json!({ "textDocument": doc })),
            (
                "textDocument/selectionRange",
                json!({ "textDocument": doc, "positions": [position] }),
            ),
            ("textDocument/documentLink", json!({ "textDocument": doc })),
            ("textDocument/documentColor", json!({ "textDocument": doc })),
            (
                "textDocument/semanticTokens/full",
                json!({ "textDocument": doc }),
            ),
            (
                "textDocument/prepareRename",
                json!({ "textDocument": doc, "position": position }),
            ),
            (
                "textDocument/rename",
                json!({ "textDocument": doc, "position": position, "newName": "z" }),
            ),
            (
                "textDocument/prepareCallHierarchy",
                json!({ "textDocument": doc, "position": position }),
            ),
            (
                "textDocument/prepareTypeHierarchy",
                json!({ "textDocument": doc, "position": position }),
            ),
        ];

        for (method, params) in cases {
            // The DESCRIPTION case: decline, and spend nothing doing it.
            let (mut state, rig) = test_state();
            state.on_notification(did_open(&uri, "Package: testpkg\nDepends: R\n", "r"));
            // Drain the lint request the open queued, so the read assertion below
            // is about this request alone.
            let _ = rig.try_lint_msg();

            state.on_request(Request::new(
                RequestId::from(1),
                method.to_string(),
                params.clone(),
            ));

            let resp = rig
                .try_response()
                .unwrap_or_else(|| panic!("{method} answered nothing"));
            assert_eq!(
                resp.response_result.clone().ok(),
                Some(serde_json::Value::Null),
                "{method} must answer null for a DESCRIPTION, got {resp:?}"
            );
            assert!(
                rig.no_read_work(),
                "{method} queued read work for a DESCRIPTION"
            );

            // The negative control, without which the assertions above would
            // pass just as happily for a misspelled method name: the same
            // request against an R buffer must reach a handler that does work.
            let (mut state, rig) = test_state();
            let r_uri = test_uri();
            let r_params = params_for(&params, &r_uri);
            state.on_notification(did_open(&r_uri, "x <- 1\ny <- x\n", "r"));
            let _ = rig.try_lint_msg();

            state.on_request(Request::new(
                RequestId::from(1),
                method.to_string(),
                r_params,
            ));

            assert!(
                !rig.no_read_work() || rig.try_response().is_some(),
                "{method} did nothing at all for an R buffer — is it routed?"
            );
        }
    }

    #[test]
    fn did_close_description_asks_the_lint_thread_to_refresh_from_disk() {
        let (mut state, rig) = test_state();
        let uri = description_uri();
        state.on_notification(did_open(&uri, "Package: testpkg\n", "r-description"));
        let _ = rig.try_lint_msg();

        state.on_notification(Notification::new(
            DidCloseTextDocument::METHOD.to_string(),
            DidCloseTextDocumentParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
            },
        ));

        match rig.try_lint_msg() {
            Some(LintMsg::WatchedFiles { batch }) => {
                let expected = uri::to_path(&uri).expect("a file uri");
                assert_eq!(
                    batch.meta_changed,
                    vec![(expected, WatchedKind::Description)]
                );
            }
            other => panic!(
                "expected a watched-files refresh, got {:?}",
                other.is_some()
            ),
        }
    }

    /// Closing an R buffer must *not* send the refresh: it has no
    /// `DESCRIPTION` input to restore, and a spurious batch would re-lint the
    /// world on every tab close.
    #[test]
    fn did_close_r_document_sends_no_refresh() {
        let (mut state, rig) = test_state();
        let uri = test_uri();
        state.on_notification(did_open(&uri, "x <- 1\n", "r"));
        let _ = rig.try_lint_msg();

        state.on_notification(Notification::new(
            DidCloseTextDocument::METHOD.to_string(),
            DidCloseTextDocumentParams {
                text_document: TextDocumentIdentifier { uri },
            },
        ));

        assert!(rig.try_lint_msg().is_none(), "no lint message expected");
    }

    /// Retarget a request's `textDocument.uri` at `uri`, leaving the rest alone.
    fn params_for(params: &serde_json::Value, uri: &Uri) -> serde_json::Value {
        let mut params = params.clone();
        params["textDocument"]["uri"] = json!(uri.as_str());
        params
    }
}
