use std::path::Path;
use std::process::{Command, Stdio};

use arity::config::LintConfig;
use arity::linter::{
    Applicability, LintResult, LintStatus, apply_fixes, check_document, check_paths,
};
use tempfile::tempdir;

/// Rule ids reported for the file named `file_name` in `result`.
fn rules_for<'a>(result: &'a LintResult, file_name: &str) -> Vec<&'a str> {
    result
        .reports
        .iter()
        .find(|r| r.path.file_name().and_then(|n| n.to_str()) == Some(file_name))
        .map(|r| r.diagnostics.iter().map(|d| d.rule).collect())
        .unwrap_or_default()
}

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
fn package_resolves_bindings_across_files() {
    // A package shares one namespace across R/*.R: `foo` defined in a.R both
    // resolves from b.R (no undefined-symbol) and counts as used (no
    // unused-binding in a.R).
    let dir = tempdir().expect("failed to create temp dir");
    std::fs::write(dir.path().join("DESCRIPTION"), "Package: testpkg\n").unwrap();
    let r_dir = dir.path().join("R");
    std::fs::create_dir(&r_dir).unwrap();
    std::fs::write(r_dir.join("a.R"), "foo <- function() 1\n").unwrap();
    std::fs::write(r_dir.join("b.R"), "foo()\n").unwrap();

    let result =
        check_paths(std::slice::from_ref(&dir.path().to_path_buf())).expect("lint should succeed");

    assert_eq!(result.total_findings, 0, "reports: {:?}", result.reports);
    assert!(rules_for(&result, "a.R").is_empty());
    assert!(rules_for(&result, "b.R").is_empty());
}

#[test]
fn source_closure_resolves_bindings_across_scripts() {
    // a.R sources helpers.R, so `greet` resolves there, and helpers.R's `greet`
    // is used (not flagged unused).
    let dir = tempdir().expect("failed to create temp dir");
    std::fs::write(dir.path().join("a.R"), "source(\"helpers.R\")\ngreet()\n").unwrap();
    std::fs::write(dir.path().join("helpers.R"), "greet <- function() \"hi\"\n").unwrap();

    let result =
        check_paths(std::slice::from_ref(&dir.path().to_path_buf())).expect("lint should succeed");

    assert!(
        !rules_for(&result, "a.R").contains(&"undefined-symbol"),
        "a.R: {:?}",
        rules_for(&result, "a.R")
    );
    assert!(
        !rules_for(&result, "helpers.R").contains(&"unused-binding"),
        "helpers.R: {:?}",
        rules_for(&result, "helpers.R")
    );
}

#[test]
fn dynamic_source_suppresses_undefined_symbol() {
    // A `source()` we can't resolve statically could define anything, so we must
    // not flag otherwise-unresolved names in that file.
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("a.R");
    std::fs::write(&path, "source(paste0(d, \".R\"))\nmystery()\n").unwrap();

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");

    assert!(
        !rules_for(&result, "a.R").contains(&"undefined-symbol"),
        "a.R: {:?}",
        rules_for(&result, "a.R")
    );
}

/// Write a minimal package (DESCRIPTION + NAMESPACE + R/a.R) and lint it.
fn lint_package(namespace: &str, a_r: &str) -> LintResult {
    let dir = tempdir().expect("failed to create temp dir");
    std::fs::write(dir.path().join("DESCRIPTION"), "Package: testpkg\n").unwrap();
    std::fs::write(dir.path().join("NAMESPACE"), namespace).unwrap();
    let r_dir = dir.path().join("R");
    std::fs::create_dir(&r_dir).unwrap();
    std::fs::write(r_dir.join("a.R"), a_r).unwrap();
    check_paths(std::slice::from_ref(&dir.path().to_path_buf())).expect("lint should succeed")
}

#[test]
fn namespace_export_is_not_unused() {
    // `foo` is exported, so it's public API — not an unused binding even though
    // nothing in the package reads it.
    let result = lint_package("export(foo)\n", "foo <- function() 1\n");
    assert!(
        !rules_for(&result, "a.R").contains(&"unused-binding"),
        "a.R: {:?}",
        rules_for(&result, "a.R")
    );
}

#[test]
fn namespace_import_from_resolves_symbol() {
    // `filter` is imported from dplyr, so it resolves.
    let result = lint_package(
        "importFrom(dplyr, filter)\n",
        "my_fun <- function(d) filter(d)\nmy_fun(1)\n",
    );
    assert!(
        !rules_for(&result, "a.R").contains(&"undefined-symbol"),
        "a.R: {:?}",
        rules_for(&result, "a.R")
    );
}

#[test]
fn namespace_wholesale_import_suppresses_undefined_symbol() {
    // `import(rlang)` brings unknown exports into scope, so `abort` must not
    // flag as undefined.
    let result = lint_package("import(rlang)\n", "f <- function() abort(\"boom\")\nf()\n");
    assert!(
        !rules_for(&result, "a.R").contains(&"undefined-symbol"),
        "a.R: {:?}",
        rules_for(&result, "a.R")
    );
}

