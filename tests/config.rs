use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{ChildStdin, Command, Stdio};

use tempfile::tempdir;

const LONG_FN_INPUT: &str = "x <- function(aaaaa, bbbbb, ccccc, ddddd) { 1 }\n";

fn write_stdin(mut stdin: ChildStdin, input: &str) {
    if let Err(error) = stdin.write_all(input.as_bytes()) {
        // Argument validation can close stdin early; still collect its output.
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe,
            "failed to write stdin: {error}"
        );
    }
}

fn run_cli<const N: usize>(args: [&str; N], stdin_input: &str) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_arity"));
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to spawn arity cli");
    let stdin = child.stdin.take().expect("failed to open stdin");
    write_stdin(stdin, stdin_input);
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
    let stdin = child.stdin.take().expect("failed to open stdin");
    write_stdin(stdin, stdin_input);
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
fn cli_explicit_config_extends_relative_base() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("base.toml"), "[format]\nline-width = 30\n").unwrap();
    fs::write(dir.path().join("arity.toml"), "extend = \"base.toml\"\n").unwrap();

    let output = run_cli_in(
        dir.path(),
        ["format", "--config", "arity.toml"],
        LONG_FN_INPUT,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("function(\n"));
}

#[test]
fn cli_missing_config_file_errors() {
    let dir = tempdir().unwrap();
    let cfg = dir.path().join("does-not-exist.toml");

    let output = run_cli_in_no_stdin(dir.path(), ["format", "--config", cfg.to_str().unwrap()]);
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

fn run_cli_with_env_config(
    cwd: &Path,
    config: &Path,
    args: &[&str],
    input: &str,
) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_arity"))
        .args(args)
        .current_dir(cwd)
        .env("ARITY_CONFIG", config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn arity");
    // Config validation can exit before reading stdin; callers assert the resulting error.
    if let Err(err) = child.stdin.take().unwrap().write_all(input.as_bytes()) {
        assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe, "{err}");
    }
    child.wait_with_output().expect("wait for arity")
}

#[test]
fn cli_env_config_fallback_and_precedence() {
    let dir = tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join(".git")).unwrap();
    let config = dir.path().join("user.toml");
    fs::write(&config, "[format]\nline-width = 30\nindent-width = 4\n").unwrap();

    for path in [config.as_path(), Path::new("../user.toml")] {
        let output = run_cli_with_env_config(&project, path, &["format", "-"], LONG_FN_INPUT);
        assert!(output.status.success(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("function(\n    aaaaa,"), "{stdout}");
    }

    let inline = "x <- function(aaaaa, bbbbb, ccccc, ddddd) 1\n";
    for args in [
        vec!["format", "--no-config"],
        vec!["format", "--line-width", "80"],
    ] {
        let output = run_cli_with_env_config(&project, &config, &args, LONG_FN_INPUT);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(String::from_utf8(output.stdout).unwrap(), inline);
    }

    // An empty value is the same as leaving the fallback unset.
    let output = run_cli_with_env_config(&project, Path::new(""), &["format"], LONG_FN_INPUT);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), inline);

    let explicit = project.join("explicit.toml");
    fs::write(&explicit, "[format]\nline-width = 80\n").unwrap();
    let missing = dir.path().join("missing.toml");
    for path in [&config, &missing] {
        let output = run_cli_with_env_config(
            &project,
            path,
            &["format", "--config", explicit.to_str().unwrap()],
            LONG_FN_INPUT,
        );
        assert!(output.status.success(), "{output:?}");
        assert_eq!(String::from_utf8(output.stdout).unwrap(), inline);
    }

    // A project config replaces the fallback, including its unspecified keys.
    fs::write(project.join("arity.toml"), "[format]\nline-width = 30\n").unwrap();
    for path in [&config, &missing] {
        let output = run_cli_with_env_config(&project, path, &["format"], LONG_FN_INPUT);
        assert!(output.status.success(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("function(\n  aaaaa,"), "{stdout}");
    }
}

