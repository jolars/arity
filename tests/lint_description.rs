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
// description-duplicate-field
// ---------------------------------------------------------------------------

/// Lint one `DESCRIPTION` buffer and report the rule ids it produced.
fn ids(description: &str) -> Vec<&'static str> {
    check_description_document(
        Path::new("DESCRIPTION"),
        description,
        &LintConfig::default(),
    )
    .expect("linting should not error")
    .iter()
    .map(|d| d.rule)
    .collect()
}

/// Lint one `DESCRIPTION` buffer and report the message bodies for `rule`.
fn messages(description: &str, rule: &str) -> Vec<String> {
    check_description_document(
        Path::new("DESCRIPTION"),
        description,
        &LintConfig::default(),
    )
    .expect("linting should not error")
    .iter()
    .filter(|d| d.rule == rule)
    .map(|d| d.message.body.clone())
    .collect()
}

#[test]
fn duplicate_field_is_flagged_once_per_repeat() {
    let text = format!("{COMPLETE_DESCRIPTION}Version: 0.2.0\n");
    assert_eq!(
        ids(&text)
            .into_iter()
            .filter(|id| *id == "description-duplicate-field")
            .count(),
        1
    );
}

/// The span is the *later* name: the first occurrence is the one arity reads,
/// so the repeat is what the author has to resolve.
#[test]
fn duplicate_field_spans_the_later_occurrence() {
    let text = format!("{COMPLETE_DESCRIPTION}Version: 0.2.0\n");
    let diagnostics =
        check_description_document(Path::new("DESCRIPTION"), &text, &LintConfig::default())
            .expect("linting should not error");
    let finding = diagnostics
        .iter()
        .find(|d| d.rule == "description-duplicate-field")
        .expect("a duplicate-field finding");
    let at: usize = finding.range.start().into();
    assert_eq!(&text[at..at + "Version".len()], "Version");
    assert!(
        at > text.find("Version: 0.1.0").expect("the first Version"),
        "the finding should be on the second `Version`, not the first",
    );
}

/// The message states which occurrence arity reads *and* which one R reads —
/// the two disagree, and a duplicate field is where that becomes visible.
#[test]
fn duplicate_field_message_names_both_readings() {
    let text = format!("{COMPLETE_DESCRIPTION}Version: 0.2.0\n");
    let body = messages(&text, "description-duplicate-field")
        .pop()
        .expect("a message");
    assert!(body.contains("Version"), "{body}");
    assert!(body.contains("first"), "{body}");
    assert!(body.contains("read.dcf"), "{body}");
}

/// Three occurrences are two repeats, not one.
#[test]
fn duplicate_field_flags_every_repeat() {
    let text = format!("{COMPLETE_DESCRIPTION}Version: 0.2.0\nVersion: 0.3.0\n");
    assert_eq!(
        ids(&text)
            .into_iter()
            .filter(|id| *id == "description-duplicate-field")
            .count(),
        2
    );
}

/// Record-blind, like every other `DESCRIPTION` reader: a stray blank line
/// splits the file into two DCF records but does not make a repeat a new field.
#[test]
fn duplicate_field_sees_across_records() {
    let text = format!("{COMPLETE_DESCRIPTION}\nVersion: 0.2.0\n");
    assert!(ids(&text).contains(&"description-duplicate-field"));
}

#[test]
fn distinct_fields_are_not_duplicates() {
    assert!(!ids(COMPLETE_DESCRIPTION).contains(&"description-duplicate-field"));
}

/// Field names are case-sensitive to `read.dcf`, so these are two fields.
#[test]
fn a_differently_cased_field_is_not_a_duplicate() {
    let text = format!("{COMPLETE_DESCRIPTION}version: 0.2.0\n");
    assert!(!ids(&text).contains(&"description-duplicate-field"));
}

// ---------------------------------------------------------------------------
// description-missing-field
// ---------------------------------------------------------------------------

#[test]
fn a_complete_description_reports_no_missing_field() {
    assert!(!ids(COMPLETE_DESCRIPTION).contains(&"description-missing-field"));
}

#[test]
fn missing_fields_are_reported_together_in_canonical_order() {
    let body = messages(
        "Package: testpkg\nVersion: 0.1.0\n",
        "description-missing-field",
    )
    .pop()
    .expect("a message");
    assert!(
        body.contains("`Title`, `Description`, `Author`, `Maintainer`, `License`"),
        "{body}"
    );
}

/// One finding, not one per field: the defect is that this DESCRIPTION is
/// incomplete, and stacking five diagnostics on one line says it five times.
#[test]
fn missing_fields_are_one_finding() {
    assert_eq!(
        ids("Package: testpkg\n")
            .into_iter()
            .filter(|id| *id == "description-missing-field")
            .count(),
        1
    );
}

