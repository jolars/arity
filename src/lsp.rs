//! Stdio-based LSP server: formatting + pushed diagnostics.
//!
//! Document changes are debounced (200 ms) per URI and serialized by an i32
//! version counter so a fast typist can't trigger overlapping lint runs.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use smol_str::SmolStr;
use tokio::time;
use tower_lsp_server::jsonrpc::Result as JsonRpcResult;
use tower_lsp_server::ls_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionProviderCapability, CodeActionResponse, Diagnostic as LspDiagnostic,
    DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentFormattingParams, InitializeParams, InitializeResult,
    InitializedParams, MessageType, NumberOrString, OneOf, Position, Range, ServerCapabilities,
    ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Uri, WorkspaceEdit,
};
use tower_lsp_server::{Client, LanguageServer, LspService, Server};

use crate::config::{Config, IndexConfig, LintConfig};
use crate::formatter::{FormatStyle, format_with_style};
use crate::linter::{Diagnostic, Severity};
use crate::rindex::build::{BuildOptions, build_index};
use crate::rindex::cache::{Cache, resolve_cache_root};
use crate::rindex::discover::referenced_in_source;
use crate::rindex::libpaths::LibrarySearch;
use crate::rindex::provider::{CompositeProvider, IndexedProvider};
use crate::semantic::SymbolProvider as _;
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

#[derive(Debug)]
struct State {
    documents: HashMap<Uri, Document>,
    config_cache: HashMap<PathBuf, ResolvedSettings>,
    /// The symbol provider used for linting. Starts base-R-only and is replaced
    /// once the index cache is loaded (and again after a background build).
    index: Arc<CompositeProvider>,
    /// Workspace anchors whose index cache has already been loaded into `index`.
    index_loaded: HashSet<PathBuf>,
    /// Packages a background harvest has already been scheduled for this session
    /// — never retried, so a not-installed package doesn't loop.
    index_attempts: HashSet<SmolStr>,
}

impl Default for State {
    fn default() -> Self {
        State {
            documents: HashMap::new(),
            config_cache: HashMap::new(),
            index: Arc::new(CompositeProvider::base_only()),
            index_loaded: HashSet::new(),
            index_attempts: HashSet::new(),
        }
    }
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
            index: config.index,
        };

        let mut state = self.state.lock().expect("state mutex poisoned");
        state.config_cache.insert(anchor, resolved.clone());
        Ok(resolved)
    }

    /// Lint `uri` after the debounce window and publish diagnostics.
    fn schedule_lint(&self, uri: Uri) {
        let state = Arc::clone(&self.state);
        let client = self.client.clone();
        tokio::spawn(async move {
            time::sleep(LINT_DEBOUNCE).await;
            lint_and_publish(state, client, uri).await;
        });
    }
}

/// Resolve the (lint, index) config for a document path's anchor, preferring the
/// per-anchor cache and falling back to a fresh `Config::resolve`.
fn resolve_doc_config(state: &Arc<Mutex<State>>, anchor: &Path) -> (LintConfig, IndexConfig) {
    let cached = {
        let s = state.lock().expect("state mutex poisoned");
        s.config_cache.get(anchor).cloned()
    };
    if let Some(s) = cached {
        return (s.lint, s.index);
    }
    Config::resolve(None, false, anchor)
        .ok()
        .map(|(c, _)| (c.lint, c.index))
        .unwrap_or_default()
}

/// Lint a single document with the current index-backed provider and publish.
/// Reused by the debounced `schedule_lint` and by background-build completion.
async fn lint_and_publish(state: Arc<Mutex<State>>, client: Client, uri: Uri) {
    let snapshot = {
        let s = state.lock().expect("state mutex poisoned");
        s.documents.get(&uri).cloned()
    };
    let Some(doc) = snapshot else { return };

    let path = match uri.to_file_path() {
        Some(p) => p.into_owned(),
        None => PathBuf::from("untitled.R"),
    };
    let anchor = path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let (lint_config, index_config) = resolve_doc_config(&state, &anchor);

    // Make sure the index cache for this workspace is loaded, then lint against
    // it; finally, kick off a background harvest for any still-unknown packages.
    let provider = ensure_index(&state, &anchor, &index_config);
    let diagnostics = crate::linter::check::check_document_with_provider(
        &path,
        &doc.text,
        &lint_config,
        &*provider,
    )
    .unwrap_or_default();
    let line_index = LineIndex::new(&doc.text);
    let lsp_diags: Vec<LspDiagnostic> = diagnostics
        .iter()
        .map(|d| to_lsp_diagnostic(d, &line_index))
        .collect();

    let still_current = {
        let s = state.lock().expect("state mutex poisoned");
        matches!(s.documents.get(&uri), Some(cur) if cur.version == doc.version)
    };
    if still_current {
        client
            .publish_diagnostics(uri, lsp_diags, Some(doc.version))
            .await;
    }

    if index_config.auto_build {
        schedule_index_build(&state, &client, anchor, index_config, &doc.text, &provider);
    }
}

