use super::*;

pub(crate) type DynError = Box<dyn std::error::Error + Sync + Send>;

/// Run the language server on stdio until the client disconnects.
pub fn run() -> Result<(), DynError> {
    let (connection, io_threads) = Connection::stdio();
    serve(connection)?;
    io_threads.join()?;
    Ok(())
}

/// Run the LSP protocol over an already-established connection: perform the
/// initialize handshake, then run the main loop until the client disconnects.
/// Shared by [`run`] (stdio) and the in-memory integration harness
/// ([`Connection::memory`]). The public error type is spelled out rather than
/// [`DynError`] (which is `pub(crate)`) so this stays reachable from
/// integration tests.
pub fn serve(connection: Connection) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (id, params) = connection.initialize_start()?;
    let editor_settings = params
        .get("initializationOptions")
        .map(EditorSettings::from_client_value)
        .unwrap_or_default();
    let workspace_roots = workspace_roots_from_params(&params);
    // If the client supports the pull model, suppress push for this session and
    // serve diagnostics on demand instead (avoids duplicate diagnostics).
    let pull_mode = client_supports_pull(&params);
    // Watched-file notifications are only available via dynamic registration; skip
    // it for clients that don't support it (they fall back to buffer-edit refresh).
    let register_watchers = client_supports_dynamic_watch(&params);
    // Server-initiated work-done progress is gated purely on the client
    // capability (there is no server capability to advertise for it); without it
    // the background index/sidecar jobs stay silent.
    let work_done_progress = client_supports_work_done_progress(&params);
    // Inlay hints have no push channel, so a fresh package index only reaches an
    // already-open `DESCRIPTION` if the client can be asked to re-request.
    let inlay_hint_refresh = client_supports_inlay_refresh(&params);
    // Pick the position encoding: prefer UTF-8 (arity stores text as UTF-8, so no
    // re-encoding) when the client offers it, else the UTF-16 default.
    let position_encoding = negotiate_position_encoding(&params);
    let init_result = InitializeResult {
        capabilities: server_capabilities(position_encoding),
        server_info: Some(ServerInfo {
            name: "arity".to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }),
    };
    // `lsp-types` 0.97 has no `type_hierarchy_provider` field on
    // `ServerCapabilities`, so advertise it by injecting the key directly into
    // the serialized capabilities. The method-string dispatch (see
    // `GlobalState::on_request`) is independent of the typed struct.
    let mut init_value = serde_json::to_value(init_result)?;
    init_value["capabilities"]["typeHierarchyProvider"] = serde_json::json!(true);
    connection.initialize_finish(id, init_value)?;

    main_loop(
        connection,
        editor_settings,
        workspace_roots,
        pull_mode,
        register_watchers,
        work_done_progress,
        inlay_hint_refresh,
        position_encoding,
    )?;
    Ok(())
}

/// Pick the LSP position encoding from the client's
/// `capabilities.general.positionEncodings` list. Prefer [`PositionEncoding::Utf8`]
/// when offered — arity stores text as UTF-8, so it needs no re-encoding — else
/// fall back to [`PositionEncoding::Utf16`], which is what the spec mandates when
/// the client offers no list (or only UTF-16).
pub(crate) fn negotiate_position_encoding(params: &serde_json::Value) -> PositionEncoding {
    let offers_utf8 = params
        .get("capabilities")
        .and_then(|c| c.get("general"))
        .and_then(|g| g.get("positionEncodings"))
        .and_then(|e| e.as_array())
        .is_some_and(|kinds| {
            kinds
                .iter()
                .filter_map(|k| k.as_str())
                .any(|k| k == PositionEncodingKind::UTF8.as_str())
        });
    if offers_utf8 {
        PositionEncoding::Utf8
    } else {
        PositionEncoding::Utf16
    }
}

