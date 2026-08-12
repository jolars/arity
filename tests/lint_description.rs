//! Linting `DESCRIPTION`: discovery, the driver's DCF pass, and the rules that
//! run over it.
//!
//! Separate from `tests/lint.rs` because the subject is a second grammar with
//! its own driver (`run_dcf_rules`) and its own discovery policy — not because
//! the rules are any different a kind of thing.

use std::path::{Path, PathBuf};

use arity::config::LintConfig;
use arity::linter::{
    LintError, LintResult, LintStatus, check_description_document, check_paths,
    check_paths_with_config,
};
use tempfile::TempDir;

/// A DESCRIPTION with every field `R CMD check` requires, so a fixture only
/// varies what its test is actually about.
const COMPLETE_DESCRIPTION: &str = "\
Package: testpkg
Version: 0.1.0
Title: A Test Package
Description: Fixture data for arity's own tests.
License: MIT + file LICENSE
Authors@R: person(\"Test\", \"User\", role = c(\"aut\", \"cre\"))
";

/// Write a package rooted at a fresh temp dir: `DESCRIPTION`, `NAMESPACE`, and
/// one `R/` source per entry. Returns the dir (kept alive by the caller) and
/// its path.
fn package(description: &str, namespace: &str, files: &[(&str, &str)]) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let root = dir.path().to_path_buf();
    std::fs::write(root.join("DESCRIPTION"), description).unwrap();
    std::fs::write(root.join("NAMESPACE"), namespace).unwrap();
    std::fs::create_dir(root.join("R")).unwrap();
    for (name, src) in files {
        std::fs::write(root.join("R").join(name), src).unwrap();
    }
    (dir, root)
}

/// The report for the package's `DESCRIPTION`, which every test here wants.
fn description_report(result: &LintResult) -> &arity::linter::LintFileReport {
    result
        .reports
        .iter()
        .find(|r| r.path.file_name().and_then(|n| n.to_str()) == Some("DESCRIPTION"))
        .expect("a report for DESCRIPTION")
}

fn rules_reported(result: &LintResult) -> Vec<&str> {
    description_report(result)
        .diagnostics
        .iter()
        .map(|d| d.rule)
        .collect()
}

// ---------------------------------------------------------------------------
// Discovery and the driver pass
// ---------------------------------------------------------------------------

#[test]
fn a_packages_description_is_linted_by_a_directory_walk() {
    let (_dir, root) = package(
        COMPLETE_DESCRIPTION,
        "export(f)\n",
        &[("a.R", "f <- function() 1\n")],
    );

    let result = check_paths(std::slice::from_ref(&root)).expect("lint should succeed");
    assert_eq!(
        result.checked_files, 2,
        "DESCRIPTION should be counted alongside R/a.R"
    );
    assert_eq!(description_report(&result).status, LintStatus::Clean);
}

/// An explicitly named `DESCRIPTION` used to be a hard `NonRFilePath` error.
#[test]
fn an_explicitly_named_description_is_linted() {
    let (_dir, root) = package(COMPLETE_DESCRIPTION, "", &[("a.R", "f <- function() 1\n")]);
    let path = root.join("DESCRIPTION");

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");
    assert_eq!(result.checked_files, 1);
    assert_eq!(description_report(&result).status, LintStatus::Clean);
}

/// A file named `DESCRIPTION` that does not sit at a package root is somebody
/// else's data — a fixture, a vendored copy, a scraped corpus — and linting it
/// would report on a file the project does not own.
#[test]
fn a_description_outside_a_package_root_is_not_walked() {
    let (_dir, root) = package(COMPLETE_DESCRIPTION, "", &[("a.R", "f <- function() 1\n")]);
    let fixtures = root.join("inst").join("extdata");
    std::fs::create_dir_all(&fixtures).unwrap();
    std::fs::write(fixtures.join("DESCRIPTION"), "this is not a DESCRIPTION\n").unwrap();

    let result = check_paths(std::slice::from_ref(&root)).expect("lint should succeed");
    let described: Vec<&PathBuf> = result
        .reports
        .iter()
        .map(|r| &r.path)
        .filter(|p| p.file_name().and_then(|n| n.to_str()) == Some("DESCRIPTION"))
        .collect();
    assert_eq!(described, vec![&root.join("DESCRIPTION")]);
}

/// ...but naming it explicitly still lints it, exactly as naming an excluded
/// `.R` file does.
#[test]
fn an_explicitly_named_description_outside_a_package_root_is_linted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("DESCRIPTION");
    std::fs::write(&path, COMPLETE_DESCRIPTION).unwrap();

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");
    assert_eq!(result.checked_files, 1);
}

/// A malformed `DESCRIPTION` blocks its rules but is never swallowed: the DCF
/// parser's diagnostics surface as `syntax-error` findings, exactly as the R
/// parser's do.
#[test]
fn a_malformed_description_reports_syntax_errors() {
    let (_dir, root) = package(
        "Package: testpkg\nthis line is neither a field nor a continuation\n",
        "",
        &[("a.R", "f <- function() 1\n")],
    );

    let result = check_paths(std::slice::from_ref(&root)).expect("lint should succeed");
    let report = description_report(&result);
    assert_eq!(report.status, LintStatus::ParseDiagnostics { count: 1 });
    assert_eq!(rules_reported(&result), vec!["syntax-error"]);
}

/// `syntax-error` is not a rule, so it is not subject to `select` — a broken
/// `DESCRIPTION` surfaces even when only one R rule was asked for.
#[test]
fn description_syntax_errors_survive_select() {
    let (_dir, root) = package(
        "Package: testpkg\nthis line is neither a field nor a continuation\n",
        "",
        &[("a.R", "f <- function() 1\n")],
    );
    let config = LintConfig {
        select: Some(vec!["browser".to_string()]),
        ..LintConfig::default()
    };

    let result =
        check_paths_with_config(std::slice::from_ref(&root), &config).expect("lint should succeed");
    assert_eq!(rules_reported(&result), vec!["syntax-error"]);
}

#[test]
fn a_non_lintable_explicit_path_is_still_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "hello\n").unwrap();

    let err = check_paths(std::slice::from_ref(&path)).expect_err("should reject the path");
    assert_eq!(err, LintError::NonRFilePath { path });
}

// ---------------------------------------------------------------------------
// The single-file entry point
// ---------------------------------------------------------------------------

#[test]
fn check_description_document_reports_parse_errors() {
    let diagnostics = check_description_document(
        Path::new("DESCRIPTION"),
        "  orphan continuation\n",
        &LintConfig::default(),
    )
    .expect("linting should not error");
    assert_eq!(
        diagnostics.iter().map(|d| d.rule).collect::<Vec<_>>(),
        vec!["syntax-error"]
    );
}

#[test]
fn check_description_document_is_clean_on_a_good_file() {
    let diagnostics = check_description_document(
        Path::new("DESCRIPTION"),
        COMPLETE_DESCRIPTION,
        &LintConfig::default(),
    )
    .expect("linting should not error");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}
