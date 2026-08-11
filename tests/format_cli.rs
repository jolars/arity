//! CLI-level tests for `arity format`, exercising the built binary.

use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn run_cli_no_stdin<const N: usize>(args: [&str; N]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_arity"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to run cli")
}

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

#[test]
fn cli_format_verify_formats_stdin() {
    let output = run_cli(["format", "--verify"], "if(x){y<-1+2}else{z<-3}\n");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "if (x) {\n  y <- 1 + 2\n} else {\n  z <- 3\n}\n"
    );
}

#[test]
fn cli_format_writes_single_file_in_place() {
    let dir = tempdir().expect("failed to create temp dir");
    let file = dir.path().join("in_place.R");
    std::fs::write(&file, "x<-1+2\n").expect("failed to write input file");

    let output = run_cli_no_stdin([
        "format",
        file.to_str().expect("temp file path should be utf-8"),
    ]);
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let after = std::fs::read_to_string(&file).expect("failed to read formatted file");
    assert_eq!(after, "x <- 1 + 2\n");
}

#[test]
fn cli_format_writes_directory_files_in_place() {
    let dir = tempdir().expect("failed to create temp dir");
    let a = dir.path().join("a.R");
    let b = dir.path().join("sub").join("b.R");
    std::fs::create_dir_all(b.parent().expect("subdir parent should exist"))
        .expect("failed to create nested dir");
    std::fs::write(&a, "x<-1+2\n").expect("failed to write a.R");
    std::fs::write(&b, "y<-3+4\n").expect("failed to write b.R");

    let output = run_cli_no_stdin([
        "format",
        dir.path().to_str().expect("temp dir path should be utf-8"),
    ]);
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        std::fs::read_to_string(&a).expect("failed to read a.R"),
        "x <- 1 + 2\n"
    );
    assert_eq!(
        std::fs::read_to_string(&b).expect("failed to read b.R"),
        "y <- 3 + 4\n"
    );
}

#[test]
fn cli_format_check_reports_changed_files() {
    let dir = tempdir().expect("failed to create temp dir");
    let changed = dir.path().join("changed.R");
    let unchanged = dir.path().join("unchanged.R");

    std::fs::write(&changed, "x<-1+2\n").expect("failed to write changed file");
    std::fs::write(&unchanged, "x <- 1 + 2\n").expect("failed to write unchanged file");

    let output = run_cli_no_stdin([
        "format",
        "--check",
        dir.path().to_str().expect("temp dir path should be utf-8"),
    ]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("changed.R"));
    assert!(!stdout.contains("unchanged.R"));
}

