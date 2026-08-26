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

/// Like [`run_cli_no_stdin`], but from `dir` — config discovery anchors at the
/// working directory, so a test with its own `arity.toml` has to run there.
fn run_cli_in<const N: usize>(dir: &std::path::Path, args: [&str; N]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_arity"))
        .args(args)
        .current_dir(dir)
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
fn cli_format_check_reports_outdated_format_directive() {
    let dir = tempdir().expect("failed to create temp dir");
    let file = dir.path().join("outdated.R");
    std::fs::write(&file, "# arity-format skip: legacy workaround\nx <- 1\n")
        .expect("failed to write input file");

    let output = run_cli_no_stdin([
        "format",
        "--check",
        file.to_str().expect("temp file path should be utf-8"),
    ]);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    assert!(stdout.contains("outdated.R:1:1"), "{stdout}");
    assert!(stdout.contains("outdated format directive"), "{stdout}");
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

// --- DESCRIPTION ---------------------------------------------------------

/// A package whose `DESCRIPTION` is unformatted and whose `R/` file is already
/// canonical, so a walk's effect on the metadata file is unambiguous.
fn write_unformatted_package(root: &std::path::Path) -> std::path::PathBuf {
    std::fs::create_dir(root.join("R")).expect("R/");
    let description = root.join("DESCRIPTION");
    std::fs::write(&description, "Imports: b, a\nPackage: p\n").expect("DESCRIPTION");
    std::fs::write(root.join("R/ok.R"), "x <- 1\n").expect("ok.R");
    description
}

const FORMATTED_DESCRIPTION: &str = "Package: p\nImports:\n    a,\n    b\n";

#[test]
fn cli_format_writes_a_walked_description() {
    let dir = tempdir().expect("failed to create temp dir");
    let description = write_unformatted_package(dir.path());

    let output = run_cli_no_stdin(["format", dir.path().to_str().expect("utf-8 path")]);
    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(&description).expect("read back"),
        FORMATTED_DESCRIPTION
    );
}

#[test]
fn cli_format_writes_an_explicitly_named_description() {
    let dir = tempdir().expect("failed to create temp dir");
    let description = write_unformatted_package(dir.path());

    let output = run_cli_no_stdin(["format", description.to_str().expect("utf-8 path")]);
    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(&description).expect("read back"),
        FORMATTED_DESCRIPTION
    );
}

#[test]
fn cli_format_skips_a_nested_packages_description_on_a_walk() {
    // A package under a package is fixture data — often asserted on byte for
    // byte by its own project's tests. Naming it still formats it.
    let dir = tempdir().expect("failed to create temp dir");
    write_unformatted_package(dir.path());
    let nested = dir.path().join("tests/testthat/testpkg");
    std::fs::create_dir_all(nested.join("R")).expect("nested R/");
    let nested_description = nested.join("DESCRIPTION");
    std::fs::write(&nested_description, "Imports: b, a\nPackage: inner\n").expect("nested");

    let output = run_cli_no_stdin(["format", dir.path().to_str().expect("utf-8 path")]);
    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(&nested_description).expect("read back"),
        "Imports: b, a\nPackage: inner\n",
        "a walk must leave a nested package's DESCRIPTION alone"
    );

    let output = run_cli_no_stdin(["format", nested_description.to_str().expect("utf-8 path")]);
    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(&nested_description).expect("read back"),
        "Package: inner\nImports:\n    a,\n    b\n"
    );
}