/// Load the index cache for `anchor` into the shared provider the first time we
/// see that workspace; return the current provider either way. Cache reads
/// happen outside the state lock.
fn ensure_index(
    state: &Arc<Mutex<State>>,
    anchor: &Path,
    cfg: &IndexConfig,
) -> Arc<CompositeProvider> {
    {
        let s = state.lock().expect("state mutex poisoned");
        if s.index_loaded.contains(anchor) {
            return Arc::clone(&s.index);
        }
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
    let mut s = state.lock().expect("state mutex poisoned");
    s.index = Arc::clone(&provider);
    s.index_loaded.insert(anchor.to_path_buf());
    provider
}

/// Packages referenced in `source` that the current `provider` can't resolve and
/// that we haven't already attempted this session. Marks the returned packages
/// as attempted so they aren't built twice.
fn packages_to_build(
    state: &Arc<Mutex<State>>,
    provider: &CompositeProvider,
    source: &str,
) -> Vec<SmolStr> {
    let referenced = referenced_in_source(source);
    let mut s = state.lock().expect("state mutex poisoned");
    referenced
        .into_iter()
        .filter(|pkg| !provider.package_indexed(pkg) && s.index_attempts.insert(pkg.clone()))
        .collect()
}

/// Spawn a background harvest for the document's unknown packages. On success,
/// swap in a freshly-loaded provider and re-lint every open document.
fn schedule_index_build(
    state: &Arc<Mutex<State>>,
    client: &Client,
    anchor: PathBuf,
    cfg: IndexConfig,
    source: &str,
    provider: &CompositeProvider,
) {
    let to_build = packages_to_build(state, provider, source);
    if to_build.is_empty() {
        return;
    }
    let Ok(cache_root) = resolve_cache_root(None, cfg.cache_dir.as_deref()) else {
        return;
    };

    let state = Arc::clone(state);
    let client = client.clone();
    tokio::spawn(async move {
        let now = now_unix_secs();
        let build_anchor = anchor.clone();
        // Harvesting reads (potentially large) on-disk DBs — keep it off the
        // async worker threads.
        let reloaded = tokio::task::spawn_blocking(move || {
            let cache = Cache::new(cache_root);
            let search = LibrarySearch::discover(Some(&build_anchor), &cfg.library_paths);
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
            report
                .newly_indexed()
                .next()
                .is_some()
                .then(|| CompositeProvider::with_index(IndexedProvider::from_cache(&cache)))
        })
        .await;

        let Ok(Some(provider)) = reloaded else { return };

        let open_uris = {
            let mut s = state.lock().expect("state mutex poisoned");
            s.index = Arc::new(provider);
            s.documents.keys().cloned().collect::<Vec<_>>()
        };
        for uri in open_uris {
            tokio::spawn(lint_and_publish(Arc::clone(&state), client.clone(), uri));
        }
    });
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> JsonRpcResult<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                document_formatting_provider: Some(OneOf::Left(true)),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
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

    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> JsonRpcResult<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let text = {
            let state = self.state.lock().expect("state mutex poisoned");
            state.documents.get(&uri).map(|d| d.text.clone())
        };
        let Some(text) = text else {
            return Ok(None);
        };

        let path = uri
            .to_file_path()
            .map(|p| p.into_owned())
            .unwrap_or_else(|| PathBuf::from("untitled.R"));
        let lint = self
            .resolve_settings(&uri)
            .map(|s| s.lint)
            .unwrap_or_default();

        Ok(Some(compute_code_actions(
            &text,
            &path,
            &lint,
            &uri,
            params.range,
        )))
    }
}

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
    let line_index = LineIndex::new(text);

    diagnostics
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

    fn uri() -> Uri {
        Uri::from_file_path("/tmp/t.R").expect("valid file uri")
    }

    #[test]
    fn code_action_offers_quickfix_for_diagnostic_in_range() {
        let src = "if (x = 1) print(x)\n";
        let actions = compute_code_actions(
            src,
            Path::new("/tmp/t.R"),
            &LintConfig::default(),
            &uri(),
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
        let edits = changes.get(&uri()).expect("edits for our uri");
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
        let actions = compute_code_actions(
            src,
            Path::new("/tmp/t.R"),
            &LintConfig::default(),
            &uri(),
            far,
        );
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
        let state = Arc::new(Mutex::new(State::default()));
        let provider = indexed_dplyr();
        // dplyr is indexed (skipped); a default package (stats) is "indexed" too;
        // only tidyr needs a build.
        let src = "library(dplyr)\nlibrary(stats)\nlibrary(tidyr)\n";
        let first = packages_to_build(&state, &provider, src);
        assert_eq!(first, vec![SmolStr::new("tidyr")]);
        // A second pass returns nothing — tidyr was already attempted.
        let second = packages_to_build(&state, &provider, src);
        assert!(second.is_empty(), "expected no re-attempt, got {second:?}");
    }
}
