//! On-disk cache of harvested package indices.
//!
//! Layout under the cache root:
//!
//! ```text
//! {root}/index/v{SCHEMA_VERSION}/
//!     meta.json              # IndexMeta: package -> indexed version
//!     {pkg}@{ver}.json        # one PackageIndex each
//! ```
//!
//! A schema bump moves everything under a fresh `v{N}/`; stale `v*/` siblings
//! are garbage-collected on a successful write, never on read. An entry whose
//! `schema_version` doesn't match is treated as absent.

use std::path::{Path, PathBuf};

use smol_str::SmolStr;

use rayon::prelude::*;

use crate::rindex::schema::{IndexMeta, PackageExports, PackageIndex, SCHEMA_VERSION};

#[derive(Debug)]
pub enum CacheError {
    Io(String),
    Serde(String),
    NoCacheDir,
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CacheError::Io(s) => write!(f, "cache I/O error: {s}"),
            CacheError::Serde(s) => write!(f, "cache (de)serialization error: {s}"),
            CacheError::NoCacheDir => write!(f, "could not determine a cache directory"),
        }
    }
}

impl std::error::Error for CacheError {}

type Result<T> = std::result::Result<T, CacheError>;

/// Resolve the cache root: `cli_override` > `config_override` >
/// `$ARITY_CACHE_DIR` > platform default (`$XDG_CACHE_HOME/arity`,
/// `$HOME/.cache/arity`, or `%LOCALAPPDATA%\arity`).
pub fn resolve_cache_root(
    cli_override: Option<&Path>,
    config_override: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(p) = cli_override {
        return Ok(p.to_path_buf());
    }
    if let Some(p) = config_override {
        return Ok(p.to_path_buf());
    }
    if let Some(p) = std::env::var_os("ARITY_CACHE_DIR") {
        return Ok(PathBuf::from(p));
    }
    default_cache_root().ok_or(CacheError::NoCacheDir)
}

fn default_cache_root() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("arity"))
    } else if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME").filter(|s| !s.is_empty()) {
        Some(PathBuf::from(xdg).join("arity"))
    } else {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/arity"))
    }
}

