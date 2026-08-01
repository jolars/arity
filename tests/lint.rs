use std::path::Path;
use std::process::{Command, Stdio};

use arity::config::{DEFAULT_EXCLUDE, LintConfig};
use arity::file_discovery::ExcludeFilter;
use arity::linter::{
    Applicability, LintResult, LintStatus, apply_fixes, check_document, check_paths,
    check_paths_with_index,
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
fn super_assignment_is_not_unused() {
    // `done <<- TRUE` mutates the enclosing `done` (read via `if (done)`), so it
    // is a stateful write, not a dead local. A super-assignment must never flag
    // as an unused binding.
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("super.R");
    std::fs::write(
        &path,
        "make <- function() {\n  done <- FALSE\n  function() {\n    if (done) return(NULL)\n    done <<- TRUE\n  }\n}\nmake()\n",
    )
    .expect("failed to write file");

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");
    assert!(
        !rules_for(&result, "super.R").contains(&"unused-binding"),
        "super.R: {:?}",
        rules_for(&result, "super.R")
    );
}

#[test]
fn infix_operator_use_is_not_unused() {
    // A user-defined `%op%` used as an infix operator (`a %||% b`) reads its
    // definition, so the `` `%||%` `` binding must not flag as unused.
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("op.R");
    std::fs::write(
        &path,
        "`%||%` <- function(x, y) if (is.null(x)) y else x\nz <- a %||% b\nprint(z)\n",
    )
    .expect("failed to write file");

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");
    assert!(
        !rules_for(&result, "op.R").contains(&"unused-binding"),
        "op.R: {:?}",
        rules_for(&result, "op.R")
    );
}

#[test]
fn reassigned_binding_read_after_each_assignment_is_not_unused() {
    // A name assigned twice in one scope, with a read after each assignment, is
    // used both times. The later binding's read must resolve to it, not to the
    // first binding — otherwise the reassignment is a false unused-binding.
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("reassign.R");
    std::fs::write(
        &path,
        "f <- function() {\n  fit <- one()\n  use(fit)\n  fit <- two()\n  use(fit)\n}\nf()\n",
    )
    .expect("failed to write file");

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");
    assert!(
        !rules_for(&result, "reassign.R").contains(&"unused-binding"),
        "reassign.R: {:?}",
        result.reports[0].diagnostics,
    );
}

#[test]
fn s3method_registration_is_not_unused() {
    // S3 methods registered via `S3method(generic, class)` are public API; the
    // bound `generic.class` function must not be flagged unused even though
    // nothing in the package reads it directly.
    let result = lint_package(
        "S3method(coef, SLOPE)\n",
        "coef.SLOPE <- function(object, ...) object$coefficients\n",
    );
    assert!(
        !rules_for(&result, "a.R").contains(&"unused-binding"),
        "a.R: {:?}",
        rules_for(&result, "a.R")
    );
}

#[test]
fn named_subscript_argument_is_not_a_binding() {
    // `drop = FALSE` inside `[` is a named argument to the subset operator, not
    // a local assignment — it must not be reported as an unused binding.
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("subset.R");
    std::fs::write(&path, "f <- function(x, i) x[i, , drop = FALSE]\nf(1, 2)\n")
        .expect("failed to write file");

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");
    assert!(
        !rules_for(&result, "subset.R").contains(&"unused-binding"),
        "subset.R: {:?}",
        result.reports[0].diagnostics,
    );
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
fn excluded_generated_file_still_contributes_bindings() {
    // `cpp11.R` is in the default exclude set, so it is never linted. But it
    // defines the R wrappers (`native_fn`) that hand-written siblings call, so
    // its top-level bindings must still enter the package namespace — otherwise
    // every caller is a false `undefined-symbol`. (The exclude only suppresses
    // findings *in* the generated file, not its contribution to scope.)
    let dir = tempdir().expect("failed to create temp dir");
    std::fs::write(dir.path().join("DESCRIPTION"), "Package: testpkg\n").unwrap();
    let r_dir = dir.path().join("R");
    std::fs::create_dir(&r_dir).unwrap();
    std::fs::write(
        r_dir.join("cpp11.R"),
        "native_fn <- function(x) .Call(`_testpkg_native_fn`, x)\n",
    )
    .unwrap();
    std::fs::write(
        r_dir.join("use.R"),
        "wrap <- function(x) native_fn(x)\nwrap(1)\n",
    )
    .unwrap();

    // Exercise the real default exclude set (as the CLI builds it), which
    // `check_paths`/`check_paths_with_config` skip (they exclude nothing).
    let patterns: Vec<String> = DEFAULT_EXCLUDE.iter().map(|p| p.to_string()).collect();
    let exclude = ExcludeFilter::new(dir.path(), &patterns).expect("valid exclude patterns");
    let result = check_paths_with_index(
        std::slice::from_ref(&dir.path().to_path_buf()),
        &LintConfig::default(),
        &exclude,
        IndexedProvider::empty(),
    )
    .expect("lint should succeed");

    // The generated file is excluded from linting entirely.
    assert!(
        !result
            .reports
            .iter()
            .any(|r| r.path.file_name().and_then(|n| n.to_str()) == Some("cpp11.R")),
        "cpp11.R should be excluded from linting: {:?}",
        result.reports
    );
    // ...but the call to its wrapper resolves, so no false undefined-symbol.
    assert!(
        !rules_for(&result, "use.R").contains(&"undefined-symbol"),
        "use.R: {:?}",
        rules_for(&result, "use.R")
    );
}

/// Build a package on disk with a `tests/testthat/` test file and lint it,
/// returning the result. `r_file` is written to `R/foo.R`, `test_file` to
/// `tests/testthat/test-foo.R`.
fn lint_pkg_with_test(r_file: &str, test_file: &str, indexed: IndexedProvider) -> LintResult {
    let dir = tempdir().expect("failed to create temp dir");
    std::fs::write(dir.path().join("DESCRIPTION"), "Package: testpkg\n").unwrap();
    let r_dir = dir.path().join("R");
    std::fs::create_dir(&r_dir).unwrap();
    std::fs::write(r_dir.join("foo.R"), r_file).unwrap();
    let tt = dir.path().join("tests").join("testthat");
    std::fs::create_dir_all(&tt).unwrap();
    std::fs::write(tt.join("test-foo.R"), test_file).unwrap();

    let patterns: Vec<String> = DEFAULT_EXCLUDE.iter().map(|p| p.to_string()).collect();
    let exclude = ExcludeFilter::new(dir.path(), &patterns).expect("valid exclude patterns");
    check_paths_with_index(
        std::slice::from_ref(&dir.path().to_path_buf()),
        &LintConfig::default(),
        &exclude,
        indexed,
    )
    .expect("lint should succeed")
}

#[test]
fn testthat_verbs_resolve_in_test_files() {
    // testthat attaches itself before sourcing `tests/testthat/` files, so their
    // `test_that`/`expect_*` calls must resolve without an explicit
    // `library(testthat)`. With testthat indexed, a genuine typo is still flagged.
    let indexed =
        IndexedProvider::from_indices([indexed_pkg("testthat", &["test_that", "expect_equal"])]);
    let result = lint_pkg_with_test(
        "foo <- function() 1\n",
        "test_that(\"foo works\", {\n  expect_equal(foo(), 1)\n  bogus_undefined_fn()\n})\n",
        indexed,
    );

    let undefined: Vec<&str> = result
        .reports
        .iter()
        .find(|r| r.path.file_name().and_then(|n| n.to_str()) == Some("test-foo.R"))
        .map(|r| {
            r.diagnostics
                .iter()
                .filter(|d| d.rule == "undefined-symbol")
                .map(|d| d.message.body.as_str())
                .collect()
        })
        .unwrap_or_default();

    assert_eq!(
        undefined.len(),
        1,
        "expected only the typo, got {undefined:?}"
    );
    assert!(
        undefined[0].contains("bogus_undefined_fn"),
        "unexpected finding: {undefined:?}"
    );
}

#[test]
fn testthat_attach_is_scoped_to_test_files() {
    // The implicit testthat attach must not leak into ordinary package sources:
    // testthat unindexed (empty index) gate-suppresses the *test* file, but a
    // genuine unknown symbol in `R/foo.R` is still flagged.
    let result = lint_pkg_with_test(
        "foo <- function() bogus_in_r()\n",
        "test_that(\"x\", expect_equal(1, 1))\n",
        IndexedProvider::empty(),
    );

    assert!(
        !rules_for(&result, "test-foo.R").contains(&"undefined-symbol"),
        "test file should not flag testthat verbs: {:?}",
        rules_for(&result, "test-foo.R")
    );
    assert!(
        rules_for(&result, "foo.R").contains(&"undefined-symbol"),
        "R/ source should still flag unknown symbols: {:?}",
        rules_for(&result, "foo.R")
    );
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

#[test]
fn shadowed_builtin_flags_call_of_shadowed_name() {
    // The footgun the rule targets: `c` is bound to a *function* that shadows
    // base `c`, then `c(2, 3)` calls the local instead of base. Fire.
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("call.R");
    std::fs::write(
        &path,
        "f <- function() {\n  c <- function(x, y) x\n  c(2, 3)\n}\nf()\n",
    )
    .expect("failed to write file");

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");
    assert!(
        rules_for(&result, "call.R").contains(&"shadowed-builtin"),
        "call.R: {:?}",
        rules_for(&result, "call.R")
    );
}

#[test]
fn shadowed_builtin_ignores_value_binding_shadow() {
    // The dominant tidyverse idiom: `names <- names(data)` binds a *value*, then
    // `names(data)` is called again. R's call-position lookup skips the
    // non-function local and reaches base `names`, so there is no hazard. The
    // rule must stay silent (verified against R; this was a false positive on
    // tidyr).
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("valbind.R");
    std::fs::write(
        &path,
        "f <- function(data) {\n  names <- names(data)\n  names(data)\n}\nf(x)\n",
    )
    .expect("failed to write file");

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");
    assert!(
        !rules_for(&result, "valbind.R").contains(&"shadowed-builtin"),
        "value binding should not trigger shadowed-builtin: {:?}",
        rules_for(&result, "valbind.R")
    );
}

#[test]
fn shadowed_builtin_ignores_value_use_of_shadowed_name() {
    // `beta` shadows base `beta`, but is only ever indexed as a value
    // (`beta[[i]]`), never called — there's no "I meant base::beta()" hazard, so
    // the rule must stay silent.
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("value.R");
    std::fs::write(
        &path,
        "interp <- function(beta) {\n  beta[[1]] + beta[[2]]\n}\ninterp(x)\n",
    )
    .expect("failed to write file");

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");
    assert!(
        !rules_for(&result, "value.R").contains(&"shadowed-builtin"),
        "value use should not trigger shadowed-builtin: {:?}",
        rules_for(&result, "value.R")
    );
}

#[test]
fn shadowed_builtin_ignores_call_in_own_defining_rhs() {
    // `sign <- sign(x)` is idiomatic and safe: R evaluates the RHS `sign(x)`
    // before the local binding is live (and function lookup skips the
    // non-function local anyway). The call is part of the *defining assignment*,
    // not a "later" call, so the rule must stay silent.
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("selfrhs.R");
    std::fs::write(
        &path,
        "f <- function(x) {\n  sign <- sign(x)\n  x * sign\n}\nf(1)\n",
    )
    .expect("failed to write file");

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");
    assert!(
        !rules_for(&result, "selfrhs.R").contains(&"shadowed-builtin"),
        "defining RHS call should not trigger shadowed-builtin: {:?}",
        rules_for(&result, "selfrhs.R")
    );
}

#[test]
fn shadowed_builtin_ignores_parameters() {
    // A parameter named after a base function (`transform = identity`, then
    // `transform(x)`) is idiomatic: it's the intended target of the call, and R's
    // function-vs-value lookup resolves same-named calls correctly. The rule only
    // targets local `<-` shadowing, so a parameter must not trigger it.
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("param.R");
    std::fs::write(
        &path,
        "f <- function(x, transform = identity) {\n  transform(x)\n}\nf(1)\n",
    )
    .expect("failed to write file");

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");
    assert!(
        !rules_for(&result, "param.R").contains(&"shadowed-builtin"),
        "parameter should not trigger shadowed-builtin: {:?}",
        rules_for(&result, "param.R")
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
fn self_qualified_internal_use_is_not_unused() {
    // A non-exported helper read only via `pkg:::helper()` in a sibling file
    // (here a test) is used cross-file, so it must not flag unused. Mirrors the
    // real-world `SLOPE:::randomProblem(...)` case.
    let dir = tempdir().expect("failed to create temp dir");
    std::fs::write(dir.path().join("DESCRIPTION"), "Package: testpkg\n").unwrap();
    std::fs::write(dir.path().join("NAMESPACE"), "").unwrap();
    let r_dir = dir.path().join("R");
    std::fs::create_dir(&r_dir).unwrap();
    std::fs::write(r_dir.join("a.R"), "helper <- function() 1\n").unwrap();
    let test_dir = dir.path().join("tests").join("testthat");
    std::fs::create_dir_all(&test_dir).unwrap();
    std::fs::write(test_dir.join("test-a.R"), "testpkg:::helper()\n").unwrap();

    let result =
        check_paths(std::slice::from_ref(&dir.path().to_path_buf())).expect("lint should succeed");
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
fn project_aware_document_keeps_excluded_generated_scope() {
    // The single-document seed path (check_document_in_project -> seed_workspace_for)
    // must apply the exclude config yet keep generated package sources in scope:
    // `cpp11.R` is default-excluded (never a lint member) but defines wrappers the
    // active file calls, so dropping it from scope would be a false undefined-symbol.
    use arity::incremental::IncrementalDatabase;
    use arity::linter::check_document_in_project;
    use arity::rindex::provider::CompositeProvider;

    let dir = tempdir().expect("failed to create temp dir");
    std::fs::write(dir.path().join("DESCRIPTION"), "Package: testpkg\n").unwrap();
    let r_dir = dir.path().join("R");
    std::fs::create_dir(&r_dir).unwrap();
    std::fs::write(
        r_dir.join("cpp11.R"),
        "native_fn <- function(x) .Call(`_testpkg_native_fn`, x)\n",
    )
    .unwrap();
    let b = r_dir.join("use.R");
    std::fs::write(&b, "wrap <- function(x) native_fn(x)\nwrap(1)\n").unwrap();

    let mut db = IncrementalDatabase::default();
    let active = db.upsert_file(&b, std::fs::read_to_string(&b).unwrap());
    let provider = CompositeProvider::base_only();

    let diags = check_document_in_project(&mut db, &b, active, &LintConfig::default(), &provider)
        .expect("lint should succeed");
    let rules: Vec<&str> = diags.iter().map(|d| d.rule).collect();
    assert!(
        !rules.contains(&"undefined-symbol"),
        "excluded cpp11.R wrapper should still resolve: {rules:?}"
    );
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
    seed_workspace_for(&mut db, &b, active, &ExcludeFilter::none());
    let (rules, _) =
        arity::linter::rules::ResolvedRules::resolve(cfg.select.as_deref(), &cfg.ignore);
    let prepared = prepare_document_in_project(&mut db, &b, active, std::sync::Arc::new(rules))
        .expect("clean file should prepare");
    let snapshot = db.snapshot();
    let got = analyze_prepared(&snapshot, &prepared, &provider);
    drop(snapshot);

    assert_eq!(keys(&got), keys(&want), "split diverged from the wrapper");
}

#[test]
fn prepare_returns_none_on_parse_error() {
    // A parse-erroring active buffer skips analysis entirely (None), mirroring
    // the wrapper's empty-diagnostics early return.
    use arity::incremental::IncrementalDatabase;
    use arity::linter::prepare_document_in_project;
    use arity::linter::rules::ResolvedRules;

    let dir = tempdir().expect("failed to create temp dir");
    let f = dir.path().join("broken.R");
    std::fs::write(&f, "foo(\n").unwrap();

    let mut db = IncrementalDatabase::default();
    let active = db.upsert_file(&f, std::fs::read_to_string(&f).unwrap());
    let rules = std::sync::Arc::new(ResolvedRules::default_set());
    let prepared = prepare_document_in_project(&mut db, &f, active, rules);
    assert!(prepared.is_none(), "parse error should yield None");
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
fn syntax_error_diagnostics_map_message_range_and_rule() {
    use arity::incremental::ParseDiagnosticData;
    use arity::linter::syntax_error_diagnostics;

    let diags = vec![ParseDiagnosticData {
        message: "expected ')' to close function call".to_string(),
        start: 10,
        end: 11,
    }];
    let mapped = syntax_error_diagnostics(&diags, Path::new("bad.R"));
    assert_eq!(mapped.len(), 1);
    let d = &mapped[0];
    assert_eq!(d.rule, "syntax-error");
    assert_eq!(d.severity, arity::linter::Severity::Error);
    assert_eq!(u32::from(d.range.start()), 10);
    assert_eq!(u32::from(d.range.end()), 11);
    assert_eq!(d.message.body, "expected ')' to close function call");
    assert!(d.fix.is_none());
}

#[test]
fn render_sorts_diagnostics_by_offset() {
    use arity::incremental::ParseDiagnosticData;
    use arity::linter::{OutputMode, render_findings, syntax_error_diagnostics};

    // Parser recovery can emit diagnostics out of source order (a late-recovered
    // outer call closes after inner ones). Rendering sorts them by offset.
    let diags = syntax_error_diagnostics(
        &[
            ParseDiagnosticData {
                message: "later".to_string(),
                start: 27,
                end: 28,
            },
            ParseDiagnosticData {
                message: "earlier".to_string(),
                start: 10,
                end: 11,
            },
        ],
        Path::new("t.R"),
    );
    let src = "x <- cbind(1:5, 6:10\n\nmatrix(c(2)\n".to_string();
    let out = render_findings(&diags, OutputMode::Concise, false, &|_| Some(src.clone()));
    let earlier = out.find("earlier").expect("earlier diagnostic rendered");
    let later = out.find("later").expect("later diagnostic rendered");
    assert!(earlier < later, "diagnostics not sorted by offset:\n{out}");
}

#[test]
fn lint_surfaces_parse_diagnostics_as_findings() {
    // Parse errors block the lint rules but must still be reported, not swallowed:
    // the report carries a `syntax-error` diagnostic bearing the parser's message.
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("bad.R");
    std::fs::write(&path, "x <- cbind(1:5, 6:10\n").expect("failed to write file");

    let result = check_paths(std::slice::from_ref(&path)).expect("lint should succeed");
    let report = &result.reports[0];
    assert!(matches!(report.status, LintStatus::ParseDiagnostics { .. }));
    assert_eq!(report.diagnostics.len(), 1, "one parse diagnostic surfaced");
    let diag = &report.diagnostics[0];
    assert_eq!(diag.rule, "syntax-error");
    assert!(
        diag.message
            .body
            .contains("expected ')' to close function call"),
        "got: {}",
        diag.message.body
    );
}

#[test]
fn cli_lint_check_passes_when_no_findings() {
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("ok.R");
    std::fs::write(&path, "x <- 1\nprint(x)\n").expect("failed to write file");

    let output = run_cli([
        "lint",
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
    std::fs::write(&path, "x <- cbind(1:5, 6:10\n").expect("failed to write file");

    let output = run_cli([
        "lint",
        "--output=concise",
        dir.path().to_str().expect("temp dir path should be utf-8"),
    ]);

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    // The parser's side-channel message is surfaced, not just a blocked count.
    assert!(
        stderr.contains("bad.R:1:11: error [syntax-error]"),
        "got stderr: {stderr}"
    );
    assert!(
        stderr.contains("expected ')' to close function call"),
        "got stderr: {stderr}"
    );
}

#[test]
fn cli_lint_empty_stdin_is_clean() {
    // With no paths and empty stdin there is nothing to lint: exit 0.
    let output = run_cli(["lint"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cli_lint_reads_stdin_findings() {
    let output = run_cli_stdin(
        ["lint", "--stdin-filename", "buf.R", "--output=concise"],
        "x == NA\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("buf.R:1:1:") && stderr.contains("equals-na"),
        "got stderr: {stderr}"
    );
}

#[test]
fn cli_lint_stdin_fix_writes_to_stdout() {
    // `--fix` over stdin emits the fixed source to stdout (like `format`).
    let output = run_cli_stdin(["lint", "--fix"], "any(is.na(x))\n");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, "anyNA(x)\n");
}

#[test]
fn cli_lint_color_flag_controls_ansi() {
    let always = run_cli_stdin(["--color", "always", "lint"], "x == NA\n");
    assert!(
        String::from_utf8_lossy(&always.stderr).contains('\u{1b}'),
        "--color always should emit ANSI escapes"
    );
    let never = run_cli_stdin(["--color", "never", "lint"], "x == NA\n");
    assert!(
        !String::from_utf8_lossy(&never.stderr).contains('\u{1b}'),
        "--color never should emit no ANSI escapes"
    );
}

#[test]
fn cli_lint_emits_json_output() {
    let dir = tempdir().expect("failed to create temp dir");
    let path = dir.path().join("dup.R");
    std::fs::write(&path, "f <- function(x, x) x\nf(1, 2)\n").expect("failed to write file");

    let output = run_cli([
        "lint",
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

#[test]
fn undefined_symbol_standalone_attaches_testthat_for_test_files() {
    // Single-file (standalone) path: a `tests/testthat/` file has testthat
    // attached implicitly, so `expect_true` resolves against the indexed package
    // while a genuine typo (`bogus`) is still flagged.
    let p = CompositeProvider::with_index(IndexedProvider::from_indices([indexed_pkg(
        "testthat",
        &["expect_true"],
    )]));
    let diags = check_document_with_provider(
        Path::new("tests/testthat/test-x.R"),
        "expect_true(TRUE)\nbogus()\n",
        &LintConfig::default(),
        &p,
    )
    .expect("lint should succeed");
    let msgs: Vec<&str> = diags
        .iter()
        .filter(|d| d.rule == "undefined-symbol")
        .map(|d| d.message.body.as_str())
        .collect();
    assert_eq!(msgs.len(), 1, "expected only `bogus`, got {msgs:?}");
    assert!(msgs[0].contains("bogus"));
}

#[test]
fn undefined_symbol_skips_data_masked_columns() {
    // Inside dplyr's data-masking `mutate()`, a bare name like `a` is a column
    // reference evaluated in the data mask, not an undefined symbol. The rule
    // must not flag data-masked bare names.
    let p = CompositeProvider::with_index(IndexedProvider::from_indices([indexed_pkg(
        "dplyr",
        &["mutate", "tibble"],
    )]));
    let msgs = undefined_with("library(dplyr)\ntibble(a = 1) |> mutate(b = a + 1)\n", &p);
    assert!(
        msgs.is_empty(),
        "data-masked `a` must not be flagged, got {msgs:?}"
    );
}

#[test]
fn undefined_symbol_still_flags_outside_data_mask() {
    // Masking suppresses only the masked argument expressions: a genuine typo
    // elsewhere (and a typo'd verb name) is still flagged.
    let p = CompositeProvider::with_index(IndexedProvider::from_indices([indexed_pkg(
        "dplyr",
        &["mutate"],
    )]));
    let msgs = undefined_with("library(dplyr)\nbogus()\nmutate(df, b = a + 1)\n", &p);
    assert_eq!(msgs.len(), 1, "only `bogus`, got {msgs:?}");
    assert!(msgs[0].contains("bogus"));
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
    // formatter rewrites to `{}` — withholding here is the autofix-correctness
    // discipline (correct-by-construction or withhold). The finding is still
    // reported (exit 1) and the file stays format-clean.
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
// Phase 1 syntactic rules (built on §I1 matchers).
// ---------------------------------------------------------------------------

/// The fix carried by the single finding of `rule` in `src`, applied (safe-only).
fn fixed_output(src: &str, rule: &str) -> String {
    let d = diagnostics(src)
        .into_iter()
        .find(|d| d.rule == rule)
        .unwrap_or_else(|| panic!("expected a {rule} finding"));
    let fix = d.fix.as_ref().expect("finding should carry a fix");
    assert_eq!(fix.applicability, Applicability::Safe);
    apply_fixes(src, std::slice::from_ref(fix), false).output
}

#[test]
fn undefined_symbol_ignores_reserved_constants() {
    // Reserved literal constants are not symbol references; `undefined-symbol`
    // must never flag them.
    let src = "print(c(TRUE, FALSE, NA, NULL, Inf, NaN, NA_integer_))\n";
    let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
    assert!(!rules.contains(&"undefined-symbol"), "got: {rules:?}");
}

#[test]
fn lint_flags_duplicated_arguments() {
    let src = "f(a = 1, a = 2)\n";
    let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
    assert!(rules.contains(&"duplicated-arguments"), "got: {rules:?}");
}

#[test]
fn duplicated_arguments_ignores_distinct_and_positional() {
    let rules: Vec<&str> = diagnostics("f(a = 1, b = 2, 3, 4)\n")
        .iter()
        .map(|d| d.rule)
        .collect();
    assert!(!rules.contains(&"duplicated-arguments"), "got: {rules:?}");
}

#[test]
fn duplicated_arguments_ignores_c_call() {
    // `c()` takes `...`, so repeated names are legal and idiomatic (e.g. cli
    // message vectors: `c("i" = ..., "i" = ...)`). See the tidyr survey.
    let src = "cli::cli_abort(c(i = \"one\", i = \"two\"))\n";
    let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
    assert!(!rules.contains(&"duplicated-arguments"), "got: {rules:?}");
}

#[test]
fn duplicated_arguments_still_flags_list_call() {
    // `list()` duplicate names are almost always a bug, so keep flagging them.
    let rules: Vec<&str> = diagnostics("list(a = 1, a = 2)\n")
        .iter()
        .map(|d| d.rule)
        .collect();
    assert!(rules.contains(&"duplicated-arguments"), "got: {rules:?}");
}

#[test]
fn redundant_equals_true_drops_comparison() {
    assert_eq!(
        fixed_output("if (x == TRUE) f()\n", "redundant-equals"),
        "if (x) f()\n"
    );
    // Literal on either side.
    assert_eq!(
        fixed_output("print(TRUE == x)\n", "redundant-equals"),
        "print(x)\n"
    );
}

#[test]
fn redundant_equals_false_negates() {
    assert_eq!(
        fixed_output("print(x == FALSE)\n", "redundant-equals"),
        "print(!x)\n"
    );
}

#[test]
fn redundant_equals_withholds_fix_for_complex_operand() {
    // `a + b == FALSE` parses as `(a + b) == FALSE`; `!a + b` would misparse, so
    // the fix is withheld — but the finding is still reported.
    let d = diagnostics("print(a + b == FALSE)\n")
        .into_iter()
        .find(|d| d.rule == "redundant-equals")
        .expect("expected a redundant-equals finding");
    assert!(d.fix.is_none(), "complex operand should withhold the fix");
}

#[test]
fn equals_na_rewrites_to_is_na() {
    assert_eq!(
        fixed_output("if (x == NA) f()\n", "equals-na"),
        "if (is.na(x)) f()\n"
    );
    assert_eq!(
        fixed_output("print(NA == y)\n", "equals-na"),
        "print(is.na(y))\n"
    );
}

#[test]
fn redundant_ifelse_collapses() {
    assert_eq!(
        fixed_output("print(ifelse(cond, TRUE, FALSE))\n", "redundant-ifelse"),
        "print(cond)\n"
    );
    assert_eq!(
        fixed_output("print(ifelse(cond, FALSE, TRUE))\n", "redundant-ifelse"),
        "print(!cond)\n"
    );
}

#[test]
fn redundant_ifelse_ignores_non_constant_branches() {
    let rules: Vec<&str> = diagnostics("print(ifelse(cond, a, b))\n")
        .iter()
        .map(|d| d.rule)
        .collect();
    assert!(!rules.contains(&"redundant-ifelse"), "got: {rules:?}");
}

/// Apply every `rule` fix in `src` (safe-only) in one pass.
fn fixed_output_all(src: &str, rule: &str) -> String {
    let diags = diagnostics(src);
    let fixes: Vec<_> = diags
        .iter()
        .filter(|d| d.rule == rule)
        .map(|d| d.fix.clone().expect("finding should carry a fix"))
        .collect();
    assert!(!fixes.is_empty(), "expected at least one {rule} finding");
    apply_fixes(src, &fixes, false).output
}

#[test]
fn true_false_symbol_rewrites_value_positions() {
    assert_eq!(fixed_output("x <- T\n", "true-false-symbol"), "x <- TRUE\n");
    assert_eq!(
        fixed_output("if (F) 1\n", "true-false-symbol"),
        "if (FALSE) 1\n"
    );
}

#[test]
fn true_false_symbol_rewrites_multiple_sites() {
    assert_eq!(
        fixed_output_all("c(T, F, T)\n", "true-false-symbol"),
        "c(TRUE, FALSE, TRUE)\n"
    );
}

#[test]
fn true_false_symbol_ignores_name_positions() {
    // Named-arg names, `$` members, and list names are not value reads.
    for src in ["f(T = 1)\n", "df$T\n", "list(F = 1)\n"] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"true-false-symbol"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn true_false_symbol_ignores_reserved_literals() {
    for src in ["x <- TRUE\n", "c(NA, NULL, FALSE)\n"] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"true-false-symbol"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn true_false_symbol_skips_locally_rebound() {
    // A read that resolves to a same-file binding is the user's variable, not
    // the boolean shorthand — flagging it would be a false positive.
    for src in ["T <- FALSE\nif (T) 1\n", "function(T) T\n"] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"true-false-symbol"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn true_false_symbol_flags_reads_in_unbound_scope() {
    // No local `T` in scope, so the function-body read is the base symbol.
    assert_eq!(
        fixed_output("f <- function() T\n", "true-false-symbol"),
        "f <- function() TRUE\n"
    );
}

#[test]
fn repeat_rewrites_while_true() {
    assert_eq!(fixed_output("while (TRUE) f()\n", "repeat"), "repeat f()\n");
    assert_eq!(
        fixed_output("while (TRUE) {\n  f()\n}\n", "repeat"),
        "repeat {\n  f()\n}\n"
    );
}

#[test]
fn repeat_ignores_conditional_loops() {
    for src in [
        "while (cond) f()\n",
        "while (x > 0) f()\n",
        "while (T) f()\n",
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"repeat"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn repeat_withholds_fix_for_commented_condition() {
    // A comment inside the clause would be dropped by the rewrite, so the fix is
    // withheld — but the finding is still reported.
    let d = diagnostics("while ( # forever\n  TRUE\n) f()\n")
        .into_iter()
        .find(|d| d.rule == "repeat")
        .expect("expected a repeat finding");
    assert!(d.fix.is_none(), "commented clause should withhold the fix");
}

#[test]
fn vector_logic_rewrites_scalar_operators() {
    assert_eq!(
        fixed_output("if (a & b) f()\n", "vector-logic"),
        "if (a && b) f()\n"
    );
    assert_eq!(
        fixed_output("while (a | b) f()\n", "vector-logic"),
        "while (a || b) f()\n"
    );
}

#[test]
fn vector_logic_reaches_through_logical_scaffolding() {
    // `&`/`|` reachable through `&&`/`||`, `!`, and parens are all flagged; the
    // outer scalar operators are left alone.
    assert_eq!(
        fixed_output_all("if (x && (a | b)) f()\n", "vector-logic"),
        "if (x && (a || b)) f()\n"
    );
    assert_eq!(
        fixed_output_all("if (!(a & b)) f()\n", "vector-logic"),
        "if (!(a && b)) f()\n"
    );
    // Two operators in one condition → two fixes.
    assert_eq!(
        fixed_output_all("if (a & b & c) f()\n", "vector-logic"),
        "if (a && b && c) f()\n"
    );
}

#[test]
fn vector_logic_ignores_function_call_context() {
    // Inside a call the value is no longer a scalar condition, so vector logic
    // is appropriate — don't flag it.
    for src in [
        "if (any(a | b)) f()\n",
        "if (all(a & b)) f()\n",
        "x <- a & b\n",
        "if (a && b) f()\n",
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"vector-logic"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn comparison_negation_flips_operator() {
    assert_eq!(
        fixed_output("!(a == b)\n", "comparison-negation"),
        "a != b\n"
    );
    assert_eq!(
        fixed_output("if (!(x < y)) f()\n", "comparison-negation"),
        "if (x >= y) f()\n"
    );
    // Every comparison operator has a negation.
    assert_eq!(
        fixed_output("!(a != b)\n", "comparison-negation"),
        "a == b\n"
    );
    assert_eq!(
        fixed_output("!(a <= b)\n", "comparison-negation"),
        "a > b\n"
    );
    assert_eq!(
        fixed_output("!(a > b)\n", "comparison-negation"),
        "a <= b\n"
    );
    assert_eq!(
        fixed_output("!(a >= b)\n", "comparison-negation"),
        "a < b\n"
    );
}

#[test]
fn comparison_negation_flips_unparenthesized_form() {
    // R binds `!` looser than the comparison operators, so `!a == b` already
    // means `!(a == b)` — flag it too.
    assert_eq!(fixed_output("!a == b\n", "comparison-negation"), "a != b\n");
    assert_eq!(
        fixed_output("x <- !a < b\n", "comparison-negation"),
        "x <- a >= b\n"
    );
}

#[test]
fn comparison_negation_ignores_non_comparison() {
    // `!` of a non-comparison (logical, arithmetic, a bare paren) is not this
    // rule's business.
    for src in ["!(a & b)\n", "!(a + b)\n", "!(x)\n", "!x\n"] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"comparison-negation"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn comparison_negation_withholds_fix_for_commented_clause() {
    // A comment inside the parens would be dropped by the rewrite, so the fix is
    // withheld — but the finding is still reported.
    let d = diagnostics("!(a == b # note\n)\n")
        .into_iter()
        .find(|d| d.rule == "comparison-negation")
        .expect("expected a comparison-negation finding");
    assert!(d.fix.is_none(), "commented clause should withhold the fix");
}

#[test]
fn outer_negation_pulls_negation_out() {
    assert_eq!(
        fixed_output("if (any(!x)) f()\n", "outer-negation"),
        "if (!all(x)) f()\n"
    );
    assert_eq!(
        fixed_output("flag <- all(!x)\n", "outer-negation"),
        "flag <- !any(x)\n"
    );
    // Multiple negated args: every one loses its `!`.
    assert_eq!(
        fixed_output("z <- any(!a, !b)\n", "outer-negation"),
        "z <- !all(a, b)\n"
    );
    // `na.rm` is passed through untouched.
    assert_eq!(
        fixed_output("flag <- any(!x, na.rm = TRUE)\n", "outer-negation"),
        "flag <- !all(x, na.rm = TRUE)\n"
    );
}

#[test]
fn outer_negation_ignores_unnegated_and_mixed() {
    for src in [
        "if (any(x)) f()\n",      // nothing negated
        "z <- any(!a, b)\n",      // mixed: not a clean De Morgan
        "z <- any(x, !y)\n",      // mixed
        "z <- sum(!x)\n",         // not any/all
        "z <- any(other = !x)\n", // a non-`na.rm` named arg
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"outer-negation"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn outer_negation_withholds_fix_in_tight_context() {
    // `any(!x)` is a primary; `!all(x)` binds looser, so a parent that binds
    // tighter than `!` (here `==`) would misparse. Withhold the fix, still report.
    let d = diagnostics("z <- any(!x) == y\n")
        .into_iter()
        .find(|d| d.rule == "outer-negation")
        .expect("expected an outer-negation finding");
    assert!(
        d.fix.is_none(),
        "tight parent context should withhold the fix"
    );
}

// ---------------------------------------------------------------------------
// Phase 2 call-rewrite idioms (namespace-confirmed).
// ---------------------------------------------------------------------------

#[test]
fn any_is_na_rewrites_to_anyna() {
    assert_eq!(
        fixed_output("if (any(is.na(x))) f()\n", "any-is-na"),
        "if (anyNA(x)) f()\n"
    );
    // The inner argument is preserved verbatim, whatever its shape.
    assert_eq!(
        fixed_output("flag <- any(is.na(df$col))\n", "any-is-na"),
        "flag <- anyNA(df$col)\n"
    );
}

#[test]
fn any_is_na_ignores_other_shapes() {
    for src in [
        "anyNA(x)\n",                    // already the idiom
        "any(x)\n",                      // not is.na
        "is.na(x)\n",                    // is.na without any
        "any(is.na(x), na.rm = TRUE)\n", // extra arg — not the clean shape
        "any(is.na(x), y)\n",            // extra positional arg
        "any(is.na(x) | other)\n",       // arg is a binary expr, not is.na()
        "any(is.na(x, y))\n",            // is.na with two args
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"any-is-na"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn any_is_na_skips_shadowed_callees() {
    // A user redefinition of either callee means the call no longer invokes base
    // R, so the rewrite would be wrong — don't flag.
    for src in [
        "any <- function(...) TRUE\nany(is.na(x))\n",
        "is.na <- function(x) x\nany(is.na(x))\n",
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"any-is-na"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn any_is_na_withholds_fix_for_dropped_comment() {
    // A comment outside the preserved inner argument would be dropped by the
    // rewrite, so the fix is withheld — but the finding is still reported.
    let d = diagnostics("any(is.na(x) # note\n)\n")
        .into_iter()
        .find(|d| d.rule == "any-is-na")
        .expect("expected an any-is-na finding");
    assert!(d.fix.is_none(), "dropped comment should withhold the fix");
}

#[test]
fn any_duplicated_rewrites_to_anyduplicated() {
    assert_eq!(
        fixed_output("if (any(duplicated(x))) f()\n", "any-duplicated"),
        "if (anyDuplicated(x) > 0) f()\n"
    );
    // The inner argument is preserved verbatim, whatever its shape.
    assert_eq!(
        fixed_output("flag <- any(duplicated(df$col))\n", "any-duplicated"),
        "flag <- anyDuplicated(df$col) > 0\n"
    );
}

#[test]
fn any_duplicated_ignores_other_shapes() {
    for src in [
        "anyDuplicated(x)\n",                 // already the idiom
        "any(x)\n",                           // not duplicated
        "duplicated(x)\n",                    // duplicated without any
        "any(duplicated(x), na.rm = TRUE)\n", // extra arg — not the clean shape
        "any(duplicated(x), y)\n",            // extra positional arg
        "any(duplicated(x) | other)\n",       // arg is a binary expr, not duplicated()
        "any(duplicated(x, y))\n",            // duplicated with two args
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"any-duplicated"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn any_duplicated_skips_shadowed_callees() {
    // A user redefinition of either callee means the call no longer invokes base
    // R, so the rewrite would be wrong — don't flag.
    for src in [
        "any <- function(...) TRUE\nany(duplicated(x))\n",
        "duplicated <- function(x) x\nany(duplicated(x))\n",
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"any-duplicated"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn any_duplicated_withholds_fix_for_dropped_comment() {
    // A comment outside the preserved inner argument would be dropped by the
    // rewrite, so the fix is withheld — but the finding is still reported.
    let d = diagnostics("any(duplicated(x) # note\n)\n")
        .into_iter()
        .find(|d| d.rule == "any-duplicated")
        .expect("expected an any-duplicated finding");
    assert!(d.fix.is_none(), "dropped comment should withhold the fix");
}

#[test]
fn any_duplicated_withholds_fix_in_tight_context() {
    // The replacement `anyDuplicated(x) > 0` is a comparison, which binds looser
    // than arithmetic/indexing. In a parent that binds tighter than a comparison
    // the bare rewrite would misparse, so the fix is withheld there — but the
    // finding is still reported.
    for src in [
        "any(duplicated(x)) + 1\n", // arithmetic operand
        "-any(duplicated(x))\n",    // unary minus
        "any(duplicated(x))[1]\n",  // subset
    ] {
        let d = diagnostics(src)
            .into_iter()
            .find(|d| d.rule == "any-duplicated")
            .unwrap_or_else(|| panic!("expected an any-duplicated finding for {src:?}"));
        assert!(
            d.fix.is_none(),
            "{src:?}: tight context should withhold the fix"
        );
    }
}

#[test]
fn crossprod_rewrites_transposed_matmul() {
    // `t(x) %*% y` is `crossprod(x, y)`; `x %*% t(y)` is `tcrossprod(x, y)`.
    assert_eq!(
        fixed_output("t(x) %*% y\n", "crossprod"),
        "crossprod(x, y)\n"
    );
    assert_eq!(
        fixed_output("x %*% t(y)\n", "crossprod"),
        "tcrossprod(x, y)\n"
    );
    // The operands are preserved verbatim, whatever their shape.
    assert_eq!(
        fixed_output("out <- t(a$m) %*% b[[1]]\n", "crossprod"),
        "out <- crossprod(a$m, b[[1]])\n"
    );
}

#[test]
fn crossprod_collapses_same_symbol() {
    // When both operands are the same simple symbol, the single-argument form is
    // equivalent and more idiomatic.
    assert_eq!(fixed_output("t(x) %*% x\n", "crossprod"), "crossprod(x)\n");
    assert_eq!(fixed_output("x %*% t(x)\n", "crossprod"), "tcrossprod(x)\n");
}

#[test]
fn crossprod_fixes_inner_of_chain() {
    // `%*%` is left-associative, so `t(a) %*% b %*% c` parses as
    // `(t(a) %*% b) %*% c`; the rule fires on the inner expr and the call
    // replacement preserves associativity.
    assert_eq!(
        fixed_output("t(a) %*% b %*% c\n", "crossprod"),
        "crossprod(a, b) %*% c\n"
    );
}

#[test]
fn crossprod_both_transposed_prefers_crossprod() {
    // With `t()` on both sides the crossprod branch wins; the surviving `t()` is
    // left in the second argument (still correct, a partial win).
    assert_eq!(
        fixed_output("t(x) %*% t(y)\n", "crossprod"),
        "crossprod(x, t(y))\n"
    );
}

#[test]
fn crossprod_ignores_other_shapes() {
    for src in [
        "x %*% y\n",         // no transpose
        "crossprod(x, y)\n", // already the idiom
        "t(x) * y\n",        // elementwise, not matrix multiply
        "t(x, y) %*% z\n",   // t with two args — not the clean shape
        "x %o% t(y)\n",      // a different special operator
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"crossprod"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn crossprod_skips_shadowed_t() {
    // A user redefinition of `t` means the call no longer invokes base R's
    // transpose, so the rewrite would be wrong — don't flag.
    let src = "t <- function(z) z\nt(x) %*% y\n";
    let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
    assert!(
        !rules.contains(&"crossprod"),
        "{src:?} should not flag, got: {rules:?}"
    );
}

#[test]
fn crossprod_withholds_fix_for_dropped_comment() {
    // A comment inside the expression but outside the preserved operands would be
    // dropped by the rewrite, so the fix is withheld — the finding still reports.
    let d = diagnostics("t(x) %*% # note\n  y\n")
        .into_iter()
        .find(|d| d.rule == "crossprod")
        .expect("expected a crossprod finding");
    assert!(d.fix.is_none(), "dropped comment should withhold the fix");
}

#[test]
fn lengths_rewrites_sapply() {
    assert_eq!(
        fixed_output("n <- sapply(x, length)\n", "lengths"),
        "n <- lengths(x)\n"
    );
    // The first argument is preserved verbatim, whatever its shape.
    assert_eq!(
        fixed_output("n <- sapply(df$col, length)\n", "lengths"),
        "n <- lengths(df$col)\n"
    );
}

#[test]
fn lengths_ignores_other_shapes() {
    for src in [
        "lengths(x)\n",                           // already the idiom
        "sapply(x, sum)\n",                       // not length
        "sapply(x)\n",                            // no FUN argument
        "sapply(x, length, USE.NAMES = FALSE)\n", // extra arg — not the clean shape
        "sapply(x, length, y)\n",                 // extra positional arg
        "sapply(x, length(y))\n",                 // FUN is a call, not the bare name
        "sapply(x, \"length\")\n",                // string FUN — match.fun form
        "sapply(x, FUN = length)\n",              // named FUN — not the clean shape
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"lengths"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn lengths_skips_shadowed_names() {
    // A user redefinition of `sapply` or `length` means the call no longer
    // computes per-element base lengths, so the rewrite would be wrong.
    for src in [
        "sapply <- function(...) 1\nsapply(x, length)\n",
        "length <- function(x) 42\nsapply(x, length)\n",
        "f <- function() {\n  length <- function(x) 42\n  sapply(x, length)\n}\n",
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"lengths"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn lengths_withholds_fix_for_dropped_comment() {
    // A comment outside the preserved first argument would be dropped by the
    // rewrite, so the fix is withheld — but the finding is still reported.
    let d = diagnostics("sapply(x, # note\n  length)\n")
        .into_iter()
        .find(|d| d.rule == "lengths")
        .expect("expected a lengths finding");
    assert!(d.fix.is_none(), "dropped comment should withhold the fix");
}

/// The (unsafe) fix carried by the single finding of `rule` in `src`, applied.
fn unsafe_fixed_output(src: &str, rule: &str) -> String {
    let d = diagnostics(src)
        .into_iter()
        .find(|d| d.rule == rule)
        .unwrap_or_else(|| panic!("expected a {rule} finding"));
    let fix = d.fix.as_ref().expect("finding should carry a fix");
    assert_eq!(fix.applicability, Applicability::Unsafe);
    apply_fixes(src, std::slice::from_ref(fix), true).output
}

#[test]
fn string_boundary_rewrites_anchored_grepl() {
    // A leading `^` is a prefix test; a trailing `$` is a suffix test. The
    // subject moves to the first argument and the anchor is stripped from the
    // pattern (quote character preserved).
    assert_eq!(
        unsafe_fixed_output("grepl(\"^abc\", x)\n", "string-boundary"),
        "startsWith(x, \"abc\")\n"
    );
    assert_eq!(
        unsafe_fixed_output("grepl(\"xyz$\", y)\n", "string-boundary"),
        "endsWith(y, \"xyz\")\n"
    );
    assert_eq!(
        unsafe_fixed_output("grepl('a$', df$col)\n", "string-boundary"),
        "endsWith(df$col, 'a')\n"
    );
}

#[test]
fn string_boundary_fix_is_unsafe() {
    // `startsWith`/`endsWith` diverge from `grepl` on `NA`/non-character input,
    // so the fix must be unsafe (never applied on a plain `--fix`).
    let d = diagnostics("grepl(\"^abc\", x)\n")
        .into_iter()
        .find(|d| d.rule == "string-boundary")
        .expect("expected a string-boundary finding");
    assert_eq!(
        d.fix.as_ref().expect("should carry a fix").applicability,
        Applicability::Unsafe
    );
}

#[test]
fn string_boundary_ignores_non_boundary_shapes() {
    for src in [
        "grepl(\"^a.b\", x)\n",               // metacharacter after the anchor
        "grepl(\"a|b$\", x)\n",               // alternation is a real regex
        "grepl(\"^abc$\", x)\n",              // both ends anchored — an exact match
        "grepl(\"abc\", x)\n",                // no anchor at all
        "grepl(\"^\", x)\n",                  // anchor only, empty literal
        "grepl(\"^abc\", x, fixed = TRUE)\n", // extra (named) argument
        "grepl(\"^abc\", x, ignore.case = TRUE)\n",
        "grepl(pattern = \"^abc\", x)\n", // named pattern — not the clean shape
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"string-boundary"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn string_boundary_skips_shadowed_grepl() {
    // A user redefinition of `grepl` means the call no longer invokes base R, so
    // the rewrite would be wrong — don't flag.
    let src = "grepl <- function(p, x) TRUE\ngrepl(\"^abc\", x)\n";
    let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
    assert!(
        !rules.contains(&"string-boundary"),
        "{src:?} should not flag, got: {rules:?}"
    );
}

#[test]
fn string_boundary_withholds_fix_for_dropped_comment() {
    // A comment outside the preserved subject/pattern would be dropped, so the
    // fix is withheld — the finding still reports.
    let d = diagnostics("grepl(\"^abc\", # note\n  x)\n")
        .into_iter()
        .find(|d| d.rule == "string-boundary")
        .expect("expected a string-boundary finding");
    assert!(d.fix.is_none(), "dropped comment should withhold the fix");
}

#[test]
fn fixed_regex_adds_fixed_true_for_literal_pattern() {
    // A metacharacter-free pattern matches identically with `fixed = TRUE`, which
    // the fix inserts after the last argument.
    assert_eq!(
        fixed_output("grepl(\"abc\", x)\n", "fixed-regex"),
        "grepl(\"abc\", x, fixed = TRUE)\n"
    );
    assert_eq!(
        fixed_output("gsub(\"lit\", \"R\", s)\n", "fixed-regex"),
        "gsub(\"lit\", \"R\", s, fixed = TRUE)\n"
    );
}

#[test]
fn fixed_regex_ignores_metacharacter_patterns() {
    for src in [
        "grepl(\"a.b\", x)\n",     // `.` is a metacharacter
        "grepl(\"^abc\", x)\n",    // anchored — a real regex (and string-boundary's)
        "grepl(\"a\\\\.b\", x)\n", // an escaped literal dot — has a backslash
        "grepl(\"\", x)\n",        // empty pattern
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"fixed-regex"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn fixed_regex_skips_when_mode_flag_present() {
    // `fixed`/`ignore.case`/`perl` already govern matching mode; adding
    // `fixed = TRUE` would be redundant or contradictory.
    for src in [
        "grepl(\"abc\", x, fixed = TRUE)\n",
        "grepl(\"abc\", x, fixed = FALSE)\n",
        "grepl(\"abc\", x, ignore.case = TRUE)\n",
        "grepl(\"abc\", x, perl = TRUE)\n",
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"fixed-regex"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn fixed_regex_skips_shadowed_callee() {
    let src = "grepl <- function(p, x) TRUE\ngrepl(\"abc\", x)\n";
    let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
    assert!(
        !rules.contains(&"fixed-regex"),
        "{src:?} should not flag, got: {rules:?}"
    );
}

#[test]
fn nzchar_rewrites_nonempty_comparisons() {
    // Every "length is nonzero" spelling collapses to `nzchar(x)`, the mirrored
    // literal-first forms included.
    for src in [
        "flag <- nchar(x) > 0\n",
        "flag <- nchar(x) >= 1\n",
        "flag <- nchar(x) != 0\n",
        "flag <- 0 < nchar(x)\n",
        "flag <- 1 <= nchar(x)\n",
        "flag <- 0 != nchar(x)\n",
    ] {
        assert_eq!(
            unsafe_fixed_output(src, "nzchar"),
            "flag <- nzchar(x)\n",
            "from {src:?}"
        );
    }
}

#[test]
fn nzchar_rewrites_empty_comparisons_negated() {
    // The "length is zero" spellings negate: `!nzchar(x)`.
    for src in [
        "flag <- nchar(x) == 0\n",
        "flag <- nchar(x) <= 0\n",
        "flag <- nchar(x) < 1\n",
        "flag <- 0 == nchar(x)\n",
        "flag <- 0 >= nchar(x)\n",
        "flag <- 1 > nchar(x)\n",
    ] {
        assert_eq!(
            unsafe_fixed_output(src, "nzchar"),
            "flag <- !nzchar(x)\n",
            "from {src:?}"
        );
    }
}

#[test]
fn nzchar_fix_is_unsafe() {
    // `nzchar` diverges from the `nchar` comparison on `NA_character_` input
    // (`TRUE` vs `NA` under the default `keepNA`), so the fix must be unsafe.
    let d = diagnostics("flag <- nchar(x) > 0\n")
        .into_iter()
        .find(|d| d.rule == "nzchar")
        .expect("expected an nzchar finding");
    assert_eq!(
        d.fix.as_ref().expect("should carry a fix").applicability,
        Applicability::Unsafe
    );
}

#[test]
fn nzchar_ignores_non_emptiness_shapes() {
    for src in [
        "nchar(x) > 1\n", // a real length threshold, not an emptiness test
        "nchar(x) == 2\n",
        "nchar(x) >= 0\n",                  // vacuously true — not the shape
        "nchar(x, type = \"bytes\") > 0\n", // extra argument changes semantics
        "nchar(x, keepNA = TRUE) == 0\n",
        "nchar(x) + 0\n", // not a comparison
        "f(x) > 0\n",     // different callee
        "nchar(x) > y\n", // non-literal bound
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"nzchar"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn nzchar_skips_shadowed_nchar() {
    // A user redefinition of `nchar` means the comparison no longer tests string
    // length, so the rewrite would be wrong — don't flag.
    let src = "nchar <- function(x) 1\nnchar(x) > 0\n";
    let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
    assert!(
        !rules.contains(&"nzchar"),
        "{src:?} should not flag, got: {rules:?}"
    );
}

#[test]
fn nzchar_withholds_negating_fix_in_tight_context() {
    // The negated rewrite `!nzchar(x)` binds looser than the comparison it
    // replaces; chained into another comparison it would misparse
    // (`!nzchar(x) == y` is `!(nzchar(x) == y)`), so the fix is withheld — the
    // finding still reports.
    let d = diagnostics("z <- nchar(x) == 0 == y\n")
        .into_iter()
        .find(|d| d.rule == "nzchar")
        .expect("expected an nzchar finding");
    assert!(d.fix.is_none(), "tight context should withhold the fix");
}

#[test]
fn nzchar_withholds_fix_for_dropped_comment() {
    // A comment outside the preserved argument would be dropped by the rewrite,
    // so the fix is withheld — the finding is still reported.
    let d = diagnostics("flag <- nchar(x) > # note\n  0\n")
        .into_iter()
        .find(|d| d.rule == "nzchar")
        .expect("expected an nzchar finding");
    assert!(d.fix.is_none(), "dropped comment should withhold the fix");
}

#[test]
fn seq_rewrites_colon_length_to_seq_along() {
    assert_eq!(
        fixed_output("for (i in 1:length(x)) print(i)\n", "seq"),
        "for (i in seq_along(x)) print(i)\n"
    );
    // The argument is preserved verbatim, whatever its shape; `1L` counts too.
    assert_eq!(
        fixed_output("idx <- 1:length(df$col)\n", "seq"),
        "idx <- seq_along(df$col)\n"
    );
    assert_eq!(
        fixed_output("idx <- 1L:length(x)\n", "seq"),
        "idx <- seq_along(x)\n"
    );
}

#[test]
fn seq_rewrites_colon_ident_to_seq_len() {
    assert_eq!(
        fixed_output("for (i in 1:n) f(i)\n", "seq"),
        "for (i in seq_len(n)) f(i)\n"
    );
}

#[test]
fn seq_rewrites_dim_calls_to_seq_len() {
    // `nrow`/`ncol`/`NROW`/`NCOL` share `length`'s zero hazard; the call is
    // preserved whole inside `seq_len(...)`.
    assert_eq!(
        fixed_output("for (i in 1:nrow(df)) f(i)\n", "seq"),
        "for (i in seq_len(nrow(df))) f(i)\n"
    );
    assert_eq!(
        fixed_output("idx <- 1:NCOL(m)\n", "seq"),
        "idx <- seq_len(NCOL(m))\n"
    );
}

#[test]
fn seq_ignores_other_shapes() {
    for src in [
        "1:10\n",           // literal range — well-defined and clear
        "2:n\n",            // does not start at 1
        "n:1\n",            // descending on purpose
        "-1:n\n",           // parses as `(-1):n` — not a from-1 range
        "1:(n)\n",          // parenthesized RHS — not the bare shape
        "1:n^2\n",          // RHS is an expression, not a bare name
        "1:NA\n",           // special constant, not a length variable
        "1:foo(x)\n",       // arbitrary call — nothing says it is a length
        "1:length(x, y)\n", // not the sole-positional-argument shape
        "seq_along(x)\n",   // already the idiom
        "x[2:length(x)]\n", // tail slice — `2:` is not the hazard shape
        "pkg::foo(1:3)\n",  // literal again, inside an argument
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"seq"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn seq_skips_shadowed_length() {
    // A user redefinition of `length` (or `nrow`, ...) means the range bound is
    // no longer base R's length, so the rewrite would be wrong.
    for src in [
        "length <- function(x) 42\nfor (i in 1:length(x)) f(i)\n",
        "nrow <- function(x) 2\nidx <- 1:nrow(df)\n",
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"seq"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn seq_withholds_fix_for_dropped_comment() {
    // A comment outside the preserved operand would be dropped by the rewrite,
    // so the fix is withheld — the finding is still reported.
    let d = diagnostics("idx <- 1: # note\n  length(x)\n")
        .into_iter()
        .find(|d| d.rule == "seq")
        .expect("expected a seq finding");
    assert!(d.fix.is_none(), "dropped comment should withhold the fix");
}

#[test]
fn sort_rewrites_first_element_to_min() {
    // Taking the first element of an ascending sort is `min(x)`; the `1L`
    // spelling and an explicit `decreasing = FALSE` count too.
    for src in [
        "m <- sort(x)[1]\n",
        "m <- sort(x)[1L]\n",
        "m <- sort(x, decreasing = FALSE)[1]\n",
    ] {
        assert_eq!(
            unsafe_fixed_output(src, "sort"),
            "m <- min(x)\n",
            "from {src:?}"
        );
    }
    // The argument is preserved verbatim, whatever its shape.
    assert_eq!(
        unsafe_fixed_output("m <- sort(df$col)[1]\n", "sort"),
        "m <- min(df$col)\n"
    );
}

#[test]
fn sort_rewrites_decreasing_first_element_to_max() {
    assert_eq!(
        unsafe_fixed_output("m <- sort(x, decreasing = TRUE)[1]\n", "sort"),
        "m <- max(x)\n"
    );
}

#[test]
fn sort_fix_is_unsafe() {
    // `sort` drops `NA`s by default while `min`/`max` propagate them, and on an
    // empty vector `sort(x)[1]` is `NA` while `min(x)` warns and yields `Inf`,
    // so the fix must be unsafe.
    let d = diagnostics("m <- sort(x)[1]\n")
        .into_iter()
        .find(|d| d.rule == "sort")
        .expect("expected a sort finding");
    assert_eq!(
        d.fix.as_ref().expect("should carry a fix").applicability,
        Applicability::Unsafe
    );
}

#[test]
fn sort_ignores_other_shapes() {
    for src in [
        "sort(x)[2]\n",                 // not the first element
        "sort(x)[i]\n",                 // computed subscript
        "sort(x)[-1]\n",                // negative subscript
        "sort(x)[1, 2]\n",              // multiple subscripts
        "sort(x, na.last = TRUE)[1]\n", // extra argument changes semantics
        "sort(x, TRUE)[1]\n",           // positional decreasing — unclear
        "sort(x, decreasing = d)[1]\n", // non-literal decreasing
        "sort(x)\n",                    // no subscript at all
        "y[1]\n",                       // not a sort call
        "sort(x)[[1]]\n",               // `[[` — out of scope
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"sort"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn sort_skips_shadowed_sort() {
    // A user redefinition of `sort` means the subset is no longer a minimum at
    // all, so the rewrite would be wrong — don't flag.
    let src = "sort <- function(x) x\nm <- sort(x)[1]\n";
    let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
    assert!(
        !rules.contains(&"sort"),
        "{src:?} should not flag, got: {rules:?}"
    );
}

#[test]
fn sort_withholds_fix_for_dropped_comment() {
    // A comment outside the preserved argument would be dropped by the rewrite,
    // so the fix is withheld — the finding is still reported.
    let d = diagnostics("m <- sort( # note\n  x\n)[1]\n")
        .into_iter()
        .find(|d| d.rule == "sort")
        .expect("expected a sort finding");
    assert!(d.fix.is_none(), "dropped comment should withhold the fix");
}

#[test]
fn is_numeric_collapses_redundant_or() {
    // `is.numeric()` is already `TRUE` for integer vectors, so `||`-ing it with
    // `is.integer()` is redundant; either operand order collapses.
    assert_eq!(
        fixed_output("if (is.numeric(x) || is.integer(x)) f()\n", "is-numeric"),
        "if (is.numeric(x)) f()\n"
    );
    assert_eq!(
        fixed_output("if (is.integer(x) || is.numeric(x)) f()\n", "is-numeric"),
        "if (is.numeric(x)) f()\n"
    );
    // The vectorized `|` spelling counts too, and the argument is preserved
    // verbatim, whatever its shape.
    assert_eq!(
        fixed_output(
            "flag <- is.numeric(df$col) | is.integer(df$col)\n",
            "is-numeric"
        ),
        "flag <- is.numeric(df$col)\n"
    );
}

#[test]
fn is_numeric_ignores_other_shapes() {
    for src in [
        "is.numeric(x) || is.integer(y)\n",   // different arguments
        "is.numeric(x) && is.integer(x)\n",   // conjunction is not redundant
        "is.numeric(x) || is.character(x)\n", // a genuine two-type test
        "is.numeric(x)\n",                    // already the idiom
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"is-numeric"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn is_numeric_skips_shadowed_callee() {
    // A user redefinition of either callee means the disjunction is no longer
    // the base-R type test, so the rewrite would be wrong.
    for src in [
        "is.numeric <- function(x) FALSE\nis.numeric(x) || is.integer(x)\n",
        "is.integer <- function(x) TRUE\nis.numeric(x) || is.integer(x)\n",
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"is-numeric"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn is_numeric_withholds_fix_for_dropped_comment() {
    // A comment outside the preserved argument would be dropped by the rewrite,
    // so the fix is withheld — the finding is still reported.
    let d = diagnostics("flag <- is.numeric(x) || # note\n  is.integer(x)\n")
        .into_iter()
        .find(|d| d.rule == "is-numeric")
        .expect("expected an is-numeric finding");
    assert!(d.fix.is_none(), "dropped comment should withhold the fix");
}

#[test]
fn class_equals_rewrites_to_inherits() {
    // `class()` returns a vector, so `==` compares elementwise; `inherits()`
    // asks the intended question directly. Literal on either side.
    assert_eq!(
        unsafe_fixed_output("if (class(x) == \"factor\") f()\n", "class-equals"),
        "if (inherits(x, \"factor\")) f()\n"
    );
    assert_eq!(
        unsafe_fixed_output("if (\"factor\" == class(x)) f()\n", "class-equals"),
        "if (inherits(x, \"factor\")) f()\n"
    );
    // The original string token is preserved verbatim, quotes and all.
    assert_eq!(
        unsafe_fixed_output("flag <- class(df$col) == 'Date'\n", "class-equals"),
        "flag <- inherits(df$col, 'Date')\n"
    );
}

#[test]
fn class_equals_rewrites_not_equal_negated() {
    assert_eq!(
        unsafe_fixed_output("if (class(x) != \"factor\") f()\n", "class-equals"),
        "if (!inherits(x, \"factor\")) f()\n"
    );
}

#[test]
fn class_equals_rewrites_in_operator() {
    // `"cls" %in% class(x)` (and the mirrored membership) is the same question.
    assert_eq!(
        unsafe_fixed_output("if (\"factor\" %in% class(x)) f()\n", "class-equals"),
        "if (inherits(x, \"factor\")) f()\n"
    );
    assert_eq!(
        unsafe_fixed_output("if (class(x) %in% \"factor\") f()\n", "class-equals"),
        "if (inherits(x, \"factor\")) f()\n"
    );
}

#[test]
fn class_equals_fix_is_unsafe() {
    // `class()` returns a vector: on a multi-class object the comparison is
    // elementwise while `inherits()` is a scalar membership test, so the
    // rewrite can change behavior — the fix needs the `--unsafe-fixes` opt-in.
    let d = diagnostics("flag <- class(x) == \"factor\"\n")
        .into_iter()
        .find(|d| d.rule == "class-equals")
        .expect("expected a class-equals finding");
    let fix = d.fix.as_ref().expect("should carry a fix");
    assert_eq!(fix.applicability, Applicability::Unsafe);
}

#[test]
fn class_equals_ignores_other_shapes() {
    for src in [
        "class(x) == y\n",               // not a string literal
        "class(x) == c(\"a\", \"b\")\n", // vector comparand — deliberate
        "class(x) <- \"foo\"\n",         // assignment, not comparison
        "inherits(x, \"factor\")\n",     // already the idiom
        "typeof(x) == \"integer\"\n",    // different function
        "class(x, y) == \"foo\"\n",      // not the sole-positional shape
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"class-equals"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn class_equals_skips_shadowed_class() {
    // A user redefinition of `class` means the comparison no longer inspects
    // the class attribute, so the rewrite would be wrong.
    let src = "class <- function(x) \"a\"\nclass(x) == \"a\"\n";
    let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
    assert!(
        !rules.contains(&"class-equals"),
        "{src:?} should not flag, got: {rules:?}"
    );
}

#[test]
fn class_equals_withholds_negating_fix_in_tight_context() {
    // The negated rewrite `!inherits(...)` binds looser than the comparison it
    // replaces; chained into another comparison it would misparse, so the fix
    // is withheld — the finding still reports.
    let d = diagnostics("z <- class(x) != \"a\" == y\n")
        .into_iter()
        .find(|d| d.rule == "class-equals")
        .expect("expected a class-equals finding");
    assert!(d.fix.is_none(), "tight context should withhold the fix");
}

#[test]
fn class_equals_withholds_fix_for_dropped_comment() {
    // A comment outside the preserved argument and string would be dropped by
    // the rewrite, so the fix is withheld — the finding is still reported.
    let d = diagnostics("flag <- class(x) == # note\n  \"factor\"\n")
        .into_iter()
        .find(|d| d.rule == "class-equals")
        .expect("expected a class-equals finding");
    assert!(d.fix.is_none(), "dropped comment should withhold the fix");
}

#[test]
fn unreachable_code_flags_after_return_and_stop() {
    // A statement after an unconditional `return()` in a function body can never
    // run; the finding spans it and the (unsafe) fix deletes it.
    let src = "f <- function() {\n  return(1)\n  2\n}\n";
    let d = diagnostics(src)
        .into_iter()
        .find(|d| d.rule == "unreachable-code")
        .expect("expected an unreachable-code finding");
    let fix = d.fix.as_ref().expect("should carry a fix");
    assert_eq!(fix.applicability, Applicability::Unsafe);
    assert_eq!(
        apply_fixes(src, std::slice::from_ref(fix), true).output,
        "f <- function() {\n  return(1)\n}\n"
    );

    // `stop()` halts anywhere, so it works in a bare block too.
    let src = "{\n  stop()\n  f()\n}\n";
    let fix = diagnostics(src)
        .into_iter()
        .find(|d| d.rule == "unreachable-code")
        .and_then(|d| d.fix)
        .expect("expected an unreachable-code fix");
    assert_eq!(
        apply_fixes(src, std::slice::from_ref(&fix), true).output,
        "{\n  stop()\n}\n"
    );
}

#[test]
fn unreachable_code_covers_all_trailing_statements() {
    // Every statement after the terminator is unreachable; one finding spans them
    // all and the fix removes the lot.
    let src = "f <- function() {\n  return(1)\n  a()\n  b()\n}\n";
    let fix = diagnostics(src)
        .into_iter()
        .find(|d| d.rule == "unreachable-code")
        .and_then(|d| d.fix)
        .expect("expected an unreachable-code fix");
    assert_eq!(
        apply_fixes(src, std::slice::from_ref(&fix), true).output,
        "f <- function() {\n  return(1)\n}\n"
    );
}

#[test]
fn unreachable_code_flags_both_branches_return() {
    // An `if`/`else` that exits in both branches leaves the tail unreachable (a
    // CFG verdict); the finding spans it and the unsafe fix deletes it.
    let src = "f <- function() {\n  if (x) return(1) else return(2)\n  3\n}\n";
    let d = diagnostics(src)
        .into_iter()
        .find(|d| d.rule == "unreachable-code")
        .expect("expected an unreachable-code finding");
    assert!(
        d.message.body.contains("both branches"),
        "{:?}",
        d.message.body
    );
    let fix = d.fix.as_ref().expect("should carry a fix");
    assert_eq!(fix.applicability, Applicability::Unsafe);
    assert_eq!(
        apply_fixes(src, std::slice::from_ref(fix), true).output,
        "f <- function() {\n  if (x) return(1) else return(2)\n}\n"
    );

    // Braced arms and mixed `return`/`stop` also count.
    let src =
        "f <- function() {\n  if (x) {\n    stop(\"a\")\n  } else {\n    return(2)\n  }\n  3\n}\n";
    assert!(
        diagnostics(src)
            .iter()
            .any(|d| d.rule == "unreachable-code"),
        "braced both-branches exit should flag"
    );
}

#[test]
fn unreachable_code_both_branches_negatives() {
    for src in [
        // only one arm diverges — the tail is reachable
        "f <- function() {\n  if (x) return(1) else 2\n  3\n}\n",
        // no `else` — the false path falls through
        "f <- function() {\n  if (x) return(1)\n  3\n}\n",
        // both arms exit, but `return` is locally redefined — no longer terminates
        "f <- function() {\n  return <- identity\n  if (x) return(1) else return(2)\n  3\n}\n",
        // both arms `return` but outside any function — not the shape (needs `stop`)
        "if (x) return(1) else return(2)\n3\n",
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"unreachable-code"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn unreachable_code_ignores_reachable_shapes() {
    for src in [
        // terminator is the last statement — nothing after it
        "f <- function() {\n  g()\n  return(1)\n}\n",
        // `return()` guarded by `if` is not a direct statement; the tail is reachable
        "f <- function() {\n  if (x) return(1)\n  2\n}\n",
        // `return()` outside any function is not the unreachable-after-return shape
        "{\n  return(1)\n  2\n}\n",
        // not a terminating call
        "f <- function() {\n  g()\n  2\n}\n",
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"unreachable-code"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn unreachable_code_skips_shadowed_terminators() {
    // A user redefinition of `return`/`stop` means the call no longer terminates,
    // so the following code is reachable — don't flag.
    for src in [
        "stop <- function(...) NULL\nf <- function() {\n  stop()\n  g()\n}\n",
        "f <- function() {\n  return <- function(x) x\n  return(1)\n  2\n}\n",
    ] {
        let rules: Vec<&str> = diagnostics(src).iter().map(|d| d.rule).collect();
        assert!(
            !rules.contains(&"unreachable-code"),
            "{src:?} should not flag, got: {rules:?}"
        );
    }
}

#[test]
fn unreachable_code_withholds_fix_for_dropped_comment() {
    // Deleting the unreachable region would drop a comment between two of the
    // unreachable statements, so the fix is withheld — the finding still reports.
    let src = "f <- function() {\n  return(1)\n  a()\n  # mid\n  b()\n}\n";
    let d = diagnostics(src)
        .into_iter()
        .find(|d| d.rule == "unreachable-code")
        .expect("expected an unreachable-code finding");
    assert!(d.fix.is_none(), "dropped comment should withhold the fix");

    // A comment between the terminator and the first unreachable statement is
    // preserved by the deletion (which starts at that statement), so the fix is
    // still offered there.
    let src = "f <- function() {\n  return(1)\n  # keep me\n  g()\n}\n";
    let fix = diagnostics(src)
        .into_iter()
        .find(|d| d.rule == "unreachable-code")
        .and_then(|d| d.fix)
        .expect("expected a fix that preserves the leading comment");
    assert_eq!(
        apply_fixes(src, std::slice::from_ref(&fix), true).output,
        "f <- function() {\n  return(1)\n  # keep me\n}\n"
    );
}

// ---------------------------------------------------------------------------
// Autofix correctness: a fix is a textual edit, so the bar is that applying it
// leaves code that still parses. It does NOT owe line-width — layout is the
// formatter's job (Tenet 1), the pipeline is fix-then-format. The curated cases
// below are all width-safe, so on them the stronger `format --check`-clean
// property also holds, which makes it a useful regression guard for *local*
// layout (spacing/indent) — but that is scoped to width-safe edits, not a
// universal guarantee.
// ---------------------------------------------------------------------------

/// Format `input` to canonical form, apply every available fix to a fixpoint,
/// then assert the result still parses. For these width-safe cases, also assert
/// it stays format-clean (a local-layout regression guard, not a width promise).
fn assert_fixed_output_is_clean(input: &str) {
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

    // The guaranteed invariant: fixed output still parses.
    assert!(
        parse(&content).diagnostics.is_empty(),
        "fixed output must parse cleanly:\n{content:?}"
    );
    // Scoped check: on these width-safe cases, local layout stays clean too.
    let reformatted = format_with_style(&content, style).expect("fixed output should format");
    assert_eq!(
        content, reformatted,
        "a fix introduced a local-layout error on a width-safe case.\nstarted from:\n{clean}\n--- after fixes ---\n{content}\n--- but format produces ---\n{reformatted}"
    );
}

#[test]
fn fixed_output_is_parseable_and_clean() {
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
        // redundant-equals (`== TRUE` drop, `== FALSE` negate)
        "print(x == TRUE)\n",
        "print(x == FALSE)\n",
        "print(TRUE == f(x))\n",
        // equals-na (`== NA` → is.na)
        "print(x == NA)\n",
        "print(NA == g(y))\n",
        // redundant-ifelse collapse
        "print(ifelse(cond, TRUE, FALSE))\n",
        "print(ifelse(cond, FALSE, TRUE))\n",
        // true-false-symbol (`T`/`F` → `TRUE`/`FALSE`)
        "x <- T\n",
        "if (F) g()\n",
        "c(T, F, T)\n",
        // repeat (`while (TRUE)` → `repeat`)
        "while (TRUE) f()\n",
        "while (TRUE) {\n  f()\n}\n",
        // vector-logic (`&`/`|` → `&&`/`||` in a condition)
        "if (a & b) f()\n",
        "while (a | b) f()\n",
        "if (x && (a | b)) f()\n",
        "if (a & b & c) f()\n",
        // comparison-negation (`!(a == b)` → `a != b`)
        "print(!(a == b))\n",
        "if (!(x < y)) f()\n",
        "flag <- !a == b\n",
        // outer-negation (`any(!x)` → `!all(x)`)
        "if (any(!x)) f()\n",
        "flag <- all(!x)\n",
        "z <- any(!a, !b)\n",
        // any-is-na (`any(is.na(x))` → `anyNA(x)`)
        "if (any(is.na(x))) f()\n",
        "flag <- any(is.na(df$col))\n",
        // any-duplicated (`any(duplicated(x))` → `anyDuplicated(x) > 0`)
        "if (any(duplicated(x))) f()\n",
        "flag <- any(duplicated(df$col))\n",
        // crossprod (`t(x) %*% y` → `crossprod`, `x %*% t(y)` → `tcrossprod`)
        "z <- t(x) %*% y\n",
        "z <- x %*% t(y)\n",
        "z <- t(x) %*% x\n",
        // lengths (`sapply(x, length)` → `lengths(x)`)
        "n <- sapply(x, length)\n",
        "n <- sapply(df$col, length)\n",
        // seq (`1:length(x)` → `seq_along(x)`, `1:n` → `seq_len(n)`)
        "for (i in 1:length(x)) print(i)\n",
        "for (i in 1:n) f(i)\n",
        "idx <- 1:nrow(df)\n",
        // nzchar (`nchar(x) > 0` → `nzchar(x)`; unsafe)
        "flag <- nchar(x) > 0\n",
        "flag <- nchar(x) == 0\n",
        "if (nchar(x) >= 1) f()\n",
        // is-numeric (`is.numeric(x) || is.integer(x)` → `is.numeric(x)`)
        "if (is.numeric(x) || is.integer(x)) f()\n",
        "flag <- is.integer(df$col) | is.numeric(df$col)\n",
        // class-equals (`class(x) == "cls"` → `inherits(x, "cls")`; unsafe)
        "if (class(x) == \"factor\") f()\n",
        "flag <- class(x) != \"factor\"\n",
        "if (\"data.frame\" %in% class(x)) f()\n",
        // sort (`sort(x)[1]` → `min(x)`; unsafe)
        "m <- sort(x)[1]\n",
        "m <- sort(x, decreasing = TRUE)[1]\n",
        // string-boundary (`grepl("^a", x)` → `startsWith`; unsafe)
        "flag <- grepl(\"^abc\", x)\n",
        "flag <- grepl(\"xyz$\", y)\n",
        // fixed-regex (add `fixed = TRUE` for a literal pattern)
        "flag <- grepl(\"abc\", x)\n",
        "out <- gsub(\"lit\", \"R\", s)\n",
        // unreachable-code deletion (after `return()`/`stop()`)
        "f <- function() {\n  g()\n  return(1)\n  2\n}\nf()\n",
        "{\n  stop()\n  f()\n}\n",
    ];
    for case in cases {
        assert_fixed_output_is_clean(case);
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

fn run_cli_stdin<const N: usize>(args: [&str; N], stdin_input: &str) -> std::process::Output {
    use std::io::Write as _;
    let mut child = Command::new(env!("CARGO_BIN_EXE_arity"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn cli");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin_input.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("failed to wait for cli")
}

// --- roxygen documentation rules --------------------------------------------

#[test]
fn roxygen_unknown_tag_flags_misspelled_tag() {
    let src = "#' Title\n#' @exprot\nf <- function() 1\n";
    let d = diagnostics(src)
        .into_iter()
        .find(|d| d.rule == "roxygen-unknown-tag")
        .expect("expected a roxygen-unknown-tag finding");
    // The span is the `@` + tag name, and there is no fix (the intended tag
    // is unknowable).
    assert_eq!(&src[d.range], "@exprot");
    assert!(d.fix.is_none());
}

#[test]
fn roxygen_unknown_tag_accepts_registry_tags() {
    let src = "#' Title\n#'\n#' Description.\n#'\n#' @md\n#' @param x X.\n#' @returns Value.\n#' @seealso [g()]\n#' @examples\n#' f(1)\n#' @export\nf <- function(x) x\n";
    assert!(
        diagnostics(src)
            .into_iter()
            .all(|d| d.rule != "roxygen-unknown-tag"),
        "registry tags must not be flagged"
    );
}

#[test]
fn roxygen_unknown_tag_ignores_escaped_and_bare_at() {
    // `@@` is the escape for a literal `@`, and `@ ` / `@1` are prose — none
    // lex as tags, so none can be flagged.
    let src = "#' Send to a@@b.com @ home, @1st\nf <- function() 1\n";
    assert!(
        diagnostics(src)
            .into_iter()
            .all(|d| d.rule != "roxygen-unknown-tag")
    );
}

#[test]
fn roxygen_title_flags_tag_only_documented_function() {
    let src = "#' @param x A number.\n#' @export\nf <- function(x) x\n";
    let d = diagnostics(src)
        .into_iter()
        .find(|d| d.rule == "roxygen-title")
        .expect("expected a roxygen-title finding");
    // The span is the block's first marker; no fix (the title is prose).
    assert_eq!(&src[d.range], "#'");
    assert_eq!(u32::from(d.range.start()), 0);
    assert!(d.fix.is_none());
}

#[test]
fn roxygen_title_flags_export_only_block() {
    // An `@export`ed function with no documentation at all: roxygen2 stays
    // silent (no Rd topic is generated), but `R CMD check` flags the export
    // as undocumented, so arity reports it here.
    let src = "#' @export\nf <- function(x) x\n";
    assert!(
        diagnostics(src)
            .into_iter()
            .any(|d| d.rule == "roxygen-title")
    );
}

#[test]
fn roxygen_title_negatives() {
    let cases: &[&str] = &[
        // A leading prose paragraph is the title.
        "#' Adds things.\n#' @export\nf <- function() 1\n",
        // Explicit @title.
        "#' @title Adds things.\n#' @export\nf <- function() 1\n",
        // @noRd blocks generate no topic.
        "#' @noRd\n#' @param x X.\nf <- function(x) x\n",
        // Merged/inherited topics can carry the title elsewhere.
        "#' @rdname topic\n#' @export\nf <- function() 1\n",
        "#' @describeIn topic Variant.\n#' @export\nf <- function() 1\n",
        // Import-attachment block: no topic, nothing exported.
        "#' @importFrom pkg fn\nf <- function() 1\n",
        // Unassociated block: not a classifiable function.
        "#' @param x X.\nsetMethod(\"show\", \"C\", function(object) 1)\n",
    ];
    for src in cases {
        assert!(
            diagnostics(src)
                .into_iter()
                .all(|d| d.rule != "roxygen-title"),
            "should not flag: {src:?}"
        );
    }
}

#[test]
fn roxygen_return_flags_exported_function_without_return() {
    let src = "#' Add one\n#' @param x A number.\n#' @export\nadd_one <- function(x) x + 1\n";
    let d = diagnostics(src)
        .into_iter()
        .find(|d| d.rule == "roxygen-return")
        .expect("expected a roxygen-return finding");
    // The span is the `@export` tag; no fix (the value description is prose).
    assert_eq!(&src[d.range], "@export");
    assert!(d.fix.is_none());
}

#[test]
fn roxygen_return_negatives() {
    let cases: &[&str] = &[
        // @return / @returns satisfy the check.
        "#' Add one\n#' @return The sum.\n#' @export\nf <- function(x) x + 1\n",
        "#' Add one\n#' @returns The sum.\n#' @export\nf <- function(x) x + 1\n",
        // Not exported: internal helpers owe no @return.
        "#' Add one\n#' @param x X.\nf <- function(x) x + 1\n",
        // @noRd blocks generate no topic.
        "#' Add one\n#' @export\n#' @noRd\nf <- function(x) x + 1\n",
        // Inherited/merged topics may document the value elsewhere.
        "#' Add one\n#' @rdname topic\n#' @export\nf <- function(x) x + 1\n",
        "#' Add one\n#' @inherit other return\n#' @export\nf <- function(x) x + 1\n",
        // Not a classifiable function shape.
        "#' Add one\n#' @export\nsetMethod(\"show\", \"C\", function(object) 1)\n",
        "#' Data set docs\n#' @export\nx <- 1\n",
    ];
    for src in cases {
        assert!(
            diagnostics(src)
                .into_iter()
                .all(|d| d.rule != "roxygen-return"),
            "should not flag: {src:?}"
        );
    }
}

fn roxygen_param_findings(src: &str) -> Vec<arity::linter::Diagnostic> {
    diagnostics(src)
        .into_iter()
        .filter(|d| d.rule == "roxygen-param")
        .collect()
}

#[test]
fn roxygen_param_flags_undocumented_formal() {
    let src = "#' Add\n#' @param x X.\n#' @export\nf <- function(x, y) x + y\n";
    let findings = roxygen_param_findings(src);
    assert_eq!(findings.len(), 1);
    // The span is the undocumented formal's name token in the signature.
    assert_eq!(&src[findings[0].range], "y");
    assert!(u32::from(findings[0].range.start()) > src.find("function").unwrap() as u32);
    assert!(findings[0].fix.is_none());
}

#[test]
fn roxygen_param_flags_nonexistent_formal() {
    let src = "#' Add\n#' @param x X.\n#' @param z Z.\nf <- function(x) x\n";
    let findings = roxygen_param_findings(src);
    assert_eq!(findings.len(), 1);
    assert_eq!(&src[findings[0].range], "z");
}

#[test]
fn roxygen_param_flags_duplicate_name() {
    let src = "#' Add\n#' @param x X.\n#' @param x Again.\nf <- function(x) x\n";
    let findings = roxygen_param_findings(src);
    assert_eq!(findings.len(), 1);
    // The second occurrence is the duplicate.
    assert_eq!(&src[findings[0].range], "x");
    assert!(u32::from(findings[0].range.start()) > src.find("Again").unwrap() as u32 - 20);
}

#[test]
fn roxygen_param_duplicate_checked_even_in_merged_topics() {
    let src = "#' @rdname topic\n#' @param x X.\n#' @param x Again.\nf <- function(x) x\n";
    let findings = roxygen_param_findings(src);
    assert_eq!(findings.len(), 1, "duplicate is a per-block fact");
}

#[test]
fn roxygen_param_flags_missing_description() {
    let src = "#' Add\n#' @param x\nf <- function(x) x\n";
    let findings = roxygen_param_findings(src);
    assert_eq!(findings.len(), 1);
    assert_eq!(&src[findings[0].range], "@param x");
}

#[test]
fn roxygen_param_flags_bare_tag() {
    let src = "#' Add\n#' @param\n#' @export\nf <- function() 1\n";
    let findings = roxygen_param_findings(src);
    assert_eq!(findings.len(), 1);
    assert_eq!(&src[findings[0].range], "@param");
}

#[test]
fn roxygen_param_negatives() {
    let cases: &[&str] = &[
        // Exact coverage.
        "#' Add\n#' @param x X.\n#' @param y Y.\nf <- function(x, y) x\n",
        // Multi-name arg documents both formals.
        "#' Add\n#' @param a,b Both.\nf <- function(a, b) a\n",
        // `...` is documented like any formal.
        "#' Add\n#' @param ... Extra.\nf <- function(...) 1\n",
        // The name may sit on a continuation line (roxygen2 splits the folded
        // value on whitespace).
        "#' Add\n#' @param\n#' x The x value.\nf <- function(x) x\n",
        // @inheritParams pulls the missing docs in from elsewhere.
        "#' Add\n#' @inheritParams other\n#' @export\nf <- function(x, y) x\n",
        // In a merged topic a param may belong to a sibling function.
        "#' @rdname topic\n#' @param extra E.\nf <- function(x) x\n",
        // Unassociated block: no formals to judge.
        "#' Add\n#' @param x X.\nsetMethod(\"show\", \"C\", function(object) 1)\n",
        // No roxygen block at all.
        "f <- function(x) x\n",
    ];
    for src in cases {
        assert!(
            roxygen_param_findings(src).is_empty(),
            "should not flag: {src:?}"
        );
    }
}

#[test]
fn roxygen_examples_flags_parse_error() {
    // An unclosed call: the same shape the parser diagnoses in plain R code.
    let src = "#' Add\n#' @examples\n#' add(1\n#' @export\nf <- function(x) x\n";
    let d = diagnostics(src)
        .into_iter()
        .find(|d| d.rule == "roxygen-examples")
        .expect("expected a roxygen-examples finding");
    // The span maps back into the comment, at the offending code.
    let example_line = src.find("add(1").unwrap() as u32;
    assert!(
        u32::from(d.range.start()) >= example_line,
        "range: {:?}",
        d.range
    );
    assert!(u32::from(d.range.end()) <= src.find("@export").unwrap() as u32);
    assert!(d.fix.is_none());
}

#[test]
fn roxygen_examples_flags_error_under_md_fragmentation() {
    // Under `@md` the example body is tokenized as markdown; the rule must
    // reassemble the line and still see the stray brace.
    let src = "#' Add\n#' @md\n#' @examples\n#' x <- *emph* + 1}\nf <- function(x) x\n";
    assert!(
        diagnostics(src)
            .into_iter()
            .any(|d| d.rule == "roxygen-examples")
    );
}

#[test]
fn roxygen_examples_flags_bad_examples_if_condition() {
    let src = "#' Add\n#' @examplesIf interactive((\n#' f(1)\nf <- function(x) x\n";
    let d = diagnostics(src)
        .into_iter()
        .find(|d| d.rule == "roxygen-examples")
        .expect("expected a finding for the condition");
    assert!(u32::from(d.range.start()) >= src.find("interactive").unwrap() as u32);
    assert!(
        u32::from(d.range.end())
            <= src
                .find('\n')
                .map(|_| src.find("\n#' f(1)").unwrap())
                .unwrap() as u32
                + 1
    );
}

#[test]
fn roxygen_examples_negatives() {
    let cases: &[&str] = &[
        // Clean examples, including multi-line and blank separator lines.
        "#' Add\n#' @examples\n#' f(1)\n#'\n#' f(\n#'   2\n#' )\nf <- function(x) x\n",
        // Same-line code after the tag.
        "#' Add\n#' @examples f(1)\nf <- function(x) x\n",
        // \dontrun{} is valid R (a call with a braced arg).
        "#' Add\n#' @examples\n#' \\dontrun{\n#'   f(1)\n#' }\nf <- function(x) x\n",
        // Good condition and body.
        "#' Add\n#' @examplesIf interactive()\n#' f(1)\nf <- function(x) x\n",
        // Empty examples section: nothing to parse.
        "#' Add\n#' @examples\nf <- function(x) x\n",
    ];
    for src in cases {
        assert!(
            diagnostics(src)
                .into_iter()
                .all(|d| d.rule != "roxygen-examples"),
            "should not flag: {src:?}"
        );
    }
}

#[test]
fn roxygen_examples_multiline_error_range_stays_in_block() {
    // A diagnostic spanning extracted lines must map back inside the roxygen
    // block, not spill into the code that follows it.
    let src = "#' Add\n#' @examples\n#' f(1\n#' g(2\nf <- function(x) x\n";
    for d in diagnostics(src)
        .into_iter()
        .filter(|d| d.rule == "roxygen-examples")
    {
        assert!(
            u32::from(d.range.end()) <= src.rfind("f <- function").unwrap() as u32,
            "range {:?} escapes the block",
            d.range
        );
    }
}
