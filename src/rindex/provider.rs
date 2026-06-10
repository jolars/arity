//! Symbol providers backed by the harvested index.
//!
//! - [`IndexedProvider`] resolves names against the *attached* packages it has
//!   indexed, and exposes the rich per-symbol data ([`lookup`](IndexedProvider::lookup))
//!   for future LSP features. It deliberately knows nothing about base R.
//! - [`CompositeProvider`] layers [`IndexedProvider`] over
//!   [`StaticBaseR`](crate::semantic::symbols::StaticBaseR) and implements the
//!   thin [`SymbolProvider`] trait with R's search-path masking semantics:
//!   default packages attach first, then `library()`-loaded packages in source
//!   order, and the last attacher masks.

use std::collections::{HashMap, HashSet};

use smol_str::SmolStr;

use crate::rindex::cache::Cache;
use crate::rindex::schema::{PackageIndex, SymbolEntry};
use crate::semantic::symbols::{
    BundledPackages, LoadedPackage, PackageOrigin, StaticBaseR, SymbolProvider,
};

/// Resolves names against indexed, attached packages and holds the rich data.
#[derive(Debug, Default)]
pub struct IndexedProvider {
    /// package → set of exported names (for `origin` membership tests).
    pkg_exports: HashMap<SmolStr, HashSet<SmolStr>>,
    /// package → full harvested index (for `lookup`).
    indices: HashMap<SmolStr, PackageIndex>,
}

impl IndexedProvider {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build from a set of harvested package indices.
    pub fn from_indices(indices: impl IntoIterator<Item = PackageIndex>) -> Self {
        let mut pkg_exports: HashMap<SmolStr, HashSet<SmolStr>> = HashMap::new();
        let mut map: HashMap<SmolStr, PackageIndex> = HashMap::new();
        for idx in indices {
            let names: HashSet<SmolStr> = idx
                .symbols
                .iter()
                .filter(|s| s.exported)
                .map(|s| s.name.clone())
                .collect();
            pkg_exports.insert(idx.package.clone(), names);
            map.insert(idx.package.clone(), idx);
        }
        IndexedProvider {
            pkg_exports,
            indices: map,
        }
    }

    /// Load every package index currently named by the cache's `meta.json`.
    pub fn from_cache(cache: &Cache) -> Self {
        Self::from_indices(cache.load_all())
    }

    /// True if this provider has an index for `package`.
    pub fn has_package(&self, package: &str) -> bool {
        self.pkg_exports.contains_key(package)
    }

    /// The rich entry for `pkg::name`, if indexed.
    pub fn lookup(&self, package: &str, name: &str) -> Option<&SymbolEntry> {
        self.indices
            .get(package)?
            .symbols
            .iter()
            .find(|s| s.name == name)
    }

    /// The full harvested index for a package, if present.
    pub fn package(&self, package: &str) -> Option<&PackageIndex> {
        self.indices.get(package)
    }

    fn exports(&self, package: &str, name: &str) -> bool {
        self.pkg_exports
            .get(package)
            .is_some_and(|set| set.contains(name))
    }
}

/// `StaticBaseR` + an `IndexedProvider` + bundled CRAN export lists, honoring
/// search-path masking. Precedence per package: locally harvested index
/// (version-exact) → base defaults → bundled CRAN (approximate latest).
#[derive(Debug)]
pub struct CompositeProvider {
    base: StaticBaseR,
    indexed: IndexedProvider,
    bundled: BundledPackages,
}

impl CompositeProvider {
    /// No local index — base defaults plus the bundled CRAN export lists.
    pub fn base_only() -> Self {
        CompositeProvider {
            base: StaticBaseR::new(),
            indexed: IndexedProvider::empty(),
            bundled: BundledPackages::new(),
        }
    }

    pub fn with_index(indexed: IndexedProvider) -> Self {
        CompositeProvider {
            base: StaticBaseR::new(),
            indexed,
            bundled: BundledPackages::new(),
        }
    }

    /// The indexed layer, for callers that need the rich data (e.g. the LSP).
    pub fn indexed(&self) -> &IndexedProvider {
        &self.indexed
    }
}

impl SymbolProvider for CompositeProvider {
    fn origin(&self, name: &str, loaded: &[LoadedPackage]) -> PackageOrigin {
        // Default packages attach first.
        let mut candidates: Vec<SmolStr> = match self.base.origin(name, &[]) {
            PackageOrigin::Resolved(p) => vec![p],
            PackageOrigin::Ambiguous(v) => v,
            PackageOrigin::Unknown => Vec::new(),
        };
        // Then `library()`-attached packages in source order; the last attacher
        // masks the rest. Prefer the version-exact installed index when present,
        // otherwise fall back to the bundled CRAN export list.
        for pkg in loaded {
            let exports_it = if self.indexed.has_package(&pkg.name) {
                self.indexed.exports(&pkg.name, name)
            } else {
                self.bundled.exports(&pkg.name, name)
            };
            if exports_it && !candidates.contains(&pkg.name) {
                candidates.push(pkg.name.clone());
            }
        }
        match candidates.len() {
            0 => PackageOrigin::Unknown,
            1 => PackageOrigin::Resolved(candidates.into_iter().next().unwrap()),
            _ => PackageOrigin::Ambiguous(candidates),
        }
    }