#[test]
fn cli_env_config_errors_are_reported_unless_disabled() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    let config = dir.path().join("user.toml");
    for contents in [
        None,
        Some("[format]\nline-widht = 30\n"),
        Some("[format]\nline-width = 0\n"),
    ] {
        if let Some(contents) = contents {
            fs::write(&config, contents).unwrap();
        }
        let output = run_cli_with_env_config(dir.path(), &config, &["format"], LONG_FN_INPUT);
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("user.toml"),
            "{output:?}"
        );

        let output = run_cli_with_env_config(
            dir.path(),
            &config,
            &["format", "--no-config"],
            LONG_FN_INPUT,
        );
        assert!(output.status.success(), "{output:?}");
    }
}

#[test]
fn cli_env_config_does_not_hide_invalid_project_config() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("arity.toml"), "[format]\nline-widht = 80\n").unwrap();
    let config = dir.path().join("user.toml");
    fs::write(&config, "").unwrap();
    // Exceed the pipe capacity so an early config error closes stdin during the write.
    let input = LONG_FN_INPUT.repeat(65_536);
    let output = run_cli_with_env_config(dir.path(), &config, &["format"], &input);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("arity.toml"),
        "{output:?}"
    );
}

#[test]
fn cli_env_config_excludes_are_relative_to_the_working_directory() {
    let dir = tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join(".git")).unwrap();
    fs::create_dir(project.join("generated")).unwrap();
    fs::write(project.join("generated/bad.R"), "x==NA\n").unwrap();
    fs::write(project.join("main.R"), "x <- 1\n").unwrap();
    let config = dir.path().join("user.toml");
    fs::write(
        &config,
        "exclude = [\"/generated/\"]\n[lint]\nselect = [\"equals-na\"]\n",
    )
    .unwrap();

    for args in [
        vec!["format", "--check", "--no-cache", "."],
        vec!["format", "--check", "--no-cache", project.to_str().unwrap()],
        vec!["lint", "."],
        vec!["lint", project.to_str().unwrap()],
    ] {
        let output = run_cli_with_env_config(&project, &config, &args, "");
        assert!(output.status.success(), "{output:?}");
    }

    // Naming the same file explicitly gives it a project anchor instead.
    let output = run_cli_with_env_config(
        &project,
        &config,
        &[
            "lint",
            "--config",
            config.to_str().unwrap(),
            project.to_str().unwrap(),
        ],
        "",
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("generated"),
        "{output:?}"
    );

    fs::write(project.join("main.R"), "x == NA\n").unwrap();
    let output = run_cli_with_env_config(&project, &config, &["lint", "."], "");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("equals-na"), "{output:?}");
    assert!(!stderr.contains("generated"), "{output:?}");
}

#[test]
#[cfg(unix)]
fn cli_env_config_excludes_hold_when_the_walk_uses_a_symlink() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join(".git")).unwrap();
    fs::create_dir(project.join("generated")).unwrap();
    fs::write(project.join("generated/bad.R"), "x==NA\n").unwrap();
    fs::write(project.join("main.R"), "x <- 1\n").unwrap();
    let link = dir.path().join("link");
    symlink(&project, &link).unwrap();
    let config = dir.path().join("user.toml");
    fs::write(
        &config,
        "exclude = [\"/generated/\"]\n[lint]\nselect = [\"equals-na\"]\n",
    )
    .unwrap();

    for target in [".", "../link", link.to_str().unwrap()] {
        for args in [
            vec!["format", "--check", "--no-cache", target],
            vec!["lint", target],
        ] {
            let output = run_cli_with_env_config(&link, &config, &args, "");
            assert!(output.status.success(), "{args:?}: {output:?}");
        }
    }
}

