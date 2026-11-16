use std::path::Path;

fn findings(
    root: &Path,
    files: &[(&str, &str)],
) -> Vec<(String, String, usize, std::path::PathBuf)> {
    for (name, text) in files {
        std::fs::create_dir_all(root.join(name).parent().unwrap()).unwrap();
        std::fs::write(root.join(name), text).unwrap();
    }
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_arity"))
        .current_dir(root)
        .args([
            "--no-config",
            "lint",
            "--select",
            "unused-binding,undefined-symbol",
            "--output",
            "json",
            ".",
        ])
        .output()
        .unwrap();
    assert!(matches!(result.status.code(), Some(0 | 1)), "{:?}", result);
    let diagnostics: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    diagnostics
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["rule"].as_str().unwrap().to_string(),
                d["message"]["body"].as_str().unwrap().to_string(),
                d["range"]["start"].as_u64().unwrap() as usize,
                d["path"].as_str().unwrap().into(),
            )
        })
        .collect()
}

#[test]
fn literal_sources_respect_the_destination_environment() {
    for main in [
        "{ source('helper.R'); helper() }\n",
        "{ source('helper.R', local = TRUE) }; helper()\n",
        "run <- function() { source('helper.R', local = TRUE); helper() }; run()\n",
        "helpers <- new.env(parent = baseenv()); source('helper.R', local = helpers); helpers$helper()\n",
        "helpers <- new.env(parent = baseenv()); sys.source('helper.R', envir = helpers); helpers$helper()\n",
        "base::source('helper.R', local = environment()); helper()\n",
        "base::sys.source('helper.R', envir = environment()); helper()\n",
        "helpers <- new.env(parent = baseenv()); sys.source('helper.R', envir = (helpers)); helpers$helper()\n",
        "if (TRUE) source('helper.R'); helper()\n",
        "source('helper.R', local = F); helper()\n",
        "source('helper.R', local = (F)); helper()\n",
        "run <- function() { source('helper.R', local = T); helper() }; run()\n",
        "run <- function() { on.exit(helper()); source('helper.R', local = TRUE) }; run()\n",
        "run <- function(x = helper()) { source('helper.R', local = TRUE); x }; run()\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let actual = findings(
            dir.path(),
            &[
                ("main.R", main),
                ("helper.R", "helper <- function() 42L\ndead <- 1L\n"),
            ],
        );
        assert_eq!(actual.len(), 1, "{main}: {actual:?}");
        assert!(actual[0].1.contains("`dead`"), "{main}: {actual:?}");
    }
}

#[test]
fn helper_environments_do_not_supply_bare_names_or_other_environments() {
    for loader in [
        "source('helper.R', local = helpers)",
        "sys.source('helper.R', envir = helpers)",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let main = format!(
            "helpers <- new.env(); other <- new.env(); {loader}; other$helper(); helper()\n"
        );
        let actual = findings(
            dir.path(),
            &[
                ("main.R", &main),
                ("helper.R", "helper <- function() 42L\n"),
            ],
        );
        assert_eq!(actual.len(), 2, "{actual:?}");
        assert!(
            actual
                .iter()
                .any(|d| d.0 == "unused-binding" && d.1.contains("`helper`")),
            "{actual:?}"
        );
        assert!(
            actual
                .iter()
                .any(|d| d.0 == "undefined-symbol" && d.1.contains("`helper`")),
            "{actual:?}"
        );
    }
}

#[test]
fn function_local_sources_do_not_populate_the_file_scope() {
    let dir = tempfile::tempdir().unwrap();
    let actual = findings(
        dir.path(),
        &[
            (
                "main.R",
                "run <- function() { source('helper.R', local = TRUE); helper() }; run(); helper()\n",
            ),
            ("helper.R", "helper <- function() 42L\n"),
        ],
    );
    assert_eq!(actual.len(), 1, "{actual:?}");
    assert_eq!(actual[0].0, "undefined-symbol");
    assert!(actual[0].1.contains("`helper`"));
}

