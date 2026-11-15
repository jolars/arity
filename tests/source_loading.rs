use std::path::{Path, PathBuf};

use arity::incremental::{IncrementalDatabase, QueryKind, source_edges};
use arity::project::{ReadBinding, SourceTarget, project_graph, reverse_source_edges};

const TEST_DESCRIPTION: &str = "\
Package: testpkg
Version: 0.1.0
Title: A Test Package
Description: Fixture data for arity's own tests.
License: MIT + file LICENSE
Authors@R: person(\"Test\", \"User\", email = \"test@example.com\", role = c(\"aut\", \"cre\"))
";

fn write(root: &Path, path: &str, text: &str) -> PathBuf {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}

fn workspace(root: &Path, files: &[(&str, &str)]) -> IncrementalDatabase {
    let mut db = IncrementalDatabase::default();
    let members = files
        .iter()
        .map(|(path, text)| db.upsert_file(&write(root, path, text), *text))
        .collect();
    db.set_workspace_members(members, vec![root.to_path_buf()]);
    db
}

#[test]
fn source_directory_uses_package_git_config_then_script_directory() {
    for marker in ["package", "git-directory", "git-file", "config", "none"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join("scripts/tests")).unwrap();
        match marker {
            "package" => {
                write(&root, "DESCRIPTION", TEST_DESCRIPTION);
                std::fs::create_dir(root.join("R")).unwrap();
                // The package root outranks both closer markers.
                std::fs::create_dir(root.join("scripts/.git")).unwrap();
                write(&root, "scripts/tests/arity.toml", "");
            }
            "git-directory" | "git-file" => {
                if marker == "git-directory" {
                    std::fs::create_dir(root.join(".git")).unwrap();
                } else {
                    write(&root, ".git", "gitdir: /external/worktree\n");
                }
                write(&root, "scripts/tests/arity.toml", "");
            }
            "config" => {
                write(&root, "arity.toml", "");
                write(&root, "scripts/arity.toml", "");
            }
            _ => {}
        }
        let mut db = IncrementalDatabase::default();
        let path = root.join("scripts/tests/entry.R");
        let file = db.upsert_file(&path, "source('helper.R')\n");
        let base = match marker {
            "config" => root.join("scripts"),
            "none" => root.join("scripts/tests"),
            _ => root.clone(),
        };
        assert_eq!(
            source_edges(&db, file)[0].target,
            SourceTarget::Path(base.join("helper.R")),
            "{marker}",
        );
    }
}

#[test]
fn source_paths_inherit_the_callers_directory_and_restore_after_chdir() {
    for chdir in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".git", "gitdir: /external/worktree\n");
        let entry = format!(
            "source('sub/loader.R', chdir = {flag})\nfoo()\nsource('tail.R')\ntail_value\n",
            flag = if chdir { "TRUE" } else { "FALSE" },
        );
        // Both alternatives exist, so a wrong directory cannot hide behind
        // the conservative handling of an unanalyzed source.
        let db = workspace(
            dir.path(),
            &[
                ("scripts/entry.R", &entry),
                ("sub/loader.R", "source('helper.R')\nfoo()\n"),
                ("helper.R", "foo <- function() 1L\n"),
                ("sub/helper.R", "foo <- function() 2L\n"),
                ("tail.R", "tail_value <- 3L\n"),
                ("sub/tail.R", "wrong_tail <- 4L\n"),
            ],
        );
        let path = dir.path().join("scripts/entry.R");
        let helper = dir
            .path()
            .join(if chdir { "sub/helper.R" } else { "helper.R" });
        let other = dir
            .path()
            .join(if chdir { "helper.R" } else { "sub/helper.R" });
        let graph = project_graph(&db);
        assert!(graph.sees(&path).contains(&helper), "chdir={chdir}");
        assert!(!graph.sees(&path).contains(&other), "chdir={chdir}");
        assert!(
            !graph.for_file(&path).resolution_incomplete,
            "chdir={chdir}"
        );
        assert_eq!(
            graph.top_level_read_binding(&path, "foo"),
            ReadBinding::Resolved(helper.clone()),
            "chdir={chdir}",
        );
        assert!(graph.for_file(&helper).read_elsewhere("foo"));
        assert!(graph.sees(&path).contains(&dir.path().join("tail.R")));
        assert!(!graph.sees(&path).contains(&dir.path().join("sub/tail.R")));
        let reverse = reverse_source_edges(&db);
        assert!(reverse.sourced_by[&helper].contains(&dir.path().join("sub/loader.R")));
    }
}

#[test]
fn script_directory_fallback_is_inherited_across_source_calls() {
    let dir = tempfile::tempdir().unwrap();
    let db = workspace(
        dir.path(),
        &[
            ("entry.R", "source('sub/loader.R')\nfoo()\n"),
            ("sub/loader.R", "source('helper.R')\n"),
            ("helper.R", "foo <- function() 1L\n"),
            ("sub/helper.R", "other <- function() 2L\n"),
        ],
    );
    let graph = project_graph(&db);
    let entry = dir.path().join("entry.R");
    assert_eq!(
        graph.top_level_read_binding(&entry, "foo"),
        ReadBinding::Resolved(dir.path().join("helper.R")),
    );
    assert!(
        !graph
            .sees(&entry)
            .contains(&dir.path().join("sub/helper.R"))
    );
}

