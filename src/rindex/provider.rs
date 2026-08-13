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
use std::sync::LazyLock;

use smol_str::SmolStr;

use crate::rindex::cache::Cache;
use crate::rindex::remote::RemoteExports;
use crate::rindex::schema::{PackageIndex, SymbolEntry};
use crate::semantic::symbols::{
    BundledPackages, LoadedPackage, PackageOrigin, StaticBaseR, SymbolProvider,
    meta_package_members, unbacktick,
};

/// R's default-package export lists. A compile-time constant (baked-in symbol
/// lists), shared process-wide so it is parsed once rather than per provider.
static BASE_R: LazyLock<StaticBaseR> = LazyLock::new(StaticBaseR::new);

/// The bundled top-N CRAN export lists. Also a compile-time constant, shared.
static BUNDLED: LazyLock<BundledPackages> = LazyLock::new(BundledPackages::new);

/// Resolve a bare `name` against R's default packages, the harvested `indexed`
/// layer, the network `remote` sidecar, and the bundled CRAN lists, honoring
/// search-path masking: default packages attach first, then `loaded` packages in
/// source order, and the last attacher masks. Per package the precedence is
/// version-exact installed index → remote sidecar (names-only, all CRAN) →
/// bundled list (names-only, baked-in top-N).
///
/// This is the single masking implementation; both [`CompositeProvider`] and the
/// salsa `external_resolution` query call it. The static layers are read from
/// the shared [`BASE_R`]/[`BUNDLED`] singletons; `indexed` and `remote` vary at
/// runtime.
pub fn resolve_origin(
    indexed: &IndexedProvider,
    remote: &RemoteExports,
    name: &str,
    loaded: &[LoadedPackage],
) -> PackageOrigin {
    // Default packages attach first.
    let mut candidates: Vec<SmolStr> = match BASE_R.origin(name, &[]) {
        PackageOrigin::Resolved(p) => vec![p],
        PackageOrigin::Ambiguous(v) => v,
        PackageOrigin::Unknown => Vec::new(),
    };
    // Then `library()`-attached packages in source order; the last attacher
    // masks. Prefer the version-exact installed index, then the remote sidecar,
    // and finally the baked-in bundled CRAN export list.
    let mut consider = |pkg: &str| {
        let exports_it = if indexed.has_package(pkg) {
            indexed.exports(pkg, name)
        } else if remote.has_package(pkg) {
            remote.exports(pkg, name)
        } else {
            BUNDLED.exports(pkg, name)
        };
        if exports_it && !candidates.iter().any(|c| c == pkg) {
            candidates.push(SmolStr::new(pkg));
        }
    };
    for pkg in loaded {
        consider(&pkg.name);
        // A meta-package (e.g. tidyverse) also attaches its core members, whose
        // exports must resolve even though they aren't the meta-package's own.
        for member in attach_members(indexed, &pkg.name) {
            consider(member);
        }
    }
    match candidates.len() {
        0 => PackageOrigin::Unknown,
        1 => PackageOrigin::Resolved(candidates.into_iter().next().unwrap()),
        _ => PackageOrigin::Ambiguous(candidates),
    }
}

/// The packages `pkg` attaches beyond itself when `library(pkg)` runs — a
/// meta-package's core set. Prefers the version-exact attach set captured at
/// harvest time when `pkg` is indexed with a non-empty one; otherwise the
/// static curated table ([`meta_package_members`]). An installed meta-package
/// whose capture found nothing therefore keeps the known-correct table.
pub fn attach_members<'a>(
    indexed: &'a IndexedProvider,
    pkg: &str,
) -> impl Iterator<Item = &'a str> {
    match indexed.attaches(pkg) {
        Some(harvested) => AttachMembers::Harvested(harvested.iter()),
        None => AttachMembers::Static(meta_package_members(pkg).iter()),
    }
}

/// Two-variant iterator so [`attach_members`] allocates nothing on
/// `resolve_origin`'s per-name path.
enum AttachMembers<'a> {
    Harvested(std::slice::Iter<'a, SmolStr>),
    Static(std::slice::Iter<'static, &'static str>),
}

impl<'a> Iterator for AttachMembers<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        match self {
            AttachMembers::Harvested(it) => it.next().map(SmolStr::as_str),
            AttachMembers::Static(it) => it.next().copied(),
        }
    }
}

