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

fn rules_reported(result: &LintResult) -> Vec<&'static str> {
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

/// A *complete* fake package under `tests/` is fixture data for a test —
/// roxygen2 and devtools keep dozens — and its deliberately minimal
/// `DESCRIPTION` describes nothing anybody ships.
#[test]
fn a_fixture_package_inside_a_package_is_not_walked() {
    let (_dir, root) = package(COMPLETE_DESCRIPTION, "", &[("a.R", "f <- function() 1\n")]);
    let fixture = root.join("tests").join("testthat").join("minimal");
    std::fs::create_dir_all(fixture.join("R")).unwrap();
    std::fs::write(fixture.join("DESCRIPTION"), "Package: minimal\n").unwrap();
    std::fs::write(fixture.join("R").join("a.R"), "g <- function() 1\n").unwrap();

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

// ---------------------------------------------------------------------------
// undeclared-dependency
//
// An R-file rule, but a packaging one: what it reads is DESCRIPTION.
// ---------------------------------------------------------------------------

/// Rule ids reported for `R/a.R` after linting a package whose `DESCRIPTION`
/// declares `declares` (appended to the complete fixture) and whose `R/a.R` is
/// `source`.
fn package_rules(declares: &str, source: &str) -> Vec<&'static str> {
    let (_dir, root) = package(
        &format!("{COMPLETE_DESCRIPTION}{declares}"),
        "",
        &[("a.R", source)],
    );
    let result = check_paths(std::slice::from_ref(&root)).expect("lint should succeed");
    result
        .reports
        .iter()
        .find(|r| r.path.file_name().and_then(|n| n.to_str()) == Some("a.R"))
        .map(|r| r.diagnostics.iter().map(|d| d.rule).collect())
        .unwrap_or_default()
}

fn flags_undeclared(declares: &str, source: &str) -> bool {
    package_rules(declares, source).contains(&"undeclared-dependency")
}

#[test]
fn a_qualified_call_to_an_undeclared_package_is_flagged() {
    assert!(flags_undeclared("", "dplyr::filter(x)\n"));
}

#[test]
fn an_internal_access_to_an_undeclared_package_is_flagged() {
    let rules = package_rules("", "dplyr:::internal(x)\n");
    assert!(rules.contains(&"undeclared-dependency"));
    // Two different facts about the same line, both worth saying.
    assert!(rules.contains(&"internal-function"));
}

#[test]
fn attaching_an_undeclared_package_is_flagged() {
    assert!(flags_undeclared("", "library(dplyr)\n"));
    assert!(flags_undeclared("", "require(dplyr)\n"));
}

/// The conditional-dependency idiom lives inside a function body, and the
/// semantic model records attaches only at depth zero — so this rule cannot
/// read them off the model.
#[test]
fn a_conditional_load_inside_a_function_is_flagged() {
    assert!(flags_undeclared(
        "",
        "f <- function() {\n  if (requireNamespace(\"dplyr\", quietly = TRUE)) 1\n}\n"
    ));
}

/// `methods` ships with R and is even attached by default, but `R CMD check`
/// still requires a package that uses it to declare it.
#[test]
fn methods_must_be_declared() {
    assert!(flags_undeclared("", "methods::new(\"A\")\n"));
}

/// Every site, not just the first: each one is separately suppressible, and one
/// DESCRIPTION line clears them all.
#[test]
fn every_site_is_reported() {
    let rules = package_rules("", "dplyr::filter(x)\ndplyr::mutate(y)\n");
    assert_eq!(
        rules
            .iter()
            .filter(|id| **id == "undeclared-dependency")
            .count(),
        2
    );
}

#[test]
fn a_declared_package_is_not_flagged() {
    for field in ["Depends", "Imports", "Suggests", "LinkingTo", "Enhances"] {
        let declares = format!("{field}: dplyr\n");
        assert!(
            !flags_undeclared(&declares, "dplyr::filter(x)\n"),
            "declared in {field} but still flagged",
        );
    }
}

/// R exempts `Suggests` here too. Flagging *unconditional* use of a suggested
/// package is a different, control-flow question.
#[test]
fn a_suggested_package_is_not_flagged_when_attached() {
    assert!(!flags_undeclared("Suggests: dplyr\n", "library(dplyr)\n"));
}

/// R's base-priority set, which is *not* arity's `default_packages()`:
/// `parallel`, `tools`, `grid`, and `compiler` ship with R but are not
/// attached, and using them needs no declaration.
#[test]
fn base_priority_packages_need_no_declaration() {
    for source in [
        "stats::median(x)\n",
        "parallel::mclapply(x, f)\n",
        "tools::file_ext(p)\n",
        "grid::unit(1, \"cm\")\n",
        "compiler::cmpfun(f)\n",
        "splines::ns(x)\n",
        "utils::head(x)\n",
    ] {
        assert!(!flags_undeclared("", source), "{source} was flagged");
    }
}

