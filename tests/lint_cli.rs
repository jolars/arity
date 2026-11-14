//! CLI-level tests for lint output and `arity lint --fix`, exercising the built binary.
//!
//! The fix loop must see the same project scope the reporting pass does.
//! Anything less lets `--fix` delete a binding that `lint` deliberately never
//! reported — silent data loss, not a formatting difference.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use tempfile::tempdir;

const TEST_DESCRIPTION: &str = "Package: testpkg\n\
     Version: 0.1.0\n\
     Title: Test\n\
     Description: A test package.\n\
     Author: A B\n\
     Maintainer: A B <a@b.com>\n\
     License: MIT\n";

fn run_lint<const N: usize>(args: [&str; N]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_arity"))
        .arg("lint")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to run cli")
}

fn run_lint_stdin(args: &[&str], input: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_arity"))
        .arg("lint")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run cli");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn clean_json_files_and_directories_emit_empty_array() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("clean.R");
    std::fs::write(&path, "print(1)\n").unwrap();
    for target in [path.as_path(), dir.path()] {
        for verbosity in ["--quiet", "--verbose"] {
            let output = run_lint([
                "--no-config",
                "--select",
                "seq",
                "--output",
                "json",
                verbosity,
                target.to_str().unwrap(),
            ]);
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            assert_eq!(output.stdout, b"[]\n", "{output:?}");
            if verbosity == "--verbose" {
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains("no findings"),
                    "{output:?}"
                );
            } else {
                assert!(output.stderr.is_empty(), "{output:?}");
            }
        }
    }
}

#[test]
fn clean_json_stdin_emits_empty_array() {
    // Both explicit and implicit stdin, including an empty buffer.
    for input in ["print(1)\n", ""] {
        for explicit in [false, true] {
            let mut args = vec!["--no-config", "--select", "seq", "--output", "json"];
            if explicit {
                args.push("-");
            }
            let output = run_lint_stdin(&args, input);
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            assert_eq!(output.stdout, b"[]\n", "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
        }
    }
}

#[test]
fn json_stdin_findings_preserve_exit_code_and_routing() {
    let output = run_lint_stdin(
        &["--no-config", "--select", "seq", "--output", "json", "-"],
        "1:length(x)\n",
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let findings: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(findings.as_array().unwrap().len(), 1);
    assert_eq!(findings[0]["rule"], "seq");
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn json_fix_reports_empty_array_after_clearing_last_finding() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("fix.R");
    std::fs::write(&path, "1:length(x)\n").unwrap();
    let args = [
        "--no-config",
        "--select",
        "seq",
        "--output",
        "json",
        path.to_str().unwrap(),
    ];
    let before = run_lint(args);
    assert_eq!(before.status.code(), Some(1), "{before:?}");
    let findings: serde_json::Value = serde_json::from_slice(&before.stdout).unwrap();
    assert_eq!(findings.as_array().unwrap().len(), 1);
    assert_eq!(findings[0]["rule"], "seq");
    assert!(before.stderr.is_empty(), "{before:?}");

    let output = run_lint([
        "--no-config",
        "--select",
        "seq",
        "--output",
        "json",
        "--fix",
        path.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"[]\n", "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("1 fix applied"),
        "{output:?}"
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), "seq_along(x)\n");
}

#[test]
fn stdin_fix_rejects_json_output() {
    for explicit in [false, true] {
        // Reject both clean input and input with findings.
        for input in ["print(1)\n", "1:length(x)\n"] {
            let mut args = vec![
                "--no-config",
                "--select",
                "seq",
                "--fix",
                "--output",
                "json",
            ];
            if explicit {
                args.push("-");
            }
            let output = run_lint_stdin(&args, input);
            assert_eq!(output.status.code(), Some(2), "{output:?}");
            assert!(output.stdout.is_empty(), "{output:?}");
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("--fix cannot be combined with --output json when reading stdin"),
                "{output:?}"
            );
        }
    }
}

#[test]
fn human_stdin_output_preserves_source_and_diagnostic_routing() {
    for format in ["pretty", "concise"] {
        for fix in [false, true] {
            let mut args = vec!["--no-config", "--select", "seq", "--output", format, "-"];
            if fix {
                args.push("--fix");
            }
            let output = run_lint_stdin(&args, "1:length(x)\n");
            if fix {
                assert_eq!(output.status.code(), Some(0), "{output:?}");
                assert_eq!(output.stdout, b"seq_along(x)\n", "{output:?}");
                assert!(output.stderr.is_empty(), "{output:?}");
            } else {
                assert_eq!(output.status.code(), Some(1), "{output:?}");
                assert!(output.stdout.is_empty(), "{output:?}");
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains("seq"),
                    "{output:?}"
                );
            }
            let clean = run_lint_stdin(&args, "print(1)\n");
            assert_eq!(clean.status.code(), Some(0), "{clean:?}");
            assert_eq!(
                clean.stdout,
                if fix { b"print(1)\n".as_slice() } else { b"" },
                "{clean:?}"
            );
            assert!(clean.stderr.is_empty(), "{clean:?}");
        }
    }
}