/// A handle to the cache rooted at a directory.
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(root: PathBuf) -> Self {
        Cache { root }
    }

    /// The versioned index directory (`{root}/index/v{N}`).
    pub fn index_dir(&self) -> PathBuf {
        self.root.join("index").join(format!("v{SCHEMA_VERSION}"))
    }

    fn meta_path(&self) -> PathBuf {
        self.index_dir().join("meta.json")
    }

    fn package_path(&self, package: &str, version: &str) -> PathBuf {
        self.index_dir().join(format!("{package}@{version}.json"))
    }

    /// Read the metadata file, returning a fresh empty `IndexMeta` if it is
    /// missing or carries a different schema version.
    pub fn read_meta(&self) -> IndexMeta {
        let Ok(bytes) = std::fs::read(self.meta_path()) else {
            return IndexMeta::new();
        };
        match serde_json::from_slice::<IndexMeta>(&bytes) {
            Ok(m) if m.schema_version == SCHEMA_VERSION => m,
            _ => IndexMeta::new(),
        }
    }

    fn write_meta(&self, meta: &IndexMeta) -> Result<()> {
        std::fs::create_dir_all(self.index_dir()).map_err(|e| CacheError::Io(e.to_string()))?;
        let json = serde_json::to_vec_pretty(meta).map_err(|e| CacheError::Serde(e.to_string()))?;
        atomic_write(&self.meta_path(), &json)
    }

    /// Read a single package index by name + version, if present and current.
    pub fn read_package(&self, package: &str, version: &str) -> Option<PackageIndex> {
        let bytes = std::fs::read(self.package_path(package, version)).ok()?;
        let idx = serde_json::from_slice::<PackageIndex>(&bytes).ok()?;
        (idx.schema_version == SCHEMA_VERSION).then_some(idx)
    }

    /// Read only the export-membership view of a package index (see
    /// [`PackageExports`]): same file, same staleness rule, but the rich
    /// payload (formals + help) is skipped during deserialization.
    pub fn read_package_exports(&self, package: &str, version: &str) -> Option<PackageExports> {
        let bytes = std::fs::read(self.package_path(package, version)).ok()?;
        let idx = serde_json::from_slice::<PackageExports>(&bytes).ok()?;
        (idx.schema_version == SCHEMA_VERSION).then_some(idx)
    }

    /// Write a single package index file (`pkg@ver.json`) **without** touching
    /// `meta.json`. Safe to call concurrently for distinct packages: each writes
    /// its own path (via a uniquely-named temp file), and the only shared
    /// mutation — `meta.json` — is deferred to [`record_indexed`]. Compact JSON,
    /// not pretty: these files are large (they carry help bodies) and only ever
    /// machine-read, so indentation just bloats them.
    pub fn write_package_file(&self, idx: &PackageIndex) -> Result<()> {
        std::fs::create_dir_all(self.index_dir()).map_err(|e| CacheError::Io(e.to_string()))?;
        let json = serde_json::to_vec(idx).map_err(|e| CacheError::Serde(e.to_string()))?;
        atomic_write(&self.package_path(&idx.package, &idx.version), &json)
    }

    /// Fold newly-indexed `(package, version)` pairs into `meta.json` in one
    /// read-modify-write. Call once, *sequentially*, after a (possibly parallel)
    /// batch of [`write_package_file`] calls: concurrent callers would race on
    /// the shared meta file and silently lose entries.
    pub fn record_indexed(&self, entries: &[(SmolStr, SmolStr)]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut meta = self.read_meta();
        meta.schema_version = SCHEMA_VERSION;
        for (package, version) in entries {
            meta.packages.insert(package.clone(), version.clone());
        }
        self.write_meta(&meta)
    }

    /// Write a package index and update `meta.json` to point at this version.
    /// Convenience for single writes; the batch build path uses
    /// [`write_package_file`] + [`record_indexed`] so it can parallelize harvest.
    pub fn write_package(&self, idx: &PackageIndex) -> Result<()> {
        self.write_package_file(idx)?;
        self.record_indexed(&[(idx.package.clone(), idx.version.clone())])
    }

    /// Load every package index currently named by `meta.json`. Reads and
    /// deserializes packages in parallel: the files are independent and the
    /// big ones (base, stats) dominate a sequential load.
    pub fn load_all(&self) -> Vec<PackageIndex> {
        let meta = self.read_meta();
        let entries: Vec<_> = meta.packages.iter().collect();
        entries
            .par_iter()
            .filter_map(|(pkg, ver)| self.read_package(pkg, ver))
            .collect()
    }

    /// Load the export-membership view of every package named by `meta.json`,
    /// in parallel. The cheap counterpart of [`load_all`] for consumers that
    /// never touch the rich per-symbol data (the lint CLI).
    pub fn load_all_exports(&self) -> Vec<PackageExports> {
        let meta = self.read_meta();
        let entries: Vec<_> = meta.packages.iter().collect();
        entries
            .par_iter()
            .filter_map(|(pkg, ver)| self.read_package_exports(pkg, ver))
            .collect()
    }

    /// The version currently indexed for `package`, per `meta.json`.
    pub fn indexed_version(&self, package: &str) -> Option<SmolStr> {
        self.read_meta().packages.get(package).cloned()
    }

    /// Garbage-collect index directories for other schema versions.
    pub fn gc_old_schema_dirs(&self) -> Result<()> {
        let index_root = self.root.join("index");
        let keep = format!("v{SCHEMA_VERSION}");
        let Ok(entries) = std::fs::read_dir(&index_root) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('v') && name != keep && entry.path().is_dir() {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
        Ok(())
    }
}