#[test]
fn a_self_reference_is_not_flagged() {
    assert!(!flags_undeclared("", "testpkg::exported()\n"));
}

/// Only `R/` is package code. R does not scan `tests/` for this check either,
/// and a test file's dependencies belong in `Suggests`.
#[test]
fn a_test_file_is_not_package_code() {
    let (_dir, root) = package(COMPLETE_DESCRIPTION, "", &[("a.R", "f <- function() 1\n")]);
    let tests = root.join("tests").join("testthat");
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(tests.join("test-a.R"), "dplyr::filter(x)\n").unwrap();

    let result = check_paths(std::slice::from_ref(&root)).expect("lint should succeed");
    let reported: Vec<&str> = result
        .reports
        .iter()
        .find(|r| r.path.file_name().and_then(|n| n.to_str()) == Some("test-a.R"))
        .map(|r| r.diagnostics.iter().map(|d| d.rule).collect())
        .unwrap_or_default();
    assert!(!reported.contains(&"undeclared-dependency"), "{reported:?}");
}

/// A loose script is not a package, and there is no DESCRIPTION to consult.
#[test]
fn a_loose_script_is_not_flagged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("script.R");
    std::fs::write(&path, "dplyr::filter(x)\n").unwrap();

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");
    assert_eq!(result.total_findings, 0, "{:?}", result.reports);
}

/// `library(pkg)` where `pkg` is a variable names no package statically, so
/// matching it could only invent a finding. R's own `common_names`.
#[test]
fn a_package_name_placeholder_is_not_flagged() {
    assert!(!flags_undeclared(
        "",
        "attach_it <- function(pkg) library(pkg)\n"
    ));
}

/// `character.only = TRUE` says the argument is a variable, by contract.
#[test]
fn character_only_is_not_flagged() {
    assert!(!flags_undeclared(
        "",
        "f <- function(x) library(x, character.only = TRUE)\n"
    ));
}

/// The span is the package token alone, not the whole access.
#[test]
fn undeclared_dependency_spans_the_package_name() {
    let (_dir, root) = package(COMPLETE_DESCRIPTION, "", &[("a.R", "dplyr::filter(x)\n")]);
    let result = check_paths(std::slice::from_ref(&root)).expect("lint should succeed");
    let finding = result
        .reports
        .iter()
        .find(|r| r.path.file_name().and_then(|n| n.to_str()) == Some("a.R"))
        .and_then(|r| {
            r.diagnostics
                .iter()
                .find(|d| d.rule == "undeclared-dependency")
        })
        .expect("an undeclared-dependency finding");
    let start: usize = finding.range.start().into();
    let end: usize = finding.range.end().into();
    assert_eq!((start, end), (0, "dplyr".len()));
}

/// `base::library(dplyr)` is one finding on `dplyr`, not also one on `base`.
#[test]
fn a_qualified_load_call_reports_once() {
    let rules = package_rules("", "base::library(dplyr)\n");
    assert_eq!(
        rules
            .iter()
            .filter(|id| **id == "undeclared-dependency")
            .count(),
        1
    );
}

/// Nothing to check against: a DESCRIPTION with no `Package` field would make
/// every dependency look undeclared.
#[test]
fn a_description_without_a_package_field_flags_nothing() {
    let (_dir, root) = package(
        "Title: Not A Package\n",
        "",
        &[("a.R", "dplyr::filter(x)\n")],
    );
    let result = check_paths(std::slice::from_ref(&root)).expect("lint should succeed");
    let reported: Vec<&str> = result
        .reports
        .iter()
        .find(|r| r.path.file_name().and_then(|n| n.to_str()) == Some("a.R"))
        .map(|r| r.diagnostics.iter().map(|d| d.rule).collect())
        .unwrap_or_default();
    assert!(!reported.contains(&"undeclared-dependency"), "{reported:?}");
}

// ---------------------------------------------------------------------------
// unused-dependency
//
// Default-off, so every test selects it. It reports on *absence*, which is why
// most of these tests are about the reasons it must stay quiet.
// ---------------------------------------------------------------------------

fn unused_dependency_only() -> LintConfig {
    LintConfig {
        select: Some(vec!["unused-dependency".to_string()]),
        ..LintConfig::default()
    }
}

/// Lint a whole package and report the rule ids on its `DESCRIPTION`.
fn unused_in(description: &str, namespace: &str, files: &[(&str, &str)]) -> Vec<&'static str> {
    let (_dir, root) = package(description, namespace, files);
    let result = check_paths_with_config(std::slice::from_ref(&root), &unused_dependency_only())
        .expect("lint should succeed");
    rules_reported(&result)
}

