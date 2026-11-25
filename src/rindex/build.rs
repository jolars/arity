//! Orchestrate an index build: resolve each referenced package to an installed
//! library, harvest it, and write it to the cache. Shared by the `arity index`
//! CLI command and (later) the LSP's lazy background build.

use rayon::prelude::*;
use smol_str::SmolStr;

use crate::rindex::attach_probe;
use crate::rindex::cache::Cache;
use crate::rindex::harvest::{HelpMode, harvest_with_help_mode};
use crate::rindex::lazyload;
use crate::rindex::libpaths::LibrarySearch;

#[derive(Debug, Clone, Copy)]
pub struct BuildOptions {
    pub help: bool,
    /// Re-harvest and rewrite even when the installed version is already indexed.
    pub force: bool,
    /// Run the opt-in `search()`-diff attach probe (spawns R, executes package
    /// attach hooks) for meta-packages the harvest heuristic missed. Callers
    /// resolve the `--attach-probe` flag or [`attach_probe::ENV_VAR`] — this
    /// function never reads the environment itself.
    pub attach_probe: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        BuildOptions {
            help: true,
            force: false,
            attach_probe: false,
        }
    }
}

/// What happened to one package during a build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageOutcome {
    Indexed { version: SmolStr, symbols: usize },
    UpToDate { version: SmolStr },
    NotInstalled,
    Failed { reason: String },
}

#[derive(Debug, Clone)]
pub struct BuildReport {
    pub packages: Vec<(SmolStr, PackageOutcome)>,
}

impl BuildReport {
    pub fn newly_indexed(&self) -> impl Iterator<Item = &SmolStr> {
        self.packages.iter().filter_map(|(name, outcome)| {
            matches!(outcome, PackageOutcome::Indexed { .. }).then_some(name)
        })
    }
}

/// Harvest each package in `packages` into `cache`. `now` is the wall-clock
/// timestamp (Unix seconds) stamped onto fresh indices; the caller supplies it
/// so this stays deterministic.
pub fn build_index(
    packages: &[SmolStr],
    cache: &Cache,
    search: &LibrarySearch,
    opts: BuildOptions,
    now: u64,
) -> BuildReport {
    build(
        packages,
        cache,
        search,
        opts,
        HelpMode::from_enabled(opts.help),
        now,
    )
}

/// The language server needs package exports and signatures at open. Its
/// installed help pages are decoded by the provider in editor requests.
pub(crate) fn build_index_for_lsp(
    packages: &[SmolStr],
    cache: &Cache,
    search: &LibrarySearch,
    opts: BuildOptions,
    now: u64,
) -> BuildReport {
    let help = if opts.help {
        HelpMode::OnDemand
    } else {
        HelpMode::Disabled
    };
    build(packages, cache, search, opts, help, now)
}