/// True if `pkg`'s exports are fully known — a default package, a harvested
/// package, a remote-sidecar package, or a bundled CRAN package — so an
/// unresolved name attributed to it is genuinely undefined rather than merely
/// un-indexed.
pub fn package_indexed(indexed: &IndexedProvider, remote: &RemoteExports, pkg: &str) -> bool {
    BASE_R.package_indexed(pkg)
        || indexed.has_package(pkg)
        || remote.has_package(pkg)
        || BUNDLED.has_package(pkg)
}

/// True if `name` is exported by one of R's default packages.
pub fn is_base(name: &str) -> bool {
    BASE_R.is_base(name)
}

/// Iterate every base/default-package export name, for completion candidates.
pub fn base_names() -> impl Iterator<Item = &'static SmolStr> {
    let base: &'static StaticBaseR = &BASE_R;
    base.base_names()
}

/// The default package that exports a base `name`, if any (for `resolve` to
/// attach docs to a bare base-R completion once `base` is harvested).
pub fn base_package_of(name: &str) -> Option<&'static SmolStr> {
    let base: &'static StaticBaseR = &BASE_R;
    base.package_of(name)
}

/// Iterate every bundled CRAN package *name* — the candidate pool for
/// completing a dependency field, where the question is which packages exist
/// rather than what any one of them exports.
pub fn bundled_packages() -> impl Iterator<Item = &'static SmolStr> {
    let bundled: &'static BundledPackages = &BUNDLED;
    bundled.packages()
}

/// Iterate a bundled CRAN package's export names, if bundled — completion's
/// member fallback when the package isn't locally harvested.
pub fn bundled_exports(package: &str) -> Option<impl Iterator<Item = &'static SmolStr>> {
    let bundled: &'static BundledPackages = &BUNDLED;
    bundled.package_exports(package)
}

/// Resolves names against indexed, attached packages and holds the rich data.
#[derive(Debug, Default)]
pub struct IndexedProvider {
    /// package → set of exported names (for `origin` membership tests).
    pkg_exports: HashMap<SmolStr, HashSet<SmolStr>>,
    /// package → full harvested index (for `lookup`).
    indices: HashMap<SmolStr, PackageIndex>,
    /// package → harvested attach set; only non-empty sets are recorded. Kept
    /// separate from `indices` because the lean exports-only load
    /// ([`from_cache_exports`](Self::from_cache_exports)) never populates
    /// `indices` but still needs resolution to see attach sets.
    attaches: HashMap<SmolStr, Vec<SmolStr>>,
}

impl IndexedProvider {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build from a set of harvested package indices.
    pub fn from_indices(indices: impl IntoIterator<Item = PackageIndex>) -> Self {
        let mut pkg_exports: HashMap<SmolStr, HashSet<SmolStr>> = HashMap::new();
        let mut map: HashMap<SmolStr, PackageIndex> = HashMap::new();
        let mut attaches: HashMap<SmolStr, Vec<SmolStr>> = HashMap::new();
        for idx in indices {
            let names: HashSet<SmolStr> = idx
                .symbols
                .iter()
                .filter(|s| s.exported)
                .map(|s| s.name.clone())
                .collect();
            pkg_exports.insert(idx.package.clone(), names);
            if !idx.attaches.is_empty() {
                attaches.insert(idx.package.clone(), idx.attaches.clone());
            }
            map.insert(idx.package.clone(), idx);
        }
        IndexedProvider {
            pkg_exports,
            indices: map,
            attaches,
        }
    }

    /// Load every package index currently named by the cache's `meta.json`.
    pub fn from_cache(cache: &Cache) -> Self {
        Self::from_indices(cache.load_all())
    }

    /// Load only export *membership* from the cache: `has_package` and the
    /// `exports` tests behave exactly as after [`from_cache`], but the rich
    /// per-symbol data is never deserialized, so [`lookup`](Self::lookup) and
    /// [`package`](Self::package) return `None`. This is the lint CLI's load:
    /// resolution (`resolve_origin`/`package_indexed`) only asks membership
    /// questions, and skipping the formals + help bodies makes loading a large
    /// harvested cache cheap. Consumers of the rich data (LSP hover and
    /// completion) must use [`from_cache`].
    pub fn from_cache_exports(cache: &Cache) -> Self {
        let mut pkg_exports: HashMap<SmolStr, HashSet<SmolStr>> = HashMap::new();
        let mut attaches: HashMap<SmolStr, Vec<SmolStr>> = HashMap::new();
        for exp in cache.load_all_exports() {
            let names: HashSet<SmolStr> = exp
                .symbols
                .into_iter()
                .filter(|s| s.exported)
                .map(|s| s.name)
                .collect();
            if !exp.attaches.is_empty() {
                attaches.insert(exp.package.clone(), exp.attaches);
            }
            pkg_exports.insert(exp.package, names);
        }
        IndexedProvider {
            pkg_exports,
            indices: HashMap::new(),
            attaches,
        }
    }

