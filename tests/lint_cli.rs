//! CLI-level tests for `arity lint --fix`, exercising the built binary.
//!
//! The fix loop must see the same project scope the reporting pass does.
//! Anything less lets `--fix` delete a binding that `lint` deliberately never
//! reported — silent data loss, not a formatting difference.

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