#[test]
fn cli_format_check_prints_diff() {
    let dir = tempdir().expect("failed to create temp dir");
    let changed = dir.path().join("changed.R");
    std::fs::write(&changed, "x<-1+2\n").expect("failed to write changed file");

    let output = run_cli_no_stdin([
        "format",
        "--check",
        changed.to_str().expect("temp file path should be utf-8"),
    ]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    // A unified-style diff: the original line removed, the formatted line added.
    assert!(stdout.contains("-x<-1+2"));
    assert!(stdout.contains("+x <- 1 + 2"));
    // No ANSI color when piped (not a terminal).
    assert!(!stdout.contains('\u{1b}'));
}

#[test]
fn cli_format_check_quiet_lists_files_without_the_diff() {
    let dir = tempdir().expect("failed to create temp dir");
    let changed = dir.path().join("changed.R");
    std::fs::write(&changed, "x<-1+2\n").expect("failed to write changed file");

    let output = run_cli_no_stdin([
        "format",
        "--check",
        "--quiet",
        changed.to_str().expect("temp file path should be utf-8"),
    ]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("would reformat"),
        "expected the file list, got:\n{stdout}"
    );
    assert!(
        stdout.contains("1 of 1 file(s) would be reformatted"),
        "expected the summary, got:\n{stdout}"
    );
    // The point of `--quiet`: the hunks are gone.
    assert!(
        !stdout.contains("-x<-1+2"),
        "diff should be suppressed, got:\n{stdout}"
    );
}

#[test]
fn cli_format_check_succeeds_for_unchanged_files() {
    let dir = tempdir().expect("failed to create temp dir");
    let unchanged = dir.path().join("unchanged.R");
    std::fs::write(&unchanged, "x <- 1 + 2\n").expect("failed to write unchanged file");

    let output = run_cli_no_stdin([
        "format",
        "--check",
        dir.path().to_str().expect("temp dir path should be utf-8"),
    ]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
}

#[test]
fn cli_format_check_requires_paths() {
    let output = run_cli_no_stdin(["format", "--check"]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--check requires at least one input path"));
}

#[test]
fn cli_format_check_disallows_verify() {
    let output = run_cli_no_stdin(["format", "--check", "--verify", "."]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--verify cannot be combined with --check"));
}

/// A roxygen block already in markdown-canonical form: an indented code block
/// that Rd-first formatting would reflow (stripping the indent) but markdown
/// mode preserves.
const MD_CANONICAL: &str = "#' Title\n#'\n#' @details\n#' Some prose before the code.\n#'\n#'     code_looking <- \"indented\"\n#' @param x an argument\nNULL\n";

/// Write a minimal R package at `root` whose DESCRIPTION carries `extra`
/// after the Package field, with `MD_CANONICAL` as `R/doc.R`.
fn write_package(root: &std::path::Path, description_extra: &str) -> std::path::PathBuf {
    std::fs::create_dir(root.join("R")).expect("R/");
    std::fs::write(
        root.join("DESCRIPTION"),
        format!("Package: p\n{description_extra}"),
    )
    .expect("DESCRIPTION");
    let file = root.join("R/doc.R");
    std::fs::write(&file, MD_CANONICAL).expect("doc.R");
    file
}

#[test]
fn cli_format_honors_package_markdown_default() {
    let dir = tempdir().expect("failed to create temp dir");
    let file = write_package(dir.path(), "Roxygen: list(markdown = TRUE)\n");

    let output = run_cli_no_stdin(["format", file.to_str().expect("utf-8 path")]);
    assert!(output.status.success());
    let after = std::fs::read_to_string(&file).expect("read back");
    assert_eq!(after, MD_CANONICAL, "markdown-first package is untouched");
}

#[test]
fn cli_format_without_markdown_default_reflows() {
    let dir = tempdir().expect("failed to create temp dir");
    let file = write_package(dir.path(), "");

    let output = run_cli_no_stdin(["format", file.to_str().expect("utf-8 path")]);
    assert!(output.status.success());
    let after = std::fs::read_to_string(&file).expect("read back");
    assert!(
        after.contains("#' code_looking"),
        "Rd-first package reflows the indent:\n{after}"
    );
}

#[test]
fn cli_format_check_honors_package_markdown_default() {
    let dir = tempdir().expect("failed to create temp dir");
    write_package(dir.path(), "Roxygen: list(markdown = TRUE)\n");

    let output = run_cli_no_stdin([
        "format",
        "--check",
        dir.path().to_str().expect("utf-8 path"),
    ]);
    assert!(
        output.status.success(),
        "markdown-first package is check-clean: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn cli_format_check_without_markdown_default_flags_reflow() {
    let dir = tempdir().expect("failed to create temp dir");
    write_package(dir.path(), "");

    let output = run_cli_no_stdin([
        "format",
        "--check",
        dir.path().to_str().expect("utf-8 path"),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("-#'     code_looking"));
    assert!(stdout.contains("+#' code_looking"));
}

/// The positional-input contract: `-` is the explicit stdin spelling, an
/// implicit (piped) stdin still works, and neither can be mixed with paths. The
/// gated case — no paths at an interactive terminal — is a usage error rather
/// than a silent wait; it needs a pty to reproduce, so the decision itself is
/// unit-tested in `main.rs` (`resolve_inputs`).
#[test]
fn cli_format_dash_reads_stdin() {
    let output = run_cli(["format", "-"], "x<-1+2\n");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "x <- 1 + 2\n");
}

#[test]
fn cli_lint_dash_reads_stdin() {
    // `T` for `TRUE` is a stock finding, so stdin reaching the linter is visible.
    let output = run_cli(["lint", "-"], "x <- T\n");
    assert!(!output.status.success(), "a finding should exit non-zero");
}

#[test]
fn cli_format_dash_cannot_be_mixed_with_paths() {
    let dir = tempdir().expect("failed to create temp dir");
    let file = dir.path().join("mixed.R");
    std::fs::write(&file, "x<-1\n").expect("failed to write input file");

    let output = run_cli_no_stdin([
        "format",
        "-",
        file.to_str().expect("temp file path should be utf-8"),
    ]);

    // Clap's own usage-error exit code, so the message reads like any other
    // argument mistake.
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot be combined with other paths"),
        "expected the conflict error, got:\n{stderr}"
    );
    // The named file must be left alone.
    assert_eq!(
        std::fs::read_to_string(&file).expect("failed to read file"),
        "x<-1\n"
    );
}

#[test]
fn cli_format_check_rejects_stdin() {
    let output = run_cli(["format", "--check", "-"], "x<-1\n");

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot read from stdin"),
        "`--check` reports on files it leaves on disk"
    );
}
