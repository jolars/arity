use std::path::Path;
use std::process::{Command, Stdio};

use ravel::config::LintConfig;
use ravel::linter::{Applicability, LintStatus, apply_fixes, check_document, check_paths};
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

// ---------------------------------------------------------------------------
// undefined-symbol: default-on, gated on all attached packages being indexed
// ---------------------------------------------------------------------------

use ravel::linter::check_document_with_provider;
use ravel::rindex::provider::{CompositeProvider, IndexedProvider};
use ravel::rindex::schema::{PackageIndex, SymbolEntry, SymbolKind};

fn indexed_pkg(name: &str, exports: &[&str]) -> PackageIndex {
    PackageIndex {
        schema_version: ravel::rindex::schema::SCHEMA_VERSION,
        package: name.into(),
        version: "1.0".into(),
        lib_path: "/lib".into(),
        r_version: None,
        harvested_at: 0,
        symbols: exports
            .iter()
            .map(|n| SymbolEntry {
                name: (*n).into(),
                kind: SymbolKind::Function,
                exported: true,
                formals: None,
                help: None,
            })
            .collect(),
    }
}

fn undefined_with(src: &str, provider: &CompositeProvider) -> Vec<String> {
    check_document_with_provider(Path::new("t.R"), src, &LintConfig::default(), provider)
        .expect("lint should succeed")
        .into_iter()
        .filter(|d| d.rule == "undefined-symbol")
        .map(|d| d.message.body.clone())
        .collect()
}

#[test]
fn undefined_symbol_flags_base_only_typo() {
    // No attached packages: base R is fully known, so a typo is genuinely
    // undefined and is flagged.
    let p = CompositeProvider::base_only();
    let msgs = undefined_with("lenth(1)\n", &p);
    assert_eq!(msgs.len(), 1, "expected one finding, got {msgs:?}");
    assert!(msgs[0].contains("lenth"));
}

#[test]
fn undefined_symbol_gated_off_when_attached_package_unindexed() {
    // `library(somepkg)` with somepkg un-indexed: the rule stays silent for the
    // whole file because somepkg could export `bogus`.
    let p = CompositeProvider::base_only();
    let msgs = undefined_with("library(somepkg)\nbogus()\n", &p);
    assert!(msgs.is_empty(), "gate should suppress, got {msgs:?}");
}

#[test]
fn undefined_symbol_resolves_indexed_export_and_flags_others() {
    // dplyr indexed (exports `across`): `across()` resolves, `bogus()` doesn't.
    let p = CompositeProvider::with_index(IndexedProvider::from_indices([indexed_pkg(
        "dplyr",
        &["across"],
    )]));
    let msgs = undefined_with("library(dplyr)\nacross()\nbogus()\n", &p);
    assert_eq!(msgs.len(), 1, "expected only `bogus`, got {msgs:?}");
    assert!(msgs[0].contains("bogus"));
}

// ---------------------------------------------------------------------------
// Autofix
// ---------------------------------------------------------------------------

fn diagnostics(src: &str) -> Vec<ravel::linter::Diagnostic> {
    check_document(Path::new("t.R"), src, &LintConfig::default()).expect("lint should succeed")
}

#[test]
fn assignment_in_condition_emits_safe_eq_fix() {
    let src = "if (x = 1) print(x)\n";
    let d = diagnostics(src)
        .into_iter()
        .find(|d| d.rule == "assignment-in-condition")
        .expect("expected an assignment-in-condition finding");
    let fix = d.fix.as_ref().expect("should carry a fix");
    assert_eq!(fix.applicability, Applicability::Safe);
    assert_eq!(fix.content, "==");

    let out = apply_fixes(src, std::slice::from_ref(fix), false);
    assert_eq!(out.output, "if (x == 1) print(x)\n");
}

#[test]
fn unused_binding_emits_unsafe_deletion_fix() {
    let src = "x <- 1\nprint(2)\n";
    let diags = diagnostics(src);
    let d = diags
        .iter()
        .find(|d| d.rule == "unused-binding")
        .expect("expected an unused-binding finding");
    let fix = d.fix.as_ref().expect("should carry a fix");
    assert_eq!(fix.applicability, Applicability::Unsafe);

    // Safe-only application is a no-op; opting in deletes the whole statement,
    // leaving no orphaned blank line.
    assert_eq!(
        apply_fixes(src, std::slice::from_ref(fix), false).output,
        src
    );
    assert_eq!(
        apply_fixes(src, std::slice::from_ref(fix), true).output,
        "print(2)\n"
    );
}

#[test]
fn fix_output_parses_and_is_format_idempotent() {
    use ravel::formatter::{FormatStyle, format_with_style};
    use ravel::parser::parse;

    let src = "x <- 1\nprint(2)\n";
    let fixes: Vec<_> = diagnostics(src).into_iter().filter_map(|d| d.fix).collect();
    let fixed = apply_fixes(src, &fixes, true).output;
    assert_eq!(fixed, "print(2)\n");

    assert!(
        parse(&fixed).diagnostics.is_empty(),
        "fixed output must parse cleanly"
    );
    let formatted = format_with_style(&fixed, FormatStyle::default()).expect("formats");
    let twice = format_with_style(&formatted, FormatStyle::default()).expect("formats");
    assert_eq!(formatted, twice, "fixed output should be format-idempotent");
}