#[test]
fn source_directory_refresh_preserves_the_parse_and_unchanged_graph() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = workspace(
        dir.path(),
        &[
            ("scripts/entry.R", "source('helper.R')\nfoo()\n"),
            ("helper.R", "foo <- function() 1L\n"),
        ],
    );
    let entry = dir.path().join("scripts/entry.R");
    assert!(
        !project_graph(&db)
            .sees(&entry)
            .contains(&dir.path().join("helper.R"))
    );
    write(dir.path(), "arity.toml", "");
    db.refresh_package_graph();
    assert!(
        project_graph(&db)
            .sees(&entry)
            .contains(&dir.path().join("helper.R"))
    );

    db.clear_query_log();
    assert!(!db.refresh_package_graph());
    let _ = project_graph(&db);
    assert!(db.query_log().iter().all(|entry| !matches!(
        entry.kind,
        QueryKind::ParsedDocument | QueryKind::ProjectGraph
    )));
}

#[test]
fn cli_source_paths_use_the_git_root_despite_a_nested_config() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".git", "gitdir: /external/worktree\n");
    write(
        dir.path(),
        "arity.toml",
        "[lint]\nselect = ['unused-binding', 'undefined-symbol']\n",
    );
    write(dir.path(), "tests/arity.toml", "");
    write(dir.path(), "tests/entry.R", "source('helper.R')\nfoo()\n");
    write(
        dir.path(),
        "helper.R",
        "foo <- function() 42L\ndead <- 1L\n",
    );
    for paths in [vec!["."], vec!["tests/entry.R", "helper.R"]] {
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_arity"))
            .current_dir(dir.path())
            .args([
                "lint",
                "--output",
                "json",
                "--select",
                "unused-binding,undefined-symbol",
            ])
            .args(paths)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1));
        let diagnostics: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        let diagnostics = diagnostics.as_array().unwrap();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(
            diagnostics[0]["message"]["body"]
                .as_str()
                .unwrap()
                .contains("dead")
        );
    }
}

#[test]
fn source_file_rename_keeps_literals_relative_to_the_execution_root() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".git", "gitdir: /external/worktree\n");
    let db = workspace(
        dir.path(),
        &[
            ("scripts/entry.R", "source('lib/helper.R')\nfoo()\n"),
            ("lib/helper.R", "foo <- function() 42L\n"),
        ],
    );
    let snapshot = db.snapshot();
    let edits = snapshot.source_rename_edits(&[(
        dir.path().join("lib/helper.R"),
        dir.path().join("lib/renamed.R"),
    )]);
    assert_eq!(edits.len(), 1, "{edits:?}");
    assert_eq!(edits[0].2, "'lib/renamed.R'");
    assert!(
        snapshot
            .source_rename_edits(&[(
                dir.path().join("scripts/entry.R"),
                dir.path().join("other/entry.R"),
            )])
            .is_empty()
    );
}

#[test]
fn source_file_rename_withholds_conflicting_replacements_across_contexts() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".git", "gitdir: /external/worktree\n");
    let db = workspace(
        dir.path(),
        &[
            ("entry.R", "source('sub/loader.R', chdir = TRUE)\nfoo()\n"),
            ("sub/loader.R", "source('helper.R')\nfoo()\n"),
            ("helper.R", "foo <- function() 1L\n"),
            ("sub/helper.R", "foo <- function() 2L\n"),
        ],
    );
    assert_eq!(
        project_graph(&db).top_level_read_binding(&dir.path().join("sub/loader.R"), "foo"),
        ReadBinding::OrderUnknown,
    );
    assert!(
        db.snapshot()
            .source_rename_edits(&[(
                dir.path().join("sub/helper.R"),
                dir.path().join("sub/renamed.R"),
            )])
            .is_empty()
    );
    let edits = db.snapshot().source_rename_edits(&[
        (dir.path().join("helper.R"), dir.path().join("renamed.R")),
        (
            dir.path().join("sub/helper.R"),
            dir.path().join("sub/renamed.R"),
        ),
    ]);
    assert_eq!(edits.len(), 1, "{edits:?}");
    assert_eq!(edits[0].2, "'renamed.R'");
}

#[test]
fn source_chdir_uses_r_argument_matching() {
    for arguments in [
        "chdir = TRUE",
        "chd = TRUE",
        "FALSE, FALSE, FALSE, , FALSE, FALSE, '> ', 150L, 60L, 'showAttributes', TRUE",
    ] {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".git", "gitdir: /external/worktree\n");
        let entry = format!("source('sub/loader.R', {arguments})\nfoo()\n");
        let db = workspace(
            dir.path(),
            &[
                ("entry.R", &entry),
                ("sub/loader.R", "source('helper.R')\n"),
                ("helper.R", "other <- 1L\n"),
                ("sub/helper.R", "foo <- function() 2L\n"),
            ],
        );
        assert_eq!(
            project_graph(&db).top_level_read_binding(&dir.path().join("entry.R"), "foo"),
            ReadBinding::Resolved(dir.path().join("sub/helper.R")),
            "{arguments}",
        );
    }
}
