//! Stdio-based LSP server: formatting + pushed diagnostics.
//!
//! Document changes are debounced (200 ms) per URI and serialized by an i32
//! version counter so a fast typist can't trigger overlapping lint runs.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::time;
use tower_lsp_server::jsonrpc::Result as JsonRpcResult;
use tower_lsp_server::ls_types::{
    Diagnostic as LspDiagnostic, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DocumentFormattingParams,
    InitializeParams, InitializeResult, InitializedParams, MessageType, NumberOrString, OneOf,
    Position, Range, ServerCapabilities, ServerInfo, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextEdit, Uri,
};
use tower_lsp_server::{Client, LanguageServer, LspService, Server};

use crate::config::{Config, LintConfig};
use crate::formatter::{FormatStyle, format_with_style};
use crate::linter::{Diagnostic, Severity};
use crate::text::LineIndex;

const LINT_DEBOUNCE: Duration = Duration::from_millis(200);

/// Run the language server on stdio until the client disconnects.
pub async fn run() {
    let (service, socket) = LspService::new(Backend::new);
    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .serve(service)
        .await;
}

#[derive(Debug)]
struct Backend {
    client: Client,
    state: Arc<Mutex<State>>,
}

#[derive(Debug, Default)]
struct State {
    documents: HashMap<Uri, Document>,
    config_cache: HashMap<PathBuf, ResolvedSettings>,
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
}

impl Backend {
    fn new(client: Client) -> Self {
        Self {
            client,
            state: Arc::new(Mutex::new(State::default())),
        }
    }

    fn resolve_settings(&self, uri: &Uri) -> Result<ResolvedSettings, ConfigResolveError> {
        if !uri.scheme().as_str().eq_ignore_ascii_case("file") {
            return Err(ConfigResolveError::NonFileUri);
        }
        let path = uri
            .to_file_path()
            .ok_or(ConfigResolveError::NonFileUri)?
            .into_owned();
        let anchor = path
            .parent()
            .ok_or(ConfigResolveError::NoParentDirectory)?
            .to_path_buf();

        {
            let state = self.state.lock().expect("state mutex poisoned");
            if let Some(s) = state.config_cache.get(&anchor) {
                return Ok(s.clone());
            }
        }

        let (config, _source) = Config::resolve(None, false, &anchor)
            .map_err(|err| ConfigResolveError::Config(err.to_string()))?;
        let resolved = ResolvedSettings {
            style: FormatStyle::from(&config.format),
            lint: config.lint,
        };

        let mut state = self.state.lock().expect("state mutex poisoned");
        state.config_cache.insert(anchor, resolved.clone());
        Ok(resolved)
    }

    fn schedule_lint(&self, uri: Uri) {
        let state = Arc::clone(&self.state);
        let client = self.client.clone();
        let backend_uri = uri.clone();
        tokio::spawn(async move {
            time::sleep(LINT_DEBOUNCE).await;
            // Snapshot the document & its version. If the version changed
            // during the debounce window, another scheduled lint will run.
            let snapshot = {
                let s = state.lock().expect("state mutex poisoned");
                s.documents.get(&backend_uri).cloned()
            };
            let Some(doc) = snapshot else { return };

            let path = match backend_uri.to_file_path() {
                Some(p) => p.into_owned(),
                None => PathBuf::from("untitled.R"),
            };

            // Resolve lint config (use a fresh lookup; cheap).
            let lint_config = {
                let anchor = path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| PathBuf::from("."));
                let cached = {
                    let s = state.lock().expect("state mutex poisoned");
                    s.config_cache.get(&anchor).cloned()
                };
                cached
                    .map(|s| s.lint)
                    .or_else(|| {
                        Config::resolve(None, false, &anchor)
                            .ok()
                            .map(|(c, _)| c.lint)
                    })
                    .unwrap_or_default()
            };

            let diagnostics = crate::linter::check::check_document(&path, &doc.text, &lint_config)
                .unwrap_or_default();
            let line_index = LineIndex::new(&doc.text);
            let lsp_diags: Vec<LspDiagnostic> = diagnostics
                .iter()
                .map(|d| to_lsp_diagnostic(d, &line_index))
                .collect();

            // Only publish if the document version hasn't moved past our snapshot.
            let still_current = {
                let s = state.lock().expect("state mutex poisoned");
                matches!(s.documents.get(&backend_uri), Some(cur) if cur.version == doc.version)
            };
            if still_current {
                client
                    .publish_diagnostics(backend_uri, lsp_diags, Some(doc.version))
                    .await;
            }
        });
    }
}

impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> JsonRpcResult<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                document_formatting_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "ravel".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "ravel LSP ready")
            .await;
    }

    async fn shutdown(&self) -> JsonRpcResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        {
            let mut state = self.state.lock().expect("state mutex poisoned");
            state.documents.insert(
                uri.clone(),
                Document {
                    text: params.text_document.text,
                    version: params.text_document.version,
                },
            );
        }
        self.schedule_lint(uri);
    }

    async fn did_change(&self, mut params: DidChangeTextDocumentParams) {
        let Some(change) = params.content_changes.pop() else {
            return;
        };
        let uri = params.text_document.uri.clone();
        {
            let mut state = self.state.lock().expect("state mutex poisoned");
            state.documents.insert(
                uri.clone(),
                Document {
                    text: change.text,
                    version: params.text_document.version,
                },
            );
        }
        self.schedule_lint(uri);
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        {
            let mut state = self.state.lock().expect("state mutex poisoned");
            state.documents.remove(&uri);
        }
        // Tell the client to clear stale diagnostics.
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> JsonRpcResult<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;
        let text = {
            let state = self.state.lock().expect("state mutex poisoned");
            state.documents.get(&uri).map(|d| d.text.clone())
        };
        let Some(text) = text else {
            self.client
                .log_message(
                    MessageType::WARNING,
                    format!("format request for unknown document: {}", uri.as_str()),
                )
                .await;
            return Ok(None);
        };

        let settings = match self.resolve_settings(&uri) {
            Ok(s) => s,
            Err(err) => {
                self.client
                    .log_message(
                        MessageType::ERROR,
                        format!("config error for {}: {err}", uri.as_str()),
                    )
                    .await;
                return Ok(None);
            }
        };

        match compute_format_edits(&text, settings.style) {
            Some(edits) => Ok(Some(edits)),
            None => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("ravel could not format {}", uri.as_str()),
                    )
                    .await;
                Ok(None)
            }
        }
    }
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

/// Compute the LSP `TextEdit`s to format `text` with `style`.
///
/// Returns `None` when the formatter rejects the input (e.g. parse error).
/// An empty `Vec` means the document is already formatted.
pub fn compute_format_edits(text: &str, style: FormatStyle) -> Option<Vec<TextEdit>> {
    let formatted = format_with_style(text, style).ok()?;
    if formatted == text {
        return Some(Vec::new());
    }
    let line_index = LineIndex::new(text);
    let end = line_index.byte_to_position(text.len());
    Some(vec![TextEdit {
        range: Range {
            start: Position::new(0, 0),
            end,
        },
        new_text: formatted,
    }])
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
