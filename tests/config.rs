use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use tempfile::tempdir;

const LONG_FN_INPUT: &str = "x <- function(aaaaa, bbbbb, ccccc, ddddd) { 1 }\n";

fn run_cli<const N: usize>(args: [&str; N], stdin_input: &str) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_arity"));
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to spawn arity cli");
    let mut stdin = child.stdin.take().expect("failed to open stdin");
    stdin
        .write_all(stdin_input.as_bytes())
        .expect("failed to write stdin");
    drop(stdin);
    child.wait_with_output().expect("failed to wait for cli")
}

fn run_cli_in<const N: usize>(
    cwd: &Path,
    args: [&str; N],
    stdin_input: &str,
) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_arity"));
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to spawn arity cli");
    let mut stdin = child.stdin.take().expect("failed to open stdin");
    stdin
        .write_all(stdin_input.as_bytes())
        .expect("failed to write stdin");
    drop(stdin);
    child.wait_with_output().expect("failed to wait for cli")
}

fn run_cli_in_no_stdin<const N: usize>(cwd: &Path, args: [&str; N]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_arity"))
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to run cli")
}

#[test]
fn cli_line_width_default_keeps_input_inline() {
    // At the default 80, the input fits on one line as a bare function body.
    let output = run_cli(["format"], LONG_FN_INPUT);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    assert_eq!(stdout, "x <- function(aaaaa, bbbbb, ccccc, ddddd) 1\n");
}

#[test]
fn cli_line_width_override_breaks_output() {
    // At 30, the function call must wrap.
    let output = run_cli(["format", "--line-width", "30"], LONG_FN_INPUT);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    assert!(
        stdout.contains("function(\n"),
        "expected wrapped function args, got:\n{stdout}"
    );
}

#[test]
fn cli_indent_width_override_changes_output() {
    let output = run_cli(
        ["format", "--line-width", "30", "--indent-width", "4"],
        LONG_FN_INPUT,
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    // First indented arg should sit at column 4 (four spaces), not 2.
    assert!(
        stdout.contains("\n    aaaaa,"),
        "expected 4-space indent, got:\n{stdout}"
    );
}

#[test]
fn cli_explicit_config_is_applied() {
    let dir = tempdir().unwrap();
    let cfg = dir.path().join("custom.toml");
    fs::write(&cfg, "[format]\nline-width = 30\n").unwrap();

    let output = run_cli(["format", "--config", cfg.to_str().unwrap()], LONG_FN_INPUT);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    assert!(stdout.contains("function(\n"), "got:\n{stdout}");
}

#[test]
fn cli_missing_config_file_errors() {
    let dir = tempdir().unwrap();
    let cfg = dir.path().join("does-not-exist.toml");

    let output = run_cli(["format", "--config", cfg.to_str().unwrap()], LONG_FN_INPUT);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does-not-exist.toml"), "stderr: {stderr}");
}

#[test]
fn cli_no_config_ignores_discovered_arity_toml() {
    let dir = tempdir().unwrap();
    // Ancestor arity.toml would force a tight line width — we must ignore it.
    fs::write(dir.path().join("arity.toml"), "[format]\nline-width = 30\n").unwrap();

    let output = run_cli_in(dir.path(), ["format", "--no-config"], LONG_FN_INPUT);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    // With defaults the call fits inline; no break.
    assert!(!stdout.contains("function(\n"), "got:\n{stdout}");
}

#[test]
fn cli_config_discovered_from_cwd() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("arity.toml"), "[format]\nline-width = 30\n").unwrap();

    let output = run_cli_in(dir.path(), ["format"], LONG_FN_INPUT);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    assert!(stdout.contains("function(\n"), "got:\n{stdout}");
}

#[test]
fn cli_config_and_no_config_conflict() {
    let dir = tempdir().unwrap();
    let cfg = dir.path().join("custom.toml");
    fs::write(&cfg, "[format]\n").unwrap();

    let output = run_cli(
        ["format", "--config", cfg.to_str().unwrap(), "--no-config"],
        LONG_FN_INPUT,
    );
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot be used with") || stderr.contains("conflicts"),
        "expected clap conflict error, got: {stderr}"
    );
}

#[test]
fn cli_bad_config_field_reports_file_and_line() {
    let dir = tempdir().unwrap();
    let cfg = dir.path().join("bad.toml");
    fs::write(&cfg, "[format]\nline-widht = 80\n").unwrap();

    let output = run_cli(["format", "--config", cfg.to_str().unwrap()], LONG_FN_INPUT);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("bad.toml"), "stderr: {stderr}");
    assert!(
        stderr.contains("line-widht") || stderr.contains("unknown"),
        "stderr: {stderr}"
    );
}

