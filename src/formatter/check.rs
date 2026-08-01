use std::fmt;
use std::fs;
use std::path::PathBuf;

use super::{FormatError, FormatStyle, format_with_style};
use crate::file_discovery::{ExcludeFilter, FileDiscoveryError, collect_r_files};
use crate::formatter::cache::FormatCache;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub checked_files: usize,
    pub changed_files: Vec<ChangedFile>,
}

/// A file whose formatted output differs from its on-disk contents, carrying
/// both versions so callers can render a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: PathBuf,
    pub original: String,
    pub formatted: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckError {
    MissingPaths,
    NoRFiles,
    NonRFilePath { path: PathBuf },
    WalkError { path: PathBuf, message: String },
    ReadError { path: PathBuf, source: String },
    FormatError { path: PathBuf, source: FormatError },
}

impl fmt::Display for CheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPaths => {
                write!(
                    f,
                    "--check requires at least one input path (file or directory)"
                )
            }
            Self::NoRFiles => {
                write!(f, "no .R files found under the provided input paths")
            }
            Self::NonRFilePath { path } => {
                write!(
                    f,
                    "input file {} is not an .R file; --check only supports .R files",
                    path.display()
                )
            }
            Self::WalkError { path, message } => {
                write!(f, "failed while scanning {}: {message}", path.display())
            }
            Self::ReadError { path, source } => {
                write!(f, "failed to read {}: {source}", path.display())
            }
            Self::FormatError { path, source } => {
                write!(f, "failed to format {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for CheckError {}

impl From<FileDiscoveryError> for CheckError {
    fn from(value: FileDiscoveryError) -> Self {
        match value {
            FileDiscoveryError::NonRFilePath { path } => Self::NonRFilePath { path },
            FileDiscoveryError::WalkError { path, message } => Self::WalkError { path, message },
        }
    }
}

pub fn check_paths(paths: &[PathBuf]) -> Result<CheckResult, CheckError> {
    check_paths_with_style(paths, FormatStyle::default(), &ExcludeFilter::none())
}

pub fn check_paths_with_style(
    paths: &[PathBuf],
    style: FormatStyle,
    exclude: &ExcludeFilter,
) -> Result<CheckResult, CheckError> {
    check_paths_with_style_cached(paths, style, exclude, None)
}

/// Like [`check_paths_with_style`], but consults an optional persistent
/// [`FormatCache`]. A file whose content is a known fixed point (already
/// formatted under this style and arity version) is counted as checked and
/// skipped without parsing; newly-confirmed clean files are recorded. The cache
/// is persisted once, best-effort, before returning.
pub fn check_paths_with_style_cached(
    paths: &[PathBuf],
    style: FormatStyle,
    exclude: &ExcludeFilter,
    mut cache: Option<&mut FormatCache>,
) -> Result<CheckResult, CheckError> {
    if paths.is_empty() {
        return Err(CheckError::MissingPaths);
    }

    let files = collect_r_files(paths, exclude).map_err(CheckError::from)?;
    if files.is_empty() {
        // Under force-exclude every named file may be excluded; that is an
        // expected clean no-op, not an error.
        if exclude.force() {
            return Ok(CheckResult {
                checked_files: 0,
                changed_files: Vec::new(),
            });
        }
        return Err(CheckError::NoRFiles);
    }

    let checked_files = files.len();
    let mut changed_files = Vec::new();

    for path in files {
        let content = fs::read_to_string(&path).map_err(|err| CheckError::ReadError {
            path: path.clone(),
            source: err.to_string(),
        })?;

        // Cache hit: already-formatted, skip parse+format.
        if cache.as_deref().is_some_and(|c| c.is_fixed_point(&content)) {
            continue;
        }

        let formatted =
            format_with_style(&content, style).map_err(|err| CheckError::FormatError {
                path: path.clone(),
                source: err,
            })?;
        if formatted != content {
            changed_files.push(ChangedFile {
                path,
                original: content,
                formatted,
            });
        } else if let Some(c) = cache.as_deref_mut() {
            c.record_fixed_point(&content);
        }
    }

    // Persist best-effort: a cache-write failure must never fail the run.
    if let Some(c) = cache.as_deref()
        && let Err(err) = c.store()
    {
        log::warn!("failed to write format cache: {err}");
    }

    Ok(CheckResult {
        checked_files,
        changed_files,
    })
}