#[test]
fn cli_config_and_no_config_conflict() {
    let dir = tempdir().unwrap();
    let cfg = dir.path().join("custom.toml");
    fs::write(&cfg, "[format]\n").unwrap();

    // Clap rejects conflicting arguments before reading stdin.
    let output = run_cli_in_no_stdin(
        dir.path(),
        ["format", "--config", cfg.to_str().unwrap(), "--no-config"],
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

#[test]
fn cli_invalid_override_value_errors_with_large_stdin() {
    // Exceed the pipe capacity so rejection closes stdin during the write.
    let input = LONG_FN_INPUT.repeat(64 * 1024);
    let dir = tempdir().unwrap();
    for output in [
        run_cli(["format", "--line-width", "0"], &input),
        run_cli_in(dir.path(), ["format", "--line-width", "0"], &input),
    ] {
        assert_eq!(output.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("line-width"), "stderr: {stderr}");
    }
}

// A fixture that trips two distinct default-enabled rules: `unused-binding`
// (the `x <- 1` local is never read) and `equals-na` (the `a == NA` comparison).
const LINT_TWO_RULES: &str = "f <- function(a) {\n  x <- 1\n  a == NA\n}\n";

#[test]
fn cli_lint_extend_select_adds_rules_and_ignore_takes_precedence() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("a.R"),
        "f <- function(a) {\n  a == NA\n  setwd(\".\")\n}\nf(1)\n",
    )
    .unwrap();

    for (settings, expected) in [
        ("", vec!["equals-na"]),
        ("extend-select = []", vec!["equals-na"]),
        (
            "extend-select = [\"undesirable-function\"]",
            vec!["equals-na", "undesirable-function"],
        ),
        (
            "select = [\"equals-na\"]\nextend-select = [\"undesirable-function\", \"equals-na\", \"undesirable-function\"]",
            vec!["equals-na", "undesirable-function"],
        ),
        (
            "select = []\nextend-select = [\"undesirable-function\"]",
            vec!["undesirable-function"],
        ),
        (
            "extend-select = [\"undesirable-function\"]\nignore = [\"undesirable-function\"]",
            vec!["equals-na"],
        ),
        (
            "extend-select = [\"undesirable-function\"]\nignore = [\"equals-na\"]",
            vec!["undesirable-function"],
        ),
    ] {
        fs::write(
            dir.path().join("arity.toml"),
            format!("[lint]\n{settings}\n"),
        )
        .unwrap();
        let output = run_cli_in_no_stdin(dir.path(), ["lint", "--output", "json", "a.R"]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{settings}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let diagnostics: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
        let mut rules: Vec<_> = diagnostics
            .iter()
            .map(|d| d["rule"].as_str().unwrap())
            .collect();
        rules.sort_unstable();
        assert_eq!(rules, expected, "{settings}");
    }
}

#[test]
fn cli_lint_unknown_extend_selected_rule_errors() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("arity.toml"),
        "[lint]\nextend-select = [\"no-such-rule\"]\n",
    )
    .unwrap();
    let output = run_cli_in(dir.path(), ["lint", "-"], "1\n");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown lint rule") && stderr.contains("no-such-rule"),
        "{stderr}"
    );
}

