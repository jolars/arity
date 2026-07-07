use super::*;

#[derive(Debug, Clone)]
pub(crate) struct Document {
    text: String,
    version: i32,
}

/// A `textDocument/diagnostic` request awaiting fresh findings: the buffer had no
/// current cached findings when the pull arrived, so the request is parked here
/// (keyed by URI) until the lint thread reports back. See
/// [`GlobalState::on_document_diagnostic`].
pub(crate) struct PendingPull {
    id: RequestId,
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
}

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
    /// Pull requests parked while the lint thread computes fresh findings, keyed
    /// by URI. Drained on the next `Outbound::Diagnostics` for that URI (or on
    /// close, with an empty report, so a request never hangs).
    pending_pull: HashMap<Uri, Vec<PendingPull>>,
    /// The opaque `resultId` of the report most recently delivered per URI. Bumped
    /// every time findings are cached (a *lint generation*, not the document
    /// version), so a cross-file change that leaves the version untouched still
    /// yields a fresh id — `unchanged` is only returned when this matches the
    /// client's `previousResultId`.
    report_ids: HashMap<Uri, String>,
    /// Monotonic source for [`report_ids`](Self::report_ids).
    result_seq: u64,
    /// Monotonic id source for server→client requests (`workspace/diagnostic/
    /// refresh`); the client's responses are ignored by the main loop.
    next_req_id: i32,
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
    pub(crate) fn new(
        sender: Sender<Message>,
        lint_tx: Sender<LintMsg>,
        read_tx: Sender<ReadJob>,
        read_spawner: Spawner,
        editor_settings: EditorSettings,
        pull_mode: bool,
    ) -> Self {
        Self {
            documents: HashMap::new(),
            findings: HashMap::new(),
            rename_anchors: HashMap::new(),
            pull_mode,
            pending_pull: HashMap::new(),
            report_ids: HashMap::new(),
            result_seq: 0,
            next_req_id: 0,
            config_cache: HashMap::new(),
            editor_settings,
            sender,
            lint_tx,
            read_tx,
            read_spawner,
        }
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
            DocumentLinkRequest::METHOD => self.on_document_link(req),
            SemanticTokensFullRequest::METHOD => self.on_semantic_tokens(req),
            PrepareRenameRequest::METHOD => self.on_prepare_rename(req),
            Rename::METHOD => self.on_rename(req),
            WillRenameFiles::METHOD => self.on_will_rename_files(req),
            WorkspaceSymbolRequest::METHOD => self.on_workspace_symbol(req),
            CallHierarchyPrepare::METHOD => self.on_prepare_call_hierarchy(req),
            CallHierarchyIncomingCalls::METHOD => self.on_incoming_calls(req),
            CallHierarchyOutgoingCalls::METHOD => self.on_outgoing_calls(req),
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
        let Some((text, version)) = self
            .documents
            .get(&uri)
            .map(|d| (d.text.clone(), d.version))
        else {
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
                        let items = findings_to_items(findings, &text);
                        DiagnosticReport::Full(items, result_id)
                    }
                };
            self.respond_diagnostic(id, report);
            return;
        }

        // Cold path: no current findings yet (a pull before the lint caught up).
        // Park the request and lint; `on_outbound` answers it with fresh results.
        self.pending_pull
            .entry(uri.clone())
            .or_default()
            .push(PendingPull { id });
        self.send_lint(uri);
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
        let Some(text) = self.documents.get(&uri).map(|d| d.text.clone()) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        self.dispatch_read(ReadJob::SignatureHelp {
            id,
            path,
            text,
            position,
            sender: self.sender.clone(),
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
        let Some(text) = self.documents.get(&uri).map(|d| d.text.clone()) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        self.dispatch_read(ReadJob::Completion {
            id,
            path,
            text,
            position,
            sender: self.sender.clone(),
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
        self.dispatch_read(ReadJob::ResolveCompletion {
            id,
            item: Box::new(item),
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
        let Some(text) = self.documents.get(&uri).map(|d| d.text.clone()) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let sender = self.sender.clone();
        self.read_spawner.spawn(move || {
            let ranges = compute_folding_ranges(&text);
            let _ = sender.send(Message::Response(Response::new_ok(id, ranges)));
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
        let Some(text) = self.documents.get(&uri).map(|d| d.text.clone()) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let base_dir = uri::to_path(&uri).and_then(|p| p.parent().map(Path::to_path_buf));
        let size_limit = self.editor_settings.link_file_size_limit();
        let sender = self.sender.clone();
        self.read_spawner.spawn(move || {
            let links = compute_document_links(&text, base_dir.as_deref(), size_limit);
            let _ = sender.send(Message::Response(Response::new_ok(id, links)));
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
        let Some(text) = self.documents.get(&uri).map(|d| d.text.clone()) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let sender = self.sender.clone();
        self.read_spawner.spawn(move || {
            let tokens = compute_semantic_tokens(&text);
            let result = SemanticTokensResult::Tokens(tokens);
            let _ = sender.send(Message::Response(Response::new_ok(id, result)));
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
        self.dispatch_read(ReadJob::WillRenameFiles {
            id,
            renames,
            sender: self.sender.clone(),
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
        self.dispatch_read(ReadJob::WorkspaceSymbol {
            id,
            query: params.query,
            sender: self.sender.clone(),
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
        let Some(text) = self.documents.get(&uri).map(|d| d.text.clone()) else {
            self.respond_ok(id, serde_json::Value::Null);
            return;
        };
        let path = uri::to_path(&uri).unwrap_or_else(|| PathBuf::from("untitled.R"));
        self.dispatch_read(ReadJob::PrepareCallHierarchy {
            id,
            path,
            uri,
            text,
            position,
            sender: self.sender.clone(),
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
        self.dispatch_read(ReadJob::IncomingCalls {
            id,
            item: Box::new(params.item),
            sender: self.sender.clone(),
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
        self.dispatch_read(ReadJob::OutgoingCalls {
            id,
            item: Box::new(params.item),
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
                ReadJob::Completion { id, sender, .. } => (id, sender),
                ReadJob::SignatureHelp { id, sender, .. } => (id, sender),
                ReadJob::ResolveCompletion { id, sender, .. } => (id, sender),
                ReadJob::Definition { id, sender, .. } => (id, sender),
                ReadJob::References { id, sender, .. } => (id, sender),
                ReadJob::Rename { id, sender, .. } => (id, sender),
                ReadJob::WillRenameFiles { id, sender, .. } => (id, sender),
                ReadJob::WorkspaceSymbol { id, sender, .. } => (id, sender),
                ReadJob::PrepareCallHierarchy { id, sender, .. } => (id, sender),
                ReadJob::IncomingCalls { id, sender, .. } => (id, sender),
                ReadJob::OutgoingCalls { id, sender, .. } => (id, sender),
            };
            let _ = sender.send(Message::Response(Response::new_ok(
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
                    self.report_ids.remove(&uri);
                    self.rename_anchors.remove(&uri);
                    // Resolve any parked pulls with an empty report so they don't
                    // hang now that the buffer is gone.
                    for PendingPull { id } in self.pending_pull.remove(&uri).unwrap_or_default() {
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
                // Cache findings (code actions still need them) and mint a fresh
                // report id for this lint generation.
                self.findings.insert(uri.clone(), (version, findings));
                let result_id = self.bump_result_id();
                self.report_ids.insert(uri.clone(), result_id.clone());

                // Answer any parked pulls for this URI; otherwise deliver via the
                // active channel (pull clients re-pull on their own cadence).
                let pending = self.pending_pull.remove(&uri).unwrap_or_default();
                for PendingPull { id } in pending {
                    self.respond_diagnostic(
                        id,
                        DiagnosticReport::Full(diags.clone(), Some(result_id.clone())),
                    );
                }
                if !self.pull_mode {
                    self.publish(uri, diags, Some(version));
                }
            }
            Outbound::RelintAll => {
                if self.pull_mode {
                    // Diagnostics may have changed without any document edit (a new
                    // index/sibling). Invalidate caches so a re-pull recomputes,
                    // then ask pull clients to re-request.
                    let uris: Vec<Uri> = self.documents.keys().cloned().collect();
                    for uri in &uris {
                        self.findings.remove(uri);
                        self.report_ids.remove(uri);
                    }
                    self.send_workspace_refresh();
                } else {
                    let uris: Vec<Uri> = self.documents.keys().cloned().collect();
                    for uri in uris {
                        self.send_lint(uri);
                    }
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

    /// Next opaque `resultId` for a pull report (a monotonic lint generation).
    fn bump_result_id(&mut self) -> String {
        self.result_seq += 1;
        self.result_seq.to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

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