    /// True if this provider has an index for `package`.
    pub fn has_package(&self, package: &str) -> bool {
        self.pkg_exports.contains_key(package)
    }

    /// Iterate every locally harvested package name.
    ///
    /// Reads `pkg_exports`, not `indices`, so it stays correct under the lean
    /// [`from_cache_exports`](Self::from_cache_exports) load, which never
    /// populates the rich map.
    pub fn packages(&self) -> impl Iterator<Item = &SmolStr> {
        self.pkg_exports.keys()
    }

    /// The rich entry for `pkg::name`, if indexed.
    pub fn lookup(&self, package: &str, name: &str) -> Option<&SymbolEntry> {
        let name = unbacktick(name);
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

    /// The harvested attach set for `package`, if one was captured. `None`
    /// when the package isn't indexed *or* its capture found nothing — both
    /// mean "fall back to the static table" (see [`attach_members`]).
    pub fn attaches(&self, package: &str) -> Option<&[SmolStr]> {
        self.attaches.get(package).map(Vec::as_slice)
    }

    fn exports(&self, package: &str, name: &str) -> bool {
        self.pkg_exports
            .get(package)
            .is_some_and(|set| set.contains(unbacktick(name)))
    }
}

/// The default-package + bundled-CRAN + harvested-index resolver, honoring
/// search-path masking. Precedence per package: locally harvested index
/// (version-exact) → base defaults → bundled CRAN (approximate latest).
///
/// Holds only the harvested [`IndexedProvider`]; the static default-package and
/// bundled-CRAN layers live in the shared [`BASE_R`]/[`BUNDLED`] singletons, and
/// all three are combined by the free [`resolve_origin`]/[`package_indexed`]
/// functions — the same ones the salsa `external_resolution` query uses.
#[derive(Debug)]
pub struct CompositeProvider {
    indexed: IndexedProvider,
    remote: RemoteExports,
}

impl CompositeProvider {
    /// No local index — base defaults plus the bundled CRAN export lists.
    pub fn base_only() -> Self {
        CompositeProvider {
            indexed: IndexedProvider::empty(),
            remote: RemoteExports::new(),
        }
    }

    pub fn with_index(indexed: IndexedProvider) -> Self {
        CompositeProvider {
            indexed,
            remote: RemoteExports::new(),
        }
    }

    /// Attach a remote-sidecar tier (builder style).
    pub fn with_remote(mut self, remote: RemoteExports) -> Self {
        self.remote = remote;
        self
    }

    /// The indexed layer, for callers that need the rich data (e.g. the LSP).
    pub fn indexed(&self) -> &IndexedProvider {
        &self.indexed
    }
}

impl SymbolProvider for CompositeProvider {
    fn origin(&self, name: &str, loaded: &[LoadedPackage]) -> PackageOrigin {
        resolve_origin(&self.indexed, &self.remote, name, loaded)
    }

    fn is_base(&self, name: &str) -> bool {
        is_base(name)
    }

    fn package_indexed(&self, pkg: &str) -> bool {
        package_indexed(&self.indexed, &self.remote, pkg)
    }

