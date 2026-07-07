//! Discover which packages a project references, so `arity index` harvests
//! only those. A package is "referenced" if it is attached via
//! `library()`/`require()`/`requireNamespace()` or named via `pkg::` / `pkg:::`
//! anywhere in the project's `.R` files.

use std::collections::BTreeSet;

use smol_str::SmolStr;

use crate::file_discovery::{ExcludeFilter, FileDiscoveryError, collect_r_files};
use crate::parser::parse;
use crate::semantic::SemanticModel;

/// The set of package names referenced anywhere under `paths`, sorted and
/// deduplicated. Files matching `exclude` are skipped so `arity index` never
/// harvests packages referenced only from generated or vendored sources. Parse
/// failures on individual files are skipped (discovery is best-effort).
pub fn referenced_packages(
    paths: &[std::path::PathBuf],
    exclude: &ExcludeFilter,
) -> Result<Vec<SmolStr>, FileDiscoveryError> {
    let files = collect_r_files(paths, exclude)?;
    let mut set: BTreeSet<SmolStr> = BTreeSet::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        collect_into(&text, &mut set);
    }
    Ok(set.into_iter().collect())
}

/// Collect referenced package names from a single source string.
pub fn referenced_in_source(source: &str) -> Vec<SmolStr> {
    let mut set = BTreeSet::new();
    collect_into(source, &mut set);
    set.into_iter().collect()
}

/// R's always-attached default packages followed by `referenced`, deduplicated.
/// An index should always cover the default packages (`base`, `stats`, …) so
/// hover and signatures resolve for base-R symbols, which no source file
/// `library()`s explicitly. Defaults come first; `referenced` entries already in
/// the default set are dropped.
pub fn with_default_packages(referenced: Vec<SmolStr>) -> Vec<SmolStr> {
    let mut out: Vec<SmolStr> = crate::semantic::symbols::default_packages()
        .iter()
        .map(|p| SmolStr::new(*p))
        .collect();
    for pkg in referenced {
        if !out.contains(&pkg) {
            out.push(pkg);
        }
    }
    out
}

fn collect_into(source: &str, set: &mut BTreeSet<SmolStr>) {
    let parsed = parse(source);
    let model = SemanticModel::build(&parsed.cst);
    for pkg in model.loaded_packages() {
        set.insert(pkg.name.clone());
    }
    for pkg in model.referenced_packages() {
        set.insert(pkg.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_attached_and_referenced() {
        let src = r#"
            library(dplyr)
            require(tidyr)
            requireNamespace("purrr")
            rlang::abort("x")
            stringr:::impl()
        "#;
        let found = referenced_in_source(src);
        let pkgs: Vec<&str> = found.iter().map(|s| s.as_str()).collect();
        for expected in ["dplyr", "tidyr", "purrr", "rlang", "stringr"] {
            assert!(pkgs.contains(&expected), "missing {expected} in {pkgs:?}");
        }
    }

    #[test]
    fn referenced_packages_honors_exclude() {
        use std::fs;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("keep.R"), "library(dplyr)\n").unwrap();
        fs::create_dir(root.join("renv")).unwrap();
        fs::write(root.join("renv").join("activate.R"), "library(secret)\n").unwrap();

        let filter = ExcludeFilter::new(root, &["renv/".to_string()]).unwrap();
        let pkgs = referenced_packages(&[root.to_path_buf()], &filter).unwrap();
        let names: Vec<&str> = pkgs.iter().map(|s| s.as_str()).collect();
        assert!(
            names.contains(&"dplyr"),
            "kept file should be scanned: {names:?}"
        );
        assert!(
            !names.contains(&"secret"),
            "excluded renv/ leaked into discovery: {names:?}"
        );
    }

    #[test]
    fn deduplicates() {
        let src = "library(dplyr)\ndplyr::filter(x)\ndplyr::select(y)";
        assert_eq!(referenced_in_source(src), vec![SmolStr::new("dplyr")]);
    }

    #[test]
    fn with_defaults_prepends_and_dedups() {
        let out = with_default_packages(vec![SmolStr::new("dplyr"), SmolStr::new("stats")]);
        // Default packages lead, base first.
        assert_eq!(out.first().map(SmolStr::as_str), Some("base"));
        // A non-default reference is appended.
        assert!(out.contains(&SmolStr::new("dplyr")));
        // `stats` is a default, so it appears once, not twice.
        assert_eq!(out.iter().filter(|s| s.as_str() == "stats").count(), 1);
    }
}