/// `Authors@R` is how modern packages declare both, and `R CMD build` derives
/// `Author` and `Maintainer` from it.
#[test]
fn authors_at_r_satisfies_author_and_maintainer() {
    let text = "\
Package: testpkg
Version: 0.1.0
Title: A Test Package
Description: Fixture data.
License: MIT + file LICENSE
Authors@R: person(\"Test\", \"User\", role = c(\"aut\", \"cre\"))
";
    assert!(!ids(text).contains(&"description-missing-field"));
}

/// The legacy pair satisfies it too — plenty of packages still write both.
#[test]
fn author_and_maintainer_satisfy_the_pair() {
    let text = "\
Package: testpkg
Version: 0.1.0
Title: A Test Package
Description: Fixture data.
License: MIT + file LICENSE
Author: Test User
Maintainer: Test User <test@example.com>
";
    assert!(!ids(text).contains(&"description-missing-field"));
}

/// A field present but empty declares nothing — `R CMD check` says the same.
#[test]
fn an_empty_required_field_is_missing() {
    let text = format!(
        "{}Title:\n",
        COMPLETE_DESCRIPTION.replace("Title: A Test Package\n", "")
    );
    assert!(ids(&text).contains(&"description-missing-field"));
}

/// A file with no fields at all is not an incomplete package description; it
/// is not a package description.
#[test]
fn an_empty_file_reports_nothing() {
    assert!(ids("").is_empty());
    assert!(ids("\n\n").is_empty());
}

// ---------------------------------------------------------------------------
// description-version-constraint
// ---------------------------------------------------------------------------

#[test]
fn a_constraint_with_no_operator_is_flagged() {
    let text = format!("{COMPLETE_DESCRIPTION}Imports: dplyr (1.0.0)\n");
    assert!(ids(&text).contains(&"description-version-constraint"));
}

#[test]
fn an_empty_constraint_is_flagged() {
    let text = format!("{COMPLETE_DESCRIPTION}Imports: dplyr (>=)\n");
    assert!(ids(&text).contains(&"description-version-constraint"));
}

#[test]
fn a_well_formed_constraint_is_not_flagged() {
    let text = format!("{COMPLETE_DESCRIPTION}Imports: dplyr (>= 1.0.0), rlang\n");
    assert!(!ids(&text).contains(&"description-version-constraint"));
}

/// `Depends: R (>= 4.1)` is the most common constraint in any DESCRIPTION, and
/// the language entry is checked exactly like a package entry.
#[test]
fn the_r_entry_is_checked_too() {
    let text = format!("{COMPLETE_DESCRIPTION}Depends: R (4.1)\n");
    assert!(ids(&text).contains(&"description-version-constraint"));
}

#[test]
fn every_dependency_field_is_checked() {
    for field in ["Depends", "Imports", "Suggests", "LinkingTo", "Enhances"] {
        let text = format!("{COMPLETE_DESCRIPTION}{field}: dplyr (1.0.0)\n");
        assert!(
            ids(&text).contains(&"description-version-constraint"),
            "{field} was not checked",
        );
    }
}

/// A non-dependency field's parentheses are prose, not a constraint.
#[test]
fn a_non_dependency_field_is_not_parsed_as_dependencies() {
    let text = format!("{COMPLETE_DESCRIPTION}Note: see the manual (section 2) for details\n");
    assert!(!ids(&text).contains(&"description-version-constraint"));
}

/// The span is the whole entry, constraint included: the name alone would point
/// away from the part that is wrong.
#[test]
fn version_constraint_spans_the_whole_entry() {
    let text = format!("{COMPLETE_DESCRIPTION}Imports: dplyr (1.0.0)\n");
    let diagnostics =
        check_description_document(Path::new("DESCRIPTION"), &text, &LintConfig::default())
            .expect("linting should not error");
    let finding = diagnostics
        .iter()
        .find(|d| d.rule == "description-version-constraint")
        .expect("a version-constraint finding");
    let start: usize = finding.range.start().into();
    let end: usize = finding.range.end().into();
    assert_eq!(&text[start..end], "dplyr (1.0.0)");
}

// ---------------------------------------------------------------------------
// Suppression
// ---------------------------------------------------------------------------

/// The escape hatch works in `DESCRIPTION` too: a directive line suppresses the
/// field that follows it.
#[test]
fn a_directive_suppresses_a_description_finding() {
    let text = format!(
        "{COMPLETE_DESCRIPTION}# arity-ignore description-duplicate-field: deliberate\nVersion: 0.2.0\n"
    );
    assert!(!ids(&text).contains(&"description-duplicate-field"));
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