#[test]
fn source_order_applies_inside_blocks_and_functions() {
    for main in [
        "{ helper(); source('helper.R'); helper() }\n",
        "run <- function() { helper(); source('helper.R', local = TRUE); helper() }; run()\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let actual = findings(
            dir.path(),
            &[("main.R", main), ("helper.R", "helper <- function() 42L\n")],
        );
        assert_eq!(actual.len(), 1, "{main}: {actual:?}");
        assert_eq!(actual[0].0, "undefined-symbol");
        assert_eq!(actual[0].2, main.find("helper()").unwrap());
    }
}

#[test]
fn uninvoked_loaders_do_not_establish_globals() {
    for main in [
        "run <- function() source('helper.R'); print(run); helper()\n",
        "quote(source('helper.R')); helper()\n",
        "identity(source('helper.R')); helper()\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let actual = findings(
            dir.path(),
            &[("main.R", main), ("helper.R", "helper <- function() 42L\n")],
        );
        assert!(
            actual
                .iter()
                .any(|d| d.0 == "undefined-symbol" && d.1.contains("`helper`")),
            "{main}: {actual:?}"
        );
    }
}

#[test]
fn conditional_loads_remain_possible_without_becoming_unconditional() {
    use arity::incremental::IncrementalDatabase;
    use arity::project::project_graph;
    use arity::project::source_env::SourceRead;
    for main in [
        "if (interactive()) source('helper.R'); helper(); unrelated_typo()\n",
        "interactive() && source('helper.R'); helper(); unrelated_typo()\n",
        "interactive() || source('helper.R'); helper(); unrelated_typo()\n",
        "if (interactive()) source <- function(file) NULL; source('helper.R'); helper(); unrelated_typo()\n",
        "if (interactive()) sys.source <- function(...) NULL; sys.source('helper.R', globalenv()); helper(); unrelated_typo()\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let helper = "helper <- function() 42L\n";
        let actual = findings(dir.path(), &[("main.R", main), ("helper.R", helper)]);
        assert_eq!(actual.len(), 1, "{main}: {actual:?}");
        assert!(actual[0].1.contains("`unrelated_typo`"));
        let mut db = IncrementalDatabase::default();
        let entry_path = dir.path().join("main.R");
        let helper_path = dir.path().join("helper.R");
        let entry = db.upsert_file(&entry_path, main);
        let helper = db.upsert_file(&helper_path, helper);
        db.set_workspace_members(vec![entry, helper], vec![dir.path().to_path_buf()]);
        let scope = project_graph(&db).for_file(&entry_path);
        assert!(!scope.resolves("helper"));
        let reads = &scope.source_reads().unwrap().reads;
        assert!(
            reads
                .iter()
                .any(|(key, state)| key.name == "helper" && *state == SourceRead::Unknown)
        );
        assert!(
            db.snapshot()
                .cross_file_binding(&helper_path, "helper")
                .source_environment_risk
        );
    }
}

#[test]
fn nested_sources_keep_directories_separate_from_destinations() {
    let dir = tempfile::tempdir().unwrap();
    let actual = findings(
        dir.path(),
        &[
            (
                "main.R",
                "helpers <- new.env(); sys.source('sub/loader.R', helpers, chdir = TRUE); helpers$helper(); global_helper(); helper()\n",
            ),
            (
                "sub/loader.R",
                "source('helper.R', local = TRUE); source('../global.R')\n",
            ),
            ("sub/helper.R", "helper <- function() 42L\n"),
            ("global.R", "global_helper <- function() 42L\n"),
        ],
    );
    assert_eq!(actual.len(), 1, "{actual:?}");
    assert!(actual[0].1.contains("`helper`"), "{actual:?}");
    assert_eq!(actual[0].0, "undefined-symbol");
}

#[test]
fn environment_aliases_follow_identity_and_rebinding() {
    let dir = tempfile::tempdir().unwrap();
    let actual = findings(
        dir.path(),
        &[
            (
                "main.R",
                "helpers <- new.env(); alias <- helpers; source('helper.R', local = alias); helpers$helper(); helpers <- new.env(); print(helpers)\n",
            ),
            ("helper.R", "helper <- function() 42L\n"),
        ],
    );
    assert!(actual.is_empty(), "{actual:?}");
    let actual = findings(
        dir.path(),
        &[
            (
                "main.R",
                "helpers <- new.env(); source('helper.R', local = helpers); helpers <- new.env(); helpers$helper()\n",
            ),
            ("helper.R", "helper <- function() 42L\n"),
        ],
    );
    assert_eq!(actual.len(), 1, "{actual:?}");
    assert_eq!(actual[0].0, "unused-binding");
}