#[test]
fn project_aware_document_resolves_cross_file() {
    // The LSP entry: linting b.R (live buffer) resolves `foo` from a sibling
    // a.R read off disk.
    use arity::incremental::IncrementalDatabase;
    use arity::linter::check_document_in_project;
    use arity::rindex::provider::CompositeProvider;

    let dir = tempdir().expect("failed to create temp dir");
    std::fs::write(dir.path().join("DESCRIPTION"), "Package: testpkg\n").unwrap();
    let r_dir = dir.path().join("R");
    std::fs::create_dir(&r_dir).unwrap();
    std::fs::write(r_dir.join("a.R"), "foo <- function() 1\n").unwrap();
    let b = r_dir.join("b.R");
    std::fs::write(&b, "foo()\n").unwrap();

    let mut db = IncrementalDatabase::default();
    let active = db.upsert_file(&b, std::fs::read_to_string(&b).unwrap());
    let provider = CompositeProvider::base_only();

    let diags = check_document_in_project(&mut db, &b, active, &LintConfig::default(), &provider)
        .expect("lint should succeed");
    let rules: Vec<&str> = diags.iter().map(|d| d.rule).collect();
    assert!(!rules.contains(&"undefined-symbol"), "diags: {rules:?}");
}

#[test]
fn project_aware_relint_reuses_unchanged_siblings() {
    // Re-linting with unchanged content must not re-parse sibling files: the
    // salsa caches stay warm across LSP keystrokes.
    use arity::incremental::IncrementalDatabase;
    use arity::linter::check_document_in_project;
    use arity::rindex::provider::CompositeProvider;

    let dir = tempdir().expect("failed to create temp dir");
    std::fs::write(dir.path().join("DESCRIPTION"), "Package: testpkg\n").unwrap();
    let r_dir = dir.path().join("R");
    std::fs::create_dir(&r_dir).unwrap();
    std::fs::write(r_dir.join("a.R"), "foo <- function() 1\n").unwrap();
    let b = r_dir.join("b.R");
    std::fs::write(&b, "foo()\n").unwrap();

    let mut db = IncrementalDatabase::default();
    let active = db.upsert_file(&b, std::fs::read_to_string(&b).unwrap());
    let provider = CompositeProvider::base_only();

    check_document_in_project(&mut db, &b, active, &LintConfig::default(), &provider).unwrap();
    db.clear_query_log();
    check_document_in_project(&mut db, &b, active, &LintConfig::default(), &provider).unwrap();

    assert!(
        db.query_log().is_empty(),
        "unchanged re-lint re-ran queries: {:?}",
        db.query_log().len()
    );
}

#[test]
fn body_edit_relint_does_not_rebuild_project_scope() {
    // The firewall on the real two-phase path: editing the active file's
    // function *body* re-parses it but must not rebuild the cross-file project
    // graph, since its exports / free reads / source edges are unchanged.
    use arity::incremental::{IncrementalDatabase, QueryKind};
    use arity::linter::check_document_in_project;
    use arity::rindex::provider::CompositeProvider;

    let dir = tempdir().expect("failed to create temp dir");
    std::fs::write(dir.path().join("DESCRIPTION"), "Package: testpkg\n").unwrap();
    let r_dir = dir.path().join("R");
    std::fs::create_dir(&r_dir).unwrap();
    std::fs::write(r_dir.join("a.R"), "foo <- function() 1\n").unwrap();
    let b = r_dir.join("b.R");
    std::fs::write(&b, "bar <- function() {\n  foo()\n}\n").unwrap();

    let mut db = IncrementalDatabase::default();
    let provider = CompositeProvider::base_only();
    let cfg = LintConfig::default();

    let active = db.upsert_file(&b, std::fs::read_to_string(&b).unwrap());
    check_document_in_project(&mut db, &b, active, &cfg, &provider).unwrap();
    db.clear_query_log();

    // Edit b's body only (still defines `bar`, still reads `foo`).
    let active = db.upsert_file(&b, "bar <- function() {\n  foo()\n  2\n}\n".to_string());
    check_document_in_project(&mut db, &b, active, &cfg, &provider).unwrap();

    let kinds: Vec<QueryKind> = db.query_log().iter().map(|e| e.kind).collect();
    assert!(
        !kinds.contains(&QueryKind::ProjectGraph),
        "body edit rebuilt the project graph: {kinds:?}"
    );
}

