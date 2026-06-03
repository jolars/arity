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
use crate::semantic::symbols::{LoadedPackage, PackageOrigin, StaticBaseR, SymbolProvider};

/// Resolves names against indexed, attached packages and holds the rich data.
#[derive(Default)]
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

/// `StaticBaseR` + an `IndexedProvider`, honoring search-path masking.
pub struct CompositeProvider {
    base: StaticBaseR,
    indexed: IndexedProvider,
}

impl CompositeProvider {
    /// Base only — equivalent to the historical `StaticBaseR` behavior.
    pub fn base_only() -> Self {
        CompositeProvider {
            base: StaticBaseR::new(),
            indexed: IndexedProvider::empty(),
        }
    }

    pub fn with_index(indexed: IndexedProvider) -> Self {
        CompositeProvider {
            base: StaticBaseR::new(),
            indexed,
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
        // masks the rest.
        for pkg in loaded {
            if self.indexed.exports(&pkg.name, name) && !candidates.contains(&pkg.name) {
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
        // A default package (known to base) or a harvested package (in the
        // index) is one whose exports we know in full.
        self.base.package_indexed(pkg) || self.indexed.has_package(pkg)
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
    fn unindexed_loaded_package_leaves_name_unknown() {
        let p = CompositeProvider::base_only();
        // dplyr is attached but not indexed: a name only dplyr would export
        // stays Unknown (conservative).
        assert_eq!(
            p.origin("across", &[loaded("dplyr")]),
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
