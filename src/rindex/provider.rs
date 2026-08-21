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
use std::sync::{LazyLock, OnceLock};

use smol_str::SmolStr;

use crate::rindex::cache::Cache;
use crate::rindex::remote::RemoteExports;
use crate::rindex::schema::{PackageExports, PackageIndex, SymbolEntry};
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

/// A package's membership view: everything resolution asks about it.
#[derive(Debug, Default)]
struct Membership {
    /// The package's exported names.
    exports: HashSet<SmolStr>,
    /// Its harvested attach set, empty when capture found nothing.
    attaches: Vec<SmolStr>,
}

impl From<PackageExports> for Membership {
    fn from(exp: PackageExports) -> Self {
        Membership {
            exports: exp
                .symbols
                .into_iter()
                .filter(|s| s.exported)
                .map(|s| s.name)
                .collect(),
            attaches: exp.attaches,
        }
    }
}

/// One package the index knows of, with its membership and rich index either
/// already resolved (the eager constructors) or filled from disk on first use
/// (the lazy one). The inner `Option`s distinguish "read and empty" from "file
/// missing or stale", which must read as *not indexed*.
#[derive(Debug)]
struct PackageSlot {
    /// The version `meta.json` names — the file a lazy fill reads.
    version: SmolStr,
    membership: OnceLock<Option<Membership>>,
    /// The full harvested index (formals + help bodies), filled per package on
    /// the first rich question. Keeping this per-slot is what lets the language
    /// server hold the lazy load: resident rich data scales with the packages a
    /// session actually asks about, not with everything ever harvested.
    index: OnceLock<Option<PackageIndex>>,
}

impl PackageSlot {
    /// A slot whose membership and rich index are already known.
    fn resolved(membership: Membership, index: PackageIndex) -> Self {
        let membership_cell = OnceLock::new();
        let _ = membership_cell.set(Some(membership));
        let index_cell = OnceLock::new();
        let _ = index_cell.set(Some(index));
        PackageSlot {
            version: SmolStr::default(),
            membership: membership_cell,
            index: index_cell,
        }
    }
}

/// Resolves names against indexed, attached packages and holds the rich data.
#[derive(Debug, Default)]
pub struct IndexedProvider {
    /// package → membership + rich index, resolved eagerly or filled on use.
    packages: HashMap<SmolStr, PackageSlot>,
    /// Where an unfilled slot reads from. `None` once every slot is resolved,
    /// which is what the eager constructors produce.
    source: Option<Cache>,
}

impl IndexedProvider {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build from a set of harvested package indices.
    pub fn from_indices(indices: impl IntoIterator<Item = PackageIndex>) -> Self {
        let mut packages: HashMap<SmolStr, PackageSlot> = HashMap::new();
        for idx in indices {
            let membership = Membership {
                exports: idx
                    .symbols
                    .iter()
                    .filter(|s| s.exported)
                    .map(|s| s.name.clone())
                    .collect(),
                attaches: idx.attaches.clone(),
            };
            packages.insert(idx.package.clone(), PackageSlot::resolved(membership, idx));
        }
        IndexedProvider {
            packages,
            source: None,
        }
    }

    /// Load every package index currently named by the cache's `meta.json`.
    pub fn from_cache(cache: &Cache) -> Self {
        Self::from_indices(cache.load_all())
    }

    /// Note which packages the cache holds, deferring every package file to the
    /// first question asked about that package. `has_package` and the `exports`
    /// tests answer exactly as after [`from_cache`], but the rich per-symbol
    /// data is never deserialized, so [`lookup`](Self::lookup) and
    /// [`package`](Self::package) return `None`.
    ///
    /// This is the lint CLI's load, and it is lazy because resolution
    /// (`resolve_origin`/`package_indexed`) only ever asks about the packages a
    /// file *attaches* — `library()`, a NAMESPACE `import()`, a package's own
    /// declared attaches. A file that attaches nothing reads `meta.json` and
    /// nothing else, where the eager load deserialized every harvested package
    /// (megabytes of formals and help bodies) to answer no question at all.
    /// Consumers of the rich data (LSP hover and completion) must use
    /// [`from_cache`].
    pub fn from_cache_lazy(cache: &Cache) -> Self {
        let packages = cache
            .read_meta()
            .packages
            .into_iter()
            .map(|(package, version)| {
                (
                    package,
                    PackageSlot {
                        version,
                        membership: OnceLock::new(),
                        index: OnceLock::new(),
                    },
                )
            })
            .collect();
        IndexedProvider {
            packages,
            source: Some(cache.clone()),
        }
    }

