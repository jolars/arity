use std::fmt;
use std::fs;
use std::path::PathBuf;

use super::{FormatError, FormatStyle, format_with_style};
use crate::file_discovery::{ExcludeFilter, FileDiscoveryError, collect_r_files};

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
    if paths.is_empty() {
        return Err(CheckError::MissingPaths);
    }

    let files = collect_r_files(paths, exclude).map_err(CheckError::from)?;
    if files.is_empty() {
        return Err(CheckError::NoRFiles);
    }

    let checked_files = files.len();
    let mut changed_files = Vec::new();

    for path in files {
        let content = fs::read_to_string(&path).map_err(|err| CheckError::ReadError {
            path: path.clone(),
            source: err.to_string(),
        })?;

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
        }
    }

    Ok(CheckResult {
        checked_files,
        changed_files,
    })
}
