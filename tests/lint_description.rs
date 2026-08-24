//! Linting `DESCRIPTION`: discovery, the driver's DCF pass, and the rules that
//! run over it.
//!
//! Separate from `tests/lint.rs` because the subject is a second grammar with
//! its own driver (`run_dcf_rules`) and its own discovery policy — not because
//! the rules are any different a kind of thing.

use std::path::{Path, PathBuf};

use arity::config::LintConfig;
use arity::linter::{
    LintError, LintResult, LintStatus, apply_fixes, check_description_document, check_paths,
    check_paths_with_config,
};
use tempfile::TempDir;

/// A DESCRIPTION with every field `R CMD check` requires, so a fixture only
/// varies what its test is actually about.
///
/// The creator carries an `email` because R derives `Maintainer` from
/// `Authors@R` and refuses to without one — a fixture missing it is not
/// complete, whatever arity currently reports
/// (`a_creator_without_an_email_is_not_yet_reported`).
const COMPLETE_DESCRIPTION: &str = "\
Package: testpkg
Version: 0.1.0
Title: A Test Package
Description: Fixture data for arity's own tests.
License: MIT + file LICENSE
Authors@R: person(\"Test\", \"User\", email = \"test@example.com\", role = c(\"aut\", \"cre\"))
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

/// An explicitly named `DESCRIPTION` used to be a hard `UnsupportedFilePath` error.
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
    assert_eq!(err, LintError::UnsupportedFilePath { path });
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

// ---------------------------------------------------------------------------
// description-encoding
// ---------------------------------------------------------------------------

#[test]
fn description_encoding_adds_utf8_for_non_ascii_text() {
    let text = COMPLETE_DESCRIPTION.replace("A Test Package", "A 日本語 Package");
    let diagnostics =
        check_description_document(Path::new("DESCRIPTION"), &text, &LintConfig::default())
            .unwrap();
    let finding = diagnostics
        .iter()
        .find(|d| d.rule == "description-encoding")
        .expect("expected a description-encoding finding");
    let fix = finding
        .fix
        .as_ref()
        .expect("missing Encoding has a safe fix");

    assert_eq!(
        apply_fixes(&text, std::slice::from_ref(fix), false).output,
        format!("{text}Encoding: UTF-8\n")
    );
}

#[test]
fn description_encoding_flags_non_ascii_in_ascii_only_fields_without_a_fix() {
    for (field, value) in [
        ("Package", "téstpkg"),
        ("Version", "0.1.é"),
        ("License", "MÍT"),
        ("Encoding", "UTF-é"),
    ] {
        let mut text = COMPLETE_DESCRIPTION.to_string();
        if field == "Encoding" {
            text.push_str(&format!("Encoding: {value}\n"));
        } else {
            let original = match field {
                "Package" => "testpkg",
                "Version" => "0.1.0",
                "License" => "MIT + file LICENSE",
                _ => unreachable!(),
            };
            text = text.replacen(
                &format!("{field}: {original}"),
                &format!("{field}: {value}"),
                1,
            );
            text.push_str("Encoding: UTF-8\n");
        }
        let diagnostics =
            check_description_document(Path::new("DESCRIPTION"), &text, &LintConfig::default())
                .unwrap();
        let finding = diagnostics
            .iter()
            .find(|d| d.rule == "description-encoding")
            .unwrap_or_else(|| panic!("expected a finding for {field}"));
        assert!(finding.message.body.contains(field));
        assert!(finding.fix.is_none());
    }
}

#[test]
fn description_encoding_ignores_ascii_and_declared_utf8() {
    assert!(!ids(COMPLETE_DESCRIPTION).contains(&"description-encoding"));
    let latin1_representable = COMPLETE_DESCRIPTION.replace("A Test Package", "A naïve Package");
    assert!(!ids(&latin1_representable).contains(&"description-encoding"));
    let declared = format!("{COMPLETE_DESCRIPTION}Encoding: UTF-8\n");
    let non_ascii = declared.replace("A Test Package", "A 日本語 Package");
    assert!(!ids(&non_ascii).contains(&"description-encoding"));
}

#[test]
fn description_encoding_fixed_output_is_parseable_and_clean() {
    for text in [
        COMPLETE_DESCRIPTION.replace("A Test Package", "A 日本語 Package"),
        COMPLETE_DESCRIPTION
            .trim_end_matches('\n')
            .replace("A Test Package", "A 日本語 Package"),
        COMPLETE_DESCRIPTION
            .replace('\n', "\r\n")
            .replace("A Test Package", "A 日本語 Package"),
    ] {
        let diagnostics =
            check_description_document(Path::new("DESCRIPTION"), &text, &LintConfig::default())
                .unwrap();
        let fixes = diagnostics
            .iter()
            .filter(|d| d.rule == "description-encoding")
            .filter_map(|d| d.fix.clone())
            .collect::<Vec<_>>();
        let fixed = apply_fixes(&text, &fixes, false).output;
        let after =
            check_description_document(Path::new("DESCRIPTION"), &fixed, &LintConfig::default())
                .expect("fixed DESCRIPTION should parse");
        assert!(
            !after.iter().any(|d| d.rule == "description-encoding"),
            "fixed output was not clean:\n{fixed}"
        );
    }
}

// ---------------------------------------------------------------------------
// description-unknown-field
// ---------------------------------------------------------------------------

#[test]
fn whitespace_before_a_field_colon_is_flagged() {
    let text = COMPLETE_DESCRIPTION.replacen("Package:", "Package :", 1);
    let diagnostics =
        check_description_document(Path::new("DESCRIPTION"), &text, &LintConfig::default())
            .expect("linting should not error");
    let finding = diagnostics
        .iter()
        .find(|d| d.rule == "description-unknown-field")
        .expect("a whitespace-before-colon finding");
    let start: usize = finding.range.start().into();
    let end: usize = finding.range.end().into();
    assert_eq!(&text[start..end], "Package ");
    assert_eq!(
        finding.message.body,
        "`Package ` is not a standard DESCRIPTION field; did you mean `Package`?"
    );
    assert!(finding.fix.is_none());
}

#[test]
fn one_edit_field_name_typos_are_flagged() {
    for (written, expected) in [
        ("Suggest", "Suggests"),
        ("Depend", "Depends"),
        ("Mantainer", "Maintainer"),
    ] {
        let text = format!("{COMPLETE_DESCRIPTION}{written}: value\n");
        assert_eq!(
            messages(&text, "description-unknown-field"),
            vec![format!(
                "`{written}` is not a standard DESCRIPTION field; did you mean `{expected}`?"
            )]
        );
    }
}

#[test]
fn custom_and_standard_extension_fields_are_not_flagged() {
    let text = format!(
        "{COMPLETE_DESCRIPTION}\
Config/Needs/website: tidyverse/tidytemplate\n\
Remotes: r-lib/pkgload\n\
RoxygenNote: 7.3.2\n\
X-Custom-Metadata: value\n"
    );
    assert!(
        !ids(&text).contains(&"description-unknown-field"),
        "unknown fields that are not near misses must remain legal"
    );
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

/// The span is the *later* name: it is both the repeat and the value that takes
/// effect.
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

/// The message states that the later occurrence silently replaces the first.
#[test]
fn duplicate_field_message_explains_last_wins() {
    let text = format!("{COMPLETE_DESCRIPTION}Version: 0.2.0\n");
    let body = messages(&text, "description-duplicate-field")
        .pop()
        .expect("a message");
    assert!(body.contains("Version"), "{body}");
    assert!(body.contains("line 2"), "{body}");
    assert!(body.contains("later"), "{body}");
    assert!(body.contains("replaces"), "{body}");
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
Authors@R: person(\"Test\", \"User\", email = \"test@example.com\", role = c(\"aut\", \"cre\"))
";
    assert!(!ids(text).contains(&"description-missing-field"));
}

/// **Known gap**, pinned so it reads as a defect rather than as intent.
///
/// The rule accepts any non-empty `Authors@R`, but R only *derives* `Author` and
/// `Maintainer` from a person with role `"cre"`, a **valid email**, and a
/// non-empty name. Without the email R rejects the package outright —
/// "Authors@R field gives no person with maintainer role, valid email address
/// and non-empty name", signal
/// `bad_authors_at_R_field_has_no_valid_maintainer`, confirmed by
/// `tests/description_oracle.rs`.
///
/// `description-authors-at-r` has since landed and *does* report the file, so
/// the defect is no longer silent — see `a_creator_without_an_email_is_flagged`.
/// What is left is this rule's own reading: closing it means consulting the
/// parsed field instead of testing it for non-emptiness (`TODO.md`). When that
/// lands, invert the first assertion.
#[test]
fn a_creator_without_an_email_is_not_yet_reported() {
    let text = "\
Package: testpkg
Version: 0.1.0
Title: A Test Package
Description: Fixture data.
License: MIT + file LICENSE
Authors@R: person(\"Test\", \"User\", role = c(\"aut\", \"cre\"))
";
    assert!(
        !ids(text).contains(&"description-missing-field"),
        "the gap has been closed — invert this assertion",
    );
    assert!(
        ids(text).contains(&"description-authors-at-r"),
        "the defect is reported, just not by the rule whose subject it also is",
    );
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
// description-package-in-multiple-fields
// ---------------------------------------------------------------------------

const MULTI: &str = "description-package-in-multiple-fields";

/// The findings of `MULTI`, as the source text each one spans.
fn multi_hits(text: &str) -> Vec<String> {
    check_description_document(Path::new("DESCRIPTION"), text, &LintConfig::default())
        .expect("linting should not error")
        .iter()
        .filter(|d| d.rule == MULTI)
        .map(|d| {
            let start: usize = d.range.start().into();
            let end: usize = d.range.end().into();
            text[start..end].to_string()
        })
        .collect()
}

#[test]
fn a_package_in_two_dependency_fields_is_flagged() {
    let text = format!("{COMPLETE_DESCRIPTION}Imports: dplyr, rlang\nSuggests: dplyr, testthat\n");
    assert_eq!(multi_hits(&text).len(), 1);
}

#[test]
fn a_package_in_one_dependency_field_is_not_flagged() {
    let text = format!("{COMPLETE_DESCRIPTION}Imports: dplyr, rlang\nSuggests: testthat\n");
    assert!(multi_hits(&text).is_empty());
}

/// The span is the *later* listing's package name, and nothing else: the
/// earlier field is the one the message points back at, and R reports the bare
/// name, which is what the differential oracle compares against.
#[test]
fn the_span_is_the_later_package_name() {
    let text = format!("{COMPLETE_DESCRIPTION}Imports: dplyr (>= 1.0.0)\nSuggests: dplyr\n");
    let diagnostics =
        check_description_document(Path::new("DESCRIPTION"), &text, &LintConfig::default())
            .expect("linting should not error");
    let finding = diagnostics
        .iter()
        .find(|d| d.rule == MULTI)
        .expect("a multiple-fields finding");
    let start: usize = finding.range.start().into();
    let end: usize = finding.range.end().into();
    assert_eq!(&text[start..end], "dplyr");
    assert!(
        start > text.find("Suggests:").expect("the Suggests field"),
        "the finding should be on the `Suggests` entry, not the `Imports` one",
    );
}

/// A version constraint is not part of the identity: `dplyr (>= 1.0.0)` and
/// `dplyr` are the same package listed twice.
#[test]
fn a_version_constraint_does_not_hide_the_second_listing() {
    let text = format!("{COMPLETE_DESCRIPTION}Imports: dplyr (>= 1.0.0)\nSuggests: dplyr\n");
    assert_eq!(multi_hits(&text), ["dplyr"]);
}

/// The message names the field the package was already listed in — that field
/// is unique in the file, so it locates the other listing on its own.
#[test]
fn the_message_names_the_earlier_field() {
    let text = format!("{COMPLETE_DESCRIPTION}Imports: dplyr\nSuggests: dplyr\n");
    let body = messages(&text, MULTI).pop().expect("a message");
    assert!(body.contains("dplyr"), "{body}");
    assert!(body.contains("Imports"), "{body}");
}

/// `LinkingTo` is not one of the four. A C++ package belongs in both
/// `LinkingTo` (headers) and `Imports` (its R code), and R's own check excludes
/// `LinkingTo` for exactly that reason.
#[test]
fn linking_to_alongside_imports_is_not_flagged() {
    let text = format!("{COMPLETE_DESCRIPTION}Imports: Rcpp\nLinkingTo: Rcpp\n");
    assert!(multi_hits(&text).is_empty());
}

/// `R` names the language, not a package, and is never a dependency.
#[test]
fn the_r_entry_is_never_a_duplicate() {
    let text = format!("{COMPLETE_DESCRIPTION}Depends: R (>= 4.1)\nImports: R (>= 4.1)\n");
    assert!(multi_hits(&text).is_empty());
}

/// A package repeated inside one field is a different defect, and R's
/// `duplicates` check uniques each field before comparing them.
#[test]
fn a_package_repeated_within_one_field_is_not_this_rule() {
    let text = format!("{COMPLETE_DESCRIPTION}Imports: dplyr, dplyr\n");
    assert!(multi_hits(&text).is_empty());
}

/// Three listings are two redundant ones.
#[test]
fn a_package_in_three_fields_is_flagged_twice() {
    let text = format!("{COMPLETE_DESCRIPTION}Depends: dplyr\nImports: dplyr\nSuggests: dplyr\n");
    assert_eq!(multi_hits(&text), ["dplyr", "dplyr"]);
}

/// Every pair drawn from the four fields, not just the common `Imports`
/// /`Suggests` one.
#[test]
fn every_pair_of_dependency_fields_is_checked() {
    let fields = ["Depends", "Imports", "Suggests", "Enhances"];
    for (i, first) in fields.iter().enumerate() {
        for second in &fields[i + 1..] {
            let text = format!("{COMPLETE_DESCRIPTION}{first}: dplyr\n{second}: dplyr\n");
            assert_eq!(
                multi_hits(&text).len(),
                1,
                "`{first}` + `{second}` was not flagged",
            );
        }
    }
}

/// Case-sensitive, like every R package name.
#[test]
fn a_differently_cased_package_is_a_different_package() {
    let text = format!("{COMPLETE_DESCRIPTION}Imports: dplyr\nSuggests: Dplyr\n");
    assert!(multi_hits(&text).is_empty());
}

/// No autofix: deleting a listing means deciding which field the package
/// belongs in, and `Imports` versus `Suggests` is a decision about whether the
/// code may rely on it at all.
#[test]
fn multiple_fields_ships_no_fix() {
    let text = format!("{COMPLETE_DESCRIPTION}Imports: dplyr\nSuggests: dplyr\n");
    let diagnostics =
        check_description_document(Path::new("DESCRIPTION"), &text, &LintConfig::default())
            .expect("linting should not error");
    let finding = diagnostics
        .iter()
        .find(|d| d.rule == MULTI)
        .expect("a multiple-fields finding");
    assert!(finding.fix.is_none());
}

// ---------------------------------------------------------------------------
// description-malformed-name
// ---------------------------------------------------------------------------

const MALFORMED_NAME: &str = "description-malformed-name";

/// `COMPLETE_DESCRIPTION` with a different package name, so a case varies only
/// the field the rule reads.
fn named(package: &str) -> String {
    COMPLETE_DESCRIPTION.replacen("Package: testpkg", &format!("Package: {package}"), 1)
}

/// The findings of `MALFORMED_NAME`, as the source text each one spans.
fn name_hits(text: &str) -> Vec<String> {
    check_description_document(Path::new("DESCRIPTION"), text, &LintConfig::default())
        .expect("linting should not error")
        .iter()
        .filter(|d| d.rule == MALFORMED_NAME)
        .map(|d| {
            let start: usize = d.range.start().into();
            let end: usize = d.range.end().into();
            text[start..end].to_string()
        })
        .collect()
}

/// Every shape R's `valid_package_name` rejects, one per clause of the regexp
/// `[[:alpha:]][[:alnum:].]*[[:alnum:]]`.
#[test]
fn a_name_r_would_reject_is_flagged() {
    for name in [
        "3bad",       // does not start with a letter
        ".hidden",    // nor with a period
        "my_pkg",     // underscores are not allowed anywhere
        "my-pkg",     // nor hyphens
        "mypkg.",     // must end in a letter or digit
        "p",          // at least two characters
        "my pkg",     // a space is not a name character
        "my pkg (1)", // nor is anything else
    ] {
        assert_eq!(
            name_hits(&named(name)),
            [name],
            "`{name}` should be flagged, spanning the name",
        );
    }
}

#[test]
fn a_name_r_accepts_is_not_flagged() {
    for name in ["testpkg", "data.table", "R6", "Rcpp", "ggplot2", "A1"] {
        assert!(
            name_hits(&named(name)).is_empty(),
            "`{name}` should not be flagged",
        );
    }
}

/// R's regexp is `^(R|[[:alpha:]][[:alnum:].]*[[:alnum:]])$`: the language's own
/// name is spelled out as an alternative, so it survives the two-character
/// floor that rejects every other single letter.
#[test]
fn the_literal_r_is_accepted() {
    assert!(name_hits(&named("R")).is_empty());
}

/// `[[:alpha:]]` is locale-dependent, and R runs this check in the session's
/// locale: under a UTF-8 one it matches any Unicode letter, so `café` is a name
/// R accepts. Flagging it would be a false positive against `R CMD check`.
#[test]
fn a_non_ascii_letter_is_not_flagged() {
    assert!(name_hits(&named("caf\u{e9}")).is_empty());
}

/// Case-sensitive, like every R package name — and `Stats` is nobody's base
/// package.
#[test]
fn a_differently_cased_base_name_is_not_a_base_package() {
    assert!(name_hits(&named("Stats")).is_empty());
}

#[test]
fn a_base_package_name_is_flagged() {
    for name in ["stats", "utils", "methods", "tools", "parallel"] {
        assert_eq!(
            name_hits(&named(name)),
            [name],
            "`{name}` is the name of a base package",
        );
    }
}

/// The two defects are different repairs, so they read differently: one asks
/// for a well-formed name, the other for one nothing already claims.
#[test]
fn the_message_names_which_defect_it_is() {
    let base = messages(&named("stats"), MALFORMED_NAME)
        .pop()
        .expect("a message");
    assert!(base.contains("base"), "{base}");

    let malformed = messages(&named("3bad"), MALFORMED_NAME)
        .pop()
        .expect("a message");
    assert!(!malformed.contains("base"), "{malformed}");
}

/// R skips the base-name clause for a package that declares `Priority: base` —
/// which is how the real `stats` describes itself.
#[test]
fn a_base_package_may_name_itself() {
    let text = format!("{}Priority: base\n", named("stats"));
    assert!(name_hits(&text).is_empty());
}

/// The exemption is R's exactly: the *name* is still checked against the
/// regexp, so `Priority: base` does not license anything.
#[test]
fn priority_base_does_not_excuse_a_malformed_name() {
    let text = format!("{}Priority: base\n", named("3bad"));
    assert_eq!(name_hits(&text), ["3bad"]);
}

/// A wrapped `Package` is one R rejects — `read.dcf` folds the continuation in
/// with a newline, and no newline is a name character.
#[test]
fn a_wrapped_name_is_flagged() {
    let text = named("my\n  pkg");
    assert_eq!(name_hits(&text), ["my\n  pkg"]);
}

/// An absent or empty `Package` is `description-missing-field`'s finding, and
/// "malformed" is the wrong word for a field with no value in it.
#[test]
fn a_package_field_without_a_value_is_silent() {
    assert!(name_hits("Version: 0.1.0\n").is_empty());
    assert!(name_hits("Package:\nVersion: 0.1.0\n").is_empty());
    assert!(name_hits("Package:   \nVersion: 0.1.0\n").is_empty());
}

/// No autofix: the package's name is in its NAMESPACE, its file names, its
/// tests, and every `pkg::` that reaches it, so a name is renamed by its
/// author, not by a textual edit to one field.
#[test]
fn malformed_name_ships_no_fix() {
    let text = named("3bad");
    let finding =
        check_description_document(Path::new("DESCRIPTION"), &text, &LintConfig::default())
            .expect("linting should not error")
            .into_iter()
            .find(|d| d.rule == MALFORMED_NAME)
            .expect("a malformed-name finding");
    assert!(finding.fix.is_none());
}

// ---------------------------------------------------------------------------
// description-malformed-version
// ---------------------------------------------------------------------------

const MALFORMED_VERSION: &str = "description-malformed-version";

/// `COMPLETE_DESCRIPTION` with a different version, so a case varies only the
/// field the rule reads.
fn versioned(version: &str) -> String {
    COMPLETE_DESCRIPTION.replacen("Version: 0.1.0", &format!("Version: {version}"), 1)
}

/// The findings of `MALFORMED_VERSION`, as the source text each one spans.
fn version_hits(text: &str) -> Vec<String> {
    check_description_document(Path::new("DESCRIPTION"), text, &LintConfig::default())
        .expect("linting should not error")
        .iter()
        .filter(|d| d.rule == MALFORMED_VERSION)
        .map(|d| {
            let start: usize = d.range.start().into();
            let end: usize = d.range.end().into();
            text[start..end].to_string()
        })
        .collect()
}

/// Every shape R's `valid_package_version` rejects. The regexp is
/// `([[:digit:]]+[.-]){1,}[[:digit:]]+`: digit runs joined by `.` or `-`, and
/// because the trailing run is written separately, **at least two components**.
#[test]
fn a_version_r_would_reject_is_flagged() {
    for version in [
        "1",          // at least two components
        "1.0.0.beta", // every component is digits
        "1.",         // and the last one has to be there
        "1..0",       // exactly one separator between them
        "1.0_1",      // `.` and `-` are the only separators
        "v1.0",       // no prefix
        "1.0rc1",     // and no suffix
    ] {
        assert_eq!(
            version_hits(&versioned(version)),
            [version],
            "`{version}` should be flagged, spanning the version",
        );
    }
}

#[test]
fn a_version_r_accepts_is_not_flagged() {
    for version in ["0.1.0", "1.0", "1-0", "1.0-3", "0.1.0.9000", "1.27.1"] {
        assert!(
            version_hits(&versioned(version)).is_empty(),
            "`{version}` should not be flagged",
        );
    }
}

/// Unlike `[[:alpha:]]` in `description-malformed-name`, R's `[[:digit:]]`
/// matches **ASCII only** here — verified against `grepl` under a UTF-8 locale,
/// where an Arabic-Indic digit is rejected. The two rules read their POSIX
/// classes differently because R does.
#[test]
fn a_non_ascii_digit_is_flagged() {
    assert_eq!(
        version_hits(&versioned("\u{661}.0")),
        ["\u{661}.0"],
        "R's `[[:digit:]]` does not match an Arabic-Indic digit",
    );
}

/// CRAN's `version_with_leading_zeroes`: `(^|[.-])0[0-9]+`. A lone `0`
/// component is not a leading zero — `0.1.0` is the most ordinary version there
/// is.
#[test]
fn a_component_with_a_leading_zero_is_flagged() {
    for version in ["1.01", "01.2", "1.0.010"] {
        assert_eq!(
            version_hits(&versioned(version)),
            [version],
            "`{version}` has a component with a leading zero",
        );
    }
}

/// CRAN's own carve-out, `^[0-9]{4}[.-][0-9]{2}`: calendar versioning is the
/// one place a leading zero is intended.
#[test]
fn calendar_versioning_is_not_a_leading_zero() {
    for version in ["2024.01", "2024-01-15", "2024.06.3"] {
        assert!(
            version_hits(&versioned(version)).is_empty(),
            "`{version}` is calendar versioning, which CRAN exempts",
        );
    }
}

/// CRAN's `version_with_large_components`, at its threshold of 1234.
#[test]
fn an_absurd_component_is_flagged() {
    for version in ["1234.0", "1.0.5000", "20240115.1"] {
        assert_eq!(
            version_hits(&versioned(version)),
            [version],
            "`{version}` has an implausibly large component",
        );
    }
}

/// CRAN exempts a component equal to the submission year, so that calendar
/// versioning survives. arity has no submission date and refuses to make a
/// diagnostic depend on the wall clock, so it exempts the whole four-digit year
/// band instead. That is strictly *more* permissive than CRAN — every component
/// arity flags, CRAN flags too, in any year through 2999 — which is the
/// direction the oracle's containment gate requires.
#[test]
fn a_year_component_is_not_absurd() {
    for version in ["2026.1", "2026.1.0", "0.1.2026", "1999.4"] {
        assert!(
            version_hits(&versioned(version)).is_empty(),
            "`{version}` reads as a calendar year, not an absurd component",
        );
    }
}

/// `usethis::use_dev_version()` appends `.9000`, so this is what a package
/// under development looks like nearly everywhere. CRAN's check reports it and
/// is right to — nobody submits a development version — but a linter reads
/// packages in exactly that state, so a *trailing* component of 9000 or more is
/// read as the marker it is.
#[test]
fn a_development_version_suffix_is_not_absurd() {
    for version in ["0.0.0.9000", "1.2.3.9000", "0.1.0.9017"] {
        assert!(
            version_hits(&versioned(version)).is_empty(),
            "`{version}` is a development version, not an absurd component",
        );
    }
}

/// The carve-out is for the *trailing* component only: a large number anywhere
/// else is the typo the check exists for.
#[test]
fn a_large_component_before_the_last_is_still_absurd() {
    assert_eq!(version_hits(&versioned("9000.1")), ["9000.1"]);
    assert_eq!(version_hits(&versioned("1.9000.2")), ["1.9000.2"]);
}

/// A component just under the threshold, and one just over the exempt band.
#[test]
fn the_absurd_component_boundaries_are_rs() {
    assert!(version_hits(&versioned("1233.0")).is_empty());
    assert_eq!(version_hits(&versioned("1234.0")), ["1234.0"]);
    assert!(version_hits(&versioned("1900.1")).is_empty());
    assert_eq!(version_hits(&versioned("3000.1")), ["3000.1"]);
}

/// R's `bad_version` clause is guarded by `!is_base_package`, and a package
/// declaring `Priority: base` is R's own — its version is R's to spell,
/// `@VERSION@` placeholder included. CRAN's two clauses never see a base
/// package at all, so the exemption covers the whole rule.
#[test]
fn a_base_package_version_is_exempt() {
    for version in ["@VERSION@", "1.01", "9999.1"] {
        let text = format!("{}Priority: base\n", versioned(version));
        assert!(
            version_hits(&text).is_empty(),
            "`{version}` is exempt in a package declaring `Priority: base`",
        );
    }
}

/// The three defects are three different repairs, so they read differently.
#[test]
fn the_message_names_which_version_defect_it_is() {
    let malformed = messages(&versioned("1.0.0.beta"), MALFORMED_VERSION)
        .pop()
        .expect("a message");
    assert!(
        malformed.contains("not a valid package version"),
        "{malformed}"
    );

    let zeroes = messages(&versioned("1.01"), MALFORMED_VERSION)
        .pop()
        .expect("a message");
    assert!(zeroes.contains("leading zero"), "{zeroes}");

    let large = messages(&versioned("1.5000"), MALFORMED_VERSION)
        .pop()
        .expect("a message");
    assert!(large.contains("5000"), "{large}");
}

/// A wrapped `Version` is one R rejects — `read.dcf` folds the continuation in
/// with a newline, and no newline is part of any version.
#[test]
fn a_wrapped_version_is_flagged() {
    let text = versioned("1.\n  0");
    assert_eq!(version_hits(&text), ["1.\n  0"]);
}

/// An absent or empty `Version` is `description-missing-field`'s finding, and
/// "malformed" is the wrong word for a field with no value in it.
#[test]
fn a_version_field_without_a_value_is_silent() {
    assert!(version_hits("Package: testpkg\n").is_empty());
    assert!(version_hits("Package: testpkg\nVersion:\n").is_empty());
    assert!(version_hits("Package: testpkg\nVersion:   \n").is_empty());
}

/// No autofix: which version number a release carries is a decision about the
/// release, and it is also in the package's tags, its `NEWS.md`, and every
/// dependency's constraint on it.
#[test]
fn malformed_version_ships_no_fix() {
    let text = versioned("1.0.0.beta");
    let finding =
        check_description_document(Path::new("DESCRIPTION"), &text, &LintConfig::default())
            .expect("linting should not error")
            .into_iter()
            .find(|d| d.rule == MALFORMED_VERSION)
            .expect("a malformed-version finding");
    assert!(finding.fix.is_none());
}

// ---------------------------------------------------------------------------
// description-malformed-maintainer
// ---------------------------------------------------------------------------

const MALFORMED_MAINTAINER: &str = "description-malformed-maintainer";

/// `COMPLETE_DESCRIPTION` with a `Maintainer` added, so a case varies only the
/// field the rule reads. The base fixture has none — R derives one from
/// `Authors@R` — which is itself the `an_absent_maintainer_is_silent` case.
fn maintained(maintainer: &str) -> String {
    format!("{COMPLETE_DESCRIPTION}Maintainer: {maintainer}\n")
}

/// The findings of `MALFORMED_MAINTAINER`, as the source text each one spans.
fn maintainer_hits(text: &str) -> Vec<String> {
    check_description_document(Path::new("DESCRIPTION"), text, &LintConfig::default())
        .expect("linting should not error")
        .iter()
        .filter(|d| d.rule == MALFORMED_MAINTAINER)
        .map(|d| {
            let start: usize = d.range.start().into();
            let end: usize = d.range.end().into();
            text[start..end].to_string()
        })
        .collect()
}

/// Every shape R's `.valid_maintainer_field_regexp` rejects. Verified case by
/// case against `grepl` in R 4.6.1.
#[test]
fn a_maintainer_r_would_reject_is_flagged() {
    for maintainer in [
        "Jane Doe",                             // no address at all — the common case
        "Jane Doe <jane at example.com>",       // nor an obfuscated one
        "Jane <>",                              // the angle brackets need contents
        "Jane Doe <@example.com>",              // a local part
        "Jane Doe <jane@>",                     // and a domain
        "Jane Doe <jane..doe@example.com>",     // no empty local-part component
        "Jane Doe <jane@example..com>",         // nor an empty domain label
        "Jane Doe <jane@exa_mple.com>",         // `_` is not a domain character
        "Jane Doe < jane@example.com>",         // no space inside the brackets
        "Jane Doe <jane@example.com >",         // on either side
        "Jane Doe <jane@example.com> and John", // and nothing after them
        "orphaned",                             // the literal is spelled in capitals
    ] {
        assert_eq!(
            maintainer_hits(&maintained(maintainer)),
            [maintainer],
            "`{maintainer}` should be flagged, spanning the field's value",
        );
    }
}

/// R's regexp is deliberately looser than RFC 5322, and a stricter reading
/// would report addresses `R CMD check` accepts. Every one of these is verified
/// against `grepl`.
#[test]
fn a_maintainer_r_accepts_is_not_flagged() {
    for maintainer in [
        "Jane Doe <jane@example.com>",
        "R Core Team <R-core@r-project.org>",
        "Jane Doe <jane@example>",             // no TLD is required
        "Jane Doe <\"jane doe\"@example.com>", // a quoted local part
        "Jane Doe <jane+x@sub.example.co.uk>",
        "Jane Doe <jane@-example.com>", // a domain label may start with `-`
        "Jos\u{e9} Caf\u{e9} <jose@example.com>",
        "Jane Doe<jane@example.com>", // the space before `<` is optional
        "\"Doe, Jane\" <jane@example.com>",
    ] {
        assert!(
            maintainer_hits(&maintained(maintainer)).is_empty(),
            "`{maintainer}` should not be flagged",
        );
    }
}

/// The regexp's second alternative, spelled out: an orphaned package names no
/// maintainer, and the literal is case-sensitive.
#[test]
fn the_literal_orphaned_is_accepted() {
    assert!(maintainer_hits(&maintained("ORPHANED")).is_empty());
    assert_eq!(maintainer_hits(&maintained("Orphaned")), ["Orphaned"]);
}

/// CRAN's `Maintainer_invalid_or_multi_person`: text after the `<...>`. Two
/// maintainers is the shape it exists for, and it is one R's own regexp accepts
/// — the trailing `.*<` before the address happily swallows the first person.
#[test]
fn a_second_maintainer_is_flagged() {
    for maintainer in [
        "Jane Doe <jane@example.com>, John Roe <john@example.org>",
        "Jane Doe <jane@example.com> <john@example.org>",
    ] {
        assert_eq!(
            maintainer_hits(&maintained(maintainer)),
            [maintainer],
            "`{maintainer}` names more than one person",
        );
    }
}

/// CRAN's `empty_Maintainer_name`: an address with nobody in front of it.
#[test]
fn an_address_without_a_name_is_flagged() {
    assert_eq!(
        maintainer_hits(&maintained("<jane@example.com>")),
        ["<jane@example.com>"],
    );
}

/// CRAN's `Maintainer_needs_quotes`: a comma in an unquoted display name reads
/// as a list of people to everything that parses one.
#[test]
fn an_unquoted_comma_in_the_name_is_flagged() {
    assert_eq!(
        maintainer_hits(&maintained("Doe, Jane <jane@example.com>")),
        ["Doe, Jane <jane@example.com>"],
    );
    assert!(
        maintainer_hits(&maintained("\"Doe, Jane\" <jane@example.com>")).is_empty(),
        "quoting the display name is exactly the repair CRAN asks for",
    );
}

/// A comma *after* the address is somebody else's clause, and it is the
/// multi-person one: `display` is cut at the first `<`, so the name half never
/// sees it.
#[test]
fn a_comma_after_the_address_is_not_a_quoting_problem() {
    let text = maintained("Jane Doe <jane@example.com>, John Roe <john@example.org>");
    let message = messages(&text, MALFORMED_MAINTAINER)
        .pop()
        .expect("a message");
    assert!(!message.contains("quote"), "{message}");
}

/// `read.dcf` folds a continuation line in with a `\n`, and R's regexp is
/// written with a `.` that matches one — so a wrapped `Maintainer` is a
/// `Maintainer` R accepts. Confirmed against `.check_package_description`.
#[test]
fn a_wrapped_maintainer_is_accepted() {
    assert!(maintainer_hits(&maintained("Jane Doe\n  <jane@example.com>")).is_empty());
}

/// ...but the fold does not license trailing text: the address still has to be
/// the last thing in the field.
#[test]
fn text_after_a_wrapped_address_is_flagged() {
    let text = maintained("Jane Doe <jane@example.com>\n  and John Roe");
    assert_eq!(
        maintainer_hits(&text),
        ["Jane Doe <jane@example.com>\n  and John Roe"],
    );
}

/// The four defects are four different repairs, so they read differently.
#[test]
fn the_message_names_which_maintainer_defect_it_is() {
    let missing = messages(&maintained("Jane Doe"), MALFORMED_MAINTAINER)
        .pop()
        .expect("a message");
    assert!(missing.contains("email address"), "{missing}");

    let malformed = messages(&maintained("Jane Doe <jane@>"), MALFORMED_MAINTAINER)
        .pop()
        .expect("a message");
    assert!(malformed.contains("not a valid"), "{malformed}");

    let multi = messages(
        &maintained("Jane Doe <jane@example.com>, John Roe <john@example.org>"),
        MALFORMED_MAINTAINER,
    )
    .pop()
    .expect("a message");
    assert!(multi.contains("one person"), "{multi}");

    let nameless = messages(&maintained("<jane@example.com>"), MALFORMED_MAINTAINER)
        .pop()
        .expect("a message");
    assert!(nameless.contains("name"), "{nameless}");

    let quotes = messages(
        &maintained("Doe, Jane <jane@example.com>"),
        MALFORMED_MAINTAINER,
    )
    .pop()
    .expect("a message");
    assert!(quotes.contains("quote"), "{quotes}");
}

/// R checks the `Maintainer` it *has*, deriving one from `Authors@R` when the
/// field is absent — and a derived one is well formed by construction. Whether
/// the package names a maintainer at all is `description-missing-field`'s
/// subject, and "malformed" is the wrong word for a field with no value in it.
#[test]
fn a_maintainer_field_without_a_value_is_silent() {
    assert!(maintainer_hits(COMPLETE_DESCRIPTION).is_empty());
    assert!(maintainer_hits(&maintained("")).is_empty());
    assert!(maintainer_hits(&maintained("   ")).is_empty());
}

/// No autofix. Three of the four defects have nothing to edit *to* — an address
/// cannot be invented, a name cannot be invented, and choosing between two
/// people is not a spelling — and the fourth, quoting a comma'd name, is a
/// judgment about whether that comma separates a surname from a given name or
/// separates two maintainers.
#[test]
fn malformed_maintainer_ships_no_fix() {
    let text = maintained("Doe, Jane <jane@example.com>");
    let finding =
        check_description_document(Path::new("DESCRIPTION"), &text, &LintConfig::default())
            .expect("linting should not error")
            .into_iter()
            .find(|d| d.rule == MALFORMED_MAINTAINER)
            .expect("a malformed-maintainer finding");
    assert!(finding.fix.is_none());
}

// ---------------------------------------------------------------------------
// description-authors-at-r
// ---------------------------------------------------------------------------

const AUTHORS_AT_R: &str = "description-authors-at-r";

/// `COMPLETE_DESCRIPTION` with its `Authors@R` replaced, so a case varies only
/// the field the rule reads.
fn authored(value: &str) -> String {
    let base = COMPLETE_DESCRIPTION
        .lines()
        .filter(|line| !line.starts_with("Authors@R:"))
        .map(|line| format!("{line}\n"))
        .collect::<String>();
    format!("{base}Authors@R: {value}\n")
}

/// The findings of `AUTHORS_AT_R`, as the source text each one spans.
fn authors_hits(text: &str) -> Vec<String> {
    check_description_document(Path::new("DESCRIPTION"), text, &LintConfig::default())
        .expect("linting should not error")
        .iter()
        .filter(|d| d.rule == AUTHORS_AT_R)
        .map(|d| {
            let start: usize = d.range.start().into();
            let end: usize = d.range.end().into();
            text[start..end].to_string()
        })
        .collect()
}

/// The headline check, and the one the roadmap wants first: R derives
/// `Maintainer` only from a person with role `cre`, a non-empty name, **and**
/// an email, and errors out otherwise
/// (`bad_authors_at_R_field_has_no_valid_maintainer`, confirmed against R
/// 4.6.1).
#[test]
fn a_creator_without_an_email_is_flagged() {
    let text = authored("person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"))");
    assert_eq!(
        authors_hits(&text),
        ["person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"))"],
    );
    assert!(messages(&text, AUTHORS_AT_R)[0].contains("email"));
}

/// The same defect from the other two directions: no `cre` at all, and a `cre`
/// with an address but no name.
#[test]
fn a_field_that_derives_no_maintainer_is_flagged() {
    for value in [
        "person(\"Jane\", \"Doe\", role = \"aut\", email = \"jane@example.com\")",
        "person(role = \"cre\", email = \"jane@example.com\")",
        "c(person(\"Jane\", \"Doe\", role = \"aut\"), person(\"John\", \"Roe\", role = \"ctb\"))",
    ] {
        assert!(
            !authors_hits(&authored(value)).is_empty(),
            "`{value}` names nobody R can derive a maintainer from",
        );
    }
}

/// The shape every modern `DESCRIPTION` writes, in both spellings R accepts.
#[test]
fn a_well_formed_authors_at_r_is_silent() {
    for value in [
        "person(\"Jane\", \"Doe\", email = \"jane@example.com\", role = c(\"aut\", \"cre\"))",
        "person(given = \"Jane\", family = \"Doe\", role = c(\"aut\", \"cre\"), \
         email = \"jane@example.com\")",
        "c(person(\"Jane\", \"Doe\", , \"jane@example.com\", c(\"aut\", \"cre\")), \
         person(\"John\", \"Roe\", role = \"ctb\"))",
        "person(\"Posit Software, PBC\", role = c(\"cph\", \"fnd\", \"cre\"), \
         email = \"info@posit.co\", comment = c(ROR = \"03wc8by49\"))",
    ] {
        assert!(
            authors_hits(&authored(value)).is_empty(),
            "`{value}` is a field R reads without complaint",
        );
    }
}

/// R reads the field with `str2expression`, so text that is not R is the first
/// thing it rejects — `bad_authors_at_R_field`, and nothing else is worth
/// saying about the value until it parses.
#[test]
fn an_unparseable_authors_at_r_is_flagged() {
    let text = authored("person(\"Jane\",");
    assert_eq!(authors_hits(&text), ["person(\"Jane\","]);
    assert!(messages(&text, AUTHORS_AT_R)[0].contains("parse"));
}

/// `.read_authors_at_R_field(strict = TRUE)` refuses any call outside
/// `person`, `as.person`, `c`, `list`, `paste`, `paste0`, and `(` — it is about
/// to *evaluate* the field. `utils::person(...)` is refused for the same
/// reason: `::` is itself a call.
#[test]
fn an_unsafe_call_in_authors_at_r_is_flagged() {
    let text = authored("person(\"Jane\", \"Doe\", email = Sys.getenv(\"EMAIL\"), role = \"cre\")");
    assert_eq!(authors_hits(&text), ["Sys.getenv"]);
    assert!(messages(&text, AUTHORS_AT_R)[0].contains("evaluate"));

    assert_eq!(
        authors_hits(&authored("utils::person(\"Jane\", \"Doe\")")),
        ["utils::person"],
    );
}

/// The calls R spells out as safe, so the rule is not just "any call".
#[test]
fn the_calls_r_allows_are_not_flagged() {
    let value = "c(person(paste(\"Jane\", \"Q\"), \"Doe\", role = c(\"aut\", \"cre\"), \
                 email = paste0(\"jane\", \"@example.com\")))";
    assert!(authors_hits(&authored(value)).is_empty(), "{value}");
}

/// `strict >= 1`: a person R can put neither in `Author` nor in `Maintainer`.
#[test]
fn a_person_without_a_name_is_flagged() {
    let text = authored(
        "c(person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), email = \"jane@example.com\"), \
         person(role = \"ctb\"))",
    );
    assert_eq!(authors_hits(&text), ["person(role = \"ctb\")"]);
    assert!(messages(&text, AUTHORS_AT_R)[0].contains("no name"));
}

/// `strict >= 1`: a person with no role is credited nowhere at all —
/// `.format_person_for_plain_author_spec` drops them from `Author` outright.
#[test]
fn a_person_without_a_role_is_flagged() {
    let text = authored(
        "c(person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), email = \"jane@example.com\"), \
         person(\"John\", \"Roe\"))",
    );
    assert_eq!(authors_hits(&text), ["person(\"John\", \"Roe\")"]);
    assert!(messages(&text, AUTHORS_AT_R)[0].contains("no role"));
}

/// A role outside the MARC relator table is *dropped* by `person()`, so the
/// credit the author wrote is silently lost.
#[test]
fn a_role_r_does_not_know_is_flagged() {
    let text = authored(
        "person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\", \"zzz\"), \
         email = \"jane@example.com\")",
    );
    assert_eq!(authors_hits(&text), ["\"zzz\""]);
    assert!(messages(&text, AUTHORS_AT_R)[0].contains("role"));
}

/// The table is the whole 302-code MARC relator db, not the eleven codes CRAN
/// suggests — `person()` accepts every one of them. A role spelled as a
/// relator *term* is left alone too: R's own name fallback is what would read
/// it, and reporting it would be arity's opinion rather than R's.
#[test]
fn a_marc_relator_role_is_not_flagged() {
    for role in ["\"spy\"", "\"aud\"", "\"Author\"", "\"compiler\""] {
        let value = format!(
            "person(\"Jane\", \"Doe\", role = c(\"cre\", {role}), email = \"jane@example.com\")"
        );
        assert!(
            authors_hits(&authored(&value)).is_empty(),
            "`{role}` is a role R reads",
        );
    }
}

/// R stores exactly one maintainer, so a second `cre` is a field R rejects
/// (`bad_authors_at_R_field_too_many_maintainers`).
#[test]
fn two_creators_are_flagged() {
    let text = authored(
        "c(person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), email = \"jane@example.com\"), \
         person(\"John\", \"Roe\", role = \"cre\", email = \"john@example.org\"))",
    );
    assert_eq!(authors_hits(&text).len(), 1, "{:?}", authors_hits(&text));
    assert!(messages(&text, AUTHORS_AT_R)[0].contains("`cre` role to 2 people"));
}

/// ORCID iDs carry a MOD 11-2 check digit, so a mistyped one is decidable
/// offline — which is the whole reason this is a lint and not a network call.
#[test]
fn a_malformed_orcid_is_flagged() {
    let bad = authored(
        "person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), email = \"jane@example.com\", \
         comment = c(ORCID = \"1234-5678-9012-3456\"))",
    );
    assert_eq!(authors_hits(&bad), ["\"1234-5678-9012-3456\""]);
    assert!(messages(&bad, AUTHORS_AT_R)[0].contains("ORCID"));

    let good = authored(
        "person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), email = \"jane@example.com\", \
         comment = c(ORCID = \"0000-0002-1825-0097\"))",
    );
    assert!(authors_hits(&good).is_empty());
}

/// R accepts the identifier in any of its written forms, and so must the rule.
#[test]
fn an_orcid_url_is_read_in_every_variant() {
    for id in [
        "0000-0002-1825-0097",
        "https://orcid.org/0000-0002-1825-0097",
        "orcid.org/0000-0002-1825-0097",
        "<https://orcid.org/0000-0002-1825-0097>",
    ] {
        let value = format!(
            "person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), \
             email = \"jane@example.com\", comment = c(ORCID = \"{id}\"))"
        );
        assert!(authors_hits(&authored(&value)).is_empty(), "`{id}`");
    }
}

/// Two people cannot share one ORCID iD; one of the two is a copy-paste.
#[test]
fn a_duplicated_orcid_is_flagged() {
    let text = authored(
        "c(person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), email = \"jane@example.com\", \
         comment = c(ORCID = \"0000-0002-1825-0097\")), \
         person(\"John\", \"Roe\", role = \"ctb\", \
         comment = c(ORCID = \"0000-0002-1825-0097\")))",
    );
    let messages = messages(&text, AUTHORS_AT_R);
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(messages[0].contains("same ORCID"), "{messages:?}");
}

/// A ROR ID is nine characters, in the same variant spellings.
#[test]
fn a_malformed_ror_is_flagged() {
    let bad = authored(
        "person(\"Posit Software, PBC\", role = c(\"cph\", \"cre\"), \
         email = \"info@posit.co\", comment = c(ROR = \"12345\"))",
    );
    assert_eq!(authors_hits(&bad), ["\"12345\""]);
    assert!(messages(&bad, AUTHORS_AT_R)[0].contains("ROR"));
}

/// Nothing about a computed value is decidable without running R, and running R
/// is not something arity does. The rule resolves what it can and says nothing
/// about the rest.
#[test]
fn a_computed_authors_at_r_is_silent() {
    for value in [
        "person(\"Jane\", \"Doe\", role = ROLES, email = \"jane@example.com\")",
        "person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), email = the_email)",
        "as.person(AUTHORS)",
        "paste0(\"Jane Doe <jane@example.com>\")",
    ] {
        assert!(
            authors_hits(&authored(value)).is_empty(),
            "`{value}` is not statically resolvable, so the rule has nothing to say",
        );
    }
}

/// A field wrapped across continuation lines is what a real `DESCRIPTION` looks
/// like, and the span has to land on the person, not on the whole field.
#[test]
fn a_wrapped_field_spans_the_offending_person() {
    let text = authored(
        "c(\n    person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), \
         email = \"jane@example.com\"),\n    person(\"John\", \"Roe\")\n  )",
    );
    assert_eq!(authors_hits(&text), ["person(\"John\", \"Roe\")"]);
}

/// `person()` with no arguments returns a **zero-length** person vector, so it
/// names nobody rather than naming a nameless somebody — every clause this rule
/// has is about a person R actually holds. The leftover call itself is
/// `description-empty-person`'s subject.
#[test]
fn an_argument_less_person_names_nobody() {
    let text = authored(
        "c(person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), email = \"jane@example.com\"), \
         person())",
    );
    assert!(authors_hits(&text).is_empty(), "{:?}", authors_hits(&text));
}

/// No `Authors@R` at all is `description-missing-field`'s subject, not this
/// rule's.
#[test]
fn an_absent_authors_at_r_is_silent() {
    let text = "Package: testpkg\nVersion: 0.1.0\nAuthor: Jane Doe\n\
                Maintainer: Jane Doe <jane@example.com>\n";
    assert!(authors_hits(text).is_empty());
}

/// CRAN's `author_starts_with_Author`: a value that begins with the field
/// header is one someone pasted in whole.
#[test]
fn an_author_field_repeating_its_own_header_is_flagged() {
    let text = format!("{COMPLETE_DESCRIPTION}Author: Author: Jane Doe [aut, cre]\n");
    assert_eq!(authors_hits(&text), ["Author: Jane Doe [aut, cre]"]);
    assert!(messages(&text, AUTHORS_AT_R)[0].contains("field name"));
}

/// CRAN's `author_should_be_authors_at_R`: `Author` is a plain string R never
/// evaluates, so a `person(...)` written there is displayed verbatim, brackets
/// and all.
#[test]
fn an_author_field_holding_r_code_is_flagged() {
    for value in [
        "person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"))",
        "c(person(\"Jane\", \"Doe\"))",
        "Authors@R: person(\"Jane\", \"Doe\")",
    ] {
        let text = format!("{COMPLETE_DESCRIPTION}Author: {value}\n");
        assert_eq!(authors_hits(&text), [value], "`{value}`");
    }
    let text = format!("{COMPLETE_DESCRIPTION}Author: Jane Doe [aut, cre]\n");
    assert!(authors_hits(&text).is_empty());
}

/// No autofix anywhere in the rule: an email, a name, a role, and an ORCID
/// check digit are all facts about a person that only that person has.
#[test]
fn authors_at_r_ships_no_fix() {
    let text = authored("person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"))");
    let finding =
        check_description_document(Path::new("DESCRIPTION"), &text, &LintConfig::default())
            .expect("linting should not error")
            .into_iter()
            .find(|d| d.rule == AUTHORS_AT_R)
            .expect("an authors-at-r finding");
    assert!(finding.fix.is_none());
}

