use std::fmt;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::project::is_package_root;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDiscoveryError {
    UnsupportedFilePath { path: PathBuf },
    WalkError { path: PathBuf, message: String },
}

/// A compiled set of exclude patterns applied during directory discovery.
///
/// Patterns use gitignore semantics and are resolved relative to a root (the
/// directory containing `arity.toml`, or the working directory when there is no
/// config). The filter prunes matching directories and files from the walk; by
/// default it does **not** affect paths a user names explicitly on the command
/// line (those are always processed, matching ruff's default behavior). With
/// `force` set (the `--force-exclude` flag), explicitly named files that match
/// a pattern are skipped too — for runners like pre-commit that pass staged
/// files as arguments.
#[derive(Debug, Clone)]
pub struct ExcludeFilter {
    matcher: Option<Gitignore>,
    /// Every spelling of the pattern root a candidate path might be written
    /// with. See [`ExcludeFilter::relativize`].
    roots: Vec<PathBuf>,
    force: bool,
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
        Self {
            matcher: None,
            roots: Vec::new(),
            force: false,
        }
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
            roots: root_spellings(root),
            force: false,
        })
    }

    /// Also apply the patterns to explicitly named files (`--force-exclude`).
    pub fn with_force_exclude(mut self, force: bool) -> Self {
        self.force = force;
        self
    }

    /// Whether explicitly named files are subject to the patterns.
    pub fn force(&self) -> bool {
        self.force
    }

    /// Whether `path`, named explicitly on the command line, should be skipped.
    /// Always `false` without force mode. Unlike the walk (where pruning an
    /// excluded directory hides everything beneath it), an explicit file must
    /// be tested against its ancestors too, so `renv/` catches
    /// `renv/activate.R`.
    pub fn force_excludes(&self, path: &Path) -> bool {
        if !self.force {
            return false;
        }
        match &self.matcher {
            Some(matcher) => {
                let path = self.relativize(path);
                // `matched_path_or_any_parents` asserts that `path`, after
                // stripping the matcher root, has no root component left; a
                // rooted path outside the matcher root cannot match its
                // root-relative patterns anyway. `has_root` rather than
                // `is_absolute`: on Windows a driveless path like `\foo` is
                // rooted but not absolute, and would still trip the assert.
                if path.has_root() {
                    return false;
                }
                matcher.matched_path_or_any_parents(path, false).is_ignore()
            }
            None => false,
        }
    }

    fn is_excluded(&self, path: &Path, is_dir: bool) -> bool {
        match &self.matcher {
            Some(matcher) => matcher.matched(self.relativize(path), is_dir).is_ignore(),
            None => false,
        }
    }

    /// `path` re-expressed relative to the pattern root.
    ///
    /// [`Gitignore`] strips its root from a candidate **textually**, so a path
    /// naming the same directory through a different spelling strips nothing and
    /// every anchored pattern (`tests/fixtures/`, anything with a `/`) silently
    /// stops matching. That is not a corner case: config discovery
    /// canonicalizes, walk paths come from the command line, and on Windows
    /// `canonicalize` returns a `\\?\`-verbatim path that nothing else in the
    /// process produces — so *every* absolute walk path missed.
    ///
    /// Covers the spellings [`root_spellings`] can **derive from the root**. The
    /// reverse — a candidate reached through a symlink the root does not name —
    /// would need a `canonicalize` per entry, which is a syscall per file to fix
    /// a case that needs an explicitly symlinked path on the command line.
    ///
    /// Relativizing here rather than teaching the matcher about spellings keeps
    /// this in one place: what a pattern is anchored to is this filter's
    /// business, not `ignore`'s.
    fn relativize<'a>(&self, path: &'a Path) -> &'a Path {
        for root in &self.roots {
            if let Ok(relative) = path.strip_prefix(root) {
                return relative;
            }
        }
        path
    }
}

/// Every spelling of `root` a candidate path might arrive written with: as
/// given, canonicalized (symlinks resolved), and each with any Windows verbatim
/// prefix removed. Computed once, at construction, so matching stays syscall-free.
fn root_spellings(root: &Path) -> Vec<PathBuf> {
    let mut roots = vec![root.to_path_buf()];
    if let Ok(canonical) = root.canonicalize() {
        roots.push(canonical);
    }
    for index in 0..roots.len() {
        if let Some(simplified) = strip_verbatim_prefix(&roots[index]) {
            roots.push(simplified);
        }
    }
    roots.dedup();
    roots
}

/// `\\?\D:\pkg` as `D:\pkg`, and `\\?\UNC\server\share` as `\\server\share`.
///
/// `Path::canonicalize` returns the verbatim form on Windows; no other path in
/// the process is written that way, so it has to be matchable against both.
/// `None` when there is no verbatim prefix to remove.
fn strip_verbatim_prefix(path: &Path) -> Option<PathBuf> {
    let rest = path.to_str()?.strip_prefix(r"\\?\")?;
    match rest.strip_prefix(r"UNC\") {
        Some(share) => Some(PathBuf::from(format!(r"\\{share}"))),
        None => Some(PathBuf::from(rest)),
    }
}

/// The files one run will process, split by grammar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoveredFiles {
    pub r: Vec<PathBuf>,
    /// Package `DESCRIPTION`s — see [`collect_source_files`] for which ones a walk
    /// picks up.
    pub description: Vec<PathBuf>,
}