/// `COMPLETE_DESCRIPTION` plus an `Imports:` line.
fn imports(line: &str) -> String {
    format!("{COMPLETE_DESCRIPTION}Imports: {line}\n")
}

const NOTHING: &[(&str, &str)] = &[("a.R", "f <- function() 1\n")];

#[test]
fn an_import_nothing_reaches_is_flagged() {
    assert!(unused_in(&imports("dplyr"), "export(f)\n", NOTHING).contains(&"unused-dependency"));
}

#[test]
fn every_unused_entry_is_flagged() {
    let reported = unused_in(&imports("dplyr,\n    rlang"), "export(f)\n", NOTHING);
    assert_eq!(
        reported
            .iter()
            .filter(|id| **id == "unused-dependency")
            .count(),
        2
    );
}

/// The span is the name, not the version constraint: the constraint is not
/// what is unused.
#[test]
fn unused_dependency_spans_the_name_only() {
    let description = imports("dplyr (>= 1.0.0)");
    let (_dir, root) = package(&description, "export(f)\n", NOTHING);
    let result = check_paths_with_config(std::slice::from_ref(&root), &unused_dependency_only())
        .expect("lint should succeed");
    let finding = description_report(&result)
        .diagnostics
        .iter()
        .find(|d| d.rule == "unused-dependency")
        .expect("an unused-dependency finding");
    let start: usize = finding.range.start().into();
    let end: usize = finding.range.end().into();
    assert_eq!(&description[start..end], "dplyr");
}

// --- one negative per usage signal ---

#[test]
fn a_qualified_call_counts_as_use() {
    assert!(
        !unused_in(
            &imports("dplyr"),
            "export(f)\n",
            &[("a.R", "f <- function() dplyr::filter(x)\n")]
        )
        .contains(&"unused-dependency")
    );
}

/// The conditional-dependency idiom, inside a function body. Reading usage off
/// the semantic model's attach set would miss it and report the most careful
/// way to depend on a package as unused.
#[test]
fn a_conditional_load_inside_a_function_counts_as_use() {
    for body in [
        "if (requireNamespace(\"dplyr\", quietly = TRUE)) 1\n",
        "loadNamespace(\"dplyr\")\n",
    ] {
        let source = format!("f <- function() {{\n  {body}}}\n");
        assert!(
            !unused_in(&imports("dplyr"), "export(f)\n", &[("a.R", &source)])
                .contains(&"unused-dependency"),
            "{body} was not counted as use",
        );
    }
}

#[test]
fn a_namespace_import_counts_as_use() {
    for directive in [
        "import(dplyr)",
        "importFrom(dplyr, filter)",
        "importClassesFrom(dplyr, X)",
        "importMethodsFrom(dplyr, show)",
    ] {
        let namespace = format!("export(f)\n{directive}\n");
        assert!(
            !unused_in(&imports("dplyr"), &namespace, NOTHING).contains(&"unused-dependency"),
            "{directive} was not counted as use",
        );
    }
}

/// A roxygen tag counts even when NAMESPACE has not been regenerated yet —
/// mid-`document()` is exactly when a maintainer runs the linter.
#[test]
fn a_roxygen_import_tag_counts_as_use() {
    assert!(
        !unused_in(
            &imports("dplyr"),
            "export(f)\n",
            &[("a.R", "#' @importFrom dplyr filter\nf <- function() 1\n")]
        )
        .contains(&"unused-dependency")
    );
}

/// `Imports: Rcpp` + `LinkingTo: Rcpp` with no R-side reference is the
/// canonical Rcpp skeleton: the entry exists so the shared library loads.
#[test]
fn a_linking_to_package_is_exempt() {
    let description = format!("{COMPLETE_DESCRIPTION}Imports: Rcpp\nLinkingTo: Rcpp\n");
    assert!(!unused_in(&description, "export(f)\n", NOTHING).contains(&"unused-dependency"));
}

/// An S4 class needs `methods` with nothing naming it. R's own `uses_methods`.
#[test]
fn methods_is_exempt_for_an_s4_package() {
    assert!(
        !unused_in(
            &imports("methods"),
            "export(f)\n",
            &[("a.R", "setClass(\"A\", representation(x = \"numeric\"))\n")]
        )
        .contains(&"unused-dependency")
    );
}

/// A dynamic use names the package as a plain string, which is enough to stay
/// quiet — this rule would rather miss a real finding than invent one.
#[test]
fn a_string_mention_silences_the_finding() {
    assert!(
        !unused_in(
            &imports("dplyr"),
            "export(f)\n",
            &[(
                "a.R",
                "f <- function() do.call(\"::\", list(\"dplyr\", \"filter\"))\n"
            )]
        )
        .contains(&"unused-dependency")
    );
}

// --- fields other than Imports ---