#[test]
fn cli_lint_extend_select_flag_augments_config_and_accepts_lists() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("arity.toml"),
        "[lint]\nselect = [\"unused-binding\"]\nextend-select = [\"undesirable-function\"]\n",
    )
    .unwrap();
    let output = run_cli_in(
        dir.path(),
        [
            "lint",
            "--output",
            "json",
            "--select",
            "equals-na",
            "--extend-select",
            "implicit-assignment,equals-na",
            "--extend-select",
            "implicit-assignment",
            "-",
        ],
        "f <- function(a) {\n  a == NA\n  setwd(\".\")\n  identity(x <- 1)\n}\n",
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let diagnostics: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    let mut rules: Vec<_> = diagnostics
        .iter()
        .map(|d| d["rule"].as_str().unwrap())
        .collect();
    rules.sort_unstable();
    assert_eq!(
        rules,
        ["equals-na", "implicit-assignment", "undesirable-function"]
    );
}

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
    assert!(
        fs::read_to_string(&written)
            .unwrap()
            .contains("# roxygen = true")
    );

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
fn format_roxygen_defaults_to_enabled_and_can_be_disabled() {
    let default: arity::config::Config = toml::from_str("").expect("empty config parses");
    assert!(default.format.roxygen);

    let disabled: arity::config::Config =
        toml::from_str("[format]\nroxygen = false\n").expect("parses");
    assert!(!disabled.format.roxygen);
    assert!(!arity::formatter::FormatStyle::from(&disabled.format).roxygen);
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
        .exclude_filter(
            &arity::config::ConfigSource::Discovered(config_path),
            root,
            &[],
        )
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

#[test]
fn cli_cyclomatic_complexity_uses_configured_ceiling_without_fixing() {
    let dir = tempdir().unwrap();
    let source = "f <- function(x) if (x) 1 else 2\nf(TRUE)\n";
    let r_file = dir.path().join("a.R");
    fs::write(&r_file, source).unwrap();
    for (maximum, status) in [(1, 1), (2, 0)] {
        fs::write(dir.path().join("arity.toml"), format!(
            "[lint]\nselect = [\"cyclomatic-complexity\"]\n[lint.rules.cyclomatic-complexity]\nmax-complexity = {maximum}\n"
        )).unwrap();
        let output =
            run_cli_in_no_stdin(dir.path(), ["lint", "--fix", "--output", "concise", "a.R"]);
        assert_eq!(output.status.code(), Some(status), "{output:?}");
        assert_eq!(fs::read_to_string(&r_file).unwrap(), source);
        assert!(output.stdout.is_empty());
        if status == 1 {
            assert!(String::from_utf8_lossy(&output.stderr).contains("complexity of 2"));
        }
    }
}

const NESTED_INPUT: &str = "if (TRUE) {\nx<-1\n}\n";
const NESTED_FOUR: &str = "if (TRUE) {\n    x <- 1\n}\n";

#[test]
fn cli_nested_config_direct_file_and_check() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("R")).unwrap();
    fs::write(
        dir.path().join("R/arity.toml"),
        "[format]\nindent-width = 4\n",
    )
    .unwrap();
    fs::write(dir.path().join("R/f.R"), NESTED_INPUT).unwrap();
    let output = run_cli_in_no_stdin(dir.path(), ["format", "R/f.R"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("R/f.R")).unwrap(),
        NESTED_FOUR
    );
    let output = run_cli_in_no_stdin(dir.path(), ["format", "--check", "R/f.R"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn cli_nested_configs_recursive_and_mixed_inputs() {
    for args in [vec!["format", "."], vec!["format", "a/f.R", "b"]] {
        let dir = tempdir().unwrap();
        for (sub, indent) in [("a", 3), ("b", 4)] {
            fs::create_dir(dir.path().join(sub)).unwrap();
            fs::write(
                dir.path().join(sub).join("arity.toml"),
                format!("[format]\nindent-width = {indent}\n"),
            )
            .unwrap();
            fs::write(dir.path().join(sub).join("f.R"), NESTED_INPUT).unwrap();
        }
        let output = Command::new(env!("CARGO_BIN_EXE_arity"))
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("b/f.R")).unwrap(),
            NESTED_FOUR
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("a/f.R")).unwrap(),
            "if (TRUE) {\n   x <- 1\n}\n"
        );
    }
}

#[test]
fn cli_nested_lint_config_controls_reporting_and_fixes() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("R")).unwrap();
    fs::write(
        dir.path().join("arity.toml"),
        "[lint]\nselect = [\"seq\"]\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("R/arity.toml"),
        "extend = \"../arity.toml\"\n[lint]\nignore = [\"seq\"]\n",
    )
    .unwrap();
    fs::write(dir.path().join("R/f.R"), "1:length(x)\n").unwrap();
    for args in [["lint", "R/f.R"], ["lint", "."]] {
        let output = run_cli_in_no_stdin(dir.path(), args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = run_cli_in_no_stdin(dir.path(), ["lint", "--fix", "."]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("R/f.R")).unwrap(),
        "1:length(x)\n"
    );
}