/// Write atomically: write to a sibling temp file, then rename into place.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().ok_or(CacheError::NoCacheDir)?;
    let mut tmp =
        tempfile::NamedTempFile::new_in(dir).map_err(|e| CacheError::Io(e.to_string()))?;
    use std::io::Write;
    tmp.write_all(bytes)
        .map_err(|e| CacheError::Io(e.to_string()))?;
    tmp.persist(path)
        .map_err(|e| CacheError::Io(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rindex::schema::{SymbolEntry, SymbolKind};

    fn sample(pkg: &str, ver: &str) -> PackageIndex {
        PackageIndex {
            schema_version: SCHEMA_VERSION,
            package: SmolStr::new(pkg),
            version: SmolStr::new(ver),
            lib_path: "/lib".into(),
            title: None,
            r_version: None,
            harvested_at: 0,
            attaches: Vec::new(),
            symbols: vec![SymbolEntry {
                name: SmolStr::new("foo"),
                kind: SymbolKind::Function,
                exported: true,
                formals: None,
                help: None,
            }],
        }
    }

    #[test]
    fn write_then_read_round_trips_and_updates_meta() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf());
        let idx = sample("magrittr", "2.0.4");
        cache.write_package(&idx).unwrap();

        assert_eq!(cache.read_package("magrittr", "2.0.4"), Some(idx.clone()));
        assert_eq!(cache.indexed_version("magrittr").as_deref(), Some("2.0.4"));
        assert_eq!(cache.load_all(), vec![idx]);
    }

    #[test]
    fn load_all_follows_meta_not_stale_files() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf());
        cache.write_package(&sample("pkg", "1.0")).unwrap();
        // A newer version supersedes the old; both files exist on disk but
        // meta points only at the latest.
        cache.write_package(&sample("pkg", "2.0")).unwrap();

        assert_eq!(cache.indexed_version("pkg").as_deref(), Some("2.0"));
        let all = cache.load_all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].version, "2.0");
        // The stale file is still readable directly but not loaded by load_all.
        assert!(cache.read_package("pkg", "1.0").is_some());
    }

    #[test]
    fn load_all_exports_follows_meta_and_schema() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf());
        cache.write_package(&sample("magrittr", "2.0.4")).unwrap();
        cache.write_package(&sample("pkg", "1.0")).unwrap();
        cache.write_package(&sample("pkg", "2.0")).unwrap();

        let mut exports = cache.load_all_exports();
        exports.sort_by(|a, b| a.package.cmp(&b.package));
        // Meta names one version per package; the stale pkg@1.0 is not loaded.
        assert_eq!(exports.len(), 2);
        assert_eq!(exports[0].package, "magrittr");
        assert_eq!(exports[1].package, "pkg");
        assert_eq!(exports[1].version, "2.0");
        assert_eq!(exports[0].symbols.len(), 1);
        assert_eq!(exports[0].symbols[0].name, "foo");
        assert!(exports[0].symbols[0].exported);

        // A schema-version mismatch means "treat as absent", same as the full read.
        let mut stale = sample("old", "0.1");
        stale.schema_version = SCHEMA_VERSION + 1;
        // Write the file directly (write_package would stamp meta with it too,
        // which is exactly what a future-schema writer would do).
        cache.write_package(&stale).unwrap();
        assert!(cache.read_package_exports("old", "0.1").is_none());
    }

    #[test]
    fn resolve_cache_root_prefers_cli_then_config() {
        let cli = PathBuf::from("/cli/cache");
        let cfg = PathBuf::from("/cfg/cache");
        assert_eq!(resolve_cache_root(Some(&cli), Some(&cfg)).unwrap(), cli);
        assert_eq!(resolve_cache_root(None, Some(&cfg)).unwrap(), cfg);
    }

    #[test]
    fn gc_removes_other_schema_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf());
        let index_root = tmp.path().join("index");
        std::fs::create_dir_all(index_root.join("v0")).unwrap();
        std::fs::create_dir_all(index_root.join(format!("v{SCHEMA_VERSION}"))).unwrap();
        cache.gc_old_schema_dirs().unwrap();
        assert!(!index_root.join("v0").exists());
        assert!(index_root.join(format!("v{SCHEMA_VERSION}")).exists());
    }
}