/// Discover the `.R` files under `paths`. The R-only view, for the callers that
/// want exactly that: `arity index`, the lint driver's project-scope seeding,
/// and `lint --fix`.
pub fn collect_r_files(
    paths: &[PathBuf],
    exclude: &ExcludeFilter,
) -> Result<Vec<PathBuf>, FileDiscoveryError> {
    collect(paths, exclude, false).map(|files| files.r)
}

/// Discover both grammars' inputs under `paths` — what `arity lint` and
/// `arity format` each process.
///
/// A **walk** picks up a `DESCRIPTION` only when its directory is a package root
/// ([`is_package_root`]) that is not itself inside another package. The first
/// half skips an `inst/extdata/DESCRIPTION` fixture, a vendored copy, or a test
/// corpus's scraped metadata; the second skips a *complete* fake package under
/// `tests/`, which is fixture data for a test rather than anybody's package
/// metadata — the same reason `undeclared-dependency` looks only at `R/`.
///
/// That gate matters more to `format` than it did to `lint`, and for a different
/// reason: a fixture package's `DESCRIPTION` is often deliberately malformed and
/// asserted on byte for byte by its own project's tests. Linting one wastes the
/// reader's time; rewriting one breaks their suite.
///
/// An **explicitly named** `DESCRIPTION` is always accepted, matching how an
/// explicitly named excluded `.R` file is still processed. The user typed the
/// path; that is consent.
pub fn collect_source_files(
    paths: &[PathBuf],
    exclude: &ExcludeFilter,
) -> Result<DiscoveredFiles, FileDiscoveryError> {
    collect(paths, exclude, true)
}

fn collect(
    paths: &[PathBuf],
    exclude: &ExcludeFilter,
    descriptions: bool,
) -> Result<DiscoveredFiles, FileDiscoveryError> {
    let mut files = Vec::new();
    let mut found_descriptions = Vec::new();

    for path in paths {
        if path.is_file() {
            // The force check runs before the extension check so that an
            // excluded non-R file (as a runner like pre-commit may stage) is
            // silently skipped rather than a hard error.
            if exclude.force_excludes(path) {
                continue;
            }
            if is_r_file(path) {
                files.push(path.clone());
                continue;
            }
            if descriptions && is_description_file(path) {
                found_descriptions.push(path.clone());
                continue;
            }
            return Err(FileDiscoveryError::UnsupportedFilePath { path: path.clone() });
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
                        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                            continue;
                        }
                        if is_r_file(entry_path) {
                            files.push(entry_path.to_path_buf());
                        } else if descriptions
                            && is_description_file(entry_path)
                            && entry_path.parent().is_some_and(is_own_package_root)
                        {
                            found_descriptions.push(entry_path.to_path_buf());
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
    found_descriptions.sort();
    found_descriptions.dedup();
    Ok(DiscoveredFiles {
        r: files,
        description: found_descriptions,
    })
}

fn is_r_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("r"))
}

/// The one file name R reads package metadata from.
pub const DESCRIPTION_FILE_NAME: &str = "DESCRIPTION";

/// Whether `path` is named `DESCRIPTION`. Case-sensitive: R reads that exact
/// name, so `description` is an unrelated file.
///
/// The single path-to-grammar classifier. The language server's `DocumentKind`
/// goes through it too, so a path can never be an R file to one half of the
/// codebase and a `DESCRIPTION` to the other.
pub fn is_description_file(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name == DESCRIPTION_FILE_NAME)
}