#[test]
fn only_imports_is_checked() {
    for field in ["Depends", "Suggests", "LinkingTo", "Enhances"] {
        let description = format!("{COMPLETE_DESCRIPTION}{field}: dplyr\n");
        assert!(
            !unused_in(&description, "export(f)\n", NOTHING).contains(&"unused-dependency"),
            "{field} should not be checked",
        );
    }
}

/// Usage is folded over every analyzed member under the package root, and a
/// `tests/` file is one — so whether a test-only dependency is flagged depends
/// on which files the run covers. Pinned in both directions because the rule's
/// own documentation states it.
#[test]
fn a_test_only_dependency_counts_as_use_when_tests_are_in_the_run() {
    let (_dir, root) = package(&imports("dplyr"), "export(f)\n", NOTHING);
    let testthat = root.join("tests").join("testthat");
    std::fs::create_dir_all(&testthat).unwrap();
    std::fs::write(
        testthat.join("test-a.R"),
        "test_that(\"f\", { dplyr::filter(x) })\n",
    )
    .unwrap();

    let whole = check_paths_with_config(std::slice::from_ref(&root), &unused_dependency_only())
        .expect("lint should succeed");
    assert!(
        !rules_reported(&whole).contains(&"unused-dependency"),
        "a walk of the package covers `tests/`, so the use there counts",
    );

    // The same package, linted as R sources plus its `DESCRIPTION`: nothing in
    // the run reaches `dplyr`, and the run is still complete, so it is flagged.
    let r_only = check_paths_with_config(
        &[root.join("R"), root.join("DESCRIPTION")],
        &unused_dependency_only(),
    )
    .expect("lint should succeed");
    assert!(
        rules_reported(&r_only).contains(&"unused-dependency"),
        "{:?}",
        r_only.reports,
    );
}

#[test]
fn the_r_entry_is_never_a_dependency() {
    let description = format!("{COMPLETE_DESCRIPTION}Imports: R (>= 4.1)\n");
    assert!(!unused_in(&description, "export(f)\n", NOTHING).contains(&"unused-dependency"));
}

#[test]
fn a_self_import_is_not_flagged() {
    assert!(!unused_in(&imports("testpkg"), "export(f)\n", NOTHING).contains(&"unused-dependency"));
}

// --- the completeness guard ---

#[test]
fn the_rule_is_off_by_default() {
    let (_dir, root) = package(&imports("dplyr"), "export(f)\n", NOTHING);
    let result = check_paths(std::slice::from_ref(&root)).expect("lint should succeed");
    assert!(!rules_reported(&result).contains(&"unused-dependency"));
}

/// Linting one file of a package must not declare every *other* file's imports
/// unused. It stays silent for the right reason: the driver pulls the whole
/// expected `R/` set in as scope-only members, so the run is still complete and
/// `dplyr` is correctly seen as used.
#[test]
fn linting_one_file_still_sees_the_whole_package() {
    let (_dir, root) = package(
        &imports("dplyr"),
        "export(f)\n",
        &[
            ("a.R", "f <- function() 1\n"),
            ("b.R", "g <- function() dplyr::filter(x)\n"),
        ],
    );
    let result = check_paths_with_config(
        std::slice::from_ref(&root.join("R").join("a.R")),
        &unused_dependency_only(),
    )
    .expect("lint should succeed");
    assert_eq!(result.total_findings, 0, "{:?}", result.reports);
}

/// A parse error anywhere in `R/` means the run has not seen everything — the
/// only `dplyr::` in the package could be in the file that failed.
#[test]
fn a_parse_error_in_the_package_silences_the_rule() {
    let reported = unused_in(
        &imports("dplyr"),
        "export(f)\n",
        &[("a.R", "f <- function() 1\n"), ("b.R", "f <- function(\n")],
    );
    assert!(!reported.contains(&"unused-dependency"), "{reported:?}");
}

/// No NAMESPACE means the `import()` half of the usage set is simply absent, so
/// a package mid-`document()` is not lectured.
#[test]
fn a_package_without_a_namespace_is_silent() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    std::fs::write(root.join("DESCRIPTION"), imports("dplyr")).unwrap();
    std::fs::create_dir(root.join("R")).unwrap();
    std::fs::write(root.join("R").join("a.R"), "f <- function() 1\n").unwrap();

    let result = check_paths_with_config(std::slice::from_ref(&root), &unused_dependency_only())
        .expect("lint should succeed");
    assert!(!rules_reported(&result).contains(&"unused-dependency"));
}

/// A `DESCRIPTION` linted on its own has no package around it to check against.
#[test]
fn a_lone_description_is_silent() {
    let diagnostics = check_description_document(
        Path::new("DESCRIPTION"),
        &imports("dplyr"),
        &unused_dependency_only(),
    )
    .expect("linting should not error");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}