#[test]
fn cli_format_check_reports_an_unformatted_description() {
    let dir = tempdir().expect("failed to create temp dir");
    let description = write_unformatted_package(dir.path());

    let output = run_cli_no_stdin([
        "format",
        "--check",
        dir.path().to_str().expect("utf-8 path"),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    assert!(stdout.contains("Diff in"));
    assert_eq!(
        std::fs::read_to_string(&description).expect("read back"),
        "Imports: b, a\nPackage: p\n",
        "--check must not write"
    );
}

#[test]
fn cli_format_verify_accepts_a_description_path() {
    let dir = tempdir().expect("failed to create temp dir");
    let description = write_unformatted_package(dir.path());

    let output = run_cli_no_stdin([
        "format",
        "--verify",
        description.to_str().expect("utf-8 path"),
    ]);
    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(&description).expect("read back"),
        "Imports: b, a\nPackage: p\n",
        "--verify must not write"
    );
}

#[test]
fn cli_format_leaves_a_description_alone_when_the_config_says_so() {
    let dir = tempdir().expect("failed to create temp dir");
    let description = write_unformatted_package(dir.path());
    std::fs::write(
        dir.path().join("arity.toml"),
        "[format]\ndescription = false\n",
    )
    .expect("arity.toml");

    let output = run_cli_in(dir.path(), ["format", "."]);
    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(&description).expect("read back"),
        "Imports: b, a\nPackage: p\n"
    );

    // Naming it explicitly now reports why, rather than silently doing nothing.
    let output = run_cli_in(dir.path(), ["format", "DESCRIPTION"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).expect("utf-8");
    assert!(stderr.contains("[format] description = false"), "{stderr}");
}

#[test]
fn cli_format_refuses_a_description_it_cannot_restyle_safely() {
    let dir = tempdir().expect("failed to create temp dir");
    let description = write_unformatted_package(dir.path());
    // Two records: valid DCF, not a DESCRIPTION.
    std::fs::write(&description, "Package: p\n\nPackage: q\n").expect("write");

    let output = run_cli_no_stdin(["format", description.to_str().expect("utf-8 path")]);
    assert!(output.status.success(), "a decline is not a failure");
    assert_eq!(
        std::fs::read_to_string(&description).expect("read back"),
        "Package: p\n\nPackage: q\n"
    );
}

/// A file arity cannot format must not decide the fate of the ones it can.
///
/// `merge` sorts by path, so a package's `DESCRIPTION` sorts before its `R/`:
/// returning on the first failure would let one unparseable metadata file
/// deterministically preempt every source file the user actually asked about.
#[test]
fn cli_format_keeps_going_past_an_unformattable_description() {
    let dir = tempdir().expect("failed to create temp dir");
    let pkg = dir.path().join("pkg");
    std::fs::create_dir_all(pkg.join("R")).expect("create dirs");
    std::fs::write(pkg.join("DESCRIPTION"), "Package: p\ngarbage line\n").expect("write");
    std::fs::write(pkg.join("R").join("a.R"), "x<-1\n").expect("write");
    std::fs::write(pkg.join("R").join("z.R"), "y<-2\n").expect("write");

    let output = run_cli_in(dir.path(), ["format", "--no-config", "."]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "the failure is still reported: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(pkg.join("R").join("a.R")).expect("read back"),
        "x <- 1\n"
    );
    assert_eq!(
        std::fs::read_to_string(pkg.join("R").join("z.R")).expect("read back"),
        "y <- 2\n"
    );
}

/// A `DESCRIPTION` whose bytes are not UTF-8 is skipped, exactly as
/// `arity lint` skips it — not a failure, and not a reason to stop.
#[test]
fn cli_format_skips_a_description_it_cannot_decode() {
    let dir = tempdir().expect("failed to create temp dir");
    let pkg = dir.path().join("pkg");
    std::fs::create_dir_all(pkg.join("R")).expect("create dirs");
    std::fs::write(
        pkg.join("DESCRIPTION"),
        b"Package: p\nEncoding: latin1\nAuthor: Jos\xe9\n",
    )
    .expect("write");
    std::fs::write(pkg.join("R").join("a.R"), "x<-1\n").expect("write");

    let output = run_cli_in(dir.path(), ["format", "--no-config", "."]);
    assert!(
        output.status.success(),
        "an undecodable file is skipped, not fatal: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("skipped"),
        "the skip must be reported"
    );
    assert_eq!(
        std::fs::read_to_string(pkg.join("R").join("a.R")).expect("read back"),
        "x <- 1\n"
    );
}

#[test]
fn cli_format_stdin_is_r_unless_the_filename_says_otherwise() {
    // Valid under both grammars — as R it is two `:` expressions — so only
    // `--stdin-filename` can decide which one the buffer is.
    let source = "Package: p\nImports: b\n";

    let output = run_cli(["format", "-"], source);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf-8"),
        "Package:p\nImports:b\n",
        "an unnamed buffer is R"
    );

    let output = run_cli(["format", "--stdin-filename", "DESCRIPTION", "-"], source);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf-8"),
        "Package: p\nImports:\n    b\n"
    );
}

/// `[format] description = false` reaches the stdin door too. Stdin with
/// `--stdin-filename` is the shape editors and pre-commit hooks use — the
/// integrations most likely to need the off switch — and falling through to the
/// R formatter would produce exactly the corruption the key exists to prevent.
#[test]
fn cli_format_stdin_description_honors_the_off_switch() {
    let dir = tempdir().expect("failed to create temp dir");
    std::fs::write(
        dir.path().join("arity.toml"),
        "[format]\ndescription = false\n",
    )
    .expect("write config");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_arity"));
    cmd.args(["format", "--stdin-filename", "DESCRIPTION", "-"])
        .current_dir(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("failed to spawn arity cli");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"Package: p\nImports: b, a\n")
        .expect("write stdin");
    let output = child.wait_with_output().expect("failed to wait for cli");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf-8"),
        "Package: p\nImports: b, a\n",
        "the buffer must come back untouched, not reflowed as R"
    );
}