/// Whether `dir` is a package root in its own right, rather than a package
/// nested inside another one.
///
/// A package under a package is a *fixture* — roxygen2, devtools, and pkgbuild
/// all keep whole miniature packages under `tests/testthat/` — and its
/// `DESCRIPTION` is data for a test, deliberately minimal, describing nothing
/// anybody ships. Linting it produces a screenful of findings about files whose
/// author was never addressing us. Naming one explicitly still lints it.
pub(crate) fn is_own_package_root(dir: &Path) -> bool {
    // `package_root` starts one level up, so this asks "is any *ancestor* a
    // package root", not "is this one".
    is_package_root(dir) && crate::project::package_root(dir).is_none()
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

    #[test]
    fn force_exclude_skips_explicitly_named_file() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let rcpp = root.join("RcppExports.R");
        let keep = root.join("keep.R");
        touch(&rcpp);
        touch(&keep);

        let filter = ExcludeFilter::new(root, &defaults())
            .unwrap()
            .with_force_exclude(true);
        let files = collect_r_files(&[rcpp, keep.clone()], &filter).unwrap();
        assert_eq!(files, vec![keep]);
    }

    #[test]
    fn force_exclude_may_leave_no_files() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let rcpp = root.join("RcppExports.R");
        touch(&rcpp);

        // Every explicit input is excluded: an empty result, not an error.
        let filter = ExcludeFilter::new(root, &defaults())
            .unwrap()
            .with_force_exclude(true);
        let files = collect_r_files(&[rcpp], &filter).unwrap();
        assert_eq!(files, Vec::<PathBuf>::new());
    }

    #[test]
    fn force_exclude_matches_parent_directory_pattern() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let activate = root.join("renv").join("activate.R");
        touch(&activate);

        // The walk prunes `renv/` before its contents are seen; an explicit
        // file must be tested against its ancestors to match the same pattern.
        let filter = ExcludeFilter::new(root, &defaults())
            .unwrap()
            .with_force_exclude(true);
        let files = collect_r_files(&[activate], &filter).unwrap();
        assert_eq!(files, Vec::<PathBuf>::new());
    }

    #[test]
    fn force_exclude_skips_excluded_non_r_file() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let settings = root.join("renv").join("settings.json");
        touch(&settings);

        // The force check runs before the extension check, so an excluded
        // non-R file (as pre-commit may stage) is skipped, not a hard error.
        let filter = ExcludeFilter::new(root, &defaults())
            .unwrap()
            .with_force_exclude(true);
        let files = collect_r_files(&[settings], &filter).unwrap();
        assert_eq!(files, Vec::<PathBuf>::new());
    }

    #[test]
    fn force_exclude_ignores_paths_outside_matcher_root() {
        let dir = tempdir().unwrap();
        let other = tempdir().unwrap();
        let outside = other.path().join("RcppExports.R");
        touch(&outside);

        // An absolute path outside the matcher root cannot match root-relative
        // patterns; it must be processed, not skipped (and must not panic).
        let filter = ExcludeFilter::new(dir.path(), &defaults())
            .unwrap()
            .with_force_exclude(true);
        let files = collect_r_files(std::slice::from_ref(&outside), &filter).unwrap();
        assert_eq!(files, vec![outside]);
    }

    #[test]
    fn force_exclude_ignores_rooted_paths_outside_matcher_root() {
        // Built from literal rooted paths rather than tempdirs: on Windows
        // `/elsewhere/...` is rooted but not absolute (no drive letter), which
        // an `is_absolute` guard misses, panicking inside the `ignore` crate.
        let filter = ExcludeFilter::new(Path::new("/project"), &defaults())
            .unwrap()
            .with_force_exclude(true);
        assert!(!filter.force_excludes(Path::new("/elsewhere/RcppExports.R")));
        assert!(filter.force_excludes(Path::new("/project/RcppExports.R")));
    }

    #[test]
    fn force_exclude_does_not_change_directory_walk() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("keep.R"));
        touch(&root.join("RcppExports.R"));
        touch(&root.join("renv").join("activate.R"));

        let filter = ExcludeFilter::new(root, &defaults()).unwrap();
        let walked = collect_r_files(&[root.to_path_buf()], &filter).unwrap();
        let forced =
            collect_r_files(&[root.to_path_buf()], &filter.with_force_exclude(true)).unwrap();
        assert_eq!(walked, forced);
    }

    /// An anchored pattern must hold when the walk spells the root differently
    /// from the matcher.
    ///
    /// The two routinely differ, because config discovery canonicalizes while
    /// walk paths come from the command line. `Gitignore` strips its root
    /// textually, so without [`ExcludeFilter::relativize`] every pattern
    /// containing a `/` silently stops matching — this repo's own
    /// `tests/fixtures/` among them, which is what broke on Windows, where
    /// `canonicalize` returns a `\\?\` verbatim path and nothing else does.
    ///
    /// Modeled here with a symlink, the one spelling difference Unix has.
    #[test]
    #[cfg(unix)]
    fn an_anchored_pattern_holds_through_a_differently_spelled_root() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap().keep();
        let real = dir.join("real");
        touch(&real.join("keep.R"));
        touch(&real.join("tests").join("fixtures").join("skip.R"));
        let link = dir.join("link");
        symlink(&real, &link).unwrap();

        // Rooted at the symlinked spelling; the walk reports the real one.
        let filter = ExcludeFilter::new(&link, &["tests/fixtures/".to_string()]).unwrap();
        let files = collect_r_files(std::slice::from_ref(&real), &filter).unwrap();
        assert_eq!(files, vec![real.join("keep.R")]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_verbatim_root_prefix_is_matchable_without_it() {
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\D:\pkg")),
            Some(PathBuf::from(r"D:\pkg"))
        );
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\UNC\server\share")),
            Some(PathBuf::from(r"\\server\share"))
        );
        assert_eq!(strip_verbatim_prefix(Path::new("/project")), None);
    }
}
