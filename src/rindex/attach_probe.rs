//! Opt-in `search()`-diff attach probe.
//!
//! The harvest-time heuristic ([`detect_attaches`](crate::rindex::harvest))
//! reads well-known attach-set variables and so misses meta-packages that
//! don't follow the convention. This probe observes the ground truth instead:
//! spawn `R --vanilla`, `library()` the package, and diff `search()` before
//! and after. That *executes the package's attach hooks*, which crosses the
//! project's "no R user-code evaluation" line — so the probe never runs by
//! default. It is enabled per run (`arity index --attach-probe`) or per user
//! ([`ENV_VAR`]), the same consent model as `ARITY_REMOTE_URL`; a committed
//! project setting cannot switch it on.
//!
//! The diff naturally includes `Depends`-driven attachment, which is more
//! complete than the `.onAttach` story alone. Probe failures (no R on PATH,
//! load error, missing sentinel) are silent no-ops: the heuristic's result —
//! or the static fallback table — simply stands.

use std::path::PathBuf;
use std::process::Command;

use smol_str::SmolStr;

use crate::rindex::harvest::{is_valid_package_name, validate_attach_names};
use crate::rindex::libpaths::LibrarySearch;

/// Setting this (to anything but `""` or `"0"`) enables the probe for every
/// index build, including the LSP's background builds.
pub const ENV_VAR: &str = "ARITY_ATTACH_PROBE";

/// Everything before this line in the probe's stdout is package startup noise
/// (`.onAttach` messages are allowed to print); only lines after it are the
/// attached-package names.
const SENTINEL: &str = "ARITY-ATTACHES";

/// True when [`ENV_VAR`] opts this user into the probe.
pub fn enabled_by_env() -> bool {
    std::env::var(ENV_VAR).is_ok_and(|v| !v.is_empty() && v != "0")
}

/// Observe what `library(package)` attaches, by diffing `search()` in a fresh
/// `R --vanilla` session whose `R_LIBS` is `search`'s directories. Returns
/// `None` when the probe cannot run, the package fails to load, or the
/// observed set fails the same validation as the harvest heuristic.
pub fn probe_attaches(package: &str, search: &LibrarySearch) -> Option<Vec<SmolStr>> {
    // The name is interpolated into the R script; only probe syntactically
    // valid package names (the candidate list comes from project source text).
    if !is_valid_package_name(package) {
        return None;
    }
    let script = format!(
        "before <- search(); \
         suppressPackageStartupMessages(library({package})); \
         cat(\"{SENTINEL}\\n\"); \
         nw <- setdiff(search(), before); \
         cat(sub(\"^package:\", \"\", grep(\"^package:\", nw, value = TRUE)), sep = \"\\n\")"
    );
    let output = Command::new("R")
        .args(["--vanilla", "-s", "-e", &script])
        .env("R_LIBS", join_path_list(search.dirs()))
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let names = parse_probe_output(&stdout)?;
    let installed = |member: &str| search.find_package(member).is_some();
    validate_attach_names(
        &names.iter().map(String::as_str).collect::<Vec<_>>(),
        package,
        &installed,
    )
}

/// The attached-package names after the sentinel line, or `None` when the
/// sentinel never appeared (the load failed before reaching it).
fn parse_probe_output(stdout: &str) -> Option<Vec<String>> {
    let mut lines = stdout.lines();
    lines.find(|line| line.trim() == SENTINEL)?;
    Some(
        lines
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
    )
}

/// Join library directories into an `R_LIBS`-style list (`;` on Windows).
fn join_path_list(dirs: &[PathBuf]) -> String {
    let sep = if cfg!(windows) { ";" } else { ":" };
    dirs.iter()
        .map(|d| d.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(sep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ignores_startup_noise_before_sentinel() {
        let out = "Welcome to somepkg!\npackage:fake\nARITY-ATTACHES\ntools\nutils2\n";
        assert_eq!(
            parse_probe_output(out).unwrap(),
            vec!["tools".to_string(), "utils2".to_string()]
        );
    }

    #[test]
    fn parse_without_sentinel_is_none() {
        // A failed `library()` aborts the script before the sentinel prints.
        assert!(parse_probe_output("Error: package not found\n").is_none());
    }

    #[test]
    fn parse_empty_diff_is_empty() {
        // Sentinel present, nothing attached: an ordinary package.
        assert_eq!(
            parse_probe_output("ARITY-ATTACHES\n").unwrap(),
            Vec::<String>::new()
        );
    }
}