    fn attached_packages(&self, pkg: &str) -> Vec<SmolStr> {
        attach_members(&self.indexed, pkg)
            .map(SmolStr::new)
            .collect()
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
            title: None,
            r_version: None,
            harvested_at: 0,
            attaches: Vec::new(),
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

    fn remote(pkgs: &[(&str, &[&str])]) -> RemoteExports {
        let mut r = RemoteExports::new();
        for (pkg, names) in pkgs {
            r.insert_package(*pkg, names.iter().map(|n| SmolStr::new(*n)));
        }
        r
    }

    #[test]
    fn remote_resolves_uninstalled_unbundled_package() {
        // `tinytable` is neither installed nor in the bundled top-N; the remote
        // sidecar supplies its names so a real export resolves and `package_indexed`
        // is true (so `undefined-symbol` fires on a genuine non-export).
        let p = CompositeProvider::base_only().with_remote(remote(&[("tinytable", &["tt"])]));
        assert!(p.package_indexed("tinytable"));
        assert_eq!(
            p.origin("tt", &[loaded("tinytable")]),
            PackageOrigin::Resolved(SmolStr::new("tinytable"))
        );
        assert_eq!(
            p.origin("not_a_real_export", &[loaded("tinytable")]),
            PackageOrigin::Unknown
        );
    }

    #[test]
    fn installed_index_wins_over_remote() {
        // A version-exact local index masks the names-only remote tier for the
        // same package: the installed export resolves, a remote-only name does not.
        let p = CompositeProvider::with_index(IndexedProvider::from_indices([pkg(
            "tinytable",
            &["installed_sym"],
        )]))
        .with_remote(remote(&[("tinytable", &["remote_only_sym"])]));
        assert_eq!(
            p.origin("installed_sym", &[loaded("tinytable")]),
            PackageOrigin::Resolved(SmolStr::new("tinytable"))
        );
        assert_eq!(
            p.origin("remote_only_sym", &[loaded("tinytable")]),
            PackageOrigin::Unknown
        );
    }

    #[test]
    fn remote_wins_over_bundled() {
        // The remote tier sits above the baked-in bundled list: for a bundled
        // package, the remote names take precedence (fresher, version-aware).
        // `fread` is a real bundled `data.table` export; the remote omits it.
        let p = CompositeProvider::base_only().with_remote(remote(&[("data.table", &["new_sym"])]));
        assert_eq!(
            p.origin("new_sym", &[loaded("data.table")]),
            PackageOrigin::Resolved(SmolStr::new("data.table"))
        );
        assert_eq!(
            p.origin("fread", &[loaded("data.table")]),
            PackageOrigin::Unknown
        );
    }

    #[test]
    fn meta_package_attaches_resolve_member_exports() {
        // `library(tidyverse)` attaches tibble, dplyr, etc. via `.onAttach`.
        // Those members are not tidyverse's own exports, but their names must
        // still resolve. No local index: the bundled CRAN lists back the members.
        let p = CompositeProvider::base_only();
        // `tibble` is exported (and re-exported) by tidyverse's core members; it
        // must resolve to *some* package rather than staying Unknown. (Several
        // members re-export it, so the exact origin is legitimately Ambiguous —
        // the rule only fires on Unknown.)
        assert!(matches!(
            p.origin("tibble", &[loaded("tidyverse")]),
            PackageOrigin::Resolved(_) | PackageOrigin::Ambiguous(_)
        ));
        // `across` is a dplyr export, also attached by tidyverse.
        assert_eq!(
            p.origin("across", &[loaded("tidyverse")]),
            PackageOrigin::Resolved(SmolStr::new("dplyr"))
        );
        // A genuinely unknown name still stays Unknown so the rule can fire.
        assert_eq!(
            p.origin("not_a_real_export_xyz", &[loaded("tidyverse")]),
            PackageOrigin::Unknown
        );
    }

    fn meta_pkg(name: &str, exports: &[&str], attaches: &[&str]) -> PackageIndex {
        let mut idx = pkg(name, exports);
        idx.attaches = attaches.iter().map(|m| SmolStr::new(*m)).collect();
        idx
    }

    #[test]
    fn harvested_attach_set_resolves_member_exports() {
        // "metaverse" is not in the static curated table; only its harvested
        // attach set makes the member's exports resolve.
        let p = CompositeProvider::with_index(IndexedProvider::from_indices([
            meta_pkg("metaverse", &[], &["dplyr"]),
            pkg("dplyr", &["across"]),
        ]));
        assert_eq!(
            p.origin("across", &[loaded("metaverse")]),
            PackageOrigin::Resolved(SmolStr::new("dplyr"))
        );
    }

    #[test]
    fn harvested_attach_set_overrides_static_table() {
        // A harvested tidyverse whose attach set omits dplyr: the
        // version-exact capture is authoritative, so dplyr's exports no
        // longer resolve through the curated table.
        let p = CompositeProvider::with_index(IndexedProvider::from_indices([meta_pkg(
            "tidyverse",
            &[],
            &["stringr"],
        )]));
        assert_eq!(
            p.origin("across", &[loaded("tidyverse")]),
            PackageOrigin::Unknown
        );
    }

    #[test]
    fn empty_harvested_attach_set_falls_back_to_static_table() {
        // An installed tidyverse whose capture found nothing must keep the
        // known-correct curated members.
        let p = CompositeProvider::with_index(IndexedProvider::from_indices([meta_pkg(
            "tidyverse",
            &[],
            &[],
        )]));
        assert_eq!(
            p.origin("across", &[loaded("tidyverse")]),
            PackageOrigin::Resolved(SmolStr::new("dplyr"))
        );
    }

    #[test]
    fn attached_packages_prefers_harvested_over_static() {
        let p = CompositeProvider::with_index(IndexedProvider::from_indices([meta_pkg(
            "tidyverse",
            &[],
            &["stringr"],
        )]));
        assert_eq!(
            p.attached_packages("tidyverse"),
            vec![SmolStr::new("stringr")]
        );
        // Without a harvested set the curated table answers.
        let p = CompositeProvider::base_only();
        assert_eq!(p.attached_packages("tidyverse").len(), 9);
        assert!(p.attached_packages("dplyr").is_empty());
    }

    #[test]
    fn exports_only_load_matches_full_load_membership() {
        use crate::rindex::cache::Cache;
        use crate::rindex::schema::{Formal, HelpDoc};

        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf());
        let mut idx = pkg("dplyr", &["filter", "across"]);
        // Rich payload on one symbol, plus an unexported internal.
        idx.symbols[0].formals = Some(vec![Formal {
            name: SmolStr::new(".data"),
            default: None,
        }]);
        idx.symbols[0].help = Some(HelpDoc {
            title: Some("Keep rows".to_string()),
            ..Default::default()
        });
        idx.symbols.push(SymbolEntry {
            name: SmolStr::new("internal_helper"),
            kind: SymbolKind::Function,
            exported: false,
            formals: None,
            help: None,
        });
        cache.write_package(&idx).unwrap();
        cache
            .write_package(&meta_pkg("tidyverse", &[], &["dplyr"]))
            .unwrap();

        let full = IndexedProvider::from_cache(&cache);
        let lean = IndexedProvider::from_cache_exports(&cache);

        // Membership semantics are identical to the full load...
        assert!(lean.has_package("dplyr"));
        for name in ["filter", "across", "internal_helper", "nope"] {
            assert_eq!(
                lean.exports("dplyr", name),
                full.exports("dplyr", name),
                "membership diverged for {name}"
            );
        }
        // ...and so are the harvested attach sets (the lint CLI's lean load
        // must not silently fall back to the static table).
        for pkg in ["tidyverse", "dplyr"] {
            assert_eq!(
                lean.attaches(pkg),
                full.attaches(pkg),
                "attaches diverged for {pkg}"
            );
        }
        assert_eq!(
            full.attaches("tidyverse"),
            Some(&[SmolStr::new("dplyr")][..])
        );
        // ...but the rich per-symbol data is deliberately not loaded.
        assert!(lean.lookup("dplyr", "filter").is_none());
        assert!(lean.package("dplyr").is_none());
    }

