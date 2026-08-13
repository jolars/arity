use std::fmt;
use std::fs;
use std::path::PathBuf;

use super::FormatStyle;
use super::source::{FormatSourceError, Formatted, format_file, merge};
use crate::file_discovery::{
    DiscoveredFiles, ExcludeFilter, FileDiscoveryError, collect_r_files, collect_source_files,
    is_description_file,
};
use crate::formatter::cache::{CacheKey, FormatCache};
use crate::project::description::MarkdownDefaultResolver;

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
    NoFiles,
    UnsupportedFilePath {
        path: PathBuf,
    },
    WalkError {
        path: PathBuf,
        message: String,
    },
    ReadError {
        path: PathBuf,
        source: String,
    },
    FormatError {
        path: PathBuf,
        source: FormatSourceError,
    },
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
            });
        }
        return Err(CheckError::NoFiles);
    }

    let checked_files = files.len();
    let mut changed_files = Vec::new();
    // The package-wide roxygen markdown default, per file (memoized per
    // directory): a markdown-first package's doc comments parse — and so
    // format — in markdown mode without any per-block `@md`.
    let mut markdown = MarkdownDefaultResolver::new();

    for path in files {
        let content = fs::read_to_string(&path).map_err(|err| CheckError::ReadError {
            path: path.clone(),
            source: err.to_string(),
        })?;
        // The cache key names the grammar, so a byte string that is a fixed
        // point of both can never cross over. Resolving the markdown default
        // here keeps the directory probe off the DCF path entirely.
        let roxygen_markdown = (!is_description_file(&path)).then(|| markdown.resolve(&path));
        fn key<'a>(content: &'a str, roxygen_markdown: Option<bool>) -> CacheKey<'a> {
            match roxygen_markdown {
                Some(md) => CacheKey::r(content, md),
                None => CacheKey::dcf(content),
            }
        }

        // Cache hit: already-formatted, skip parse+format.
        if cache
            .as_deref()
            .is_some_and(|c| c.is_fixed_point(key(&content, roxygen_markdown)))
        {
            continue;
        }

        let formatted = match format_file(&path, &content, style, &mut markdown) {
            Ok(Formatted::Text(formatted)) => formatted,
            // Valid input the formatter left alone: checked, unchanged, and not
            // worth caching (the decline is cheap to re-derive).
            Ok(Formatted::Declined(_)) => continue,
            Err(err) => {
                return Err(CheckError::FormatError {
                    path: path.clone(),
                    source: err,
                });
            }
        };

        if formatted != content {
            changed_files.push(ChangedFile {
                path,
                original: content,
                formatted,
            });
        } else if let Some(c) = cache.as_deref_mut() {
            c.record_fixed_point(key(&content, roxygen_markdown));
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
