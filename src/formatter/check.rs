use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;

use similar::{DiffTag, group_diff_ops};

use super::FormatStyle;
use super::source::{Formatted, cache_key, format_file, merge};
use crate::file_discovery::{
    DiscoveredFiles, ExcludeFilter, FileDiscoveryError, collect_r_files, collect_source_files,
    is_description_file,
};
use crate::formatter::cache::FormatCache;
use crate::parser::ParseOptions;
use crate::project::description::MarkdownDefaultResolver;
use crate::text::line_diff::bounded_line_diff;
use arity_formatter::formatter::analyze_format_with_options;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    /// Files actually checked: the walk's total, less the failed and the
    /// skipped.
    pub checked_files: usize,
    pub changed_files: Vec<ChangedFile>,
    /// Honored directives that have no effect on formatter output.
    pub outdated_directives: Vec<OutdatedDirective>,
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

/// An honored `# arity-format` directive whose protected source is already
/// formatter-clean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutdatedDirective {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
}

impl OutdatedDirective {
    pub fn write_diagnostic(&self, out: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            out,
            "{}:{}:{}: outdated format directive",
            self.path.display(),
            self.line,
            self.column
        )?;
        writeln!(
            out,
            "  = help: the formatter produces the same output without this directive"
        )
    }
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

impl ChangedFile {
    /// Write the context-grouped, line-numbered diff used by `format --check`.
    /// Diff construction stays deferred until this method is called, so quiet
    /// checks pay no diff cost.
    pub fn write_diff(&self, out: &mut impl io::Write, use_color: bool) -> io::Result<()> {
        const RED: &str = "\x1b[31m";
        const GREEN: &str = "\x1b[32m";
        let diff = bounded_line_diff(&self.original, &self.formatted);
        for (group_index, group) in group_diff_ops(diff.ops().to_vec(), 3).iter().enumerate() {
            if group_index > 0 {
                writeln!(out, "---")?;
            }
            let start = group[0].old_range().start + 1;
            writeln!(out, "Diff in {}:{}:", self.path.display(), start)?;
            for op in group {
                let (tag, old_lines, new_lines) = op.as_tag_tuple();
                match tag {
                    DiffTag::Equal => {
                        write_diff_lines(out, &diff.old_lines()[old_lines], ' ', "", use_color)?;
                    }
                    DiffTag::Delete => {
                        write_diff_lines(out, &diff.old_lines()[old_lines], '-', RED, use_color)?;
                    }
                    DiffTag::Insert => {
                        write_diff_lines(out, &diff.new_lines()[new_lines], '+', GREEN, use_color)?;
                    }
                    DiffTag::Replace => {
                        write_diff_lines(out, &diff.old_lines()[old_lines], '-', RED, use_color)?;
                        write_diff_lines(out, &diff.new_lines()[new_lines], '+', GREEN, use_color)?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn write_diff_lines(
    out: &mut impl io::Write,
    lines: &[&str],
    sign: char,
    color: &str,
    use_color: bool,
) -> io::Result<()> {
    const RESET: &str = "\x1b[0m";
    for value in lines {
        let newline = value.ends_with('\n');
        let line = value.strip_suffix('\n').unwrap_or(value);
        if use_color && !color.is_empty() {
            write!(out, "{color}{sign}{line}{RESET}")?;
        } else {
            write!(out, "{sign}{line}")?;
        }
        if newline {
            writeln!(out)?;
        }
    }
    Ok(())
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
                outdated_directives: Vec::new(),
                failed_files: Vec::new(),
                skipped: Vec::new(),
            });
        }
        return Err(CheckError::NoFiles);
    }

    let total = files.len();
    let mut changed_files = Vec::new();
    let mut outdated_directives = Vec::new();
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

        let (formatted, file_outdated) = if is_description_file(&path) {
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
            (formatted, Vec::new())
        } else {
            let options =
                ParseOptions::default().with_roxygen_markdown_default(markdown.resolve(&path));
            match analyze_format_with_options(&content, style, &options) {
                Ok(analysis) => (analysis.formatted, analysis.outdated_directives),
                Err(err) => {
                    failed_files.push(FailedFile {
                        path,
                        reason: format!("failed to format: {err}"),
                    });
                    continue;
                }
            }
        };
        for range in &file_outdated {
            let start = usize::from(range.start());
            let prefix = &content[..start];
            let line_start = prefix.rfind('\n').map_or(0, |idx| idx + 1);
            outdated_directives.push(OutdatedDirective {
                path: path.clone(),
                line: prefix.bytes().filter(|byte| *byte == b'\n').count() + 1,
                column: content[line_start..start].chars().count() + 1,
            });
        }

        if formatted != content {
            changed_files.push(ChangedFile {
                path,
                original: content,
                formatted,
            });
        } else if file_outdated.is_empty()
            && let Some(c) = cache.as_deref_mut()
        {
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
        outdated_directives,
        failed_files,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_file_writes_context_grouped_diff() {
        let file = ChangedFile {
            path: PathBuf::from("example.R"),
            original: "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk".to_string(),
            formatted: "a\nB\nc\nd\ne\nf\ng\nh\ni\nJ\nk".to_string(),
        };
        let mut out = Vec::new();
        file.write_diff(&mut out, false).expect("write diff");

        assert_eq!(
            String::from_utf8(out).expect("utf-8"),
            "Diff in example.R:1:\n a\n-b\n+B\n c\n d\n e\n---\nDiff in example.R:7:\n g\n h\n i\n-j\n+J\n k"
        );
    }

    #[test]
    fn changed_file_colors_only_changed_lines() {
        let file = ChangedFile {
            path: PathBuf::from("example.R"),
            original: "a\nb\n".to_string(),
            formatted: "a\nB\n".to_string(),
        };
        let mut out = Vec::new();
        file.write_diff(&mut out, true).expect("write diff");

        assert_eq!(
            String::from_utf8(out).expect("utf-8"),
            "Diff in example.R:1:\n a\n\x1b[31m-b\x1b[0m\n\x1b[32m+B\x1b[0m\n"
        );
    }
}
