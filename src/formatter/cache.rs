//! Persistent "already-formatted" cache for `arity format --check`.
//!
//! The cache is a set of hashes naming *fixed points* of the formatter under a
//! given style and arity version: contents for which `format(content) ==
//! content`. A cache hit means "already formatted", so the file can be counted
//! as unchanged without parsing or formatting it.
//!
//! A hash keys a [`CacheKey`], not bare content — the formatter serves two
//! grammars, and some byte strings are fixed points of both.
//!
//! Formatter behavior depends on both the arity version and the [`FormatStyle`],
//! so both are baked into the on-disk path; a stale, missing, or corrupt file
//! resets to an empty cache rather than failing (Tenet: the cache is a
//! disposable optimization, never a source of errors).
//!
//! Layout under the cache root:
//!
//! ```text
//! {root}/format/{arity_version}/{style_hash}.postcard
//! ```
//!
//! On a successful write, sibling `{arity_version}/` directories from other
//! versions are garbage-collected. Content hashing uses the standard-library
//! [`DefaultHasher`](std::collections::hash_map::DefaultHasher): its output is
//! not guaranteed stable across Rust releases, but that only ever causes a miss
//! (a rebuild), never a wrong answer, and the arity version is already in the
//! path. A 64-bit hash collision could in principle skip a file that needs
//! reformatting; the probability is negligible for realistic file counts, and
//! `arity format` (write mode) still re-formats from scratch.

use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::formatter::FormatStyle;
use crate::rindex::cache::{CacheError, atomic_write};

/// Bump when the on-disk [`CacheFile`] shape changes within a release cycle.
///
/// Deliberately **not** covering the hash function. Changing how a key is hashed
/// produces misses, never wrong answers, and [`cache_path`] already embeds the
/// arity version — so a binary that hashes differently never reads an older
/// binary's file in the first place.
const SCHEMA_VERSION: u32 = 1;

const ARITY_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The on-disk payload: a schema tag plus the set of fixed-point content hashes.
#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    schema_version: u32,
    fixed_points: HashSet<u64>,
}

/// A loaded format cache scoped to one `(arity version, style)` pair.
pub struct FormatCache {
    path: PathBuf,
    fixed_points: HashSet<u64>,
    dirty: bool,
}

impl FormatCache {
    /// Load the cache for `style` under `root`, or an empty cache when the file
    /// is missing, carries a different schema version, or fails to decode.
    pub fn load(root: &Path, style: &FormatStyle) -> Self {
        let path = cache_path(root, style);
        let fixed_points = std::fs::read(&path)
            .ok()
            .and_then(|bytes| postcard::from_bytes::<CacheFile>(&bytes).ok())
            .filter(|c| c.schema_version == SCHEMA_VERSION)
            .map(|c| c.fixed_points)
            .unwrap_or_default();
        Self {
            path,
            fixed_points,
            dirty: false,
        }
    }

    /// Whether `key` names a known fixed point — content the formatter would
    /// leave unchanged, so the file can be counted clean without formatting it.
    pub fn is_fixed_point(&self, key: CacheKey<'_>) -> bool {
        self.fixed_points.contains(&key.hash())
    }

    /// Record `key` as a fixed point. Marks the cache dirty only when the hash
    /// was not already present, so an all-hit run writes nothing.
    pub fn record_fixed_point(&mut self, key: CacheKey<'_>) {
        if self.fixed_points.insert(key.hash()) {
            self.dirty = true;
        }
    }

    /// Persist the cache atomically (tempfile + rename), creating parent
    /// directories and garbage-collecting sibling version directories. A no-op
    /// when nothing was recorded since [`load`](Self::load).
    pub fn store(&self) -> Result<(), CacheError> {
        if !self.dirty {
            return Ok(());
        }
        let dir = self.path.parent().ok_or(CacheError::NoCacheDir)?;
        std::fs::create_dir_all(dir).map_err(|e| CacheError::Io(e.to_string()))?;
        let file = CacheFile {
            schema_version: SCHEMA_VERSION,
            fixed_points: self.fixed_points.clone(),
        };
        let bytes = postcard::to_allocvec(&file).map_err(|e| CacheError::Serde(e.to_string()))?;
        atomic_write(&self.path, &bytes)?;
        gc_old_version_dirs(&self.path);
        Ok(())
    }
}

