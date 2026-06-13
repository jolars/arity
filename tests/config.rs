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