// ---------------------------------------------------------------------------
// description-empty-person
// ---------------------------------------------------------------------------

const EMPTY_PERSON: &str = "description-empty-person";

/// The findings of `EMPTY_PERSON`, as the source text each one spans.
fn empty_person_hits(text: &str) -> Vec<String> {
    check_description_document(Path::new("DESCRIPTION"), text, &LintConfig::default())
        .expect("linting should not error")
        .iter()
        .filter(|d| d.rule == EMPTY_PERSON)
        .map(|d| {
            let start: usize = d.range.start().into();
            let end: usize = d.range.end().into();
            text[start..end].to_string()
        })
        .collect()
}

/// The shape this rule exists for, taken from `xfun`'s `DESCRIPTION`: a
/// `person()` opened for a contributor who was never filled in. R drops it
/// without a word, so nothing in `R CMD check` will ever mention it.
#[test]
fn an_argument_less_person_is_flagged() {
    let text = authored(
        "c(person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), email = \"jane@example.com\"), \
         person())",
    );
    assert_eq!(empty_person_hits(&text), ["person()"]);
    assert!(messages(&text, EMPTY_PERSON)[0].contains("nobody"));
}

/// `person(NULL)` is the same zero-length vector by the same branch: R returns
/// early when *every* argument is `NULL`.
#[test]
fn a_person_of_nulls_is_flagged() {
    let text = authored(
        "c(person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), email = \"jane@example.com\"), \
         person(NULL, NULL))",
    );
    assert_eq!(empty_person_hits(&text), ["person(NULL, NULL)"]);
}

