use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;

use super::FormatStyle;
use super::source::{Formatted, cache_key, format_file, merge};
use crate::file_discovery::{
    DiscoveredFiles, ExcludeFilter, FileDiscoveryError, collect_r_files, collect_source_files,
};
use crate::formatter::cache::FormatCache;
use crate::project::description::MarkdownDefaultResolver;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    /// Files actually checked: the walk's total, less the failed and the
    /// skipped.
    pub checked_files: usize,
    pub changed_files: Vec<ChangedFile>,
    /// Files the run could not check. Collected rather than returned, so one
    /// unreadable `DESCRIPTION` at a package root does not decide whether the
    /// project's `.R` files get checked at all — `merge` sorts by path, and a
    /// package's `DESCRIPTION` sorts before its `R/`, so returning here would
    /// reliably preempt everything the user actually asked about.
    pub failed_files: Vec<FailedFile>,
    /// Files whose bytes are not UTF-8. Skipped, not failed — the same answer
    /// `arity lint` gives for the same file, and a file arity cannot decode is
    /// not a file it can have an opinion about.
    pub skipped: Vec<PathBuf>,
}

/// A file the run could not check, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedFile {
    pub path: PathBuf,
    pub reason: String,
}

impl fmt::Display for FailedFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.reason)
    }
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
    NoFiles,
    UnsupportedFilePath { path: PathBuf },
    WalkError { path: PathBuf, message: String },
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
            Self::NoFiles => {
                write!(
                    f,
                    "no .R files or DESCRIPTION files found under the provided input paths"
                )
            }
            Self::UnsupportedFilePath { path } => {
                write!(
                    f,
                    "input file {} is not formattable; --check supports .R files and DESCRIPTION",
                    path.display()
                )
            }
            Self::WalkError { path, message } => {
                write!(f, "failed while scanning {}: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for CheckError {}

impl From<FileDiscoveryError> for CheckError {
    fn from(value: FileDiscoveryError) -> Self {
        match value {
            FileDiscoveryError::UnsupportedFilePath { path } => Self::UnsupportedFilePath { path },
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
    check_paths_with_style_cached(paths, style, exclude, true, None)
}

/// Like [`check_paths_with_style`], but consults an optional persistent
/// [`FormatCache`]. A file whose content is a known fixed point (already
/// formatted under this style and arity version) is counted as checked and
/// skipped without parsing; newly-confirmed clean files are recorded. The cache
/// is persisted once, best-effort, before returning.
///
/// `descriptions` is the `[format] description` config key. It is consumed here,
/// at discovery, so nothing downstream has to carry it: with the key off, a
/// `DESCRIPTION` is simply not one of the files this run is about.
///
/// A per-file problem is reported in [`CheckResult`], never returned: `Err` is
/// reserved for what makes the whole run meaningless (nothing to check, a walk
/// that failed). One file arity cannot read or parse must not decide the verdict
/// on every other.
pub fn check_paths_with_style_cached(
    paths: &[PathBuf],
    style: FormatStyle,
    exclude: &ExcludeFilter,
    descriptions: bool,
    mut cache: Option<&mut FormatCache>,
) -> Result<CheckResult, CheckError> {
    if paths.is_empty() {
        return Err(CheckError::MissingPaths);
    }

    let discovered = if descriptions {
        collect_source_files(paths, exclude).map_err(CheckError::from)?
    } else {
        DiscoveredFiles {
            r: collect_r_files(paths, exclude).map_err(CheckError::from)?,
            description: Vec::new(),
        }
    };
    let files = merge(discovered);
    if files.is_empty() {
        // Under force-exclude every named file may be excluded; that is an
        // expected clean no-op, not an error.
        if exclude.force() {
            return Ok(CheckResult {
                checked_files: 0,
                changed_files: Vec::new(),
                failed_files: Vec::new(),
                skipped: Vec::new(),
            });
        }
        return Err(CheckError::NoFiles);
    }

    let total = files.len();
    let mut changed_files = Vec::new();
    let mut failed_files = Vec::new();
    let mut skipped = Vec::new();
    // The package-wide roxygen markdown default, per file (memoized per
    // directory): a markdown-first package's doc comments parse — and so
    // format — in markdown mode without any per-block `@md`.
    let mut markdown = MarkdownDefaultResolver::new();

    for path in files {
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(err) if err.kind() == io::ErrorKind::InvalidData => {
                skipped.push(path);
                continue;
            }
            Err(err) => {
                failed_files.push(FailedFile {
                    path,
                    reason: format!("failed to read: {err}"),
                });
                continue;
            }
        };

        // Cache hit: already-formatted, skip parse+format. `cache_key` shares
        // `format_file`'s grammar branch, so a byte string that is a fixed point
        // of both grammars can never cross over.
        if cache
            .as_deref()
            .is_some_and(|c| c.is_fixed_point(cache_key(&path, &content, &mut markdown)))
        {
            continue;
        }

        let formatted = match format_file(&path, &content, style, &mut markdown) {
            Ok(Formatted::Text(formatted)) => formatted,
            // Valid input the formatter left alone: checked, unchanged, and not
            // worth caching (the decline is cheap to re-derive).
            Ok(Formatted::Declined(_)) => continue,
            Err(err) => {
                failed_files.push(FailedFile {
                    path,
                    reason: format!("failed to format: {err}"),
                });
                continue;
            }
        };

        if formatted != content {
            changed_files.push(ChangedFile {
                path,
                original: content,
                formatted,
            });
        } else if let Some(c) = cache.as_deref_mut() {
            c.record_fixed_point(cache_key(&path, &content, &mut markdown));
        }
    }

    // Persist best-effort: a cache-write failure must never fail the run.
    if let Some(c) = cache.as_deref()
        && let Err(err) = c.store()
    {
        log::warn!("failed to write format cache: {err}");
    }

    Ok(CheckResult {
        checked_files: total - failed_files.len() - skipped.len(),
        changed_files,
        failed_files,
        skipped,
    })
}