#[test]
fn computed_helper_loads_do_not_hide_unrelated_undefined_names() {
    let dir = tempfile::tempdir().unwrap();
    let actual = findings(
        dir.path(),
        &[
            (
                "main.R",
                "path <- 'helper.R'; helpers <- new.env(); source(path, local = helpers); helpers$helper(); bogus()\n",
            ),
            ("helper.R", "helper <- function() 42L\n"),
        ],
    );
    assert_eq!(actual.len(), 2, "{actual:?}");
    assert!(
        actual
            .iter()
            .any(|d| d.0 == "undefined-symbol" && d.1.contains("`bogus`")),
        "{actual:?}"
    );
    assert!(
        actual
            .iter()
            .any(|d| d.0 == "unused-binding" && d.1.contains("`helper`")),
        "{actual:?}"
    );
}

#[test]
fn helper_members_are_used_only_after_loading() {
    let dir = tempfile::tempdir().unwrap();
    let actual = findings(
        dir.path(),
        &[
            (
                "main.R",
                "helpers <- new.env(); helpers$helper(); source('helper.R', local = helpers)\n",
            ),
            ("helper.R", "helper <- function() 42L\n"),
        ],
    );
    assert_eq!(actual.len(), 1, "{actual:?}");
    assert_eq!(actual[0].0, "unused-binding");
}

#[test]
fn source_reads_in_computed_and_replacement_assignment_targets_are_used() {
    for (target, helper) in [
        ("x[helper()]", "helper <- function() 1L\n"),
        ("x[[helper()]]", "helper <- function() 1L\n"),
        ("helper(x)", "`helper<-` <- function(x, value) value\n"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let main = format!("source('helper.R'); x <- list(); {target} <- 42L; print(x)\n");
        let actual = findings(dir.path(), &[("main.R", &main), ("helper.R", helper)]);
        assert!(actual.is_empty(), "{main}: {actual:?}");
    }
}

#[test]
fn sourced_data_calls_preserve_implicit_semantic_bindings() {
    for loader in [
        "data('review_dataset')\n",
        "data(review_dataset)\n",
        "{ data('review_dataset') }\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let actual = findings(
            dir.path(),
            &[
                ("main.R", "source('helper.R'); print(review_dataset)\n"),
                ("helper.R", loader),
            ],
        );
        assert!(actual.is_empty(), "{loader}: {actual:?}");
    }
}