#[test]
fn invalid_unicode_reports_syntax_errors_before_lint() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("unicode.R");
    for (src, point) in [
        ("с <- 1\nx\u{200b} <- 1\n", "U+200B"),
        ("с <- 1\nx\u{a0}<- 1\n", "U+00A0"),
        ("с <- 1\nx <- 1\u{2212}2\n", "U+2212"),
    ] {
        std::fs::write(&path, src).unwrap();
        let output = run_lint([
            "--no-config",
            "--select",
            "confusable-identifier",
            "--output",
            "concise",
            path.to_str().unwrap(),
        ]);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let rendered = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(rendered.contains("syntax-error"), "{rendered}");
        assert!(rendered.contains(point), "{rendered}");
        assert!(!rendered.contains("confusable-identifier"), "{rendered}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), src);
    }
}

/// Write a package whose `R/` holds `files`, returning the temp dir.
fn write_package(namespace: &str, files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempdir().expect("failed to create temp dir");
    std::fs::write(dir.path().join("DESCRIPTION"), TEST_DESCRIPTION).unwrap();
    std::fs::write(dir.path().join("NAMESPACE"), namespace).unwrap();
    let r_dir = dir.path().join("R");
    std::fs::create_dir(&r_dir).unwrap();
    for (name, src) in files {
        std::fs::write(r_dir.join(name), src).unwrap();
    }
    dir
}

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join("R").join(name)).expect("file should still exist")
}

#[test]
fn fix_does_not_delete_a_namespace_exported_binding() {
    // `lint` never reports an exported binding as unused, so `--fix` must not
    // delete it. Before the fix loop built project scope it deleted the whole
    // package.
    let dir = write_package(
        "export(exported_fn)\n",
        &[("a.R", "exported_fn <- function(x) {\n  x + 1\n}\n")],
    );
    let out = run_lint(["--fix", "--unsafe-fixes", dir.path().to_str().unwrap()]);
    assert!(
        out.status.success(),
        "lint --fix failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        read(dir.path(), "a.R").contains("exported_fn <- function"),
        "exported binding was deleted: {:?}",
        read(dir.path(), "a.R")
    );
}

#[test]
fn fix_does_not_delete_a_binding_read_by_a_sibling() {
    // Cross-file use is the other project-scope signal `--fix` has to honor.
    let dir = write_package(
        "export(caller)\n",
        &[
            ("a.R", "helper <- function(x) {\n  x + 1\n}\n"),
            ("b.R", "caller <- function() {\n  helper(1)\n}\n"),
        ],
    );
    let out = run_lint(["--fix", "--unsafe-fixes", dir.path().to_str().unwrap()]);
    assert!(out.status.success());
    assert!(
        read(dir.path(), "a.R").contains("helper <- function"),
        "sibling-read binding was deleted: {:?}",
        read(dir.path(), "a.R")
    );
}

#[test]
fn fix_still_deletes_a_genuinely_unused_binding() {
    // The exemptions must not disarm the rule: a private, unread, unexported
    // binding is still deleted under `--unsafe-fixes`.
    let dir = write_package(
        "export(kept)\n",
        &[(
            "a.R",
            "kept <- function() 1\n\ndead_helper <- function(x) {\n  x + 1\n}\n",
        )],
    );
    let out = run_lint(["--fix", "--unsafe-fixes", dir.path().to_str().unwrap()]);
    assert!(out.status.success());
    let after = read(dir.path(), "a.R");
    assert!(
        !after.contains("dead_helper"),
        "genuinely dead binding survived: {after:?}"
    );
    assert!(after.contains("kept <- function"), "{after:?}");
}
