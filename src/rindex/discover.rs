//! Discover which packages a project references, so `arity index` harvests
//! only those. A package is "referenced" if it is attached via
//! `library()`/`require()`/`requireNamespace()` or named via `pkg::` / `pkg:::`
//! anywhere in the project's `.R` files.

use std::collections::BTreeSet;

use smol_str::SmolStr;

use crate::file_discovery::{FileDiscoveryError, collect_r_files};
use crate::parser::parse;
use crate::semantic::SemanticModel;

/// The set of package names referenced anywhere under `paths`, sorted and
/// deduplicated. Parse failures on individual files are skipped (discovery is
/// best-effort).
pub fn referenced_packages(
    paths: &[std::path::PathBuf],
) -> Result<Vec<SmolStr>, FileDiscoveryError> {
    let files = collect_r_files(paths)?;
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
    fn deduplicates() {
        let src = "library(dplyr)\ndplyr::filter(x)\ndplyr::select(y)";
        assert_eq!(referenced_in_source(src), vec![SmolStr::new("dplyr")]);
    }
}