#[test]
fn logical_source_aliases_honor_redefinitions_and_local_scope() {
    for main in [
        "F <- new.env(); source('helper.R', local = F); print(F); helper()\n",
        "T <- new.env(); source('helper.R', local = T); print(T); helper()\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let actual = findings(
            dir.path(),
            &[("main.R", main), ("helper.R", "helper <- function() 42L\n")],
        );
        assert_eq!(actual.len(), 2, "{main}: {actual:?}");
        assert!(
            actual
                .iter()
                .any(|d| d.0 == "unused-binding" && d.1.contains("`helper`"))
        );
        assert!(
            actual
                .iter()
                .any(|d| d.0 == "undefined-symbol" && d.1.contains("`helper`"))
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let actual = findings(
        dir.path(),
        &[
            (
                "main.R",
                "run <- function() { source('helper.R', local = T); helper() }; run(); helper()\n",
            ),
            ("helper.R", "helper <- function() 42L\n"),
        ],
    );
    assert_eq!(actual.len(), 1, "{actual:?}");
    assert_eq!(actual[0].0, "undefined-symbol");
}

#[test]
fn source_destinations_retain_possible_environments_after_branch_joins() {
    use arity::incremental::IncrementalDatabase;
    use arity::project::project_graph;
    use arity::project::source_env::SourceRead;
    for main in [
        "if (interactive()) e <- new.env() else e <- globalenv(); source('helper.R', local = e); helper(); unrelated_typo()\n",
        "private <- new.env(); if (interactive()) e <- private else e <- globalenv(); source('helper.R', local = e); helper(); unrelated_typo()\n",
        "if (interactive()) e <- globalenv() else e <- new.env(); source('helper.R', local = e); helper(); unrelated_typo()\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let helper = "helper <- function() 42L\n";
        let actual = findings(dir.path(), &[("main.R", main), ("helper.R", helper)]);
        assert_eq!(actual.len(), 1, "{main}: {actual:?}");
        assert!(
            actual[0].1.contains("`unrelated_typo`"),
            "{main}: {actual:?}"
        );
        let mut db = IncrementalDatabase::default();
        let main_path = dir.path().join("main.R");
        let entry = db.upsert_file(&main_path, main);
        let helper = db.upsert_file(&dir.path().join("helper.R"), helper);
        db.set_workspace_members(vec![entry, helper], vec![dir.path().to_path_buf()]);
        assert!(
            project_graph(&db)
                .for_file(&main_path)
                .source_reads()
                .unwrap()
                .reads
                .iter()
                .any(|(key, status)| key.name == "helper" && *status == SourceRead::Unknown),
            "{main}"
        );
    }
}

#[test]
fn source_destinations_keep_branch_local_environments_and_aliases() {
    for main in [
        "if (interactive()) e <- new.env() else e <- new.env(); source('helper.R', local = e); e$helper(); helper()\n",
        "left <- new.env(); right <- new.env(); if (interactive()) e <- left else e <- right; source('helper.R', local = e); left$helper(); right$helper(); helper()\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let actual = findings(
            dir.path(),
            &[("main.R", main), ("helper.R", "helper <- function() 42L\n")],
        );
        assert_eq!(actual.len(), 1, "{main}: {actual:?}");
        assert_eq!(actual[0].0, "undefined-symbol");
        assert!(actual[0].1.contains("`helper`"));
    }
}

#[test]
fn environment_member_reads_do_not_inherit_parent_bindings() {
    for member in ["helpers$helper", "helpers[['helper']]"] {
        for main in [
            format!("source('helper.R'); helpers <- new.env(); print({member})\n"),
            format!(
                "parent <- new.env(); source('helper.R', local = parent); helpers <- new.env(parent = parent); print({member})\n"
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let actual = findings(
                dir.path(),
                &[
                    ("main.R", &main),
                    ("helper.R", "helper <- function() 42L\n"),
                ],
            );
            assert_eq!(actual.len(), 1, "{main}: {actual:?}");
            assert_eq!(actual[0].0, "unused-binding");
            assert!(actual[0].1.contains("`helper`"));
        }
    }
}

#[test]
fn nested_load_order_marks_only_the_last_provider_used() {
    let dir = tempfile::tempdir().unwrap();
    let actual = findings(
        dir.path(),
        &[
            ("main.R", "source('loader.R'); helper()\n"),
            ("loader.R", "source('one.R'); source('two.R')\n"),
            ("one.R", "helper <- function() 1L\n"),
            ("two.R", "helper <- function() 2L\n"),
        ],
    );
    assert_eq!(actual.len(), 1, "{actual:?}");
    assert_eq!(actual[0].0, "unused-binding");
    assert!(actual[0].1.contains("`helper`"));
    assert!(actual[0].3.ends_with("one.R"), "{actual:?}");
}

#[test]
fn shadowed_loaders_and_environment_constructors_are_not_assumed_to_be_base() {
    for main in [
        "source <- function(file) NULL; source('helper.R'); helper()\n",
        "new.env <- function(...) NULL; helpers <- new.env(); source('helper.R', local = helpers); helpers$helper(); helper()\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let actual = findings(
            dir.path(),
            &[("main.R", main), ("helper.R", "helper <- function() 42L\n")],
        );
        assert_eq!(actual.len(), 2, "{actual:?}");
        assert!(
            actual
                .iter()
                .any(|d| d.0 == "undefined-symbol" && d.1.contains("`helper`"))
        );
        assert!(
            actual
                .iter()
                .any(|d| d.0 == "unused-binding" && d.1.contains("`helper`"))
        );
    }
}

