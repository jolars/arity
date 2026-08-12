//! Differential oracle: arity's dependency-exemption sets against R's own.
//!
//! `undeclared-dependency` decides what needs declaring by copying `R CMD
//! check`. The two lists it copies are data that changes with R itself — a new
//! base-priority package would silently become a false positive — so they are
//! checked against R rather than asserted in a comment:
//!
//! - `tools:::.get_standard_package_names()$base` is
//!   [`base_priority_packages`].
//! - That set minus `methods` and `stats4` — the `standard_package_names` local
//!   in `tools:::.check_packages_used` — is [`is_implicitly_available`].
//!
//! `#[ignore]`d because it needs R: run via `task deps-oracle`. A missing
//! `Rscript` is a skip, never a failure, exactly as in `dcf_oracle`.
//!
//! [`base_priority_packages`]: arity::semantic::symbols::base_priority_packages
//! [`is_implicitly_available`]: arity::semantic::symbols::is_implicitly_available

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

use arity::semantic::symbols::{base_priority_packages, is_implicitly_available};

#[test]
#[ignore = "needs R; run via `task deps-oracle`"]
fn base_priority_packages_match_r() {
    let Some(rscript) = locate_rscript() else {
        eprintln!("deps-oracle: `Rscript` not found on PATH; skipping (this is not a failure).");
        return;
    };

    let ours: BTreeSet<&str> = base_priority_packages().iter().copied().collect();
    let theirs = r_names(
        &rscript,
        "cat(tools:::.get_standard_package_names()$base, sep = \"\\n\")",
    );
    assert_eq!(
        ours,
        theirs.iter().map(String::as_str).collect::<BTreeSet<_>>(),
        "`base_priority_packages` has drifted from this R's base-priority set",
    );
}

/// The exemption set proper. Pinned separately from the list it derives from,
/// because the `methods`/`stats4` carve-out is R's own decision and not
/// something we could re-derive.
#[test]
#[ignore = "needs R; run via `task deps-oracle`"]
fn the_exempt_set_matches_r_cmd_check() {
    let Some(rscript) = locate_rscript() else {
        eprintln!("deps-oracle: `Rscript` not found on PATH; skipping (this is not a failure).");
        return;
    };

    let theirs = r_names(
        &rscript,
        "cat(setdiff(tools:::.get_standard_package_names()$base, \
         c(\"methods\", \"stats4\")), sep = \"\\n\")",
    );
    let ours: BTreeSet<&str> = base_priority_packages()
        .iter()
        .copied()
        .filter(|pkg| is_implicitly_available(pkg))
        .collect();
    assert_eq!(
        ours,
        theirs.iter().map(String::as_str).collect::<BTreeSet<_>>(),
        "`is_implicitly_available` has drifted from `tools:::.check_packages_used`",
    );
    // The carve-out itself, stated so a drift in either direction is legible.
    assert!(!is_implicitly_available("methods"));
    assert!(!is_implicitly_available("stats4"));
}

fn r_names(rscript: &PathBuf, expr: &str) -> BTreeSet<String> {
    let output = Command::new(rscript)
        .arg("-e")
        .arg(expr)
        .output()
        .expect("Rscript should run");
    assert!(
        output.status.success(),
        "Rscript failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn locate_rscript() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("ARITY_RSCRIPT") {
        return Some(PathBuf::from(path));
    }
    Command::new("Rscript")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
        .then(|| PathBuf::from("Rscript"))
}
