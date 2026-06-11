//! Orchestrate an index build: resolve each referenced package to an installed
//! library, harvest it, and write it to the cache. Shared by the `ravel index`
//! CLI command and (later) the LSP's lazy background build.

use rayon::prelude::*;
use smol_str::SmolStr;

use crate::rindex::cache::Cache;
use crate::rindex::harvest::{HarvestOptions, harvest_package};
use crate::rindex::libpaths::LibrarySearch;

#[derive(Debug, Clone, Copy)]
pub struct BuildOptions {
    pub help: bool,
    /// Re-harvest and rewrite even when the installed version is already indexed.
    pub force: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        BuildOptions {
            help: true,
            force: false,
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
    // Phase 1 (parallel): resolve, harvest, and write each package's own
    // `pkg@ver.json`. These are independent — distinct files, read-only meta
    // lookups, in-process harvesting (no subprocess) — so they fan out across
    // rayon cleanly. The shared `meta.json` is *not* touched here; deferring it
    // to phase 2 is what makes this safe to parallelize (a per-package
    // read-modify-write of meta would race and lose entries). `par_iter().map()`
    // preserves input order, so the report stays deterministic.
    let report: Vec<(SmolStr, PackageOutcome)> = packages
        .par_iter()
        .map(|pkg| {
            let harvest_opts = HarvestOptions { help: opts.help };
            let outcome = match search.find_package(pkg) {
                None => PackageOutcome::NotInstalled,
                Some(pkg_dir) => match harvest_package(&pkg_dir, harvest_opts, now) {
                    Err(e) => PackageOutcome::Failed {
                        reason: e.to_string(),
                    },
                    Ok(index) => {
                        let already =
                            cache.indexed_version(pkg).as_deref() == Some(index.version.as_str());
                        if already && !opts.force {
                            PackageOutcome::UpToDate {
                                version: index.version.clone(),
                            }
                        } else {
                            match cache.write_package_file(&index) {
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

    // Phase 2 (sequential, once): fold every newly-indexed version into
    // `meta.json` in a single read-modify-write.
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

    fn fixture_lib() -> LibrarySearch {
        // Point a LibrarySearch at the checked-in fixtures, whose layout is
        // `tests/fixtures/rindex/<pkg>/...`.
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rindex");
        LibrarySearch::discover(None, &[dir])
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
