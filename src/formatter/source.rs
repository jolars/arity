//! Formatting one discovered file, whichever grammar it is written in.
//!
//! `arity format` walks two grammars. The branch between them lives here rather
//! than in each caller so the write path and `--check` cannot drift on which
//! file is formatted how, on the roxygen markdown probe (an R-only cost), or on
//! what a refusal means.

use std::path::{Path, PathBuf};

use arity_formatter::formatter::{DeclineReason, DescriptionFormatError, FormatVerificationError};

use super::{
    FormatError, FormatStyle, format_description_with_style, format_verified_with_options,
    format_with_options,
};
use crate::file_discovery::{DiscoveredFiles, is_description_file};
use crate::formatter::cache::CacheKey;
use crate::parser::ParseOptions;
use crate::project::description::MarkdownDefaultResolver;

/// Every discovered file as one path-sorted work list.
///
/// Merged rather than processed grammar by grammar so diffs, progress, and
/// `--verbose` output all come out in path order.
pub fn merge(files: DiscoveredFiles) -> Vec<PathBuf> {
    let mut all = files.r;
    all.extend(files.description);
    all.sort();
    all
}

/// What formatting one file produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Formatted {
    Text(String),
    /// Valid input the formatter deliberately left alone. Not a failure: the
    /// file is counted as checked and unchanged.
    Declined(DeclineReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatSourceError {
    R(FormatError),
    RVerification(FormatVerificationError),
    Description(DescriptionFormatError),
    DescriptionVerification(String),
}

impl std::fmt::Display for FormatSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::R(err) => write!(f, "{err}"),
            Self::RVerification(err) => write!(f, "{err}"),
            Self::Description(err) => write!(f, "{err}"),
            Self::DescriptionVerification(message) => f.write_str(message),
        }
    }
}

/// Format one file and verify every invariant supported by its grammar.
///
/// R checks normalized syntax, ordinary comments, and idempotence. DESCRIPTION
/// retains its existing idempotence-only contract; its structural meaning has
/// a separate corpus oracle.
pub fn format_file_verified(
    path: &Path,
    content: &str,
    style: FormatStyle,
    markdown: &mut MarkdownDefaultResolver,
) -> Result<Formatted, FormatSourceError> {
    if is_description_file(path) {
        let formatted = match format_description_with_style(content, style) {
            Ok(text) => text,
            Err(DescriptionFormatError::Declined(reason)) => {
                return Ok(Formatted::Declined(reason));
            }
            Err(err) => return Err(FormatSourceError::Description(err)),
        };
        match format_description_with_style(&formatted, style) {
            Ok(reformatted) if reformatted == formatted => Ok(Formatted::Text(formatted)),
            Ok(_) => Err(FormatSourceError::DescriptionVerification(
                "formatter verification failed (non-idempotent output)".to_string(),
            )),
            Err(err) => Err(FormatSourceError::DescriptionVerification(format!(
                "formatted output failed verification: {err}"
            ))),
        }
    } else {
        let options = ParseOptions::default().with_roxygen_markdown_default(markdown.resolve(path));
        format_verified_with_options(content, style, &options)
            .map(Formatted::Text)
            .map_err(FormatSourceError::RVerification)
    }
}

impl std::error::Error for FormatSourceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::R(err) => Some(err),
            Self::RVerification(err) => Some(err),
            Self::Description(err) => Some(err),
            Self::DescriptionVerification(_) => None,
        }
    }
}

/// Format `content`, choosing the grammar from `path`.
///
/// `markdown` is consulted only for R: the package-wide roxygen markdown default
/// costs a directory probe and means nothing to DCF.
pub fn format_file(
    path: &Path,
    content: &str,
    style: FormatStyle,
    markdown: &mut MarkdownDefaultResolver,
) -> Result<Formatted, FormatSourceError> {
    if is_description_file(path) {
        return match format_description_with_style(content, style) {
            Ok(text) => Ok(Formatted::Text(text)),
            Err(DescriptionFormatError::Declined(reason)) => Ok(Formatted::Declined(reason)),
            Err(err) => Err(FormatSourceError::Description(err)),
        };
    }

    let options = ParseOptions::default().with_roxygen_markdown_default(markdown.resolve(path));
    format_with_options(content, style, &options)
        .map(Formatted::Text)
        .map_err(FormatSourceError::R)
}

/// The format-cache key for `content` at `path`.
///
/// Takes [`format_file`]'s grammar branch rather than repeating it, so a key can
/// never name a grammar the formatter did not use — a cross-grammar hit would
/// report a dirty `DESCRIPTION` clean. Resolving the roxygen markdown default
/// here also keeps the directory probe off the DCF path, where it means nothing.
pub fn cache_key<'a>(
    path: &Path,
    content: &'a str,
    markdown: &mut MarkdownDefaultResolver,
) -> CacheKey<'a> {
    if is_description_file(path) {
        return CacheKey::dcf(content);
    }
    CacheKey::r(content, markdown.resolve(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_description_path_selects_the_dcf_grammar() {
        let mut markdown = MarkdownDefaultResolver::new();
        // Valid as R *and* as DCF, so only the path can decide.
        let formatted = format_file(
            Path::new("pkg/DESCRIPTION"),
            "Package: p\nImports: b, a\n",
            FormatStyle::default(),
            &mut markdown,
        )
        .expect("formats");
        assert_eq!(
            formatted,
            Formatted::Text("Package: p\nImports:\n    a,\n    b\n".to_string())
        );
    }

    #[test]
    fn a_declined_description_is_not_a_failure() {
        let mut markdown = MarkdownDefaultResolver::new();
        let formatted = format_file(
            Path::new("pkg/DESCRIPTION"),
            "Package: p\n\nPackage: q\n",
            FormatStyle::default(),
            &mut markdown,
        )
        .expect("declines without erroring");
        assert!(matches!(
            formatted,
            Formatted::Declined(DeclineReason::MultipleRecords { .. })
        ));
    }

    #[test]
    fn a_malformed_description_is_a_failure() {
        let mut markdown = MarkdownDefaultResolver::new();
        let err = format_file(
            Path::new("pkg/DESCRIPTION"),
            "Package: p\ngarbage\n",
            FormatStyle::default(),
            &mut markdown,
        )
        .expect_err("errors");
        assert!(matches!(err, FormatSourceError::Description(_)));
    }

    #[test]
    fn verified_r_format_accepts_body_brace_normalization() {
        let mut markdown = MarkdownDefaultResolver::new();
        let formatted = format_file_verified(
            Path::new("pkg/R/code.R"),
            "if (x) y else z\n",
            FormatStyle::default(),
            &mut markdown,
        )
        .expect("formats and verifies");
        assert_eq!(
            formatted,
            Formatted::Text("if (x) {\n  y\n} else {\n  z\n}\n".to_string())
        );
    }

    #[test]
    fn merge_interleaves_both_grammars_by_path() {
        let merged = merge(DiscoveredFiles {
            r: vec![PathBuf::from("pkg/R/z.R"), PathBuf::from("pkg/R/a.R")],
            description: vec![PathBuf::from("pkg/DESCRIPTION")],
        });
        assert_eq!(
            merged,
            vec![
                PathBuf::from("pkg/DESCRIPTION"),
                PathBuf::from("pkg/R/a.R"),
                PathBuf::from("pkg/R/z.R"),
            ]
        );
    }
}