/// Extract the workspace roots from the `initialize` params: the
/// `workspaceFolders` array if present, else the legacy `rootUri`. Non-`file`
/// URIs are skipped. Drives the one-time workspace seed (see [`LintWorker`]).
pub(crate) fn workspace_roots_from_params(params: &serde_json::Value) -> Vec<PathBuf> {
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

/// Whether the client declared support for the pull diagnostic model in its
/// `initialize` capabilities (`textDocument.diagnostic`). When true we suppress
/// push and answer `textDocument/diagnostic` on demand instead.
pub(crate) fn client_supports_pull(params: &serde_json::Value) -> bool {
    params
        .get("capabilities")
        .and_then(|c| c.get("textDocument"))
        .and_then(|t| t.get("diagnostic"))
        .is_some_and(|d| d.is_object())
}

/// Whether the client declared support for *dynamic* registration of
/// `workspace/didChangeWatchedFiles`
/// (`capabilities.workspace.didChangeWatchedFiles.dynamicRegistration`). There is
/// no static server capability for watched files, so without this we cannot
/// register watchers and the feature is unavailable for that client.
pub(crate) fn client_supports_dynamic_watch(params: &serde_json::Value) -> bool {
    params
        .get("capabilities")
        .and_then(|c| c.get("workspace"))
        .and_then(|w| w.get("didChangeWatchedFiles"))
        .and_then(|d| d.get("dynamicRegistration"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Whether the client declared support for server-initiated work-done progress
/// (`capabilities.window.workDoneProgress`). LSP has no *server* capability for
/// this—the server may only call `window/workDoneProgress/create` and emit
/// `$/progress` when the client advertised it here.
pub(crate) fn client_supports_work_done_progress(params: &serde_json::Value) -> bool {
    params
        .get("capabilities")
        .and_then(|c| c.get("window"))
        .and_then(|w| w.get("workDoneProgress"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Whether the client declared support for `workspace/inlayHint/refresh`
/// (`capabilities.workspace.inlayHint.refreshSupport`). Without it a fresh
/// package index cannot reach an open `DESCRIPTION` until the user edits it —
/// inlay hints are pull-only, with no server-initiated notification.
pub(crate) fn client_supports_inlay_refresh(params: &serde_json::Value) -> bool {
    params
        .get("capabilities")
        .and_then(|c| c.get("workspace"))
        .and_then(|w| w.get("inlayHint"))
        .and_then(|h| h.get("refreshSupport"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

pub(crate) fn server_capabilities(position_encoding: PositionEncoding) -> ServerCapabilities {
    ServerCapabilities {
        position_encoding: Some(position_encoding.to_kind()),
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::INCREMENTAL,
        )),
        document_formatting_provider: Some(OneOf::Left(true)),
        document_range_formatting_provider: Some(OneOf::Left(true)),
        code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
        completion_provider: Some(CompletionOptions {
            // `:` fires after the second colon of `::` for member completion; `$`
            // and `@` fire member completion on list/S4 receivers; `.` is
            // ubiquitous in R names, so re-query as it is typed.
            trigger_characters: Some(vec![
                ":".to_string(),
                "$".to_string(),
                "@".to_string(),
                ".".to_string(),
            ]),
            resolve_provider: Some(true),
            completion_item: Some(CompletionOptionsCompletionItem {
                label_details_support: Some(true),
            }),
            ..Default::default()
        }),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        signature_help_provider: Some(SignatureHelpOptions {
            // `=` starts a named argument, so it moves the active parameter to
            // the named formal; it is a retrigger character too, since help is
            // usually already showing (from `(` or `,`) by the time it is typed.
            trigger_characters: Some(vec!["(".to_string(), ",".to_string(), "=".to_string()]),
            retrigger_characters: Some(vec![")".to_string(), "=".to_string()]),
            work_done_progress_options: Default::default(),
        }),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),
        document_highlight_provider: Some(OneOf::Left(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        workspace_symbol_provider: Some(OneOf::Left(true)),
        folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
        selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
        color_provider: Some(ColorProviderCapability::Simple(true)),
        document_link_provider: Some(DocumentLinkOptions {
            // Targets are resolved eagerly in the initial response.
            resolve_provider: Some(false),
            work_done_progress_options: Default::default(),
        }),
        semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
            SemanticTokensOptions {
                legend: semantic_tokens_legend(),
                range: Some(false),
                full: Some(SemanticTokensFullOptions::Bool(true)),
                work_done_progress_options: Default::default(),
            },
        )),
        inlay_hint_provider: Some(OneOf::Right(InlayHintServerCapabilities::Options(
            InlayHintOptions {
                // The label and its tooltip both come from one index lookup the
                // initial response has already made, so there is nothing left to
                // defer to an `inlayHint/resolve` round trip.
                resolve_provider: Some(false),
                work_done_progress_options: Default::default(),
            },
        ))),
        rename_provider: Some(OneOf::Right(RenameOptions {
            prepare_provider: Some(true),
            work_done_progress_options: Default::default(),
        })),
        diagnostic_provider: Some(DiagnosticServerCapabilities::Options(DiagnosticOptions {
            identifier: Some("arity".to_string()),
            // Editing one file can change another's diagnostics (cross-file lint).
            inter_file_dependencies: true,
            // We serve per-document pull only; a cross-file/index change asks the
            // client to re-pull via `workspace/diagnostic/refresh` instead.
            workspace_diagnostics: false,
            work_done_progress_options: Default::default(),
        })),
        workspace: Some(WorkspaceServerCapabilities {
            // Accept workspace folders and ask to be notified when the set changes,
            // so a newly-added folder is seeded into cross-file analysis (see
            // `GlobalState::on_workspace_folders_changed`).
            workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                supported: Some(true),
                change_notifications: Some(OneOf::Left(true)),
            }),
            file_operations: Some(WorkspaceFileOperationsServerCapabilities {
                will_rename: Some(r_file_rename_registration()),
                did_rename: Some(r_file_rename_registration()),
                ..Default::default()
            }),
        }),
        ..Default::default()
    }
}

/// Register `willRenameFiles`/`didRenameFiles` for `.R`/`.r` files and for any
/// folder — a moved R source is the one rename that rewrites `source()` literals
/// in dependents, and a moved folder is that same rename in bulk.
///
/// The folder glob has to be `**`: any directory may hold `.R` files, so there is
/// nothing to narrow it to client-side. A folder carrying nothing we track is
/// rejected cheaply server-side once
/// [`expand_dir_renames`](crate::incremental::expand_dir_renames) finds no paths
/// beneath it. The file filter states `File` explicitly rather than leaving
/// `matches` unset ("both"), so that a *directory* named `foo.R` is routed
/// through the folder filter instead of masquerading as a file.
pub(crate) fn r_file_rename_registration() -> FileOperationRegistrationOptions {
    FileOperationRegistrationOptions {
        filters: vec![
            FileOperationFilter {
                scheme: Some("file".to_string()),
                pattern: FileOperationPattern {
                    glob: "**/*.{R,r}".to_string(),
                    matches: Some(FileOperationPatternKind::File),
                    options: None,
                },
            },
            FileOperationFilter {
                scheme: Some("file".to_string()),
                pattern: FileOperationPattern {
                    glob: "**".to_string(),
                    matches: Some(FileOperationPatternKind::Folder),
                    options: None,
                },
            },
        ],
    }
}

/// The main event loop: dispatch incoming JSON-RPC messages and lint results.
/// Owns the connection so that returning drops the sender and lets the writer
/// thread finish; joins the lint thread before returning.
///
/// Three of the parameters are now negotiated client capabilities, threaded
/// straight through to [`GlobalState`]; the next one earns them a struct.
#[allow(clippy::too_many_arguments)]
pub(crate) fn main_loop(
    connection: Connection,
    editor_settings: EditorSettings,
    workspace_roots: Vec<PathBuf>,
    pull_mode: bool,
    register_watchers: bool,
    work_done_progress: bool,
    inlay_hint_refresh: bool,
    position_encoding: PositionEncoding,
) -> Result<(), DynError> {
    let (out_tx, out_rx) = crossbeam_channel::unbounded::<Outbound>();
    let (lint_tx, lint_rx) = crossbeam_channel::unbounded::<LintMsg>();
    let (read_tx, read_rx) = crossbeam_channel::unbounded::<ReadJob>();

    // The read pool serves latency-sensitive work (formatting, hover, the analyze
    // read-phase, code actions). Its `_workers` must outlive both `state` and the
    // lint thread; the drop order at the end of this function guarantees that.
    let read_pool = TaskPool::new("arity-lsp-read", read_pool_size());
    let lint_handle = spawn_lint_thread(
        lint_rx,
        read_rx,
        out_tx.clone(),
        read_pool.spawner(),
        position_encoding,
    );
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
        out_tx,
        lint_tx,
        read_tx,
        read_pool.spawner(),
        editor_settings,
        pull_mode,
        work_done_progress,
        inlay_hint_refresh,
        position_encoding,
    );

    // Ask the client to watch on-disk config, package-metadata, and `.R` files so
    // changes made outside the editor reach cross-file analysis. Requires dynamic
    // registration support (there is no static capability for watched files).
    if register_watchers {
        state.register_file_watchers();
    }

    loop {
        select! {
            recv(connection.receiver) -> msg => {
                let Ok(msg) = msg else { break };
                match msg {
                    Message::Request(req) => {
                        if connection.handle_shutdown(&req)? {
                            break;
                        }
                        // Guarded so a panic in one handler can't take down the
                        // main loop (and with it the server); mirrors the lint
                        // thread and read pool. State mutations here are simple
                        // bookkeeping, so surviving a panic beats dying.
                        guard("request", || state.on_request(req));
                    }
                    Message::Notification(not) => {
                        guard("notification", || state.on_notification(not));
                    }
                    Message::Response(_) => {}
                }
            }
            recv(out_rx) -> ob => {
                let Ok(ob) = ob else { break };
                guard("outbound", || state.on_outbound(ob));
            }
        }
    }

    drop(state); // drops lint_tx → the lint thread's recv disconnects → it exits
    let _ = lint_handle.join();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn advertises_pull_diagnostic_provider() {
        let DiagnosticServerCapabilities::Options(opts) =
            server_capabilities(PositionEncoding::Utf16)
                .diagnostic_provider
                .expect("diagnostic provider advertised")
        else {
            panic!("expected plain DiagnosticOptions");
        };
        assert_eq!(opts.identifier.as_deref(), Some("arity"));
        assert!(opts.inter_file_dependencies);
        assert!(!opts.workspace_diagnostics);
    }

    #[test]
    fn advertises_workspace_folders_support() {
        let ws = server_capabilities(PositionEncoding::Utf16)
            .workspace
            .expect("workspace capabilities advertised");
        let folders = ws.workspace_folders.expect("workspace folders advertised");
        assert_eq!(folders.supported, Some(true));
        assert_eq!(folders.change_notifications, Some(OneOf::Left(true)));
    }

    #[test]
    fn registers_rename_for_r_files_and_any_folder() {
        // Both kinds are stated explicitly: a directory named `foo.R` must be
        // routed by the folder filter, not matched as a file.
        let registration = r_file_rename_registration();
        let shapes: Vec<(&str, Option<FileOperationPatternKind>)> = registration
            .filters
            .iter()
            .map(|f| {
                assert_eq!(f.scheme.as_deref(), Some("file"));
                (f.pattern.glob.as_str(), f.pattern.matches.clone())
            })
            .collect();
        assert_eq!(
            shapes,
            vec![
                ("**/*.{R,r}", Some(FileOperationPatternKind::File)),
                ("**", Some(FileOperationPatternKind::Folder)),
            ]
        );
    }

    #[test]
    fn detects_client_dynamic_watch_support() {
        let with = serde_json::json!({
            "capabilities": {
                "workspace": { "didChangeWatchedFiles": { "dynamicRegistration": true } }
            }
        });
        assert!(client_supports_dynamic_watch(&with));

        // Explicitly false, or absent, means no watcher registration.
        let off = serde_json::json!({
            "capabilities": {
                "workspace": { "didChangeWatchedFiles": { "dynamicRegistration": false } }
            }
        });
        assert!(!client_supports_dynamic_watch(&off));
        assert!(!client_supports_dynamic_watch(&serde_json::json!({})));
    }

    #[test]
    fn detects_client_pull_support() {
        let with = serde_json::json!({
            "capabilities": { "textDocument": { "diagnostic": { "dynamicRegistration": false } } }
        });
        assert!(client_supports_pull(&with));

        // No `diagnostic` capability, or a non-object value, means push-only.
        let without = serde_json::json!({ "capabilities": { "textDocument": { "hover": {} } } });
        assert!(!client_supports_pull(&without));
        assert!(!client_supports_pull(&serde_json::json!({})));
    }

    #[test]
    fn detects_client_work_done_progress_support() {
        let with = serde_json::json!({
            "capabilities": { "window": { "workDoneProgress": true } }
        });
        assert!(client_supports_work_done_progress(&with));

        // Explicitly false, or absent, means no server-initiated progress.
        let off = serde_json::json!({
            "capabilities": { "window": { "workDoneProgress": false } }
        });
        assert!(!client_supports_work_done_progress(&off));
        assert!(!client_supports_work_done_progress(&serde_json::json!({})));
    }

    #[test]
    fn detects_client_inlay_refresh_support() {
        let with = serde_json::json!({
            "capabilities": { "workspace": { "inlayHint": { "refreshSupport": true } } }
        });
        assert!(client_supports_inlay_refresh(&with));

        // Explicitly false, or absent, means a stale hint waits for the next edit.
        let off = serde_json::json!({
            "capabilities": { "workspace": { "inlayHint": { "refreshSupport": false } } }
        });
        assert!(!client_supports_inlay_refresh(&off));
        assert!(!client_supports_inlay_refresh(&serde_json::json!({})));
    }

    #[test]
    fn negotiates_utf8_only_when_client_offers_it() {
        // Client advertises UTF-8 → we prefer it (arity's native encoding).
        let utf8 = serde_json::json!({
            "capabilities": { "general": { "positionEncodings": ["utf-8", "utf-16"] } }
        });
        assert_eq!(negotiate_position_encoding(&utf8), PositionEncoding::Utf8);

        // Client offers only UTF-16 → UTF-16.
        let utf16 = serde_json::json!({
            "capabilities": { "general": { "positionEncodings": ["utf-16"] } }
        });
        assert_eq!(negotiate_position_encoding(&utf16), PositionEncoding::Utf16);

        // No `general.positionEncodings` at all → UTF-16, the mandated default.
        assert_eq!(
            negotiate_position_encoding(&serde_json::json!({})),
            PositionEncoding::Utf16
        );
    }

    #[test]
    fn advertises_negotiated_position_encoding() {
        assert_eq!(
            server_capabilities(PositionEncoding::Utf8).position_encoding,
            Some(PositionEncodingKind::UTF8)
        );
        assert_eq!(
            server_capabilities(PositionEncoding::Utf16).position_encoding,
            Some(PositionEncodingKind::UTF16)
        );
    }
}