#[test]
fn cli_fix_applies_safe_fixes_and_leaves_unsafe() {
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("fix.R");
    std::fs::write(&path, "if (x = 1) {\n  y <- 2\n}\n").expect("failed to write file");

    let output = run_cli(["lint", "--fix", path.to_str().unwrap()]);
    // The `=`→`==` fix lands; the unused `y` (unsafe) remains, so exit is 1.
    assert_eq!(output.status.code(), Some(1));
    let content = std::fs::read_to_string(&path).expect("read back");
    assert_eq!(content, "if (x == 1) {\n  y <- 2\n}\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("1 fix applied"), "got: {stderr}");
}

#[test]
fn cli_fix_unsafe_clears_top_level_findings() {
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("fix.R");
    // `x` is bound so the only findings are the two this test exercises
    // (assignment-in-condition + the unused `unused`); otherwise the now-default
    // `undefined-symbol` rule would flag the read of `x`.
    std::fs::write(&path, "x <- 0\nif (x = 1) print(x)\nunused <- 2\n")
        .expect("failed to write file");

    let output = run_cli(["lint", "--fix", "--unsafe-fixes", path.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    let content = std::fs::read_to_string(&path).expect("read back");
    // `=`→`==` (safe) and the top-level unused deletion (unsafe) both land.
    assert_eq!(content, "x <- 0\nif (x == 1) print(x)\n");
}

#[test]
fn cli_fix_withholds_unsafe_deletion_that_would_empty_a_block() {
    // Deleting the sole statement of a block would leave `{\n}`, which the
    // formatter rewrites to `{}` — so the deletion is withheld (tenet 5). The
    // finding is still reported (exit 1) and the file stays format-clean.
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("fix.R");
    std::fs::write(&path, "if (cond) {\n  unused <- 2\n}\n").expect("failed to write file");

    let output = run_cli(["lint", "--fix", "--unsafe-fixes", path.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    let content = std::fs::read_to_string(&path).expect("read back");
    assert_eq!(content, "if (cond) {\n  unused <- 2\n}\n");

    // The withheld result must pass `format --check`.
    let check = run_cli(["format", "--check", path.to_str().unwrap()]);
    assert!(
        check.status.success(),
        "withheld output should be format-clean; stderr: {}",
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn cli_fix_fixpoint_clears_multiple_unused_bindings() {
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("fix.R");
    // Two independent unused bindings; both should be removed in one invocation.
    std::fs::write(&path, "a <- 1\nb <- 2\nprint(3)\n").expect("failed to write file");

    let output = run_cli(["lint", "--fix", "--unsafe-fixes", path.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    let content = std::fs::read_to_string(&path).expect("read back");
    assert_eq!(content, "print(3)\n");
}

// ---------------------------------------------------------------------------
// Tenet 5: autofixes never introduce formatting errors.
// `format` -> apply all fixes -> `format --check` must still pass.
// ---------------------------------------------------------------------------

/// Format `input` to canonical form, apply every available fix to a fixpoint,
/// then assert the result still parses and is format-clean.
fn assert_fix_is_format_stable(input: &str) {
    use ravel::formatter::{FormatStyle, format_with_style};
    use ravel::parser::parse;

    let style = FormatStyle::default();
    let clean = format_with_style(input, style).expect("input should format");

    let mut content = clean.clone();
    for _ in 0..10 {
        let diags = check_document(Path::new("t.R"), &content, &LintConfig::default())
            .expect("lint should succeed");
        let fixes: Vec<_> = diags.into_iter().filter_map(|d| d.fix).collect();
        if fixes.is_empty() {
            break;
        }
        let out = apply_fixes(&content, &fixes, true); // include unsafe
        if out.applied == 0 {
            break;
        }
        content = out.output;
    }

    assert!(
        parse(&content).diagnostics.is_empty(),
        "fixed output must parse cleanly:\n{content:?}"
    );
    let reformatted = format_with_style(&content, style).expect("fixed output should format");
    assert_eq!(
        content, reformatted,
        "a fix introduced a formatting error (tenet 5).\nstarted from:\n{clean}\n--- after fixes ---\n{content}\n--- but format produces ---\n{reformatted}"
    );
}

#[test]
fn fixes_never_introduce_formatting_errors() {
    let cases = [
        // assignment-in-condition (`=` → `==`)
        "if (x = 1) print(x)\n",
        "while (y = f()) g()\n",
        // unused-binding deletion — top level
        "unused <- 1\nprint(2)\n",
        "unused <- 1\n\nprint(2)\n",
        "print(2)\n\nunused <- 1\n",
        "a <- 1\nb <- 2\nprint(3)\n",
        "only <- 1\n",
        // unused-binding deletion — inside blocks (the dangerous shapes)
        "if (cond) {\n  unused <- 1\n}\n",
        "f <- function() {\n  unused <- 1\n  g()\n}\n",
        "f <- function() {\n  unused <- 1\n  a()\n  b()\n}\n",
        "for (i in xs) {\n  unused <- 1\n  use(i)\n}\n",
    ];
    for case in cases {
        assert_fix_is_format_stable(case);
    }
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