    /// The membership view of `package`, filling it from the cache on first
    /// use. `None` when the index has no entry for `package`, or when the entry
    /// `meta.json` names cannot be read — a missing or foreign-schema file must
    /// read as *not indexed*, matching the eager load, so that
    /// `package_indexed` never claims exports it does not have.
    fn membership(&self, package: &str) -> Option<&Membership> {
        let slot = self.packages.get(package)?;
        slot.membership
            .get_or_init(|| {
                let exports = self
                    .source
                    .as_ref()?
                    .read_package_exports(package, &slot.version)?;
                Some(Membership::from(exports))
            })
            .as_ref()
    }

    /// True if this provider has an index for `package`.
    pub fn has_package(&self, package: &str) -> bool {
        self.membership(package).is_some()
    }

    /// Iterate every locally harvested package name.
    ///
    /// Reads the slot map, not `indices`, so it stays correct under the lean
    /// [`from_cache_lazy`](Self::from_cache_lazy) load, which never populates
    /// the rich map. Naming a package here is `meta.json`'s claim and not yet a
    /// read, so under that load the iterator may include a package whose file
    /// has since gone away; ask [`has_package`](Self::has_package) to settle it.
    pub fn packages(&self) -> impl Iterator<Item = &SmolStr> {
        self.packages.keys()
    }

    /// The rich entry for `pkg::name`, if indexed. Under the lazy load the
    /// package file is read on the first rich question about that package.
    pub fn lookup(&self, package: &str, name: &str) -> Option<&SymbolEntry> {
        let name = unbacktick(name);
        self.package(package)?
            .symbols
            .iter()
            .find(|s| s.name == name)
    }

    /// The full harvested index for a package, filling it from the cache on
    /// first use. `None` when the index has no entry for `package` or the file
    /// `meta.json` names cannot be read — mirroring
    /// [`membership`](Self::membership), a missing or foreign-schema file must
    /// read as *no rich data*, never an invented empty index.
    pub fn package(&self, package: &str) -> Option<&PackageIndex> {
        let slot = self.packages.get(package)?;
        slot.index
            .get_or_init(|| self.source.as_ref()?.read_package(package, &slot.version))
            .as_ref()
    }

    /// The harvested attach set for `package`, if one was captured. `None`
    /// when the package isn't indexed *or* its capture found nothing — both
    /// mean "fall back to the static table" (see [`attach_members`]).
    pub fn attaches(&self, package: &str) -> Option<&[SmolStr]> {
        self.membership(package)
            .map(|m| m.attaches.as_slice())
            .filter(|a| !a.is_empty())
    }

    fn exports(&self, package: &str, name: &str) -> bool {
        self.membership(package)
            .is_some_and(|m| m.exports.contains(unbacktick(name)))
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
    fn lazy_load_defers_package_file_reads() {
        use crate::rindex::cache::Cache;

        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf());
        cache.write_package(&pkg("dplyr", &["across"])).unwrap();
        cache.write_package(&pkg("tibble", &["tribble"])).unwrap();

        let lazy = IndexedProvider::from_cache_lazy(&cache);
        // Construction read `meta.json` only. Deleting a package file *after*
        // construction therefore still changes what the provider can answer —
        // which an eager load could not do.
        std::fs::remove_file(cache.index_dir().join("tibble@1.0.json")).unwrap();
        assert!(lazy.exports("dplyr", "across"));
        // A package `meta.json` names but whose file cannot be read is not
        // indexed, exactly as under the eager load: `package_indexed` must not
        // claim its exports are known.
        assert!(!lazy.has_package("tibble"));
        assert!(!lazy.exports("tibble", "tribble"));
    }

    #[test]
    fn lazy_load_memoizes_a_package_it_has_read() {
        use crate::rindex::cache::Cache;

        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf());
        cache.write_package(&pkg("dplyr", &["across"])).unwrap();