#[test]
fn cli_format_check_honors_configured_line_width() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("arity.toml"), "[format]\nline-width = 30\n").unwrap();
    let r_file = dir.path().join("a.R");
    // Already formatted for the default 80 (bare body); the configured
    // line-width = 30 should force a reformat.
    fs::write(&r_file, "x <- function(aaaaa, bbbbb, ccccc, ddddd) 1\n").unwrap();

    let output = run_cli_in_no_stdin(dir.path(), ["format", "--check", r_file.to_str().unwrap()]);
    assert!(
        !output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Diff in"), "stdout: {stdout}");
    assert!(stdout.contains("a.R"), "stdout: {stdout}");
}

#[test]
fn cli_invalid_override_value_errors() {
    let output = run_cli(["format", "--line-width", "0"], LONG_FN_INPUT);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("line-width"), "stderr: {stderr}");
}

// A fixture that trips two distinct default-enabled rules: `unused-binding`
// (the `x <- 1` local is never read) and `equals-na` (the `a == NA` comparison).
const LINT_TWO_RULES: &str = "f <- function(a) {\n  x <- 1\n  a == NA\n}\n";

#[test]
fn cli_lint_select_restricts_to_named_rule() {
    let dir = tempdir().unwrap();
    let r_file = dir.path().join("a.R");
    fs::write(&r_file, LINT_TWO_RULES).unwrap();

    let output = run_cli_in_no_stdin(
        dir.path(),
        [
            "lint",
            "--output",
            "concise",
            "--select",
            "equals-na",
            r_file.to_str().unwrap(),
        ],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("equals-na"), "stderr: {stderr}");
    assert!(
        !stderr.contains("unused-binding"),
        "selecting equals-na should suppress unused-binding; stderr: {stderr}"
    );
}

#[test]
fn cli_lint_ignore_suppresses_named_rule() {
    let dir = tempdir().unwrap();
    let r_file = dir.path().join("a.R");
    fs::write(&r_file, LINT_TWO_RULES).unwrap();

    let output = run_cli_in_no_stdin(
        dir.path(),
        [
            "lint",
            "--output",
            "concise",
            "--ignore",
            "unused-binding",
            r_file.to_str().unwrap(),
        ],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unused-binding"),
        "ignored rule should not fire; stderr: {stderr}"
    );
    assert!(
        stderr.contains("equals-na"),
        "other rules should still fire; stderr: {stderr}"
    );
}

#[test]
fn cli_lint_select_accepts_comma_separated_list() {
    let dir = tempdir().unwrap();
    let r_file = dir.path().join("a.R");
    fs::write(&r_file, LINT_TWO_RULES).unwrap();

    let output = run_cli_in_no_stdin(
        dir.path(),
        [
            "lint",
            "--output",
            "concise",
            "--select",
            "equals-na,unused-binding",
            r_file.to_str().unwrap(),
        ],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("equals-na"), "stderr: {stderr}");
    assert!(stderr.contains("unused-binding"), "stderr: {stderr}");
}

#[test]
fn cli_lint_unknown_selected_rule_errors() {
    let dir = tempdir().unwrap();
    let r_file = dir.path().join("a.R");
    fs::write(&r_file, LINT_TWO_RULES).unwrap();

    let output = run_cli_in_no_stdin(
        dir.path(),
        ["lint", "--select", "no-such-rule", r_file.to_str().unwrap()],
    );
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown lint rule") && stderr.contains("no-such-rule"),
        "stderr: {stderr}"
    );
}

#[test]
fn cli_lint_rules_table_configures_a_rule() {
    // End-to-end proof that `[lint.rules.<id>]` reaches the rule: `sapply` is
    // not in the built-in set, so it only fires because the config added it.
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("arity.toml"),
        concat!(
            "[lint]\n",
            "select = [\"undesirable-function\"]\n",
            "\n",
            "[lint.rules.undesirable-function]\n",
            "extend-functions = { sapply = \"use `vapply()`\" }\n",
        ),
    )
    .unwrap();
    let r_file = dir.path().join("a.R");
    fs::write(&r_file, "attach(mtcars)\nsapply(1:3, identity)\n").unwrap();

    let output = run_cli_in_no_stdin(
        dir.path(),
        ["lint", "--output", "concise", r_file.to_str().unwrap()],
    );
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    // The built-in entry and the configured one both fire.
    assert_eq!(
        stderr.matches("undesirable-function").count(),
        2,
        "stderr: {stderr}"
    );
}