    fn is_base(&self, name: &str) -> bool {
        self.base.is_base(name)
    }

    fn package_indexed(&self, pkg: &str) -> bool {
        // A default package (known to base), a harvested package (in the index),
        // or a bundled CRAN package is one whose exports we know — enough for
        // `undefined-symbol` to fire instead of suppressing the whole file.
        self.base.package_indexed(pkg)
            || self.indexed.has_package(pkg)
            || self.bundled.has_package(pkg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rindex::schema::{SCHEMA_VERSION, SymbolKind};
    use rowan::{TextRange, TextSize};

    fn pkg(name: &str, exports: &[&str]) -> PackageIndex {
        PackageIndex {
            schema_version: SCHEMA_VERSION,
            package: SmolStr::new(name),
            version: SmolStr::new("1.0"),
            lib_path: "/lib".into(),
            r_version: None,
            harvested_at: 0,
            symbols: exports
                .iter()
                .map(|n| SymbolEntry {
                    name: SmolStr::new(*n),
                    kind: SymbolKind::Function,
                    exported: true,
                    formals: None,
                    help: None,
                })
                .collect(),
        }
    }

    fn loaded(name: &str) -> LoadedPackage {
        LoadedPackage {
            name: SmolStr::new(name),
            range: TextRange::new(TextSize::new(0), TextSize::new(0)),
        }
    }

    #[test]
    fn is_base_delegates_to_base_only() {
        let p = CompositeProvider::with_index(IndexedProvider::from_indices([pkg(
            "dplyr",
            &["across"],
        )]));
        assert!(p.is_base("c"));
        // An indexed-only package export is not "base".
        assert!(!p.is_base("across"));
    }

    #[test]
    fn loaded_package_masks_base_name() {
        // `filter` exists in stats (base set) and dplyr; with dplyr attached it
        // should be Ambiguous with dplyr masking (last).
        let p = CompositeProvider::with_index(IndexedProvider::from_indices([pkg(
            "dplyr",
            &["filter"],
        )]));
        match p.origin("filter", &[loaded("dplyr")]) {
            PackageOrigin::Ambiguous(v) => {
                assert_eq!(v.last().map(|s| s.as_str()), Some("dplyr"));
                assert!(v.iter().any(|s| s == "stats"));
            }
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn resolves_indexed_only_name() {
        let p = CompositeProvider::with_index(IndexedProvider::from_indices([pkg(
            "dplyr",
            &["across"],
        )]));
        // `across` is not a base name; attaching dplyr resolves it.
        assert_eq!(
            p.origin("across", &[loaded("dplyr")]),
            PackageOrigin::Resolved(SmolStr::new("dplyr"))
        );
    }

    #[test]
    fn unindexed_unbundled_loaded_package_leaves_name_unknown() {
        let p = CompositeProvider::base_only();
        // A package that is neither indexed nor bundled: a name only it would
        // export stays Unknown (conservative whole-file suppression still
        // applies via `package_indexed`).
        assert!(!p.package_indexed("not_a_real_package_xyz"));
        assert_eq!(
            p.origin("some_export_xyz", &[loaded("not_a_real_package_xyz")]),
            PackageOrigin::Unknown
        );
    }

    #[test]
    fn bundled_package_is_indexed_and_resolves() {
        // No local index: the bundled CRAN list backs resolution.
        let p = CompositeProvider::base_only();
        assert!(p.package_indexed("data.table"));
        assert_eq!(
            p.origin("fread", &[loaded("data.table")]),
            PackageOrigin::Resolved(SmolStr::new("data.table"))
        );
        // An unknown name with a bundled package attached stays Unknown, so
        // `undefined-symbol` can fire on it.
        assert_eq!(
            p.origin("not_a_real_export_xyz", &[loaded("data.table")]),
            PackageOrigin::Unknown
        );
    }

    #[test]
    fn installed_index_wins_over_bundled() {
        // An installed index for a bundled package is version-exact and takes
        // precedence: its export resolves, and a name only the (stale) bundled
        // list has does not.
        let p = CompositeProvider::with_index(IndexedProvider::from_indices([pkg(
            "data.table",
            &["custom_installed_sym"],
        )]));
        assert_eq!(
            p.origin("custom_installed_sym", &[loaded("data.table")]),
            PackageOrigin::Resolved(SmolStr::new("data.table"))
        );
        assert_eq!(
            p.origin("fread", &[loaded("data.table")]),
            PackageOrigin::Unknown
        );
    }

    #[test]
    fn lookup_exposes_rich_data() {
        let provider = IndexedProvider::from_indices([pkg("dplyr", &["filter"])]);
        assert!(provider.lookup("dplyr", "filter").is_some());
        assert!(provider.lookup("dplyr", "nope").is_none());
        assert!(provider.has_package("dplyr"));
    }
}
