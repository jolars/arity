//! Config-relative glob matches that disable individual lint rules.

use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};
use serde::Deserialize;

use crate::file_discovery::root_spellings;
use crate::incremental::normalize_path;

/// A validated, compiled mapping from file globs to ignored rule IDs.
///
/// Construct with [`TryFrom`] or deserialize a TOML table. Standalone values
/// use the working directory; loading a [`super::Config`] anchors them to the
/// selected config file's directory, including inherited entries.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(try_from = "BTreeMap<String, Vec<String>>")]
pub struct PerFileIgnores {
    patterns: BTreeMap<String, Vec<String>>,
    matchers: Vec<Pattern>,
    roots: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct Pattern {
    matcher: GlobMatcher,
    basename: bool,
    absolute: bool,
    negated: bool,
}

impl PartialEq for PerFileIgnores {
    fn eq(&self, other: &Self) -> bool {
        self.patterns == other.patterns && self.roots == other.roots
    }
}

impl Eq for PerFileIgnores {}

impl TryFrom<BTreeMap<String, Vec<String>>> for PerFileIgnores {
    type Error = String;

    fn try_from(patterns: BTreeMap<String, Vec<String>>) -> Result<Self, Self::Error> {
        let matchers = patterns
            .keys()
            .map(|pattern| {
                let (glob, negated) = match pattern.strip_prefix('!') {
                    Some(glob) => (glob, true),
                    None => (pattern.as_str(), false),
                };
                let matcher = GlobBuilder::new(glob)
                    .literal_separator(true)
                    .build()
                    .map_err(|error| {
                        format!("invalid `lint.per-file-ignores` pattern `{pattern}`: {error}")
                    })?
                    .compile_matcher();
                Ok(Pattern {
                    matcher,
                    basename: !glob.contains('/'),
                    absolute: Path::new(glob).is_absolute(),
                    negated,
                })
            })
            .collect::<Result<_, String>>()?;
        let mut ignores = Self {
            patterns,
            matchers,
            roots: Vec::new(),
        };
        ignores.set_root(Path::new("."));
        Ok(ignores)
    }
}

impl PerFileIgnores {
    pub(super) fn set_root(&mut self, root: &Path) {
        if !self.patterns.is_empty() {
            self.roots = root_spellings(&normalize_matching_path(root));
        }
    }

    pub(crate) fn rule_ids(&self) -> impl Iterator<Item = &String> {
        self.patterns.values().flatten()
    }

    /// Union every matching entry. Negation complements a pattern's match;
    /// it never undoes another entry's ignores.
    pub(crate) fn ignored_rules(&self, path: &Path) -> HashSet<&str> {
        let mut ignored = HashSet::new();
        if self.patterns.is_empty() || path == Path::new("-") {
            return ignored;
        }
        let path = normalize_matching_path(path);
        let relative = self.relative_path(&path);
        // Config discovery can canonicalize a directory that the CLI or editor
        // names through a symlink. Resolve aliases only when lexical matching
        // cannot relate the candidate to the config directory. Looking up an
        // ancestor also covers new files that have never been saved.
        let canonical = if relative.is_none() {
            canonicalize_with_missing_tail(&path)
        } else {
            None
        };
        let relative =
            relative.or_else(|| canonical.as_deref().and_then(|p| self.relative_path(p)));
        for (pattern, rules) in self.matchers.iter().zip(self.patterns.values()) {
            let matches = if pattern.basename {
                path.file_name()
                    .is_some_and(|name| pattern.matcher.is_match(Path::new(name)))
            } else if pattern.absolute {
                pattern.matcher.is_match(&path)
            } else {
                relative.is_some_and(|p| pattern.matcher.is_match(p))
            };
            if matches != pattern.negated {
                ignored.extend(rules.iter().map(String::as_str));
            }
        }
        ignored
    }

    fn relative_path<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        self.roots
            .iter()
            .find_map(|root| path.strip_prefix(root).ok())
    }
}

/// A symlink followed by `..` must resolve before lexical normalization, even
/// when the path's spelling is already under the config root. Other paths keep
/// their spelling so globs can still match symlink names without filesystem I/O.
fn normalize_matching_path(path: &Path) -> PathBuf {
    if path.components().any(|part| part == Component::ParentDir)
        && let Some(resolved) = canonicalize_with_missing_tail(path)
    {
        return resolved;
    }
    normalize_path(path)
}

/// Resolve the original path's deepest existing ancestor, retaining an unsaved
/// file's missing tail. Only then is it safe to collapse parent components.
fn canonicalize_with_missing_tail(path: &Path) -> Option<PathBuf> {
    path.ancestors().find_map(|ancestor| {
        let root = ancestor.canonicalize().ok()?;
        let tail = path.strip_prefix(ancestor).ok()?;
        Some(normalize_path(&root.join(tail)))
    })
}