/// Every empty call is its own leftover, and each gets its own caret.
#[test]
fn every_empty_person_is_reported() {
    let text = authored(
        "c(person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), email = \"jane@example.com\"), \
         person(), person())",
    );
    assert_eq!(empty_person_hits(&text), ["person()", "person()"]);
}

/// A person carrying anything at all is a person, however little R can make of
/// them — that is `description-authors-at-r`'s subject, not this rule's.
#[test]
fn a_person_with_any_argument_is_not_empty() {
    for value in [
        "person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), email = \"jane@example.com\")",
        "c(person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), \
         email = \"jane@example.com\"), person(role = \"ctb\"))",
        "c(person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), \
         email = \"jane@example.com\"), person(\"\"))",
    ] {
        assert!(
            empty_person_hits(&authored(value)).is_empty(),
            "`{value}` names somebody, however thinly",
        );
    }
}

/// A computed argument could be `NULL` and could be a name, and which one it is
/// needs R. The rule reports only what the text decides.
#[test]
fn a_computed_person_is_silent() {
    let text = authored(
        "c(person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), email = \"jane@example.com\"), \
         person(given = the_name))",
    );
    assert!(empty_person_hits(&text).is_empty());
}

/// A field arity cannot read at all says nothing about empty people either.
#[test]
fn an_unresolvable_field_reports_no_empty_person() {
    assert!(empty_person_hits(&authored("person(\"Jane\",")).is_empty());
    assert!(empty_person_hits(&authored("as.person(AUTHORS)")).is_empty());
}

/// No autofix: deleting the call means deleting a comma that belongs to its
/// neighbor, and filling it in is the author's.
#[test]
fn empty_person_ships_no_fix() {
    let text = authored(
        "c(person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"), email = \"jane@example.com\"), \
         person())",
    );
    let finding =
        check_description_document(Path::new("DESCRIPTION"), &text, &LintConfig::default())
            .expect("linting should not error")
            .into_iter()
            .find(|d| d.rule == EMPTY_PERSON)
            .expect("an empty-person finding");
    assert!(finding.fix.is_none());
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
