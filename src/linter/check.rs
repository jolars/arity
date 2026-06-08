//! `ravel lint` driver: walks input paths, parses, builds a semantic model,
//! runs the configured rules, filters suppressed findings, and reports.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::LintConfig;
use crate::file_discovery::{FileDiscoveryError, collect_r_files};
use crate::incremental::{IncrementalDatabase, SourceFile};
use crate::project::{
    FileFacts, FileScope, ProjectScope, collect_source_edges, file_exports, file_free_reads,
    package_root,
};
use crate::semantic::SymbolProvider;

use super::diagnostic::Diagnostic;
use super::rules::{ResolvedRules, default_symbol_provider, run_rules};
use super::suppression::SuppressionMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LintStatus {
    Clean,
    Findings { count: usize },
    ParseDiagnostics { count: usize },
}

#[derive(Debug, Clone)]
pub struct LintFileReport {
    pub path: PathBuf,
    pub status: LintStatus,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone)]
pub struct LintResult {
    pub checked_files: usize,
    pub total_findings: usize,
    pub reports: Vec<LintFileReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LintError {
    MissingPaths,
    NoRFiles,
    NonRFilePath { path: PathBuf },
    WalkError { path: PathBuf, message: String },
    ReadError { path: PathBuf, source: String },
    UnknownRule { rule: String },
}

impl fmt::Display for LintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPaths => {
                write!(
                    f,
                    "lint requires at least one input path (file or directory)"
                )
            }
            Self::NoRFiles => write!(f, "no .R files found under the provided input paths"),
            Self::NonRFilePath { path } => write!(
                f,
                "input file {} is not an .R file; lint only supports .R files",
                path.display()
            ),
            Self::WalkError { path, message } => {
                write!(f, "failed while scanning {}: {message}", path.display())
            }
            Self::ReadError { path, source } => {
                write!(f, "failed to read {}: {source}", path.display())
            }
            Self::UnknownRule { rule } => write!(f, "unknown lint rule: `{rule}`"),
        }
    }
}

impl std::error::Error for LintError {}

impl From<FileDiscoveryError> for LintError {
    fn from(value: FileDiscoveryError) -> Self {
        match value {
            FileDiscoveryError::NonRFilePath { path } => Self::NonRFilePath { path },
            FileDiscoveryError::WalkError { path, message } => Self::WalkError { path, message },
        }
    }
}

pub fn check_paths(paths: &[PathBuf]) -> Result<LintResult, LintError> {
    check_paths_with_config(paths, &LintConfig::default())
}

pub fn check_paths_with_config(
    paths: &[PathBuf],
    config: &LintConfig,
) -> Result<LintResult, LintError> {
    check_paths_with_provider(paths, config, &default_symbol_provider())
}

/// Like [`check_paths_with_config`] but with a caller-supplied symbol provider
/// (e.g. one backed by the installed-package index). The provider is built
/// once and reused across all files.
pub fn check_paths_with_provider(
    paths: &[PathBuf],
    config: &LintConfig,
    provider: &dyn SymbolProvider,
) -> Result<LintResult, LintError> {
    if paths.is_empty() {
        return Err(LintError::MissingPaths);
    }

    let (rules, unknown) = ResolvedRules::resolve(config.select.as_deref(), &config.ignore);
    if let Some(rule) = unknown.into_iter().next() {
        return Err(LintError::UnknownRule { rule });
    }

    let files = collect_r_files(paths).map_err(LintError::from)?;
    if files.is_empty() {
        return Err(LintError::NoRFiles);
    }

    let mut db = IncrementalDatabase::default();
    let mut tracked: HashMap<PathBuf, SourceFile> = HashMap::new();

    // Pass 1: track every file and collect cross-file facts for the cleanly
    // parsed ones. Files with parse diagnostics are recorded for reporting but
    // contribute nothing to the project scope.
    let mut facts: Vec<FileFacts> = Vec::new();
    let mut parse_errors: HashMap<PathBuf, usize> = HashMap::new();
    for path in &files {
        let content = fs::read_to_string(path).map_err(|err| LintError::ReadError {
            path: path.clone(),
            source: err.to_string(),
        })?;
        let file = db.upsert_file(path, content);
        tracked.insert(path.clone(), file);

        let parse_diag_count = db.parse_diagnostics(file).len();
        if parse_diag_count == 0 {
            let model = db.semantic_model(file);
            facts.push(FileFacts {
                path: path.clone(),
                exports: file_exports(model),
                free_reads: file_free_reads(model),
                source_edges: collect_source_edges(&db.parsed_tree(file), path.parent()),
                package_root: package_root(path),
            });
        } else {
            parse_errors.insert(path.clone(), parse_diag_count);
        }
    }

    // Read the NAMESPACE of each package being linted, so exported bindings
    // aren't flagged unused and imported names resolve.
    let mut namespaces: HashMap<PathBuf, String> = HashMap::new();
    for f in &facts {
        if let Some(root) = &f.package_root
            && !namespaces.contains_key(root)
            && let Ok(text) = fs::read_to_string(root.join("NAMESPACE"))
        {
            namespaces.insert(root.clone(), text);
        }
    }

    let scope = ProjectScope::build(&facts, &namespaces);

    // Pass 2: lint each cleanly parsed file with its cross-file scope.
    let mut reports = Vec::new();
    let mut total_findings = 0usize;
    for path in files {
        let file = tracked[&path];
        let (status, diagnostics) = if let Some(&count) = parse_errors.get(&path) {
            (LintStatus::ParseDiagnostics { count }, Vec::new())
        } else {
            let file_scope = scope.for_file(&path);
            let kept = lint_parsed_file(&db, file, &path, &rules, provider, Some(&file_scope));
            total_findings += kept.len();
            let status = if kept.is_empty() {
                LintStatus::Clean
            } else {
                LintStatus::Findings { count: kept.len() }
            };
            (status, kept)
        };
        reports.push(LintFileReport {
            path,
            status,
            diagnostics,
        });
    }

    Ok(LintResult {
        checked_files: tracked.len(),
        total_findings,
        reports,
    })
}

