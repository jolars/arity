//! Stdio-based LSP server (built on `lsp-server`): formatting (whole-document
//! and range), pushed and pull diagnostics, quick-fix code actions, hover,
//! completion, signature help, go-to-definition and references, rename, document
//! and workspace symbols, semantic tokens, folding ranges, and call hierarchy,
//! backed by the introspection index and a per-file semantic model.
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
//! — never cross-canceled, so a multi-URI [`Outbound::RelintAll`] still publishes
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
    DidRenameFiles, Notification as NotificationTrait, PublishDiagnostics,
};
use lsp_types::request::{
    CallHierarchyIncomingCalls, CallHierarchyOutgoingCalls, CallHierarchyPrepare,
    CodeActionRequest, Completion, DocumentDiagnosticRequest, DocumentHighlightRequest,
    DocumentSymbolRequest, FoldingRangeRequest, Formatting, GotoDefinition, HoverRequest,
    PrepareRenameRequest, RangeFormatting, References, Rename, Request as RequestTrait,
    ResolveCompletionItem, SemanticTokensFullRequest, SignatureHelpRequest, WillRenameFiles,
    WorkspaceDiagnosticRefresh, WorkspaceSymbolRequest,
};
use lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyIncomingCallsParams, CallHierarchyItem,
    CallHierarchyOutgoingCall, CallHierarchyOutgoingCallsParams, CallHierarchyPrepareParams,
    CallHierarchyServerCapability, CodeAction, CodeActionKind, CodeActionOrCommand,
    CodeActionParams, CodeActionProviderCapability, CodeActionResponse, CompletionItem,
    CompletionItemKind, CompletionList, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic as LspDiagnostic, DiagnosticOptions, DiagnosticServerCapabilities,
    DiagnosticSeverity, DidChangeConfigurationParams, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DocumentDiagnosticParams,
    DocumentDiagnosticReport, DocumentDiagnosticReportResult, DocumentFormattingParams,
    DocumentHighlight, DocumentHighlightKind, DocumentHighlightParams,
    DocumentRangeFormattingParams, DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse,
    Documentation, FileOperationFilter, FileOperationPattern, FileOperationRegistrationOptions,
    FoldingRange, FoldingRangeKind, FoldingRangeParams, FoldingRangeProviderCapability,
    FullDocumentDiagnosticReport, GotoDefinitionParams, GotoDefinitionResponse, Hover,
    HoverContents, HoverParams, HoverProviderCapability, InitializeResult, Location, MarkupContent,
    MarkupKind, NumberOrString, OneOf, ParameterInformation, ParameterLabel, Position,
    PrepareRenameResponse, PublishDiagnosticsParams, Range, ReferenceParams,
    RelatedFullDocumentDiagnosticReport, RelatedUnchangedDocumentDiagnosticReport,
    RenameFilesParams, RenameOptions, RenameParams, SemanticToken, SemanticTokenModifier,
    SemanticTokenType, SemanticTokens, SemanticTokensFullOptions, SemanticTokensLegend,
    SemanticTokensOptions, SemanticTokensParams, SemanticTokensResult,
    SemanticTokensServerCapabilities, ServerCapabilities, ServerInfo, SignatureHelp,
    SignatureHelpOptions, SignatureHelpParams, SignatureInformation, SymbolKind as LspSymbolKind,
    TextDocumentPositionParams, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit,
    UnchangedDocumentDiagnosticReport, Uri, WorkspaceEdit,
    WorkspaceFileOperationsServerCapabilities, WorkspaceServerCapabilities, WorkspaceSymbol,
    WorkspaceSymbolParams, WorkspaceSymbolResponse,
};
use rowan::{NodeOrToken, SyntaxToken, TextRange, TextSize, TokenAtOffset};
use salsa::Database as _;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::ast::{ArgList, AssignmentExpr, AstNode as _, BinaryExpr, CallExpr, FunctionExpr};
use crate::config::{Config, FormatConfig, IndexConfig, LintConfig};
use crate::formatter::{FormatStyle, format_node, format_range, format_with_style};
use crate::incremental::{Analysis, IncrementalDatabase, SourceFile};
use crate::linter::{Diagnostic, Severity};
use crate::parser::{diff_edit, map_range_through_edit, parse};
use crate::project::DefKind;
use crate::rindex::build::{BuildOptions, build_index};
use crate::rindex::cache::{Cache, resolve_cache_root};
use crate::rindex::discover::{referenced_in_source, with_default_packages};
use crate::rindex::libpaths::LibrarySearch;
use crate::rindex::provider::{
    CompositeProvider, IndexedProvider, base_names, base_package_of, bundled_exports,
    package_indexed, resolve_origin,
};
use crate::rindex::remote::{RemoteExports, Sidecar};
use crate::rindex::schema::{Formal, SymbolEntry, SymbolKind};
use crate::semantic::{BindingId, BindingKind, PackageOrigin, SemanticModel};
use crate::syntax::{NodePtr, RLanguage, SyntaxKind, SyntaxNode};
use crate::text::LineIndex;
use task_pool::{Spawner, TaskPool, read_pool_size};

mod call_hierarchy;
mod code_actions;
mod completion;
mod file_rename;
mod folding;
mod format;
mod hover;
mod lint_thread;
mod navigation;
mod read_jobs;
mod semantic_tokens;
mod server;
mod settings;
mod signature;
mod state;
mod symbols;
mod uri;
mod workspace_symbols;

pub(crate) use call_hierarchy::*;
pub(crate) use code_actions::*;
pub(crate) use completion::*;
pub(crate) use file_rename::*;
pub(crate) use format::*;
pub(crate) use hover::*;
pub(crate) use lint_thread::*;
pub(crate) use navigation::*;
pub(crate) use read_jobs::*;
pub(crate) use semantic_tokens::*;
pub(crate) use settings::*;
pub(crate) use signature::*;
pub(crate) use state::*;
pub(crate) use workspace_symbols::*;

pub use code_actions::compute_code_actions;
pub use completion::{compute_completions, resolve_completion};
pub use folding::compute_folding_ranges;
pub use format::{compute_format_edits, compute_format_range_edits};
pub use hover::compute_hover;
pub use navigation::{
    PreparedRename, RenameAnchor, compute_definition, compute_document_highlights,
    compute_prepare_rename, compute_references, compute_rename, compute_rename_with_anchor,
};
pub use semantic_tokens::compute_semantic_tokens;
pub use server::run;
pub use signature::compute_signature_help;
pub use symbols::compute_document_symbols;

#[cfg(test)]
mod test_support;
#[cfg(test)]
use test_support::*;