#[test]
fn cli_nested_excludes_stay_at_their_config_directory() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("R/generated")).unwrap();
    fs::create_dir(dir.path().join("generated")).unwrap();
    fs::write(
        dir.path().join("R/arity.toml"),
        "extend-exclude = [\"/generated/\"]\n",
    )
    .unwrap();
    for path in ["R/generated/skip.R", "generated/keep.R", "R/keep.R"] {
        fs::write(dir.path().join(path), "x<-1\n").unwrap();
    }
    let output = run_cli_in_no_stdin(dir.path(), ["format", "."]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("R/generated/skip.R")).unwrap(),
        "x<-1\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("generated/keep.R")).unwrap(),
        "x <- 1\n"
    );
}

#[test]
fn cli_nested_configs_preserve_forced_selection_and_cli_overrides() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("R")).unwrap();
    fs::write(
        dir.path().join("arity.toml"),
        "[format]\nindent-width = 3\n[lint]\nselect = [\"seq\"]\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("R/arity.toml"),
        "[format]\nindent-width = 4\n[lint]\nignore = [\"seq\"]\n",
    )
    .unwrap();
    for (flags, expected_indent) in [
        (vec!["--config", "arity.toml"], 3),
        (vec!["--no-config"], 2),
        (vec!["--indent-width", "5"], 5),
    ] {
        fs::write(dir.path().join("R/f.R"), NESTED_INPUT).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_arity"))
            .arg("format")
            .args(flags)
            .arg("R/f.R")
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("R/f.R")).unwrap(),
            format!("if (TRUE) {{\n{}x <- 1\n}}\n", " ".repeat(expected_indent))
        );
    }
    fs::write(dir.path().join("R/f.R"), "1:length(x)\n").unwrap();
    for flags in [
        vec!["--config", "arity.toml"],
        vec!["--no-config"],
        vec!["--select", "seq", "--ignore", "equals-na"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_arity"))
            .arg("lint")
            .args(flags)
            .arg("R/f.R")
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("seq"));
    }
}

#[test]
fn cli_nested_lint_mixed_configs_and_fixes_share_the_input_set() {
    let dir = tempdir().unwrap();
    for sub in ["a", "b"] {
        fs::create_dir(dir.path().join(sub)).unwrap();
    }
    fs::write(
        dir.path().join("arity.toml"),
        "[lint]\nselect = [\"seq\"]\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("b/arity.toml"),
        "extend = \"../arity.toml\"\n[lint]\nignore = [\"seq\"]\n",
    )
    .unwrap();
    for path in ["a/f.R", "b/f.R"] {
        fs::write(dir.path().join(path), "1:length(x)\n").unwrap();
    }
    let output = run_cli_in_no_stdin(dir.path(), ["lint", "--output", "json", "a", "b/f.R"]);
    assert_eq!(output.status.code(), Some(1));
    let findings: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(findings.as_array().unwrap().len(), 1);
    let output = run_cli_in_no_stdin(dir.path(), ["lint", "--fix", "."]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("a/f.R")).unwrap(),
        "seq_along(x)\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("b/f.R")).unwrap(),
        "1:length(x)\n"
    );
}

#[test]
fn cli_nested_exclusions_prune_before_loading_configs() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("R/skip")).unwrap();
    fs::write(
        dir.path().join("arity.toml"),
        "extend-exclude = [\"/R/skip/\"]\n",
    )
    .unwrap();
    fs::write(dir.path().join("R/skip/arity.toml"), "invalid TOML").unwrap();
    fs::write(dir.path().join("R/skip/bad.R"), "x<-1\n").unwrap();
    fs::write(dir.path().join("keep.R"), "print(1)\n").unwrap();
    for command in ["format", "lint"] {
        let output = run_cli_in_no_stdin(dir.path(), [command, "."]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    // A nested config replaces ancestor excludes once the directory is admitted.
    fs::write(dir.path().join("R/arity.toml"), "exclude = []\n").unwrap();
    let output = run_cli_in_no_stdin(dir.path(), ["format", "."]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("arity.toml"));
}

#[test]
fn cli_nested_force_exclude_and_cli_exclude() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("R/generated")).unwrap();
    fs::write(
        dir.path().join("R/arity.toml"),
        "extend-exclude = [\"/generated/\"]\n",
    )
    .unwrap();
    for command in ["format", "lint"] {
        fs::write(dir.path().join("R/generated/f.R"), "1:length(x)\n").unwrap();
        let output =
            run_cli_in_no_stdin(dir.path(), [command, "--force-exclude", "R/generated/f.R"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("R/generated/f.R")).unwrap(),
            "1:length(x)\n"
        );
    }
    fs::write(dir.path().join("R/f.R"), "x<-1\n").unwrap();
    fs::write(dir.path().join("R/keep.R"), "print(1)\n").unwrap();
    let output = run_cli_in_no_stdin(dir.path(), ["format", "--exclude", "/f.R", "R"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("R/f.R")).unwrap(),
        "x<-1\n"
    );
}