/// Run the resolved rules against a cleanly-parsed file, using the cached parse
/// tree and semantic model, and drop suppressed findings. Callers must have
/// already confirmed the file parses without diagnostics.
fn lint_parsed_file(
    db: &IncrementalDatabase,
    file: SourceFile,
    path: &Path,
    rules: &ResolvedRules,
    provider: &dyn SymbolProvider,
    project: Option<&FileScope<'_>>,
) -> Vec<Diagnostic> {
    let root_node = db.parsed_tree(file);
    let model = db.semantic_model(file);
    let mut diagnostics = run_rules(&rules.rules, path, &root_node, model, provider, project);
    let suppress = SuppressionMap::build(&root_node);
    diagnostics.retain(|d| !suppress.is_suppressed(d.rule, d.range));
    for d in &mut diagnostics {
        d.path = path.to_path_buf();
    }
    diagnostics
}

/// Lint a file already tracked in `db`, reusing its cached parse and model.
/// Returns no findings when the file has parse diagnostics. Used by the LSP,
/// which holds a long-lived `db` so edits don't re-parse from scratch.
pub fn check_tracked_file(
    db: &IncrementalDatabase,
    file: SourceFile,
    path: &Path,
    config: &LintConfig,
    provider: &dyn SymbolProvider,
) -> Result<Vec<Diagnostic>, LintError> {
    let (rules, unknown) = ResolvedRules::resolve(config.select.as_deref(), &config.ignore);
    if let Some(rule) = unknown.into_iter().next() {
        return Err(LintError::UnknownRule { rule });
    }
    if !db.parse_diagnostics(file).is_empty() {
        return Ok(Vec::new());
    }
    Ok(lint_parsed_file(db, file, path, &rules, provider, None))
}

/// The write-phase output of cross-file linting: everything [`analyze_prepared`]
/// needs, all derivable with read-only `&db` access afterward. Produced by
/// [`prepare_document_in_project`].
///
/// Splitting the lint into a write-phase ([`prepare_document_in_project`], needs
/// `&mut db`) and a read-phase ([`analyze_prepared`], `&db` only) lets the LSP
/// run the expensive read-phase off its lint thread on a short-lived db clone,
/// where it can be cancelled by a fresher edit (see `src/lsp.rs`).
pub struct PreparedProject {
    active: SourceFile,
    active_path: PathBuf,
    rules: ResolvedRules,
    /// Cleanly-parsing project files (path + tracked input), including `active`.
    /// Files with parse diagnostics are dropped here, matching the old behavior.
    files: Vec<(PathBuf, SourceFile)>,
    namespaces: HashMap<PathBuf, String>,
}