/// `{root}/format/{arity_version}/{style_hash}.postcard`.
fn cache_path(root: &Path, style: &FormatStyle) -> PathBuf {
    root.join("format")
        .join(ARITY_VERSION)
        .join(format!("{:016x}.postcard", style_hash(style)))
}

/// Which grammar a cached fixed point belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Grammar {
    R,
    Dcf,
}

/// What a fixed point is keyed by: the grammar, the content, and whatever else
/// changes the formatter's answer for *that* grammar.
///
/// The grammar is not decoration. Some byte strings — a lone comment line, an
/// empty file — are valid under both grammars and can be a fixed point of both,
/// and a cross-grammar hit would report a dirty `DESCRIPTION` as clean and turn
/// a CI `--check` green on unformatted output.
#[derive(Debug, Clone, Copy)]
pub struct CacheKey<'a> {
    grammar: Grammar,
    content: &'a str,
    /// The file's resolved package-wide roxygen markdown default. It changes
    /// formatting, so the same bytes can be a fixed point in a markdown-first
    /// package and not in an Rd-first one.
    roxygen_markdown: bool,
}

impl<'a> CacheKey<'a> {
    pub fn r(content: &'a str, roxygen_markdown: bool) -> Self {
        Self {
            grammar: Grammar::R,
            content,
            roxygen_markdown,
        }
    }

    /// A `DESCRIPTION`. There is no markdown default to carry: it is a fact
    /// about parsing *R*, and a `false` a reader has to decode as "not
    /// applicable" is the kind of argument that gets copy-pasted wrong.
    pub fn dcf(content: &'a str) -> Self {
        Self {
            grammar: Grammar::Dcf,
            content,
            roxygen_markdown: false,
        }
    }

    fn hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.grammar.hash(&mut hasher);
        self.content.hash(&mut hasher);
        if self.grammar == Grammar::R {
            self.roxygen_markdown.hash(&mut hasher);
        }
        hasher.finish()
    }
}

fn style_hash(style: &FormatStyle) -> u64 {
    let mut hasher = DefaultHasher::new();
    style.hash(&mut hasher);
    hasher.finish()
}

