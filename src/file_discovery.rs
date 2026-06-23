use std::fmt;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDiscoveryError {
    NonRFilePath { path: PathBuf },
    WalkError { path: PathBuf, message: String },
}

/// A compiled set of exclude patterns applied during directory discovery.
///
/// Patterns use gitignore semantics and are resolved relative to a root (the
/// directory containing `arity.toml`, or the working directory when there is no
/// config). The filter prunes matching directories and files from the walk; it
/// does **not** affect paths a user names explicitly on the command line (those
/// are always processed, matching ruff's default, non-`force-exclude` behavior).
#[derive(Debug, Clone)]
pub struct ExcludeFilter {
    matcher: Option<Gitignore>,
}

/// A malformed exclude pattern, surfaced to the CLI so it can report and exit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcludeError {
    pub pattern: String,
    pub message: String,
}

impl fmt::Display for ExcludeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid exclude pattern `{}`: {}",
            self.pattern, self.message
        )
    }
}

impl std::error::Error for ExcludeError {}

impl ExcludeFilter {
    /// A filter that excludes nothing. Used by callers that do their own scoping
    /// (the LSP, salsa-internal sibling discovery) or have no config in hand.
    pub fn none() -> Self {
        Self { matcher: None }
    }

    /// Compile `patterns` into a matcher rooted at `root`. The built-in
    /// [`DEFAULT_EXCLUDE`](crate::config::DEFAULT_EXCLUDE) set is no longer
    /// applied here: it lives as the default
    /// value of the config's `exclude` key, so callers pass the fully-resolved
    /// pattern list (`exclude` + `extend-exclude` + any CLI patterns).
    pub fn new(root: &Path, patterns: &[String]) -> Result<Self, ExcludeError> {
        if patterns.is_empty() {
            return Ok(Self::none());
        }
        let mut builder = GitignoreBuilder::new(root);
        for pattern in patterns.iter().cloned() {
            if let Err(err) = builder.add_line(None, &pattern) {
                return Err(ExcludeError {
                    pattern,
                    message: err.to_string(),
                });
            }
        }
        let matcher = builder.build().map_err(|err| ExcludeError {
            pattern: String::new(),
            message: err.to_string(),
        })?;
        Ok(Self {
            matcher: Some(matcher),
        })
    }

    fn is_excluded(&self, path: &Path, is_dir: bool) -> bool {
        match &self.matcher {
            Some(matcher) => matcher.matched(path, is_dir).is_ignore(),
            None => false,
        }
    }
}

pub fn collect_r_files(
    paths: &[PathBuf],
    exclude: &ExcludeFilter,
) -> Result<Vec<PathBuf>, FileDiscoveryError> {
    let mut files = Vec::new();

    for path in paths {
        if path.is_file() {
            if !is_r_file(path) {
                return Err(FileDiscoveryError::NonRFilePath { path: path.clone() });
            }
            // An explicitly named file is always processed, even if it matches an
            // exclude pattern (no `force-exclude` mode).
            files.push(path.clone());
            continue;
        }

        if path.is_dir() {
            let mut builder = WalkBuilder::new(path);
            builder.standard_filters(true);
            builder.hidden(false);
            // Prune excluded entries during the walk so a matched directory
            // (e.g. `renv/`) is never descended into, matching gitignore
            // semantics. The filter is cloned into the `'static` closure.
            let filter = exclude.clone();
            builder.filter_entry(move |entry| {
                let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());
                !filter.is_excluded(entry.path(), is_dir)
            });
            for entry in builder.build() {
                match entry {
                    Ok(entry) => {
                        let entry_path = entry.path();
                        if entry.file_type().is_some_and(|ft| ft.is_file()) && is_r_file(entry_path)
                        {
                            files.push(entry_path.to_path_buf());
                        }
                    }
                    Err(err) => {
                        return Err(FileDiscoveryError::WalkError {
                            path: path.clone(),
                            message: err.to_string(),
                        });
                    }
                }
            }
            continue;
        }

        return Err(FileDiscoveryError::WalkError {
            path: path.clone(),
            message: "path does not exist".to_string(),
        });
    }

    files.sort();
    files.dedup();
    Ok(files)
}

fn is_r_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("r"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, "x <- 1\n").unwrap();
    }

    /// The built-in default exclude set as owned strings. Callers now resolve
    /// the defaults themselves (they are the default value of the config's
    /// `exclude` key), so the tests pass them in explicitly.
    fn defaults() -> Vec<String> {
        crate::config::DEFAULT_EXCLUDE
            .iter()
            .map(|p| p.to_string())
            .collect()
    }

    #[test]
    fn excludes_default_generated_files() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("keep.R"));
        touch(&root.join("RcppExports.R"));
        touch(&root.join("R").join("import-standalone-types.R"));
        touch(&root.join("renv").join("activate.R"));

        let filter = ExcludeFilter::new(root, &defaults()).unwrap();
        let files = collect_r_files(&[root.to_path_buf()], &filter).unwrap();
        let names: Vec<_> = files
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["keep.R".to_string()]);
    }

    #[test]
    fn extra_patterns_apply_alongside_defaults() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("keep.R"));
        touch(&root.join("vendor").join("thing.R"));

        let mut patterns = defaults();
        patterns.push("vendor/".to_string());
        let filter = ExcludeFilter::new(root, &patterns).unwrap();
        let files = collect_r_files(&[root.to_path_buf()], &filter).unwrap();
        assert_eq!(files, vec![root.join("keep.R")]);
    }

    #[test]
    fn empty_pattern_list_excludes_nothing() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("RcppExports.R"));

        // An empty `exclude` (with no `extend-exclude`) drops the defaults too.
        let filter = ExcludeFilter::new(root, &[]).unwrap();
        let files = collect_r_files(&[root.to_path_buf()], &filter).unwrap();
        assert_eq!(files, vec![root.join("RcppExports.R")]);
    }

    #[test]
    fn explicit_file_is_not_excluded() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let rcpp = root.join("RcppExports.R");
        touch(&rcpp);

        // Named directly, an excluded file is still processed.
        let filter = ExcludeFilter::new(root, &defaults()).unwrap();
        let files = collect_r_files(std::slice::from_ref(&rcpp), &filter).unwrap();
        assert_eq!(files, vec![rcpp]);
    }

    #[test]
    fn none_filter_keeps_everything() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("keep.R"));
        touch(&root.join("RcppExports.R"));
        let files = collect_r_files(&[root.to_path_buf()], &ExcludeFilter::none()).unwrap();
        assert_eq!(files.len(), 2);
    }
}