    #[test]
    fn backtick_quoted_name_resolves_through_every_tier() {
        // Export lists — baked-in, harvested, remote, and bundled alike — store
        // operator names unquoted, but a reference to one is a single IDENT
        // token with the backticks inside it.
        let p = CompositeProvider::base_only();
        assert!(p.is_base("`%in%`"));
        assert_eq!(
            p.origin("`:`", &[]),
            PackageOrigin::Resolved(SmolStr::new("base"))
        );
        // Harvested index.
        let p = CompositeProvider::with_index(IndexedProvider::from_indices([pkg(
            "magrittr",
            &["%>%"],
        )]));
        assert_eq!(
            p.origin("`%>%`", &[loaded("magrittr")]),
            PackageOrigin::Resolved(SmolStr::new("magrittr"))
        );
        // Remote sidecar.
        let p = CompositeProvider::base_only().with_remote(remote(&[("tinytable", &["%op%"])]));
        assert_eq!(
            p.origin("`%op%`", &[loaded("tinytable")]),
            PackageOrigin::Resolved(SmolStr::new("tinytable"))
        );
        // Bundled CRAN list.
        let p = CompositeProvider::base_only();
        assert_eq!(
            p.origin("`%>%`", &[loaded("dplyr")]),
            PackageOrigin::Resolved(SmolStr::new("dplyr"))
        );
        // A backtick-quoted name that no tier exports is still Unknown.
        assert_eq!(
            p.origin("`not_a_real_export_xyz`", &[loaded("dplyr")]),
            PackageOrigin::Unknown
        );
    }

    #[test]
    fn lookup_accepts_a_backtick_quoted_name() {
        let provider = IndexedProvider::from_indices([pkg("magrittr", &["%>%"])]);
        assert!(provider.lookup("magrittr", "`%>%`").is_some());
        assert!(provider.lookup("magrittr", "`nope`").is_none());
    }

    #[test]
    fn lookup_exposes_rich_data() {
        let provider = IndexedProvider::from_indices([pkg("dplyr", &["filter"])]);
        assert!(provider.lookup("dplyr", "filter").is_some());
        assert!(provider.lookup("dplyr", "nope").is_none());
        assert!(provider.has_package("dplyr"));
    }
}