#[test]
fn cli_lint_rules_functions_replaces_the_builtin_set() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("arity.toml"),
        concat!(
            "[lint]\n",
            "select = [\"undesirable-function\"]\n",
            "\n",
            "[lint.rules.undesirable-function]\n",
            "functions = { sapply = \"use `vapply()`\" }\n",
        ),
    )
    .unwrap();
    let r_file = dir.path().join("a.R");
    fs::write(&r_file, "attach(mtcars)\nsapply(1:3, identity)\n").unwrap();

    let output = run_cli_in_no_stdin(
        dir.path(),
        ["lint", "--output", "concise", r_file.to_str().unwrap()],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("sapply"), "stderr: {stderr}");
    assert!(
        !stderr.contains("attach"),
        "`functions` replaces the built-in set; stderr: {stderr}"
    );
}

#[test]
fn cli_lint_unknown_rule_table_is_a_parse_error() {
    // Unlike `select`/`ignore`, a rule ID under `[lint.rules]` is schema, so a
    // typo fails at config-parse time with the offending key named.
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("arity.toml"),
        "[lint.rules.undesirabl-function]\nfunctions = {}\n",
    )
    .unwrap();
    let r_file = dir.path().join("a.R");
    fs::write(&r_file, "x <- 1\n").unwrap();

    let output = run_cli_in_no_stdin(
        dir.path(),
        ["lint", "--output", "concise", r_file.to_str().unwrap()],
    );
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("undesirabl-function"), "stderr: {stderr}");
}

// A misformatted file the formatter would rewrite (used to prove exclusion).
const MISFORMATTED: &str = "x<-1\n";

#[test]
fn cli_format_check_skips_configured_exclude() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("arity.toml"), "exclude = [\"skip/\"]\n").unwrap();
    fs::create_dir(dir.path().join("skip")).unwrap();
    fs::write(dir.path().join("skip").join("bad.R"), MISFORMATTED).unwrap();
    // Only the excluded file is misformatted, so the check passes.
    fs::write(dir.path().join("good.R"), "x <- 1\n").unwrap();

    let output = run_cli_in_no_stdin(dir.path(), ["format", "--check", "."]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cli_format_check_skips_default_excluded_generated_file() {
    let dir = tempdir().unwrap();
    // No config: the built-in default-exclude set still applies. A clean,
    // non-excluded file keeps the discovered set non-empty.
    fs::write(dir.path().join("good.R"), "x <- 1\n").unwrap();
    fs::write(dir.path().join("RcppExports.R"), MISFORMATTED).unwrap();

    let output = run_cli_in_no_stdin(dir.path(), ["format", "--check", "."]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cli_format_check_exclude_replaces_default_set() {
    let dir = tempdir().unwrap();
    // Setting `exclude` drops the built-in defaults, so a misformatted,
    // normally-default-excluded file is now discovered and reported.
    fs::write(dir.path().join("arity.toml"), "exclude = [\"skip/\"]\n").unwrap();
    fs::write(dir.path().join("RcppExports.R"), MISFORMATTED).unwrap();

    let output = run_cli_in_no_stdin(dir.path(), ["format", "--check", "."]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cli_format_check_extend_exclude_keeps_default_set() {
    let dir = tempdir().unwrap();
    // `extend-exclude` adds to the defaults rather than replacing them, so both
    // the default-excluded file and the extra pattern are skipped.
    fs::write(
        dir.path().join("arity.toml"),
        "extend-exclude = [\"gen/\"]\n",
    )
    .unwrap();
    fs::write(dir.path().join("good.R"), "x <- 1\n").unwrap();
    fs::write(dir.path().join("RcppExports.R"), MISFORMATTED).unwrap();
    fs::create_dir(dir.path().join("gen")).unwrap();
    fs::write(dir.path().join("gen").join("a.R"), MISFORMATTED).unwrap();

    let output = run_cli_in_no_stdin(dir.path(), ["format", "--check", "."]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cli_format_check_exclude_flag_augments() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("good.R"), "x <- 1\n").unwrap();
    fs::create_dir(dir.path().join("gen")).unwrap();
    fs::write(dir.path().join("gen").join("a.R"), MISFORMATTED).unwrap();

    // Without --exclude the misformatted file is reported (exit 1)...
    let reported = run_cli_in_no_stdin(dir.path(), ["format", "--check", "."]);
    assert_eq!(reported.status.code(), Some(1));

    // ...with --exclude it is skipped (exit 0).
    let excluded = run_cli_in_no_stdin(dir.path(), ["format", "--check", "--exclude", "gen/", "."]);
    assert_eq!(
        excluded.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&excluded.stderr)
    );
}

#[test]
fn cli_format_check_force_exclude_skips_explicit_path() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("arity.toml"), "exclude = [\"skip/\"]\n").unwrap();
    fs::create_dir(dir.path().join("skip")).unwrap();
    fs::write(dir.path().join("skip").join("bad.R"), MISFORMATTED).unwrap();

    // Named explicitly, the excluded file is normally still checked...
    let reported = run_cli_in_no_stdin(dir.path(), ["format", "--check", "skip/bad.R"]);
    assert_eq!(reported.status.code(), Some(1));

    // ...but --force-exclude skips it, and the resulting empty set is a
    // clean no-op (exit 0), not a "no .R files found" usage error.
    let excluded = run_cli_in_no_stdin(
        dir.path(),
        ["format", "--check", "--force-exclude", "skip/bad.R"],
    );
    assert_eq!(
        excluded.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&excluded.stderr)
    );
}

#[test]
fn cli_lint_force_exclude_skips_explicit_path() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("arity.toml"), "exclude = [\"skip/\"]\n").unwrap();
    fs::create_dir(dir.path().join("skip")).unwrap();
    fs::write(dir.path().join("skip").join("bad.R"), LINT_TWO_RULES).unwrap();

    let reported = run_cli_in_no_stdin(dir.path(), ["lint", "skip/bad.R"]);
    assert_eq!(reported.status.code(), Some(1));

    let excluded = run_cli_in_no_stdin(dir.path(), ["lint", "--force-exclude", "skip/bad.R"]);
    assert_eq!(
        excluded.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&excluded.stderr)
    );
}