#[test]
fn prepared_split_matches_wrapper_and_runs_on_clone() {
    // The write/read split (prepare_document_in_project + analyze_prepared) must
    // reproduce check_document_in_project exactly, and the read-phase must work
    // off a db *clone* — the property the LSP relies on to lint off its thread.
    use arity::incremental::IncrementalDatabase;
    use arity::linter::{
        analyze_prepared, check_document_in_project, prepare_document_in_project,
        seed_workspace_for,
    };
    use arity::rindex::provider::CompositeProvider;

    let dir = tempdir().expect("failed to create temp dir");
    std::fs::write(dir.path().join("DESCRIPTION"), "Package: testpkg\n").unwrap();
    let r_dir = dir.path().join("R");
    std::fs::create_dir(&r_dir).unwrap();
    std::fs::write(r_dir.join("a.R"), "foo <- function() 1\n").unwrap();
    let b = r_dir.join("b.R");
    // `foo` resolves cross-file; `bar` is genuinely undefined → one finding.
    std::fs::write(&b, "foo()\nbar()\n").unwrap();

    let keys = |diags: &[arity::linter::Diagnostic]| -> Vec<(String, u32, u32)> {
        let mut v: Vec<_> = diags
            .iter()
            .map(|d| {
                (
                    d.rule.to_string(),
                    u32::from(d.range.start()),
                    u32::from(d.range.end()),
                )
            })
            .collect();
        v.sort();
        v
    };
    let provider = CompositeProvider::base_only();
    let cfg = LintConfig::default();

    // Reference: the wrapper.
    let mut db_ref = IncrementalDatabase::default();
    let active_ref = db_ref.upsert_file(&b, std::fs::read_to_string(&b).unwrap());
    let want = check_document_in_project(&mut db_ref, &b, active_ref, &cfg, &provider).unwrap();
    assert!(
        want.iter().any(|d| d.rule == "undefined-symbol"),
        "fixture should flag `bar`: {:?}",
        keys(&want)
    );

    // Split: prepare on the owner, analyze on a clone. The caller seeds the
    // workspace first (as the wrapper and the LSP's write-phase do), since
    // membership now comes from the explicit file-set, not a per-call walk.
    let mut db = IncrementalDatabase::default();
    let active = db.upsert_file(&b, std::fs::read_to_string(&b).unwrap());
    seed_workspace_for(&mut db, &b, active);
    let prepared = prepare_document_in_project(&mut db, &b, active, &cfg)
        .unwrap()
        .expect("clean file should prepare");
    let snapshot = db.snapshot();
    let got = analyze_prepared(&snapshot, &prepared, &provider);
    drop(snapshot);

    assert_eq!(keys(&got), keys(&want), "split diverged from the wrapper");
}

#[test]
fn prepare_returns_none_on_parse_error() {
    // A parse-erroring active buffer skips analysis entirely (Ok(None)), mirroring
    // the wrapper's empty-diagnostics early return.
    use arity::incremental::IncrementalDatabase;
    use arity::linter::prepare_document_in_project;

    let dir = tempdir().expect("failed to create temp dir");
    let f = dir.path().join("broken.R");
    std::fs::write(&f, "foo(\n").unwrap();

    let mut db = IncrementalDatabase::default();
    let active = db.upsert_file(&f, std::fs::read_to_string(&f).unwrap());
    let prepared =
        prepare_document_in_project(&mut db, &f, active, &LintConfig::default()).unwrap();
    assert!(prepared.is_none(), "parse error should yield Ok(None)");
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

use arity::linter::check_document_with_provider;
use arity::rindex::provider::{CompositeProvider, IndexedProvider};
use arity::rindex::schema::{PackageIndex, SymbolEntry, SymbolKind};

fn indexed_pkg(name: &str, exports: &[&str]) -> PackageIndex {
    PackageIndex {
        schema_version: arity::rindex::schema::SCHEMA_VERSION,
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

#[test]
fn undefined_symbol_resolves_bundled_cran_export_and_flags_others() {
    // data.table is a bundled (names-only) CRAN package, not locally indexed.
    // With it attached, a real export (`fread`) resolves and the gate no longer
    // suppresses the file, so a genuine typo (`bogus`) is still flagged.
    let p = CompositeProvider::base_only();
    let msgs = undefined_with("library(data.table)\nfread(\"x.csv\")\nbogus()\n", &p);
    assert_eq!(msgs.len(), 1, "expected only `bogus`, got {msgs:?}");
    assert!(msgs[0].contains("bogus"));
}

#[test]
fn undefined_symbol_still_gated_for_unbundled_package() {
    // A package neither indexed nor bundled keeps the conservative whole-file
    // suppression — no regression for the long tail.
    let p = CompositeProvider::base_only();
    let msgs = undefined_with("library(some_obscure_pkg_xyz)\nbogus()\n", &p);
    assert!(msgs.is_empty(), "gate should suppress, got {msgs:?}");
}

// ---------------------------------------------------------------------------
// Autofix
// ---------------------------------------------------------------------------

fn diagnostics(src: &str) -> Vec<arity::linter::Diagnostic> {
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
    use arity::formatter::{FormatStyle, format_with_style};
    use arity::parser::parse;

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
    use arity::formatter::{FormatStyle, format_with_style};
    use arity::parser::parse;

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
    Command::new(env!("CARGO_BIN_EXE_arity"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to run cli")
}