#[test]
fn cli_nested_env_fallback_extend_and_repository_boundary() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("outer/repo/R")).unwrap();
    fs::create_dir(dir.path().join("outer/repo/.git")).unwrap();
    fs::write(
        dir.path().join("outer/arity.toml"),
        "[format]\nindent-width = 8\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("fallback.toml"),
        "[format]\nindent-width = 3\n",
    )
    .unwrap();
    fs::write(dir.path().join("outer/repo/R/f.R"), NESTED_INPUT).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_arity"))
            .args(["format", "outer/repo/R/f.R"])
            .env("ARITY_CONFIG", "fallback.toml")
            .current_dir(dir.path())
            .output()
            .unwrap()
    };
    let output = run();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("outer/repo/R/f.R")).unwrap(),
        "if (TRUE) {\n   x <- 1\n}\n"
    );
    fs::write(
        dir.path().join("outer/repo/R/arity.toml"),
        "extend = \"../../../fallback.toml\"\n[format]\nindent-width = 4\n",
    )
    .unwrap();
    let output = run();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("outer/repo/R/f.R")).unwrap(),
        NESTED_FOUR
    );
}

#[test]
fn cli_nested_stdin_filename_keeps_cwd_discovery() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("R")).unwrap();
    fs::write(
        dir.path().join("arity.toml"),
        "[format]\nindent-width = 3\n[lint]\nselect = [\"seq\"]\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("R/arity.toml"),
        "[format]\nindent-width = 4\n[lint]\nignore = [\"seq\"]\n",
    )
    .unwrap();
    let output = run_cli_in(
        dir.path(),
        ["format", "--stdin-filename", "R/f.R", "-"],
        NESTED_INPUT,
    );
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "if (TRUE) {\n   x <- 1\n}\n"
    );
    let output = run_cli_in(
        dir.path(),
        ["lint", "--stdin-filename", "R/f.R", "-"],
        "1:length(x)\n",
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn cli_nested_check_cache_uses_each_resolved_style() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("R")).unwrap();
    fs::write(
        dir.path().join("R/arity.toml"),
        "[format]\nindent-width = 4\n",
    )
    .unwrap();
    fs::write(dir.path().join("root.R"), "if (TRUE) {\n  x <- 1\n}\n").unwrap();
    fs::write(dir.path().join("R/f.R"), NESTED_FOUR).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_arity"))
            .args(["format", "--check", "."])
            .env("ARITY_CACHE_DIR", dir.path().join("cache"))
            .current_dir(dir.path())
            .output()
            .unwrap()
    };
    for _ in 0..2 {
        let output = run();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    if !cfg!(debug_assertions) {
        let version_dir = dir
            .path()
            .join("cache/format")
            .join(env!("CARGO_PKG_VERSION"));
        assert_eq!(
            fs::read_dir(version_dir).unwrap().count(),
            2,
            "release checks must persist separate cache files for both styles"
        );
    }
    // These bytes were cached under the nested style. They are dirty at root.
    fs::write(dir.path().join("root.R"), NESTED_FOUR).unwrap();
    let output = run();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("root.R"));
}