fn build(
    packages: &[SmolStr],
    cache: &Cache,
    search: &LibrarySearch,
    opts: BuildOptions,
    help: HelpMode,
    now: u64,
) -> BuildReport {
    // Package files are independent; defer the shared metadata update until
    // after the parallel work to avoid lost read-modify-write updates.
    let report: Vec<(SmolStr, PackageOutcome)> = packages
        .par_iter()
        .map(|pkg| {
            let directory = search.find_package(pkg).map(|directory| {
                if help == HelpMode::OnDemand && directory.is_relative() {
                    // A later server can open the same cache from another cwd.
                    std::path::absolute(&directory).unwrap_or(directory)
                } else {
                    directory
                }
            });
            let outcome = match directory {
                None => PackageOutcome::NotInstalled,
                Some(pkg_dir) => match harvest_with_help_mode(&pkg_dir, help, now, search) {
                    Err(e) => PackageOutcome::Failed {
                        reason: e.to_string(),
                    },
                    Ok(index) => {
                        let already = cache.indexed_version(pkg).as_deref()
                            == Some(index.version.as_str())
                            && cache
                                .open_package(pkg, &index.version)
                                .is_some_and(|cached| {
                                    help != HelpMode::Eager
                                        || cached.has_deferred_help() == Some(false)
                                });
                        if already && !opts.force {
                            PackageOutcome::UpToDate {
                                version: index.version.clone(),
                            }
                        } else {
                            match cache
                                .write_package_file_with_help(&index, help == HelpMode::OnDemand)
                            {
                                Ok(()) => PackageOutcome::Indexed {
                                    version: index.version.clone(),
                                    symbols: index.symbols.len(),
                                },
                                Err(e) => PackageOutcome::Failed {
                                    reason: e.to_string(),
                                },
                            }
                        }
                    }
                },
            };
            (pkg.clone(), outcome)
        })
        .collect();

    if opts.attach_probe {
        // Keep subprocess-based probing out of the parallel harvest.
        for (pkg, outcome) in &report {
            let version = match outcome {
                PackageOutcome::Indexed { version, .. } | PackageOutcome::UpToDate { version } => {
                    version
                }
                _ => continue,
            };
            let Some(file) = cache.open_package(pkg, version) else {
                continue;
            };
            let Some(mut index) = file.package() else {
                continue;
            };
            let deferred_help = file.has_deferred_help().unwrap_or_default();
            if !index.attaches.is_empty() {
                continue;
            }
            // Cheap precondition, no `.rdb` decode: a package without an
            // `.onAttach` hook cannot attach anything beyond its `Depends`.
            let Some(pkg_dir) = search.find_package(pkg) else {
                continue;
            };
            let rdx = pkg_dir.join("R").join(format!("{pkg}.rdx"));
            let Ok(names) = lazyload::read_index_names(&rdx) else {
                continue;
            };
            if !names.iter().any(|n| n == ".onAttach") {
                continue;
            }
            let Some(members) = attach_probe::probe_attaches(pkg, search) else {
                continue;
            };
            index.attaches = members;
            let _ = cache.write_package_file_with_help(&index, deferred_help);
        }
    }

    let newly: Vec<(SmolStr, SmolStr)> = report
        .iter()
        .filter_map(|(pkg, outcome)| match outcome {
            PackageOutcome::Indexed { version, .. } => Some((pkg.clone(), version.clone())),
            _ => None,
        })
        .collect();
    let _ = cache.record_indexed(&newly);

    let _ = cache.gc_old_schema_dirs();
    BuildReport { packages: report }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn attach_probe_preflight_needs_only_the_index() {
        let temp = tempfile::tempdir().unwrap();
        let index = temp.path().join("metatoy.rdx");
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/rindex/metatoy/R/metatoy.rdx");
        std::fs::copy(fixture, &index).unwrap();
        assert!(!index.with_extension("rdb").exists());
        assert!(
            lazyload::read_index_names(&index)
                .unwrap()
                .iter()
                .any(|name| name == ".onAttach")
        );
        std::fs::write(&index, b"corrupt index").unwrap();
        assert!(lazyload::read_index_names(&index).is_err());
    }

    fn fixture_lib() -> LibrarySearch {
        // Point a LibrarySearch at the checked-in fixtures, whose layout is
        // `tests/fixtures/rindex/<pkg>/...`.
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rindex");
        LibrarySearch::from_dirs(vec![dir])
    }

    #[test]
    fn lsp_build_defers_help_until_an_editor_request() {
        use crate::lsp::{
            compute_completions, compute_hover, compute_signature_help, resolve_completion,
        };
        use crate::rindex::provider::IndexedProvider;
        use crate::text::PositionEncoding;
        use lsp_types::CompletionResponse;

        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf());
        let search = fixture_lib();
        let packages = ["magrittr".into(), "oopfixture".into()];
        let opts = BuildOptions::default();
        build_index_for_lsp(&packages, &cache, &search, opts, 0);

        let package = cache.read_package("magrittr", "2.0.4").unwrap();
        assert!(package.symbols.iter().all(|symbol| symbol.help.is_none()));
        let deferred = IndexedProvider::from_cache_lazy(&cache);
        let object_version = cache.indexed_version("oopfixture").unwrap();
        let objects = cache
            .read_package("oopfixture", &object_version)
            .unwrap()
            .objects;
        assert!(objects.help.is_empty());
        let association = objects
            .classes
            .iter()
            .find_map(|class| class.help.as_ref())
            .unwrap();

        let source = "magrittr::`%>%`";
        let hover = |indexed| {
            compute_hover(source, source.len() - 2, indexed, PositionEncoding::Utf8).unwrap()
        };
        let call = "magrittr::`%>%`(x, )";
        let signature = |indexed| compute_signature_help(call, call.len() - 1, indexed).unwrap();
        let resolve = |indexed| {
            let source = "magrittr::fre";
            let Some(CompletionResponse::List(list)) =
                compute_completions(source, source.len(), indexed)
            else {
                panic!("expected namespace completions");
            };
            let item = list
                .items
                .into_iter()
                .find(|item| item.label == "freduce")
                .unwrap();
            resolve_completion(item, indexed)
        };
        let deferred_hover = hover(&deferred);
        let deferred_signature = signature(&deferred);
        let deferred_completion = resolve(&deferred);
        let deferred_object = deferred.deferred_help("oopfixture", &association.alias);

        let report = build_index(&packages, &cache, &search, opts, 0);
        assert_eq!(report.newly_indexed().count(), 2);
        let eager = IndexedProvider::from_cache_lazy(&cache);
        let eager_hover = hover(&eager);
        assert!(format!("{eager_hover:?}").contains("Pipe an object"));
        assert_eq!(deferred_hover, eager_hover);
        assert_eq!(deferred_signature, signature(&eager));
        assert!(deferred_completion.documentation.is_some());
        let eager_completion = resolve(&eager);
        assert_eq!(
            deferred_completion.documentation,
            eager_completion.documentation
        );
        assert_eq!(deferred_completion.detail, eager_completion.detail);
        let object_package = eager.object_package("oopfixture").unwrap();
        assert!(deferred_object.is_some());
        assert_eq!(
            deferred_object.as_ref(),
            object_package.objects.render_help(association)
        );
    }

    #[test]
    fn cached_help_requires_an_enabled_matching_installation() {
        use crate::rindex::provider::IndexedProvider;

        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf());
        let search = fixture_lib();
        let packages = ["magrittr".into()];
        let opts = BuildOptions {
            help: false,
            ..BuildOptions::default()
        };
        build_index_for_lsp(&packages, &cache, &search, opts, 0);
        let disabled = IndexedProvider::from_cache_lazy(&cache);
        assert!(
            disabled
                .lookup_with_help("magrittr", "%>%")
                .unwrap()
                .help
                .is_none()
        );

        // The file can still supply exports and formals if the installed
        // version has moved on. Rd from the newer installation must not mix
        // with the old symbol record.
        let mut old = cache.read_package("magrittr", "2.0.4").unwrap();
        old.version = "2.0.3".into();
        cache.write_package_file_with_help(&old, true).unwrap();
        cache.record_indexed(&[(old.package, old.version)]).unwrap();
        let stale = IndexedProvider::from_cache_lazy(&cache);
        let entry = stale.lookup_with_help("magrittr", "%>%").unwrap();
        assert!(entry.help.is_none());
        assert!(entry.formals.is_some());
    }

    #[test]
    fn builds_and_skips_up_to_date() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf());
        let search = fixture_lib();
        let pkgs = vec![SmolStr::new("magrittr"), SmolStr::new("nonexistent")];

        let report = build_index(&pkgs, &cache, &search, BuildOptions::default(), 0);
        let outcomes: std::collections::HashMap<_, _> = report.packages.iter().cloned().collect();
        assert!(matches!(
            outcomes.get(&SmolStr::new("magrittr")),
            Some(PackageOutcome::Indexed { .. })
        ));
        assert_eq!(
            outcomes.get(&SmolStr::new("nonexistent")),
            Some(&PackageOutcome::NotInstalled)
        );

        // The cache now resolves magrittr.
        assert_eq!(cache.indexed_version("magrittr").as_deref(), Some("2.0.4"));

        // A second build without --force reports it as up to date.
        let report2 = build_index(
            &[SmolStr::new("magrittr")],
            &cache,
            &search,
            BuildOptions::default(),
            0,
        );
        assert!(matches!(
            report2.packages[0].1,
            PackageOutcome::UpToDate { .. }
        ));
    }

    #[test]
    fn parallel_build_records_every_package_in_meta() {
        // Regression guard for the parallel harvest: building multiple packages
        // in one call must record *all* of them in `meta.json`. A naive
        // per-package read-modify-write of the shared meta under `par_iter`
        // would race and drop entries; the batched `record_indexed` must not.
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf());
        let search = fixture_lib();
        let pkgs = vec![SmolStr::new("magrittr"), SmolStr::new("R.oo")];

        let report = build_index(&pkgs, &cache, &search, BuildOptions::default(), 0);

        // Report order mirrors input order (deterministic `par_iter().map()`).
        assert_eq!(report.packages[0].0, SmolStr::new("magrittr"));
        assert_eq!(report.packages[1].0, SmolStr::new("R.oo"));
        for (pkg, outcome) in &report.packages {
            assert!(
                matches!(outcome, PackageOutcome::Indexed { .. }),
                "{pkg} should be freshly indexed, got {outcome:?}"
            );
        }

        // Both must survive in meta.json — the property the batched write protects.
        assert!(cache.indexed_version("magrittr").is_some());
        assert!(cache.indexed_version("R.oo").is_some());
        assert_eq!(cache.load_all().len(), 2);
    }
}
