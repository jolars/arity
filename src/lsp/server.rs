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
    let init_result = InitializeResult {
        capabilities: server_capabilities(),
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

    main_loop(connection, editor_settings, workspace_roots, pull_mode)?;
    Ok(())
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

pub(crate) fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        document_formatting_provider: Some(OneOf::Left(true)),
        document_range_formatting_provider: Some(OneOf::Left(true)),
        code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
        completion_provider: Some(CompletionOptions {
            // `:` fires after the second colon of `::` for member completion.
            trigger_characters: Some(vec![":".to_string()]),
            resolve_provider: Some(true),
            ..Default::default()
        }),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        signature_help_provider: Some(SignatureHelpOptions {
            trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
            retrigger_characters: Some(vec![")".to_string()]),
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
            workspace_folders: None,
            file_operations: Some(WorkspaceFileOperationsServerCapabilities {
                will_rename: Some(r_file_rename_registration()),
                did_rename: Some(r_file_rename_registration()),
                ..Default::default()
            }),
        }),
        ..Default::default()
    }
}

/// Register `willRenameFiles`/`didRenameFiles` for `.R`/`.r` files only — a moved
/// R source is the one rename that rewrites `source()` literals in dependents.
pub(crate) fn r_file_rename_registration() -> FileOperationRegistrationOptions {
    FileOperationRegistrationOptions {
        filters: vec![FileOperationFilter {
            scheme: Some("file".to_string()),
            pattern: FileOperationPattern {
                glob: "**/*.{R,r}".to_string(),
                matches: None,
                options: None,
            },
        }],
    }
}

/// The main event loop: dispatch incoming JSON-RPC messages and lint results.
/// Owns the connection so that returning drops the sender and lets the writer
/// thread finish; joins the lint thread before returning.
pub(crate) fn main_loop(
    connection: Connection,
    editor_settings: EditorSettings,
    workspace_roots: Vec<PathBuf>,
    pull_mode: bool,
) -> Result<(), DynError> {
    let (out_tx, out_rx) = crossbeam_channel::unbounded::<Outbound>();
    let (lint_tx, lint_rx) = crossbeam_channel::unbounded::<LintMsg>();
    let (read_tx, read_rx) = crossbeam_channel::unbounded::<ReadJob>();

    // The read pool serves latency-sensitive work (formatting, hover, the analyze
    // read-phase, code actions). Its `_workers` must outlive both `state` and the
    // lint thread; the drop order at the end of this function guarantees that.
    let read_pool = TaskPool::new("arity-lsp-read", read_pool_size());
    let lint_handle = spawn_lint_thread(lint_rx, read_rx, out_tx.clone(), read_pool.spawner());
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
        let DiagnosticServerCapabilities::Options(opts) = server_capabilities()
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
}
