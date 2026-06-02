//! `ravel lint` driver: walks input paths, parses, builds a semantic model,
//! runs the configured rules, filters suppressed findings, and reports.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use rowan::ast::AstNode as _;

use crate::ast::Root;
use crate::config::LintConfig;
use crate::file_discovery::{FileDiscoveryError, collect_r_files};
use crate::incremental::{IncrementalDatabase, SourceFile};
use crate::parser::parse;
use crate::semantic::SemanticModel;

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
    let mut reports = Vec::new();
    let mut total_findings = 0usize;
    let provider = default_symbol_provider();

    for path in files {
        let content = fs::read_to_string(&path).map_err(|err| LintError::ReadError {
            path: path.clone(),
            source: err.to_string(),
        })?;

        let file = match tracked.get(&path).copied() {
            Some(file) => {
                db.set_file_text(file, content.clone());
                file
            }
            None => {
                let file = db.add_file(content.clone());
                tracked.insert(path.clone(), file);
                file
            }
        };

        let parsed = db.parse(file);
        let (status, diagnostics) = if parsed.diagnostics.is_empty() {
            // Re-parse to get a usable rowan tree (the salsa query stores only
            // the debug-formatted CST; the real tree is cheap to rebuild and
            // does not affect cache invalidation).
            let live = parse(&content);
            let root_node = live.cst.clone();
            let model = SemanticModel::build(&root_node);
            let raw = run_rules(&rules.rules, &path, &root_node, &model, &provider);
            let suppress = SuppressionMap::build(&root_node);
            let kept: Vec<Diagnostic> = raw
                .into_iter()
                .map(|mut d| {
                    d.path = path.clone();
                    d
                })
                .filter(|d| !suppress.is_suppressed(d.rule, d.range))
                .collect();
            total_findings += kept.len();
            let status = if kept.is_empty() {
                LintStatus::Clean
            } else {
                LintStatus::Findings { count: kept.len() }
            };
            // Reference Root just to keep AstNode import alive for downstream users.
            let _ = Root::cast(root_node);
            (status, kept)
        } else {
            (
                LintStatus::ParseDiagnostics {
                    count: parsed.diagnostics.len(),
                },
                Vec::new(),
            )
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

/// Convenience: lint a single in-memory document by path + text (used by the LSP).
pub fn check_document(
    path: &Path,
    content: &str,
    config: &LintConfig,
) -> Result<Vec<Diagnostic>, LintError> {
    let (rules, unknown) = ResolvedRules::resolve(config.select.as_deref(), &config.ignore);
    if let Some(rule) = unknown.into_iter().next() {
        return Err(LintError::UnknownRule { rule });
    }
    let parsed = parse(content);
    if !parsed.diagnostics.is_empty() {
        return Ok(Vec::new());
    }
    let root_node = parsed.cst;
    let model = SemanticModel::build(&root_node);
    let provider = default_symbol_provider();
    let suppress = SuppressionMap::build(&root_node);
    let mut diagnostics = run_rules(&rules.rules, path, &root_node, &model, &provider);
    diagnostics.retain(|d| !suppress.is_suppressed(d.rule, d.range));
    for d in &mut diagnostics {
        d.path = path.to_path_buf();
    }
    Ok(diagnostics)
}