#[test]
fn cli_nested_lint_exclusions_govern_reporting_and_fixes() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("R/generated")).unwrap();
    fs::create_dir(dir.path().join("generated")).unwrap();
    fs::write(
        dir.path().join("arity.toml"),
        "[lint]\nselect = [\"seq\"]\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("R/arity.toml"),
        "extend = \"../arity.toml\"\nextend-exclude = [\"/generated/\"]\n",
    )
    .unwrap();
    for path in ["R/generated/skip.R", "generated/keep.R"] {
        fs::write(dir.path().join(path), "1:length(x)\n").unwrap();
    }
    let output = run_cli_in_no_stdin(dir.path(), ["lint", "--output", "json", "."]);
    assert_eq!(output.status.code(), Some(1));
    let findings: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(findings.as_array().unwrap().len(), 1);
    let output = run_cli_in_no_stdin(dir.path(), ["lint", "--fix", "."]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("R/generated/skip.R")).unwrap(),
        "1:length(x)\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("generated/keep.R")).unwrap(),
        "seq_along(x)\n"
    );
}

#[test]
fn cli_nested_lint_configs_preserve_cross_file_resolution() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("R")).unwrap();
    fs::write(
        dir.path().join("arity.toml"),
        "[lint]\nselect = [\"unused-binding\", \"undefined-symbol\"]\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("R/arity.toml"),
        "extend = \"../arity.toml\"\n[format]\nindent-width = 4\n",
    )
    .unwrap();
    fs::write(dir.path().join("R/helper.R"), "foo <- function() 42L\n").unwrap();
    fs::write(
        dir.path().join("main.R"),
        "source(\"R/helper.R\")\nprint(foo())\n",
    )
    .unwrap();
    let output = run_cli_in_no_stdin(dir.path(), ["lint", "."]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cli_nested_lint_configs_use_their_own_package_index() {
    use arity::rindex::cache::Cache;
    use arity::rindex::schema::{PackageIndex, SCHEMA_VERSION, SymbolEntry, SymbolKind};

    let dir = tempdir().unwrap();
    for (sub, name) in [("a", "from_a"), ("b", "from_b")] {
        fs::create_dir(dir.path().join(sub)).unwrap();
        let cache_root = dir.path().join(sub).join("cache");
        Cache::new(cache_root.clone())
            .write_package(&PackageIndex {
                schema_version: SCHEMA_VERSION,
                package: "configfixture".into(),
                version: "1.0".into(),
                lib_path: String::new(),
                title: None,
                r_version: None,
                harvested_at: 0,
                attaches: Vec::new(),
                symbols: vec![SymbolEntry {
                    name: name.into(),
                    kind: SymbolKind::Function,
                    exported: true,
                    formals: None,
                    help: None,
                }],
            })
            .unwrap();
        fs::write(
            dir.path().join(sub).join("arity.toml"),
            format!(
                "[lint]\nselect = [\"undefined-symbol\"]\n[index]\ncache-dir = {:?}\n",
                cache_root.to_string_lossy()
            ),
        )
        .unwrap();
        fs::write(
            dir.path().join(sub).join("f.R"),
            format!("library(configfixture)\nprint({name}())\n"),
        )
        .unwrap();
    }
    let output = run_cli_in_no_stdin(dir.path(), ["lint", "."]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(
        dir.path().join("a/f.R"),
        "library(configfixture)\nprint(from_b())\n",
    )
    .unwrap();
    let output = run_cli_in_no_stdin(dir.path(), ["lint", "--output", "json", "."]);
    assert_eq!(output.status.code(), Some(1));
    let findings: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(findings.as_array().unwrap().len(), 1);
}

#[test]
fn cli_nested_exclusions_do_not_hide_invalid_cli_overrides() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("R")).unwrap();
    fs::write(dir.path().join("R/arity.toml"), "exclude = [\"f.R\"]\n").unwrap();
    fs::write(dir.path().join("R/f.R"), "print(1)\n").unwrap();
    for args in [
        ["format", "--force-exclude", "--indent-width", "0", "R/f.R"],
        [
            "lint",
            "--force-exclude",
            "--select",
            "does-not-exist",
            "R/f.R",
        ],
    ] {
        let output = run_cli_in_no_stdin(dir.path(), args);
        assert_eq!(output.status.code(), Some(2), "{output:?}");
    }
}