#[test]
fn source_bindings_gate_base_rewrites_and_introduced_calls() {
    for provider in ["seq", "length", "seq_along"] {
        let dir = tempfile::tempdir().unwrap();
        let main =
            "run <- function() { source('helper.R', local = TRUE); seq(length(1:3)) }; run()\n";
        std::fs::write(dir.path().join("main.R"), main).unwrap();
        std::fs::write(
            dir.path().join("helper.R"),
            format!("{provider} <- function(...) 1L\n"),
        )
        .unwrap();
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_arity"))
            .current_dir(dir.path())
            .args([
                "--no-config",
                "lint",
                "--select",
                "seq",
                "--unsafe-fixes",
                "--fix",
                ".",
            ])
            .output()
            .unwrap();
        assert_eq!(
            result.status.code(),
            Some(i32::from(provider == "seq_along")),
            "{result:?}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("main.R")).unwrap(),
            main
        );
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.R"), "helpers <- new.env(); source('helper.R', local = helpers); seq(length(1:3)); helpers$seq_along()\n").unwrap();
    std::fs::write(
        dir.path().join("helper.R"),
        "seq_along <- function(...) 1L\n",
    )
    .unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_arity"))
        .current_dir(dir.path())
        .args([
            "--no-config",
            "lint",
            "--select",
            "seq",
            "--unsafe-fixes",
            "--fix",
            ".",
        ])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(
        std::fs::read_to_string(dir.path().join("main.R"))
            .unwrap()
            .contains("seq_along(1:3)")
    );
}

#[test]
fn sourced_reads_use_only_the_calling_functions_binding() {
    let dir = tempfile::tempdir().unwrap();
    let main = "run <- function() { secret <- 1L; source('consumer.R', local = TRUE) }; other <- function() { secret <- 2L }; run(); print(other)\n";
    let actual = findings(
        dir.path(),
        &[("main.R", main), ("consumer.R", "print(secret)\n")],
    );
    assert_eq!(actual.len(), 1, "{actual:?}");
    assert_eq!(actual[0].0, "unused-binding");
    assert_eq!(actual[0].2, main.rfind("secret <-").unwrap());
}

#[test]
fn helper_closures_keep_their_lexical_parent() {
    let dir = tempfile::tempdir().unwrap();
    let actual = findings(
        dir.path(),
        &[
            (
                "main.R",
                "answer <- 42L; helpers <- new.env(); source('helper.R', local = helpers); helpers$helper()\n",
            ),
            ("helper.R", "helper <- function() answer\n"),
        ],
    );
    assert!(actual.is_empty(), "{actual:?}");
    let actual = findings(
        dir.path(),
        &[
            (
                "main.R",
                "answer <- 42L; helpers <- new.env(parent = baseenv()); source('helper.R', local = helpers); helpers$helper()\n",
            ),
            ("helper.R", "helper <- function() answer\n"),
        ],
    );
    assert_eq!(actual.len(), 2, "{actual:?}");
    assert!(
        actual
            .iter()
            .any(|d| d.0 == "unused-binding" && d.1.contains("`answer`"))
    );
    assert!(
        actual
            .iter()
            .any(|d| d.0 == "undefined-symbol" && d.1.contains("`answer`"))
    );
}

#[test]
fn alternative_loads_keep_all_possible_providers_used() {
    let dir = tempfile::tempdir().unwrap();
    let actual = findings(
        dir.path(),
        &[
            (
                "main.R",
                "if (interactive()) source('one.R') else source('two.R'); helper()\n",
            ),
            ("one.R", "helper <- function() 1L\n"),
            ("two.R", "helper <- function() 2L\n"),
        ],
    );
    assert!(actual.is_empty(), "{actual:?}");
}

#[test]
fn literal_member_and_runtime_reads_keep_quoted_bindings_used() {
    for read in ["helpers[['hello world']]()", "helpers$`hello world`()"] {
        let dir = tempfile::tempdir().unwrap();
        let main = format!("helpers <- new.env(); sys.source('helper.R', helpers); {read}\n");
        let actual = findings(
            dir.path(),
            &[
                ("main.R", &main),
                ("helper.R", "`hello world` <- function() 42L\n"),
            ],
        );
        assert!(actual.is_empty(), "{actual:?}");
    }
    let dir = tempfile::tempdir().unwrap();
    let actual = findings(
        dir.path(),
        &[
            ("main.R", "source('helper.R'); get('helper')()\n"),
            ("helper.R", "helper <- function() 42L\n"),
        ],
    );
    assert!(actual.is_empty(), "{actual:?}");
}

#[test]
fn source_environment_facts_backdate_across_body_and_offset_edits() {
    use arity::incremental::{IncrementalDatabase, QueryKind};
    use arity::project::{external_resolution, project_graph};
    use arity::rindex::provider::IndexedProvider;
    let dir = tempfile::tempdir().unwrap();
    let main_path = dir.path().join("main.R");
    let helper_path = dir.path().join("helper.R");
    let main = "run <- function() { source('helper.R', local = TRUE); helper(1) }; run()\n";
    let mut db = IncrementalDatabase::default();
    let entry = db.upsert_file(&main_path, main);
    let helper = db.upsert_file(&helper_path, "helper <- function(x) x + 1\n");
    db.set_workspace_members(vec![entry, helper], vec![dir.path().to_path_buf()]);
    let index = db.set_library_index(IndexedProvider::empty());
    assert!(external_resolution(&db, index, entry).unresolved.is_empty());
    assert!(
        project_graph(&db)
            .for_file(&helper_path)
            .used_elsewhere("helper")
    );
    db.clear_query_log();
    db.set_file_text(entry, main.replace("helper(1)", "helper(100 + 2)"));
    db.set_file_text(
        helper,
        "# Prose shifts the function.\nhelper <- function(x) x + 10\n",
    );
    assert!(external_resolution(&db, index, entry).unresolved.is_empty());
    assert!(
        project_graph(&db)
            .for_file(&helper_path)
            .used_elsewhere("helper")
    );
    let kinds: Vec<_> = db.query_log().iter().map(|entry| entry.kind).collect();
    assert!(
        kinds.iter().all(|kind| !matches!(
            kind,
            QueryKind::ProjectGraph | QueryKind::VisibleSymbols | QueryKind::ExternalResolution
        )),
        "{kinds:?}"
    );
}

#[test]
fn changing_only_a_destination_updates_use_tracking() {
    use arity::incremental::IncrementalDatabase;
    use arity::project::project_graph;
    let dir = tempfile::tempdir().unwrap();
    let main_path = dir.path().join("main.R");
    let helper_path = dir.path().join("helper.R");
    let main = "left <- new.env(); right <- new.env(); source('helper.R', local = left); left$helper(); print(right)\n";
    let mut db = IncrementalDatabase::default();
    let entry = db.upsert_file(&main_path, main);
    let helper = db.upsert_file(&helper_path, "helper <- function() 42L\n");
    db.set_workspace_members(vec![entry, helper], vec![dir.path().to_path_buf()]);
    assert!(
        project_graph(&db)
            .for_file(&helper_path)
            .used_elsewhere("helper")
    );
    db.set_file_text(entry, main.replace("local = left", "local = right"));
    assert!(
        !project_graph(&db)
            .for_file(&helper_path)
            .used_elsewhere("helper")
    );
}

#[test]
fn rename_declines_source_reads_it_cannot_rewrite_completely() {
    use arity::incremental::IncrementalDatabase;
    for main in [
        "helpers <- new.env(); source('helper.R', local = helpers); helpers$helper()\n",
        "run <- function() { source('helper.R', local = TRUE); helper() }; run()\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let helper_path = dir.path().join("helper.R");
        let mut db = IncrementalDatabase::default();
        let entry = db.upsert_file(&dir.path().join("main.R"), main);
        let helper = db.upsert_file(&helper_path, "helper <- function() 42L\n");
        db.set_workspace_members(vec![entry, helper], vec![dir.path().to_path_buf()]);
        assert!(
            db.snapshot()
                .cross_file_binding(&helper_path, "helper")
                .source_environment_risk
        );
    }
}