/// Remove `format/{version}/` directories that are not the current version.
/// Best-effort: any I/O error is ignored (a leftover dir is harmless).
fn gc_old_version_dirs(cache_path: &Path) {
    let Some(version_dir) = cache_path.parent() else {
        return;
    };
    let Some(format_root) = version_dir.parent() else {
        return;
    };
    let keep = version_dir.file_name();
    let Ok(entries) = std::fs::read_dir(format_root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if Some(name.as_os_str()) != keep && entry.path().is_dir() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_reloads_a_fixed_point() {
        let tmp = tempfile::tempdir().unwrap();
        let style = FormatStyle::default();

        let mut cache = FormatCache::load(tmp.path(), &style);
        assert!(!cache.is_fixed_point(CacheKey::r("x <- 1\n", false)));
        cache.record_fixed_point(CacheKey::r("x <- 1\n", false));
        cache.store().unwrap();

        // A fresh load sees the persisted hash.
        let reloaded = FormatCache::load(tmp.path(), &style);
        assert!(reloaded.is_fixed_point(CacheKey::r("x <- 1\n", false)));
        assert!(!reloaded.is_fixed_point(CacheKey::r("y <- 2\n", false)));
    }

    #[test]
    fn grammar_keys_the_fixed_point() {
        // A lone comment line is valid under both grammars and is plausibly a
        // fixed point of both. Without the discriminant, formatting a
        // `DESCRIPTION` would hit an entry recorded for an `.R` file.
        let tmp = tempfile::tempdir().unwrap();
        let mut cache = FormatCache::load(tmp.path(), &FormatStyle::default());
        cache.record_fixed_point(CacheKey::r("# a comment\n", false));
        assert!(cache.is_fixed_point(CacheKey::r("# a comment\n", false)));
        assert!(!cache.is_fixed_point(CacheKey::dcf("# a comment\n")));

        cache.record_fixed_point(CacheKey::dcf("# a comment\n"));
        assert!(cache.is_fixed_point(CacheKey::dcf("# a comment\n")));
    }

    #[test]
    fn markdown_flag_keys_the_fixed_point() {
        // The same bytes can be a fixed point in a markdown-first package and
        // not in an Rd-first one — the flag must disambiguate.
        let tmp = tempfile::tempdir().unwrap();
        let mut cache = FormatCache::load(tmp.path(), &FormatStyle::default());
        cache.record_fixed_point(CacheKey::r("#' doc\nNULL\n", true));
        assert!(cache.is_fixed_point(CacheKey::r("#' doc\nNULL\n", true)));
        assert!(!cache.is_fixed_point(CacheKey::r("#' doc\nNULL\n", false)));
    }

    #[test]
    fn missing_file_loads_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = FormatCache::load(tmp.path(), &FormatStyle::default());
        assert!(!cache.is_fixed_point(CacheKey::r("anything\n", false)));
    }

    #[test]
    fn clean_run_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let style = FormatStyle::default();
        // Nothing recorded => store is a no-op => no file created.
        let cache = FormatCache::load(tmp.path(), &style);
        cache.store().unwrap();
        assert!(!cache_path(tmp.path(), &style).exists());
    }

    #[test]
    fn re_recording_same_content_stays_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let style = FormatStyle::default();
        let mut cache = FormatCache::load(tmp.path(), &style);
        cache.record_fixed_point(CacheKey::r("x <- 1\n", false));
        cache.store().unwrap();

        // Reload and re-record the same content: no new hash, so store is a no-op.
        let mut reloaded = FormatCache::load(tmp.path(), &style);
        reloaded.record_fixed_point(CacheKey::r("x <- 1\n", false));
        assert!(!reloaded.dirty);
    }

    #[test]
    fn schema_mismatch_loads_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let style = FormatStyle::default();
        let path = cache_path(tmp.path(), &style);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let stale = CacheFile {
            schema_version: SCHEMA_VERSION + 1,
            fixed_points: HashSet::from([CacheKey::r("x <- 1\n", false).hash()]),
        };
        std::fs::write(&path, postcard::to_allocvec(&stale).unwrap()).unwrap();

        let cache = FormatCache::load(tmp.path(), &style);
        assert!(!cache.is_fixed_point(CacheKey::r("x <- 1\n", false)));
    }

    #[test]
    fn corrupt_file_loads_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let style = FormatStyle::default();
        let path = cache_path(tmp.path(), &style);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not postcard at all").unwrap();

        let cache = FormatCache::load(tmp.path(), &style);
        assert!(!cache.is_fixed_point(CacheKey::r("x <- 1\n", false)));
    }

    #[test]
    fn different_styles_use_different_files() {
        let tmp = tempfile::tempdir().unwrap();
        let a = FormatStyle::default();
        let b = FormatStyle {
            line_width: 120,
            ..FormatStyle::default()
        };
        assert_ne!(cache_path(tmp.path(), &a), cache_path(tmp.path(), &b));
    }

    #[test]
    fn store_gcs_other_version_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let style = FormatStyle::default();
        // A leftover directory from a different arity version.
        let stale_dir = tmp.path().join("format").join("0.0.0-old");
        std::fs::create_dir_all(&stale_dir).unwrap();

        let mut cache = FormatCache::load(tmp.path(), &style);
        cache.record_fixed_point(CacheKey::r("x <- 1\n", false));
        cache.store().unwrap();

        assert!(!stale_dir.exists());
        assert!(cache_path(tmp.path(), &style).parent().unwrap().exists());
    }
}
