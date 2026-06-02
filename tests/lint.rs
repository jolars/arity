use std::process::{Command, Stdio};

use ravel::linter::{LintStatus, check_paths};
use tempfile::tempdir;

#[test]
fn lint_reports_clean_status_for_parseable_files() {
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("ok.R");
    std::fs::write(&path, "x <- 1\nprint(x)\n").expect("failed to write file");

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");
    assert_eq!(result.checked_files, 1);
    assert_eq!(result.total_findings, 0);
    assert_eq!(result.reports.len(), 1);
    assert_eq!(result.reports[0].status, LintStatus::Clean);
}

#[test]
fn lint_flags_duplicate_formal() {
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("dup.R");
    std::fs::write(&path, "f <- function(x, x) x\nf(1, 2)\n").expect("failed to write file");

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");
    let report = &result.reports[0];
    assert!(matches!(report.status, LintStatus::Findings { count: 1 }));
    assert_eq!(report.diagnostics[0].rule, "duplicate-formal");
}

#[test]
fn lint_flags_unused_binding() {
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("unused.R");
    std::fs::write(&path, "x <- 1\nprint(2)\n").expect("failed to write file");

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");
    let diags: Vec<&str> = result.reports[0]
        .diagnostics
        .iter()
        .map(|d| d.rule)
        .collect();
    assert!(diags.contains(&"unused-binding"));
}

#[test]
fn lint_reports_parse_diagnostics_pathway() {
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("bad.R");
    std::fs::write(&path, "x <-\n").expect("failed to write file");

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");
    assert!(matches!(
        result.reports[0].status,
        LintStatus::ParseDiagnostics { count: c } if c > 0
    ));
}

#[test]
fn cli_lint_check_passes_when_no_findings() {
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("ok.R");
    std::fs::write(&path, "x <- 1\nprint(x)\n").expect("failed to write file");

    let output = run_cli([
        "lint",
        "--check",
        dir.path().to_str().expect("temp dir path should be utf-8"),
    ]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn cli_lint_reports_concise_output() {
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("dup.R");
    std::fs::write(&path, "f <- function(x, x) x\nf(1, 2)\n").expect("failed to write file");

    let output = run_cli([
        "lint",
        "--check",
        "--output=concise",
        dir.path().to_str().expect("temp dir path should be utf-8"),
    ]);

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("dup.R:1:18: error [duplicate-formal]"),
        "got stderr: {stderr}"
    );
}

#[test]
fn cli_lint_reports_pretty_output_by_default() {
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("dup.R");
    std::fs::write(&path, "f <- function(x, x) x\nf(1, 2)\n").expect("failed to write file");

    let output = run_cli([
        "lint",
        "--check",
        dir.path().to_str().expect("temp dir path should be utf-8"),
    ]);

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    // annotate-snippets uses an arrow and pipe characters in its plain renderer.
    assert!(stderr.contains("duplicate-formal"), "got stderr: {stderr}");
    assert!(stderr.contains("dup.R"), "got stderr: {stderr}");
}

#[test]
fn cli_lint_works_without_check_flag() {
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("dup.R");
    std::fs::write(&path, "f <- function(x, x) x\nf(1, 2)\n").expect("failed to write file");

    let output = run_cli([
        "lint",
        dir.path().to_str().expect("temp dir path should be utf-8"),
    ]);

    // Without --check, the exit code still signals findings; this is consistent
    // with the formatter (where --check just controls write-vs-report).
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn cli_lint_reports_parse_diagnostics_pathway() {
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("bad.R");
    std::fs::write(&path, "x <-\n").expect("failed to write file");

    let output = run_cli([
        "lint",
        "--check",
        dir.path().to_str().expect("temp dir path should be utf-8"),
    ]);

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("lint blocked by parse diagnostics:"));
}

#[test]
fn cli_lint_requires_paths() {
    let output = run_cli(["lint"]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("at least one input path"));
}

#[test]
fn cli_lint_emits_json_output() {
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("dup.R");
    std::fs::write(&path, "f <- function(x, x) x\nf(1, 2)\n").expect("failed to write file");

    let output = run_cli([
        "lint",
        "--check",
        "--output=json",
        dir.path().to_str().expect("temp dir path should be utf-8"),
    ]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("json should parse");
    assert!(parsed.is_array());
    let array = parsed.as_array().unwrap();
    assert_eq!(array.len(), 1);
    assert_eq!(array[0]["rule"], "duplicate-formal");
}

fn run_cli<const N: usize>(args: [&str; N]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ravel"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to run cli")
}