/// Write-phase of cross-file linting (needs `&mut db`). Discovers the enclosing
/// project — the R package root, else the file's directory — loads its sibling
/// files into `db` (cached across calls, so unchanged siblings aren't re-parsed),
/// and reads the relevant `NAMESPACE` files. `active` must already be tracked in
/// `db` carrying the live editor buffer.
///
/// Returns `Ok(None)` when the active file has parse diagnostics (the caller
/// publishes no findings, as the old early-return did). All `db` *writes*
/// (`upsert_file`) happen here; the returned [`PreparedProject`] is then consumed
/// by the read-only [`analyze_prepared`].
pub fn prepare_document_in_project(
    db: &mut IncrementalDatabase,
    path: &Path,
    active: SourceFile,
    config: &LintConfig,
) -> Result<Option<PreparedProject>, LintError> {
    let (rules, unknown) = ResolvedRules::resolve(config.select.as_deref(), &config.ignore);
    if let Some(rule) = unknown.into_iter().next() {
        return Err(LintError::UnknownRule { rule });
    }
    if !db.parse_diagnostics(active).is_empty() {
        return Ok(None);
    }

    // Discover the project's files: the package root, else the file's directory.
    let search_dir =
        package_root(path).or_else(|| path.parent().filter(|p| p.is_dir()).map(Path::to_path_buf));
    let mut project_files = match &search_dir {
        Some(dir) => collect_r_files(std::slice::from_ref(dir)).unwrap_or_default(),
        None => Vec::new(),
    };
    if !project_files.iter().any(|p| p == path) {
        project_files.push(path.to_path_buf());
    }

    // Upsert each project file — the live buffer for `path`, on-disk content for
    // siblings — and keep the cleanly-parsing ones for the read-phase. These are
    // the only db writes; `analyze_prepared` reads off the inputs set here.
    let mut files = Vec::new();
    for file_path in &project_files {
        let file = if file_path == path {
            active
        } else {
            match fs::read_to_string(file_path) {
                Ok(text) => db.upsert_file(file_path, text),
                Err(_) => continue,
            }
        };
        if !db.parse_diagnostics(file).is_empty() {
            continue;
        }
        files.push((file_path.clone(), file));
    }

    let mut namespaces: HashMap<PathBuf, String> = HashMap::new();
    for (file_path, _) in &files {
        if let Some(root) = package_root(file_path)
            && !namespaces.contains_key(&root)
            && let Ok(text) = fs::read_to_string(root.join("NAMESPACE"))
        {
            namespaces.insert(root, text);
        }
    }

    Ok(Some(PreparedProject {
        active,
        active_path: path.to_path_buf(),
        rules,
        files,
        namespaces,
    }))
}

/// Read-phase of cross-file linting (`&db` only — no disk, no writes). Builds the
/// per-file facts from cached models/trees, assembles the project scope, and
/// lints the active file against it. Safe to run on a db clone; salsa aborts it
/// with [`salsa::Cancelled`] (at the next tracked-query entry) if a write races.
pub fn analyze_prepared(
    db: &IncrementalDatabase,
    prepared: &PreparedProject,
    provider: &dyn SymbolProvider,
) -> Vec<Diagnostic> {
    let mut facts = Vec::new();
    for (file_path, file) in &prepared.files {
        let model = db.semantic_model(*file);
        facts.push(FileFacts {
            path: file_path.clone(),
            exports: file_exports(model),
            free_reads: file_free_reads(model),
            source_edges: collect_source_edges(&db.parsed_tree(*file), file_path.parent()),
            package_root: package_root(file_path),
        });
    }

    let scope = ProjectScope::build(&facts, &prepared.namespaces);
    let file_scope = scope.for_file(&prepared.active_path);
    lint_parsed_file(
        db,
        prepared.active,
        &prepared.active_path,
        &prepared.rules,
        provider,
        Some(&file_scope),
    )
}

/// Lint `path` (already tracked in `db` as `active`, carrying the live editor
/// buffer) with cross-file resolution. Thin wrapper over the write-phase
/// ([`prepare_document_in_project`]) and read-phase ([`analyze_prepared`]); used
/// by the CLI and tests. The LSP drives the two phases separately so the
/// read-phase can run cancellably off its lint thread.
pub fn check_document_in_project(
    db: &mut IncrementalDatabase,
    path: &Path,
    active: SourceFile,
    config: &LintConfig,
    provider: &dyn SymbolProvider,
) -> Result<Vec<Diagnostic>, LintError> {
    match prepare_document_in_project(db, path, active, config)? {
        Some(prepared) => Ok(analyze_prepared(db, &prepared, provider)),
        None => Ok(Vec::new()),
    }
}

/// Convenience: lint a single in-memory document by path + text (used by quick
/// fixes and tests). Builds a one-shot database; the LSP's hot lint path uses
/// [`check_tracked_file`] against its persistent database instead.
pub fn check_document(
    path: &Path,
    content: &str,
    config: &LintConfig,
) -> Result<Vec<Diagnostic>, LintError> {
    check_document_with_provider(path, content, config, &default_symbol_provider())
}

/// Like [`check_document`] but with a caller-supplied symbol provider.
pub fn check_document_with_provider(
    path: &Path,
    content: &str,
    config: &LintConfig,
    provider: &dyn SymbolProvider,
) -> Result<Vec<Diagnostic>, LintError> {
    let db = IncrementalDatabase::default();
    let file = db.add_file(content.to_string());
    check_tracked_file(&db, file, path, config, provider)
}