        let lazy = IndexedProvider::from_cache_lazy(&cache);
        assert!(lazy.exports("dplyr", "across"));
        // Once filled, the answer is stable: a second query re-reads nothing.
        std::fs::remove_file(cache.index_dir().join("dplyr@1.0.json")).unwrap();
        assert!(lazy.exports("dplyr", "across"));
        assert!(lazy.has_package("dplyr"));
    }

    #[test]
    fn lazy_load_matches_full_load_membership() {
        use crate::rindex::cache::Cache;
        use crate::rindex::schema::{Formal, HelpDoc};

        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf());
        let mut idx = pkg("dplyr", &["filter", "across"]);
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
        let lazy = IndexedProvider::from_cache_lazy(&cache);

        for name in ["filter", "across", "internal_helper", "nope"] {
            assert_eq!(
                lazy.exports("dplyr", name),
                full.exports("dplyr", name),
                "membership diverged for {name}"
            );
        }
        for pkg in ["tidyverse", "dplyr", "absent"] {
            assert_eq!(
                lazy.has_package(pkg),
                full.has_package(pkg),
                "has_package diverged for {pkg}"
            );
            // The lint CLI's load must not silently fall back to the static
            // attach table where a harvested set exists.
            assert_eq!(
                lazy.attaches(pkg),
                full.attaches(pkg),
                "attaches diverged for {pkg}"
            );
        }
        assert_eq!(
            lazy.attaches("tidyverse"),
            Some(&[SmolStr::new("dplyr")][..])
        );
        // The rich per-symbol payload is served on demand and matches the
        // eager load exactly (see `lazy_load_serves_rich_data_on_demand` for
        // the deferral itself).
        assert_eq!(
            lazy.lookup("dplyr", "filter"),
            full.lookup("dplyr", "filter")
        );
        assert_eq!(lazy.package("dplyr"), full.package("dplyr"));
    }

    #[test]
    fn lazy_load_serves_rich_data_on_demand() {
        use crate::rindex::cache::Cache;
        use crate::rindex::schema::HelpDoc;

        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf());
        let mut idx = pkg("dplyr", &["filter"]);
        idx.symbols[0].help = Some(HelpDoc {
            title: Some("Keep rows".to_string()),
            ..Default::default()
        });
        cache.write_package(&idx).unwrap();
        cache.write_package(&pkg("tibble", &["tribble"])).unwrap();

        // The language server holds this provider for days over caches that
        // eagerly total gigabytes (issue #116); the rich payload must load per
        // package on first use, not wholesale at construction.
        let lazy = IndexedProvider::from_cache_lazy(&cache);
        // Deleting tibble's file after construction proves construction read
        // `meta.json` only; dplyr's rich data is still served on demand.
        std::fs::remove_file(cache.index_dir().join("tibble@1.0.json")).unwrap();
        let entry = lazy.lookup("dplyr", "filter").expect("rich entry");
        assert_eq!(
            entry.help.as_ref().and_then(|h| h.title.as_deref()),
            Some("Keep rows")
        );
        assert_eq!(
            lazy.package("dplyr").map(|p| p.package.as_str()),
            Some("dplyr")
        );
        // A named-but-unreadable package has no rich data, and answering that
        // must not panic or invent an empty index.
        assert!(lazy.package("tibble").is_none());
        assert!(lazy.lookup("tibble", "tribble").is_none());

        // Once filled, the payload is stable: a second query re-reads nothing.
        std::fs::remove_file(cache.index_dir().join("dplyr@1.0.json")).unwrap();
        assert!(lazy.lookup("dplyr", "filter").is_some());
    }

    #[test]
    fn lazy_load_resolves_through_the_composite_provider() {
        use crate::rindex::cache::Cache;

        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf());
        cache.write_package(&pkg("dplyr", &["across"])).unwrap();
        cache
            .write_package(&meta_pkg("metaverse", &[], &["dplyr"]))
            .unwrap();

        let p = CompositeProvider::with_index(IndexedProvider::from_cache_lazy(&cache));
        assert!(p.package_indexed("dplyr"));
        assert_eq!(
            p.origin("across", &[loaded("dplyr")]),
            PackageOrigin::Resolved(SmolStr::new("dplyr"))
        );
        // The harvested attach set is reachable through the lazy load too.
        assert_eq!(
            p.origin("across", &[loaded("metaverse")]),
            PackageOrigin::Resolved(SmolStr::new("dplyr"))
        );
        assert_eq!(
            p.attached_packages("metaverse"),
            vec![SmolStr::new("dplyr")]
        );
    }

    #[test]
    fn lazy_load_ignores_a_foreign_schema_version() {
        use crate::rindex::cache::Cache;

        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf());
        let mut future = pkg("dplyr", &["across"]);
        future.schema_version = SCHEMA_VERSION + 1;
        cache.write_package(&future).unwrap();

        let lazy = IndexedProvider::from_cache_lazy(&cache);
        assert!(!lazy.has_package("dplyr"));
        assert!(!lazy.exports("dplyr", "across"));
    }

    #[test]
    fn indexed_provider_is_shareable_across_threads() {
        // The lazy fill happens behind a shared `&self`; the provider lives in
        // an `Arc` inside the salsa `LibraryIndex` and is read from rayon
        // workers, so it must stay `Send + Sync`.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<IndexedProvider>();
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
