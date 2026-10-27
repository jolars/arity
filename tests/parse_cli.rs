use std::{
    io::Write,
    process::{Command, Output, Stdio},
};

fn run(args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_arity"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn rd_stdin_can_select_its_grammar_and_verify() {
    let output = run(
        &["parse", "-", "--stdin-filename", "man/foo.Rd", "--verify"],
        "\\title{Foo}\r\n",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tree = String::from_utf8(output.stdout).unwrap();
    assert!(tree.contains("MACRO_NAME"));
    assert!(tree.contains("\\\\title"));
    assert!(output.stderr.is_empty());

    let quiet = run(
        &["parse", "--stdin-filename", "foo.rd", "--verify", "--quiet"],
        "\\title{Foo}",
    );
    assert!(quiet.status.success());
    assert!(quiet.stdout.is_empty());
    assert!(quiet.stderr.is_empty());
}

#[test]
fn rd_paths_select_rd_case_insensitively() {
    let directory = tempfile::tempdir().unwrap();
    for filename in ["foo.Rd", "foo.rd", "foo.RD"] {
        let path = directory.path().join(filename);
        std::fs::write(&path, "\\description{Text.}").unwrap();
        let output = run(&["parse", path.to_str().unwrap(), "--verify"], "");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("MACRO_NAME"));
    }
}

#[test]
fn rd_errors_keep_the_tree_and_use_stderr_and_exit_one() {
    let output = run(
        &["parse", "-", "--stdin-filename", "foo.Rd", "--verify"],
        "\\title{unfinished",
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("MACRO_NAME"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("error[6..7]: unclosed brace"));

    let quiet = run(&["parse", "--stdin-filename", "foo.Rd", "--quiet"], "}");
    assert_eq!(quiet.status.code(), Some(1));
    assert!(quiet.stdout.is_empty());
    assert!(!quiet.stderr.is_empty());
}

#[test]
fn unnamed_stdin_remains_r_and_input_errors_use_exit_two() {
    let r = run(&["parse", "-", "--verify"], "x <- 1\n");
    assert!(r.status.success());
    assert!(String::from_utf8_lossy(&r.stdout).contains("ASSIGN"));
    let rd_as_r = run(&["parse", "-", "--quiet"], "\\title{Foo}");
    assert_eq!(rd_as_r.status.code(), Some(1));
    let missing = run(&["parse", "/arity-nonexistent-directory/foo.Rd"], "");
    assert_eq!(missing.status.code(), Some(2));
    assert!(missing.stdout.is_empty());
    let conflict = run(&["parse", "-", "foo.Rd"], "");
    assert_eq!(conflict.status.code(), Some(2));
}