#[test]
fn cli_completions_emits_script() {
    let output = run_cli(["completions", "bash"], "");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("_arity"), "stdout: {stdout}");
}

#[test]
fn cli_init_writes_parseable_starter_config() {
    let dir = tempdir().unwrap();
    let out = run_cli_in_no_stdin(dir.path(), ["init"]);
    assert_eq!(out.status.code(), Some(0));
    let written = dir.path().join("arity.toml");
    assert!(written.is_file());

    // The starter config must parse: format a file using it as the config.
    let r_file = dir.path().join("a.R");
    fs::write(&r_file, "x<-1\n").unwrap();
    let fmt = run_cli_in_no_stdin(
        dir.path(),
        [
            "format",
            "--config",
            written.to_str().unwrap(),
            r_file.to_str().unwrap(),
        ],
    );
    assert_eq!(
        fmt.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&fmt.stderr)
    );
    assert_eq!(fs::read_to_string(&r_file).unwrap(), "x <- 1\n");
}

#[test]
fn cli_init_refuses_to_overwrite_without_force() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("arity.toml"), "[format]\nline-width = 70\n").unwrap();
    let out = run_cli_in_no_stdin(dir.path(), ["init"]);
    assert_eq!(out.status.code(), Some(2));
    // The existing config is left untouched.
    assert_eq!(
        fs::read_to_string(dir.path().join("arity.toml")).unwrap(),
        "[format]\nline-width = 70\n"
    );
    // ...but --force overwrites it.
    let forced = run_cli_in_no_stdin(dir.path(), ["init", "--force"]);
    assert_eq!(forced.status.code(), Some(0));
}

#[test]
fn format_description_defaults_to_enabled() {
    let config: arity::config::Config = toml::from_str("").expect("empty config parses");
    assert!(config.format.description);
}

#[test]
fn format_description_can_be_turned_off() {
    let config: arity::config::Config =
        toml::from_str("[format]\ndescription = false\n").expect("parses");
    assert!(!config.format.description);
}

#[test]
fn a_mistyped_description_key_is_a_parse_error() {
    let err = toml::from_str::<arity::config::Config>("[format]\ndescriptions = true\n")
        .expect_err("deny_unknown_fields rejects the typo");
    assert!(err.to_string().contains("descriptions"), "{err}");
}

#[test]
fn the_repos_own_config_shields_its_fixture_descriptions() {
    // `tests/fixtures/rindex/*/` are complete miniature packages: a DESCRIPTION
    // beside an `R/`, which is exactly what a walk collects. They are inputs the
    // parser and oracle suites assert on, so `arity format .` in this repo must
    // not walk in and rewrite them.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let (config_path, config) = arity::config::Config::discover(root)
        .expect("config loads")
        .expect("the repo has an arity.toml");
    let exclude = config
        .exclude_filter(Some(&config_path), root, &[])
        .expect("exclude patterns compile");

    let found = arity::file_discovery::collect_source_files(&[root.to_path_buf()], &exclude)
        .expect("walk succeeds");
    let fixtures: Vec<_> = found
        .description
        .iter()
        .filter(|path| path.starts_with(root.join("tests/fixtures")))
        .collect();
    assert!(
        fixtures.is_empty(),
        "fixture DESCRIPTIONs collected: {fixtures:?}"
    );
}
