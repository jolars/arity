use std::collections::HashMap;
use std::path::{Path, PathBuf};

use arity::incremental::{
    IncrementalDatabase, QueryKind, SourceFile, description_facts, file_def_sites, line_index,
    top_level_events,
};
use arity::parser::Edit;
use arity::project::{
    DefKind, Project, ProjectMember, external_resolution, package_facts_for, package_usage,
    project_classes, project_defs, project_reads, project_roxygen_topics, reverse_source_edges,
    visible_symbols, workspace_project,
};
use arity::rindex::provider::IndexedProvider;
use arity::rindex::remote::RemoteExports;
use arity::rindex::schema::{PackageIndex, SCHEMA_VERSION, SymbolEntry, SymbolKind};
use arity::syntax::{NodePtr, SyntaxKind};

fn count_by_kind(entries: &[arity::incremental::QueryLogEntry]) -> HashMap<QueryKind, usize> {
    let mut counts = HashMap::new();
    for entry in entries {
        *counts.entry(entry.kind).or_insert(0) += 1;
    }
    counts
}

#[test]
fn file_def_sites_reused_when_input_unchanged() {
    let db = IncrementalDatabase::default();
    let file = db.add_file("f <- function() 1\nx <- 2\n");

    let _ = file_def_sites(&db, file);
    db.clear_query_log();
    let _ = file_def_sites(&db, file);

    assert!(
        db.query_log().is_empty(),
        "unchanged input must not re-run file_def_sites"
    );
}

#[test]
fn file_def_sites_backdates_across_body_edit() {
    // The firewall: editing a function *body* re-runs the model, and
    // file_def_sites re-executes, but its output (the name→kind map) is
    // unchanged, so it backdates and downstream aggregates are spared. Here we
    // assert the load-bearing half — the value is stable across the edit.
    let mut db = IncrementalDatabase::default();
    let file = db.add_file("f <- function() {\n  g()\n}\nx <- 1\n");
    let before = file_def_sites(&db, file).clone();

    db.set_file_text(file, "f <- function() {\n  g()\n  h()\n}\nx <- 1\n");
    let after = file_def_sites(&db, file).clone();

    assert_eq!(
        before, after,
        "a body edit must not change the def-site map"
    );
}

#[test]
fn line_index_is_reused_when_input_unchanged() {
    let db = IncrementalDatabase::default();
    let file = db.add_file("x <- 1\ny <- 2\n");

    let _ = line_index(&db, file);
    db.clear_query_log();
    let _ = line_index(&db, file);

    assert!(
        db.query_log().is_empty(),
        "unchanged input must not re-run line_index"
    );
}

#[test]
fn line_index_backdates_across_within_line_edit() {
    // An edit that changes content but leaves every newline (and wide-char)
    // position untouched yields an equal `LineIndex`, so salsa backdates it and
    // consumers that only depend on line geometry are spared.
    let mut db = IncrementalDatabase::default();
    let file = db.add_file("x <- 1\ny <- 2\n");
    let before = line_index(&db, file).clone();

    // Same byte length, same newline offsets — only a digit changed.
    db.set_file_text(file, "x <- 9\ny <- 2\n");
    let after = line_index(&db, file).clone();

    assert_eq!(
        before, after,
        "a within-line edit must not change the line index"
    );
}

#[test]
fn semantic_model_is_reused_when_input_unchanged() {
    let db = IncrementalDatabase::default();
    let file = db.add_file("x <- 1 + 2\n");

    assert!(db.parse_diagnostics(file).is_empty());
    let _ = db.semantic_model(file);

    db.clear_query_log();
    let _ = db.semantic_model(file);
    assert!(db.parse_diagnostics(file).is_empty());

    assert!(
        db.query_log().is_empty(),
        "expected no query re-execution for unchanged input"
    );
}

#[test]
fn control_flow_reused_when_input_unchanged() {
    let db = IncrementalDatabase::default();
    let file = db.add_file("f <- function() {\n  if (c) return(1) else return(2)\n  3\n}\n");

    let _ = db.control_flow(file);
    db.clear_query_log();
    let _ = db.control_flow(file);

    assert!(
        db.query_log().is_empty(),
        "unchanged input must not re-run control_flow"
    );
}

#[test]
fn control_flow_backdates_across_equivalent_edit() {
    // A same-length rename leaves every statement range (and the function's
    // NodePtr key) untouched, so the CFG value is unchanged — `FileControlFlow:
    // Eq` lets salsa backdate it and spare CFG-dependent consumers. Assert the
    // load-bearing half: the value is stable across the edit.
    let mut db = IncrementalDatabase::default();
    let file = db.add_file("f <- function() {\n  if (aaa) return(1) else return(2)\n}\n");
    let before = db.control_flow(file).clone();

    db.set_file_text(
        file,
        "f <- function() {\n  if (bbb) return(1) else return(2)\n}\n",
    );
    let after = db.control_flow(file).clone();

    assert_eq!(before, after, "an equivalent edit must not change the CFG");
}

#[test]
fn editing_one_file_invalidates_only_that_file_queries() {
    let mut db = IncrementalDatabase::default();
    let file_a = db.add_file("x <- 1 + 2\n");
    let file_b = db.add_file("y <- 3 + 4\n");

    // Materialize parse + model for both files.
    let _ = db.semantic_model(file_a);
    let _ = db.semantic_model(file_b);
    assert!(db.parse_diagnostics(file_a).is_empty());
    assert!(db.parse_diagnostics(file_b).is_empty());

    db.clear_query_log();
    db.set_file_text(file_a, "x <- 10 + 2\n");

    let _ = db.semantic_model(file_a);
    let _ = db.semantic_model(file_b);

    let log = db.query_log();
    let file_a_entries: Vec<_> = log
        .iter()
        .copied()
        .filter(|entry| entry.file == Some(file_a))
        .collect();
    let file_b_entries: Vec<_> = log
        .iter()
        .copied()
        .filter(|entry| entry.file == Some(file_b))
        .collect();

    assert!(
        file_b_entries.is_empty(),
        "expected file_b queries to be reused after file_a edit"
    );

    // file_a's text changed: both its parse and its semantic model re-run once.
    let counts = count_by_kind(&file_a_entries);
    assert_eq!(counts.get(&QueryKind::ParsedDocument), Some(&1));
    assert_eq!(counts.get(&QueryKind::SemanticModel), Some(&1));
}

#[test]
fn upsert_reuses_input_for_same_path() {
    use std::path::Path;
    let mut db = IncrementalDatabase::default();
    let path = Path::new("/proj/a.R");

    let first = db.upsert_file(path, "x <- 1\n".to_string());
    let second = db.upsert_file(path, "x <- 1\ny <- 2\n".to_string());

    assert!(
        first == second,
        "same path should reuse the SourceFile input"
    );
    assert_eq!(db.semantic_model(second).bindings().len(), 2);
}

#[test]
fn clone_shares_inputs_and_cached_parse() {
    // A clone is a second handle onto the same storage: it sees the same
    // path→input map and reuses the owner's memoized parse without re-running it.
    // This is the LSP read path — format/hover run off a short-lived clone.
    use std::path::Path;
    let mut db = IncrementalDatabase::default();
    let path = Path::new("/proj/a.R");
    let file = db.upsert_file(path, "x <- f(1)\n".to_string());
    // Materialize the parse on the owner.
    let _ = db.parsed_tree(file);

    let snapshot = db.clone();
    // The clone resolves the same input by path...
    assert!(
        snapshot.lookup_file(path) == Some(file),
        "clone should see the owner's tracked file"
    );
    assert_eq!(snapshot.file_text(file), "x <- f(1)\n");

    // ...and reads the memoized parse without re-executing the query.
    snapshot.clear_query_log();
    let root = snapshot.parsed_tree(file);
    assert_eq!(root.text().to_string(), "x <- f(1)\n");
    assert!(
        snapshot.query_log().is_empty(),
        "clone should reuse the owner's cached parse, ran {} queries",
        snapshot.query_log().len()
    );
}

#[test]
fn body_edit_uses_incremental_reparse_and_stays_correct() {
    // A function-body edit splices the previous tree rather than re-parsing from
    // scratch. The spliced parse must match a from-scratch parse of the new text.
    let mut db = IncrementalDatabase::default();
    let path = Path::new("/proj/a.R");
    let file = db.upsert_file(path, "f <- function() {\n  a + b\n}\n".to_string());

    // First parse: a full parse, no reparse hit.
    let _ = db.parsed_tree(file);
    assert_eq!(db.reparse_hits(), 0);

    // Edit the body: insert `c` inside the `{ a + b }` block.
    db.upsert_file(path, "f <- function() {\n  a + b + c\n}\n".to_string());
    let spliced = db.parsed_tree(file).text().to_string();
    assert_eq!(spliced, "f <- function() {\n  a + b + c\n}\n");
    assert_eq!(
        db.reparse_hits(),
        1,
        "body edit should be served by an incremental reparse"
    );

    // The spliced tree is byte-identical to a from-scratch parse.
    let fresh = arity::parser::parse("f <- function() {\n  a + b + c\n}\n");
    assert_eq!(
        db.parsed_tree(file).text().to_string(),
        fresh.cst.text().to_string()
    );
    assert!(db.parse_diagnostics(file).is_empty());
}

#[test]
fn toplevel_edit_uses_incremental_reparse_and_stays_correct() {
    // An edit to a bare, non-braced top-level statement splices just that
    // statement rather than re-parsing the whole file. The result must match a
    // from-scratch parse of the new text.
    let mut db = IncrementalDatabase::default();
    let path = Path::new("/proj/a.R");
    let file = db.upsert_file(
        path,
        "library(dplyr)\nx <- foo(a, b)\nn <- 10\n".to_string(),
    );

    // First parse: a full parse, no reparse hit.
    let _ = db.parsed_tree(file);
    assert_eq!(db.reparse_hits(), 0);

    // Edit the middle statement's call arguments (outside any `{ }` block).
    let edited = "library(dplyr)\nx <- foo(a, b, z)\nn <- 10\n";
    db.upsert_file(path, edited.to_string());
    let spliced = db.parsed_tree(file).text().to_string();
    assert_eq!(spliced, edited);
    assert_eq!(
        db.reparse_hits(),
        1,
        "top-level statement edit should be served by an incremental reparse"
    );

    // The spliced tree is byte-identical to a from-scratch parse.
    let fresh = arity::parser::parse(edited);
    assert_eq!(
        db.parsed_tree(file).text().to_string(),
        fresh.cst.text().to_string()
    );
    assert!(db.parse_diagnostics(file).is_empty());
}

#[test]
fn staged_edits_use_precise_multi_reparse() {
    // Two disjoint edits (a multi-cursor add-argument in each of two separate
    // top-level statements). A single spanning `diff_edit` would cross the
    // statement boundary and force a full reparse; the precise Stage-B path
    // reparses each statement, so it is both a reparse hit *and* a precise hit.
    let mut db = IncrementalDatabase::default();
    let path = Path::new("/proj/a.R");
    let base = "x <- foo(a)\ny <- bar(b)\n";
    let file = db.upsert_file(path, base.to_string());

    let _ = db.parsed_tree(file);
    assert_eq!(db.reparse_hits(), 0);
    assert_eq!(db.precise_reparse_hits(), 0);

    let foo_paren = base.find("foo(a)").unwrap() + "foo(a".len();
    let bar_paren = base.find("bar(b)").unwrap() + "bar(b".len();
    // Right-to-left so `base` coordinates stay valid across application.
    let edits = vec![
        Edit {
            range: bar_paren..bar_paren,
            insert: ", z".to_string(),
        },
        Edit {
            range: foo_paren..foo_paren,
            insert: ", w".to_string(),
        },
    ];
    let edited = "x <- foo(a, w)\ny <- bar(b, z)\n";
    db.stage_edits(file, edits);
    db.upsert_file(path, edited.to_string());

    let spliced = db.parsed_tree(file).text().to_string();
    assert_eq!(spliced, edited);
    assert_eq!(db.reparse_hits(), 1);
    assert_eq!(
        db.precise_reparse_hits(),
        1,
        "two-cursor edit should be served by the precise multi-edit path"
    );

    // Byte-identical to a from-scratch parse (Tenet 4).
    let fresh = arity::parser::parse(edited);
    assert_eq!(
        db.parsed_tree(file).text().to_string(),
        fresh.cst.text().to_string()
    );
    assert!(db.parse_diagnostics(file).is_empty());
}

#[test]
fn stale_staged_edits_fall_back_to_diff_edit() {
    // Staged edits that do not reconstruct the new buffer (a coalescing gap or a
    // stale sequence) must be rejected by the `reparse_edits` guard: the parse
    // falls back to the whole-text `diff_edit` and stays correct.
    let mut db = IncrementalDatabase::default();
    let path = Path::new("/proj/a.R");
    let base = "x <- foo(a)\ny <- bar(b)\n";
    let file = db.upsert_file(path, base.to_string());
    let _ = db.parsed_tree(file);

    // A bogus edit that does not describe the transform actually applied.
    db.stage_edits(
        file,
        vec![Edit {
            range: 0..0,
            insert: "WRONG".to_string(),
        }],
    );
    let edited = "x <- foo(a, w)\ny <- bar(b)\n";
    db.upsert_file(path, edited.to_string());

    let spliced = db.parsed_tree(file).text().to_string();
    assert_eq!(
        spliced, edited,
        "buffer still correct via diff_edit fallback"
    );
    assert_eq!(
        db.precise_reparse_hits(),
        0,
        "mismatched staged edits must not drive the tree"
    );

    let fresh = arity::parser::parse(edited);
    assert_eq!(
        db.parsed_tree(file).text().to_string(),
        fresh.cst.text().to_string()
    );
    assert!(db.parse_diagnostics(file).is_empty());
}

#[test]
fn body_edit_keeps_model_in_sync() {
    // Editing a file's contents recomputes its semantic model so downstream
    // consumers see the new bindings.
    let mut db = IncrementalDatabase::default();
    let file = db.add_file("x <- 1\n");
    assert_eq!(db.semantic_model(file).bindings().len(), 1);

    db.set_file_text(file, "x <- 1\ny <- 2\n");
    assert_eq!(db.semantic_model(file).bindings().len(), 2);
}

#[test]
fn upsert_dedups_equivalent_path_forms() {
    // `/pkg/R/a.R` and `/pkg/sub/../R/a.R` denote the same file; they must intern
    // to the same `SourceFile` input rather than minting two (which would double
    // the parse work and split project membership). `Path` collapses mid-path `.`
    // on its own but never `..`, so this exercises our lexical normalization.
    let mut db = IncrementalDatabase::default();
    let direct = db.upsert_file(Path::new("/pkg/R/a.R"), "x <- 1\n".to_string());
    let dotted = db.upsert_file(Path::new("/pkg/sub/../R/a.R"), "x <- 1\n".to_string());
    assert!(
        direct == dotted,
        "equivalent path forms must map to the same input"
    );
}

#[test]
fn in_memory_file_has_no_path() {
    // An in-memory document carries no on-disk path (no `<mem>/uuid` phantom).
    let db = IncrementalDatabase::default();
    let file = db.add_file("x <- 1\n");
    assert_eq!(db.file_path(file), None);
}

/// A two-file package: `a.R` defines `foo`, `b.R` reads it inside a function
/// body. Returns the db and the two tracked inputs.
fn package_ab(a_src: &str, b_src: &str) -> (IncrementalDatabase, SourceFile, SourceFile) {
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/pkg/R/a.R"), a_src.to_string());
    let b = db.upsert_file(Path::new("/pkg/R/b.R"), b_src.to_string());
    (db, a, b)
}

/// Intern the `{a, b}` package membership. Mirrors production, which re-interns
/// the (deduped) membership on every lint rather than holding an id across an
/// edit — so the interned `Project` borrow never spans a `&mut db` write.
fn project_ab(db: &IncrementalDatabase, a: SourceFile, b: SourceFile) -> Project<'_> {
    let members = vec![
        ProjectMember {
            file: a,
            path: PathBuf::from("/pkg/R/a.R"),
            package_root: Some(PathBuf::from("/pkg")),
        },
        ProjectMember {
            file: b,
            path: PathBuf::from("/pkg/R/b.R"),
            package_root: Some(PathBuf::from("/pkg")),
        },
    ];
    Project::new(db, members, Vec::new(), Vec::new(), Vec::new())
}

/// A two-script project where `a.R` sources `b.R`. No package root, so the only
/// cross-file relation is the `source()` edge.
fn scripts_ab(a_src: &str, b_src: &str) -> (IncrementalDatabase, SourceFile, SourceFile) {
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/s/a.R"), a_src.to_string());
    let b = db.upsert_file(Path::new("/s/b.R"), b_src.to_string());
    (db, a, b)
}

fn project_scripts(db: &IncrementalDatabase, a: SourceFile, b: SourceFile) -> Project<'_> {
    let members = vec![
        ProjectMember {
            file: a,
            path: PathBuf::from("/s/a.R"),
            package_root: None,
        },
        ProjectMember {
            file: b,
            path: PathBuf::from("/s/b.R"),
            package_root: None,
        },
    ];
    Project::new(db, members, Vec::new(), Vec::new(), Vec::new())
}

#[test]
fn body_edit_does_not_rebuild_reverse_source_edges() {
    // The firewall: `a.R` sources `b.R`; editing `b.R`'s function body re-runs
    // its model but not its (empty) source edges, so the reverse map's memo —
    // keyed on the interned project + per-file source_edges — must be reused.
    let (mut db, a, b) = scripts_ab("source(\"b.R\")\n", "bar <- function() {\n  baz()\n}\n");

    {
        let project = project_scripts(&db, a, b);
        let rev = reverse_source_edges(&db, project);
        assert!(
            rev.sourced_by
                .get(Path::new("/s/b.R"))
                .is_some_and(|s| s.contains(Path::new("/s/a.R"))),
            "a.R should be recorded as a sourcer of b.R"
        );
    }

    db.clear_query_log();
    db.set_file_text(b, "bar <- function() {\n  baz()\n  2\n}\n");

    let project = project_scripts(&db, a, b);
    let _ = reverse_source_edges(&db, project);

    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::ReverseSourceEdges),
        None,
        "a body edit must not rebuild the reverse source-edge map"
    );
}

#[test]
fn adding_source_call_rebuilds_reverse_edges() {
    // The complement: adding a top-level `source()` changes a.R's source edges,
    // so the reverse map *must* rebuild and gain the new edge.
    let (mut db, a, b) = scripts_ab("source(\"b.R\")\n", "bar <- 1\n");

    {
        let project = project_scripts(&db, a, b);
        let _ = reverse_source_edges(&db, project);
    }

    db.clear_query_log();
    db.set_file_text(a, "source(\"b.R\")\nsource(\"c.R\")\n");

    let project = project_scripts(&db, a, b);
    let rev = reverse_source_edges(&db, project);

    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::ReverseSourceEdges),
        Some(&1),
        "a new source() call must rebuild the reverse map"
    );
    assert!(
        rev.sourced_by
            .get(Path::new("/s/c.R"))
            .is_some_and(|s| s.contains(Path::new("/s/a.R"))),
        "the new edge a.R -> c.R must appear in the reverse map"
    );
}

#[test]
fn project_defs_aggregates_def_sites_by_name() {
    let (db, a, b) = package_ab("foo <- function() 1\n", "bar <- 2\nfoo <- 3\n");
    let project = project_ab(&db, a, b);
    let defs = project_defs(&db, project);

    // `foo` is defined in both files — a function in a.R, a value in b.R.
    let foo = defs.by_name.get("foo").expect("foo is defined");
    assert!(foo.contains(&(PathBuf::from("/pkg/R/a.R"), DefKind::Function)));
    assert!(foo.contains(&(PathBuf::from("/pkg/R/b.R"), DefKind::Value)));
    // `bar` only in b.R.
    let bar = defs.by_name.get("bar").expect("bar is defined");
    assert_eq!(bar.len(), 1);
    assert!(bar.contains(&(PathBuf::from("/pkg/R/b.R"), DefKind::Value)));
}

#[test]
fn body_edit_does_not_rebuild_project_defs() {
    // The firewall: editing b.R's body re-runs its model and file_def_sites, but
    // the def-site set is unchanged, so the project-wide aggregate is reused.
    let (mut db, a, b) = package_ab("foo <- function() 1\n", "bar <- function() {\n  foo()\n}\n");

    {
        let project = project_ab(&db, a, b);
        let _ = project_defs(&db, project);
    }

    db.clear_query_log();
    db.set_file_text(b, "bar <- function() {\n  foo()\n  2\n}\n");

    let project = project_ab(&db, a, b);
    let _ = project_defs(&db, project);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(counts.get(&QueryKind::SemanticModel), Some(&1));
    assert_eq!(
        counts.get(&QueryKind::ProjectDefs),
        None,
        "a body edit must not rebuild project_defs"
    );
}

#[test]
fn project_classes_aggregates_inheritance_edges() {
    let (db, a, b) = package_ab(
        "setClass(\"Animal\")\n",
        "setClass(\"Dog\", contains = \"Animal\")\n",
    );
    let project = project_ab(&db, a, b);
    let classes = project_classes(&db, project);

    // Forward edge: Dog -> Animal; inverse edge: Animal -> Dog.
    assert!(
        classes
            .supertypes
            .get("Dog")
            .is_some_and(|p| p.contains("Animal"))
    );
    assert!(
        classes
            .subtypes
            .get("Animal")
            .is_some_and(|c| c.contains("Dog"))
    );
    // Def sites are recorded per class, tagged by system.
    assert!(classes.def_sites.contains_key("Animal"));
    assert!(classes.def_sites.contains_key("Dog"));
}

#[test]
fn body_edit_does_not_rebuild_project_classes() {
    // The firewall: editing a function body in b.R re-runs its model, but the
    // class-def set is unchanged, so the project-wide class aggregate is reused.
    let (mut db, a, b) = package_ab(
        "setClass(\"Animal\")\n",
        "setClass(\"Dog\", contains = \"Animal\")\nf <- function() {\n  1\n}\n",
    );

    {
        let project = project_ab(&db, a, b);
        let _ = project_classes(&db, project);
    }

    db.clear_query_log();
    db.set_file_text(
        b,
        "setClass(\"Dog\", contains = \"Animal\")\nf <- function() {\n  2\n}\n",
    );

    let project = project_ab(&db, a, b);
    let _ = project_classes(&db, project);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(
        counts.get(&QueryKind::ProjectClasses),
        None,
        "a body edit must not rebuild project_classes"
    );
}

#[test]
fn adding_a_class_rebuilds_project_classes() {
    let (mut db, a, b) = package_ab("setClass(\"Animal\")\n", "x <- 1\n");

    {
        let project = project_ab(&db, a, b);
        let _ = project_classes(&db, project);
    }

    db.clear_query_log();
    db.set_file_text(b, "x <- 1\nsetClass(\"Dog\", contains = \"Animal\")\n");

    let project = project_ab(&db, a, b);
    let classes = project_classes(&db, project);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(
        counts.get(&QueryKind::ProjectClasses),
        Some(&1),
        "a new class definition must rebuild project_classes"
    );
    assert!(
        classes
            .subtypes
            .get("Animal")
            .is_some_and(|c| c.contains("Dog"))
    );
}

/// A documented `add(x, y)` in a.R whose `@param y` lives on a `@rdname add`
/// joiner in b.R — the shape the package-wide topic index exists for.
const TOPIC_A: &str =
    "#' Add\n#' @param x A number.\n#' @export\nadd <- function(x, y) {\n  x + y\n}\n";
const TOPIC_B: &str =
    "#' @rdname add\n#' @param y The other number.\nadd2 <- function(x, y) x + y\n";

#[test]
fn body_edit_does_not_rebuild_project_roxygen_topics() {
    // The firewall: the topic projection is range-free and turns only on the
    // documentation and the formals, so editing a body leaves it equal.
    let (mut db, a, b) = package_ab(TOPIC_A, TOPIC_B);

    {
        let project = project_ab(&db, a, b);
        let _ = project_roxygen_topics(&db, project);
    }

    db.clear_query_log();
    db.set_file_text(
        a,
        "#' Add\n#' @param x A number.\n#' @export\nadd <- function(x, y) {\n  x + y + 0\n}\n",
    );

    let project = project_ab(&db, a, b);
    let _ = project_roxygen_topics(&db, project);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(
        counts.get(&QueryKind::ProjectRoxygenTopics),
        None,
        "a body edit must not rebuild project_roxygen_topics"
    );
}

#[test]
fn adding_a_param_tag_rebuilds_project_roxygen_topics() {
    let (mut db, a, b) = package_ab(TOPIC_A, "#' @rdname add\nadd2 <- function(x, y) x + y\n");

    {
        let project = project_ab(&db, a, b);
        let _ = project_roxygen_topics(&db, project);
    }

    db.clear_query_log();
    db.set_file_text(b, TOPIC_B);

    let project = project_ab(&db, a, b);
    let topics = project_roxygen_topics(&db, project);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(
        counts.get(&QueryKind::ProjectRoxygenTopics),
        Some(&1),
        "a new `@param` must rebuild project_roxygen_topics"
    );
    let members = topics
        .for_package(Path::new("/pkg"))
        .expect("the package's topics")
        .members("add");
    assert!(
        members
            .iter()
            .any(|m| m.documented_params.iter().any(|p| p == "y")),
        "the new `@param y` must be in the index: {members:?}"
    );
}

#[test]
fn adding_top_level_binding_rebuilds_project_defs() {
    let (mut db, a, b) = package_ab("foo <- function() 1\n", "bar <- function() foo()\n");

    {
        let project = project_ab(&db, a, b);
        let _ = project_defs(&db, project);
    }

    db.clear_query_log();
    db.set_file_text(b, "bar <- function() foo()\nqux <- 2\n");

    let project = project_ab(&db, a, b);
    let defs = project_defs(&db, project);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(
        counts.get(&QueryKind::ProjectDefs),
        Some(&1),
        "a new top-level binding must rebuild project_defs"
    );
    assert!(defs.by_name.contains_key("qux"));
}

#[test]
fn def_range_in_recovers_live_span_after_edit() {
    // The range-free aggregate omits def spans; def_range_in recovers them from
    // the fresh model, so the span tracks the *current* text. Inserting a leading
    // line shifts foo's definition; the recovered range must still spell "foo".
    let (mut db, a, b) = package_ab("foo <- function() 1\n", "bar <- 2\n");
    {
        let project = project_ab(&db, a, b);
        let _ = project_defs(&db, project);
    }

    db.set_file_text(a, "# header\nfoo <- function() 1\n");

    let project = project_ab(&db, a, b);
    let defs = project_defs(&db, project);
    assert!(
        defs.by_name
            .get("foo")
            .is_some_and(|sites| sites.iter().any(|(p, _)| p == Path::new("/pkg/R/a.R"))),
        "project_defs should point foo at a.R"
    );

    let snapshot = db.snapshot();
    let file = snapshot.lookup_file(Path::new("/pkg/R/a.R")).unwrap();
    let range = snapshot
        .def_range_in(file, "foo")
        .expect("foo has a def span");
    let text = snapshot.file_text(file);
    let start: usize = range.start().into();
    let end: usize = range.end().into();
    assert_eq!(&text[start..end], "foo");
    assert_eq!(
        start,
        "# header\n".len(),
        "the span must reflect the post-edit position"
    );
}

#[test]
fn workspace_project_excludes_parse_error_files() {
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/s/a.R"), "foo <- 1\n".to_string());
    let bad = db.upsert_file(Path::new("/s/bad.R"), "x <- function(\n".to_string());
    assert!(
        !db.parse_diagnostics(bad).is_empty(),
        "bad.R should not parse"
    );

    db.set_workspace_members(vec![a, bad], vec![PathBuf::from("/s")]);
    let project = workspace_project(&db);

    let paths: Vec<_> = project
        .members(&db)
        .iter()
        .map(|m| m.path.clone())
        .collect();
    assert_eq!(
        paths,
        vec![PathBuf::from("/s/a.R")],
        "a file with parse errors must be dropped from the derived membership"
    );
}

#[test]
fn keystroke_backdates_workspace_project_and_spares_graph() {
    // The membership firewall: a body edit re-runs workspace_project (it reads
    // the edited file's parse status), but it backdates to the *same* interned
    // Project, so the cross-file project graph derived from it is not rebuilt.
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/s/a.R"), "foo <- function() 1\n".to_string());
    let b = db.upsert_file(
        Path::new("/s/b.R"),
        "bar <- function() {\n  foo()\n}\n".to_string(),
    );
    db.set_workspace_members(vec![a, b], vec![PathBuf::from("/s")]);

    {
        let project = workspace_project(&db);
        let _ = visible_symbols(&db, project, a);
        let _ = visible_symbols(&db, project, b);
    }

    db.clear_query_log();
    db.set_file_text(b, "bar <- function() {\n  foo()\n  2\n}\n");

    let project = workspace_project(&db);
    let _ = visible_symbols(&db, project, a);
    let _ = visible_symbols(&db, project, b);

    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::ProjectGraph),
        None,
        "a body edit must not rebuild the project graph derived from the workspace"
    );
}

#[test]
fn reseeding_identical_membership_skips_write() {
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/s/a.R"), "foo <- 1\n".to_string());
    let b = db.upsert_file(Path::new("/s/b.R"), "bar <- 2\n".to_string());
    db.set_workspace_members(vec![a, b], vec![PathBuf::from("/s")]);
    let _ = workspace_project(&db);

    db.clear_query_log();
    // Re-seed the identical membership (order swapped — the setter sorts): the
    // conditional setter must skip the write, so the memo is reused.
    db.set_workspace_members(vec![b, a], vec![PathBuf::from("/s")]);
    let _ = workspace_project(&db);

    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::WorkspaceProject),
        None,
        "re-seeding an identical membership must not re-run workspace_project"
    );
}

#[test]
fn adding_a_member_rebuilds_workspace_project() {
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/s/a.R"), "foo <- 1\n".to_string());
    db.set_workspace_members(vec![a], vec![PathBuf::from("/s")]);
    let _ = workspace_project(&db);

    db.clear_query_log();
    let b = db.upsert_file(Path::new("/s/b.R"), "bar <- 2\n".to_string());
    db.set_workspace_members(vec![a, b], vec![PathBuf::from("/s")]);
    let project = workspace_project(&db);

    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::WorkspaceProject),
        Some(&1),
        "adding a member must re-run workspace_project"
    );
    assert_eq!(project.members(&db).len(), 2);
}

#[test]
fn workspace_def_sites_finds_a_cross_file_definition() {
    // b.R reads `foo`, defined at top level in a.R. The workspace def index must
    // point a cross-file go-to-definition at a.R, with a span spelling "foo".
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/s/a.R"), "foo <- function() 1\n".to_string());
    let b = db.upsert_file(
        Path::new("/s/b.R"),
        "bar <- function() {\n  foo()\n}\n".to_string(),
    );
    db.set_workspace_members(vec![a, b], vec![PathBuf::from("/s")]);

    let snapshot = db.snapshot();
    let sites = snapshot.workspace_def_sites("foo");
    assert_eq!(sites.len(), 1, "foo is defined once, in a.R");
    let (path, range) = &sites[0];
    assert_eq!(path, Path::new("/s/a.R"));
    let text = snapshot.file_text(a);
    let start: usize = range.start().into();
    let end: usize = range.end().into();
    assert_eq!(&text[start..end], "foo");
    assert_eq!(start, 0);
}

#[test]
fn workspace_def_sites_span_tracks_current_text_after_edit() {
    // Range-free index + def_range_in: a leading-line edit in the defining file
    // must shift the recovered span to the post-edit position.
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/s/a.R"), "foo <- function() 1\n".to_string());
    let b = db.upsert_file(Path::new("/s/b.R"), "bar <- function() foo()\n".to_string());
    db.set_workspace_members(vec![a, b], vec![PathBuf::from("/s")]);

    db.set_file_text(a, "# header\nfoo <- function() 1\n");

    let snapshot = db.snapshot();
    let sites = snapshot.workspace_def_sites("foo");
    let (_, range) = sites.first().expect("foo still resolves after the edit");
    let start: usize = range.start().into();
    assert_eq!(start, "# header\n".len(), "span follows the edited text");
}

#[test]
fn workspace_def_sites_empty_without_a_workspace_or_match() {
    let db = IncrementalDatabase::default();
    let snapshot = db.snapshot();
    assert!(
        snapshot.workspace_def_sites("foo").is_empty(),
        "no workspace seeded"
    );
}

#[test]
fn project_reads_aggregates_free_reads_by_name() {
    // The read-site mirror of project_defs: `foo` is free-read in b.R but bound
    // (not free-read) in a.R, so the index points only at b.R.
    let (db, a, b) = package_ab("foo <- function() 1\n", "bar <- function() {\n  foo()\n}\n");
    let project = project_ab(&db, a, b);
    let reads = project_reads(&db, project);

    let foo = reads.by_name.get("foo").expect("foo is free-read");
    assert!(foo.contains(&PathBuf::from("/pkg/R/b.R")));
    assert!(
        !foo.contains(&PathBuf::from("/pkg/R/a.R")),
        "a.R binds foo, so it is not a free read there"
    );
}

#[test]
fn body_edit_does_not_rebuild_project_reads() {
    // The firewall: editing b.R's body re-runs its model and file_free_reads, but
    // the free-read name set is unchanged, so the project-wide aggregate is reused.
    let (mut db, a, b) = package_ab("foo <- function() 1\n", "bar <- function() {\n  foo()\n}\n");

    {
        let project = project_ab(&db, a, b);
        let _ = project_reads(&db, project);
    }

    db.clear_query_log();
    db.set_file_text(b, "bar <- function() {\n  foo()\n  2\n}\n");

    let project = project_ab(&db, a, b);
    let _ = project_reads(&db, project);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(counts.get(&QueryKind::SemanticModel), Some(&1));
    assert_eq!(
        counts.get(&QueryKind::ProjectReads),
        None,
        "a body edit must not rebuild project_reads"
    );
}

#[test]
fn workspace_read_sites_finds_a_cross_file_read() {
    // b.R reads `foo`, defined at top level in a.R. The workspace read index must
    // point a find-references at b.R, with a span spelling "foo".
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/s/a.R"), "foo <- function() 1\n".to_string());
    let b = db.upsert_file(
        Path::new("/s/b.R"),
        "bar <- function() {\n  foo()\n}\n".to_string(),
    );
    db.set_workspace_members(vec![a, b], vec![PathBuf::from("/s")]);

    let snapshot = db.snapshot();
    let sites = snapshot.workspace_read_sites("foo");
    assert_eq!(sites.len(), 1, "foo is read once, in b.R");
    let (path, range) = &sites[0];
    assert_eq!(path, Path::new("/s/b.R"));
    let text = snapshot.file_text(b);
    let start: usize = range.start().into();
    let end: usize = range.end().into();
    assert_eq!(&text[start..end], "foo");
    assert_eq!(start, text.find("foo()").expect("read site"));
}

#[test]
fn workspace_read_sites_span_tracks_current_text_after_edit() {
    // Range-free index + read_ranges_in: a leading-line edit in the *reading*
    // file must shift the recovered read span to the post-edit position.
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/s/a.R"), "foo <- function() 1\n".to_string());
    let b = db.upsert_file(Path::new("/s/b.R"), "bar <- function() foo()\n".to_string());
    db.set_workspace_members(vec![a, b], vec![PathBuf::from("/s")]);

    db.set_file_text(b, "# header\nbar <- function() foo()\n");

    let snapshot = db.snapshot();
    let sites = snapshot.workspace_read_sites("foo");
    let (_, range) = sites.first().expect("foo still read after the edit");
    let start: usize = range.start().into();
    assert_eq!(
        start,
        "# header\nbar <- function() ".len(),
        "span follows the edited text"
    );
}

#[test]
fn workspace_read_sites_empty_without_a_workspace_or_match() {
    let db = IncrementalDatabase::default();
    let snapshot = db.snapshot();
    assert!(
        snapshot.workspace_read_sites("foo").is_empty(),
        "no workspace seeded"
    );
}

#[test]
fn cross_file_binding_scopes_to_a_source_connected_reader() {
    // b sources a and reads foo; a defines foo. The binding's cohort is just a,
    // and b is a reader because it can see a.
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/s/a.R"), "foo <- function() 1\n".to_string());
    let b = db.upsert_file(
        Path::new("/s/b.R"),
        "source(\"a.R\")\nbar <- function() foo()\n".to_string(),
    );
    db.set_workspace_members(vec![a, b], vec![PathBuf::from("/s")]);

    let snapshot = db.snapshot();
    let binding = snapshot.cross_file_binding(Path::new("/s/a.R"), "foo");
    assert_eq!(binding.cohort, vec![PathBuf::from("/s/a.R")]);
    assert_eq!(binding.readers, vec![PathBuf::from("/s/b.R")]);
    assert!(!binding.conflict);
    assert!(!binding.dynamic_source_risk);
}

#[test]
fn cross_file_binding_excludes_disjoint_same_name_def() {
    // Two unconnected flat scripts each define foo. From a's perspective the
    // cohort is a alone — b's foo is an unrelated binding — even though the
    // global, name-keyed index lists both.
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/s/a.R"), "foo <- function() 1\n".to_string());
    let b = db.upsert_file(Path::new("/s/b.R"), "foo <- function() 2\n".to_string());
    db.set_workspace_members(vec![a, b], vec![PathBuf::from("/s")]);

    let snapshot = db.snapshot();
    assert_eq!(
        snapshot.workspace_def_sites("foo").len(),
        2,
        "the global index is name-keyed and lists both"
    );
    let binding = snapshot.cross_file_binding(Path::new("/s/a.R"), "foo");
    assert_eq!(binding.cohort, vec![PathBuf::from("/s/a.R")]);
    assert!(binding.readers.is_empty());
    assert!(!binding.conflict);
}

#[test]
fn body_edit_does_not_rebuild_project_scope() {
    // The firewall: editing b.R's function *body* changes its semantic model but
    // not its top-level exports / free reads / source edges, so the cross-file
    // project graph and per-file visibility memos must be reused.
    let (mut db, a, b) = package_ab("foo <- function() 1\n", "bar <- function() {\n  foo()\n}\n");

    // Materialize the graph + both files' visibility.
    {
        let project = project_ab(&db, a, b);
        let _ = visible_symbols(&db, project, a);
        let _ = visible_symbols(&db, project, b);
        assert!(visible_symbols(&db, project, b).visible.contains("foo"));
    }

    db.clear_query_log();

    // Edit b's body only: still defines `bar`, still reads `foo`.
    db.set_file_text(b, "bar <- function() {\n  foo()\n  2\n}\n");

    // Re-lint: re-intern the (unchanged) membership, as production does.
    let project = project_ab(&db, a, b);
    let _ = visible_symbols(&db, project, a);
    let _ = visible_symbols(&db, project, b);

    let counts = count_by_kind(&db.query_log());
    // b's parse + model re-run (the body changed)...
    assert_eq!(counts.get(&QueryKind::SemanticModel), Some(&1));
    // ...but its exports/free-reads are unchanged, so the graph and visibility
    // memos are reused — the whole point of the firewall.
    assert_eq!(
        counts.get(&QueryKind::ProjectGraph),
        None,
        "project graph must not rebuild on a body edit"
    );
    assert_eq!(
        counts.get(&QueryKind::VisibleSymbols),
        None,
        "per-file visibility must not rebuild on a body edit"
    );
}

#[test]
fn export_change_rebuilds_project_scope() {
    // The complement: adding a top-level binding changes b's exports, so the
    // graph and visibility *must* rebuild (the firewall doesn't over-cache).
    let (mut db, a, b) = package_ab("foo <- function() 1\n", "bar <- function() {\n  foo()\n}\n");

    {
        let project = project_ab(&db, a, b);
        let _ = visible_symbols(&db, project, a);
        let _ = visible_symbols(&db, project, b);
    }

    db.clear_query_log();

    // Add a new top-level binding: b's exports change.
    db.set_file_text(b, "bar <- function() {\n  foo()\n}\nqux <- 2\n");

    let project = project_ab(&db, a, b);
    let _ = visible_symbols(&db, project, a);
    let _ = visible_symbols(&db, project, b);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(
        counts.get(&QueryKind::ProjectGraph),
        Some(&1),
        "project graph must rebuild when an export changes"
    );
    assert!(
        counts.get(&QueryKind::VisibleSymbols).copied().unwrap_or(0) >= 1,
        "visibility must rebuild when an export changes"
    );
}

#[test]
fn body_edit_reruns_top_level_events_but_backdates() {
    // The new order-bearing firewall: editing b.R's function *body* re-extracts
    // its top-level event sequence (the tree changed), but the sequence is
    // unchanged, so it backdates and the project graph above it is spared.
    let (mut db, a, b) = package_ab("foo <- function() 1\n", "bar <- function() {\n  foo()\n}\n");
    {
        let project = project_ab(&db, a, b);
        let _ = top_level_events(&db, b);
        let _ = visible_symbols(&db, project, b);
    }

    db.clear_query_log();
    db.set_file_text(b, "bar <- function() {\n  foo()\n  2\n}\n");

    let project = project_ab(&db, a, b);
    let _ = top_level_events(&db, b);
    let _ = visible_symbols(&db, project, b);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(
        counts.get(&QueryKind::TopLevelEvents),
        Some(&1),
        "the event sequence is re-extracted when the body changes"
    );
    assert_eq!(
        counts.get(&QueryKind::ProjectGraph),
        None,
        "but the unchanged sequence backdates, sparing the project graph"
    );
}

#[test]
fn reordering_top_level_statements_rebuilds_project_graph() {
    // Reordering two top-level reads leaves exports / free-reads / source-edges
    // (all order-independent sets) unchanged — only the *ordered* top_level_events
    // firewall differs. That alone must rebuild the graph, since load-order
    // resolution depends on the order.
    let (mut db, a, b) = package_ab("foo <- function() 1\n", "foo\nbar\n");
    {
        let project = project_ab(&db, a, b);
        let _ = visible_symbols(&db, project, b);
    }

    db.clear_query_log();
    db.set_file_text(b, "bar\nfoo\n");

    let project = project_ab(&db, a, b);
    let _ = visible_symbols(&db, project, b);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(
        counts.get(&QueryKind::ProjectGraph),
        Some(&1),
        "reordered top-level events must rebuild the graph even though the sets are unchanged"
    );
}

#[test]
fn reinterning_same_membership_reuses_graph_memo() {
    // Interning a fresh `Project` from an unchanged membership snapshot yields
    // the same id, so the graph memo is reused — this is what keeps the scope
    // warm across lints that re-discover the same set of files.
    let (db, a, b) = package_ab("foo <- function() 1\n", "bar <- function() foo()\n");
    let project = project_ab(&db, a, b);
    let _ = visible_symbols(&db, project, a);
    let _ = visible_symbols(&db, project, b);

    db.clear_query_log();

    // Re-intern the identical membership (same files, same roots, no namespaces).
    let project2 = project_ab(&db, a, b);
    assert!(
        project == project2,
        "same membership should re-intern to the same id"
    );

    let _ = visible_symbols(&db, project2, b);
    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::ProjectGraph),
        None,
        "an unchanged membership must not rebuild the graph"
    );
}

/// A single-file "project" with no package root (a bare script), so cross-file
/// visibility is complete and resolution turns purely on the library index.
fn project_one<'db>(db: &'db IncrementalDatabase, file: SourceFile, path: &str) -> Project<'db> {
    let members = vec![ProjectMember {
        file,
        path: PathBuf::from(path),
        package_root: None,
    }];
    Project::new(db, members, Vec::new(), Vec::new(), Vec::new())
}

/// A harvested package index for `name` exporting `exports`.
fn index_pkg(name: &str, exports: &[&str]) -> PackageIndex {
    PackageIndex {
        schema_version: SCHEMA_VERSION,
        package: name.into(),
        version: "1.0".into(),
        lib_path: "/lib".into(),
        title: None,
        r_version: None,
        harvested_at: 0,
        attaches: Vec::new(),
        symbols: exports
            .iter()
            .map(|n| SymbolEntry {
                name: (*n).into(),
                kind: SymbolKind::Function,
                exported: true,
                formals: None,
                help: None,
            })
            .collect(),
    }
}

#[test]
fn body_edit_does_not_rerun_external_resolution() {
    // The library firewall: editing a function *body* changes the model but not
    // the free-read / loaded-package sets, so `external_resolution` — whose only
    // other dependency is the HIGH-durability library index — must backdate
    // rather than re-run. This is the "keystroke skips the library subgraph" win.
    let mut db = IncrementalDatabase::default();
    let path = "/proj/a.R";
    let file = db.upsert_file(
        Path::new(path),
        "foo <- function() {\n  bar(1)\n}\n".to_string(),
    );
    let manifest = db.set_library_index(IndexedProvider::empty());

    {
        let project = project_one(&db, file, path);
        let res = external_resolution(&db, manifest, project, file);
        assert!(
            res.unresolved.contains("bar"),
            "bar is undefined: {:?}",
            res.unresolved
        );
    }

    db.clear_query_log();

    // Edit the body only: add a literal statement. Free reads ({bar}) and loaded
    // packages ({}) are unchanged.
    db.set_file_text(file, "foo <- function() {\n  bar(1)\n  2\n}\n");

    let project = project_one(&db, file, path);
    let _ = external_resolution(&db, manifest, project, file);

    let counts = count_by_kind(&db.query_log());
    // The body changed, so the model re-runs...
    assert_eq!(counts.get(&QueryKind::SemanticModel), Some(&1));
    // ...but resolution backdates: its free-read / loaded inputs are unchanged and
    // the library index is HIGH-durability, so it is not re-executed.
    assert_eq!(
        counts.get(&QueryKind::ExternalResolution),
        None,
        "external resolution must not re-run on a body edit"
    );
}

#[test]
fn swapping_library_index_invalidates_resolution() {
    // The complement: replacing the library index re-runs resolution (it is a
    // real dependency) without touching the text-derived queries (parse/model).
    let mut db = IncrementalDatabase::default();
    let path = "/proj/a.R";
    let file = db.upsert_file(
        Path::new(path),
        "library(somepkg)\nfoo <- function() across()\n".to_string(),
    );

    // somepkg is indexed (so the rule's gate passes) but does not yet export
    // `across`, so `across` is unresolved.
    let manifest = db.set_library_index(IndexedProvider::from_indices([index_pkg("somepkg", &[])]));
    {
        let project = project_one(&db, file, path);
        let res = external_resolution(&db, manifest, project, file);
        assert!(
            res.unresolved.contains("across"),
            "across unresolved before swap: {:?}",
            res.unresolved
        );
    }

    db.clear_query_log();

    // Swap the index: somepkg now exports `across`.
    db.set_library_index(IndexedProvider::from_indices([index_pkg(
        "somepkg",
        &["across"],
    )]));

    let project = project_one(&db, file, path);
    let res = external_resolution(&db, manifest, project, file);
    assert!(
        !res.unresolved.contains("across"),
        "across resolves after the swap: {:?}",
        res.unresolved
    );

    let counts = count_by_kind(&db.query_log());
    assert_eq!(
        counts.get(&QueryKind::ExternalResolution),
        Some(&1),
        "a library-index swap must re-run resolution"
    );
    // The text is unchanged, so the parse and model are orthogonal to the swap.
    assert_eq!(
        counts.get(&QueryKind::SemanticModel),
        None,
        "model must not re-run on a library-index swap"
    );
    assert_eq!(
        counts.get(&QueryKind::ParsedDocument),
        None,
        "parse must not re-run on a library-index swap"
    );
}

#[test]
fn unindexed_attached_package_suppresses_resolution() {
    // Conservative gate: when an attached package's exports are unknown (not
    // base, not indexed, not bundled), it could define any unresolved name, so
    // resolution yields nothing for the whole file.
    let mut db = IncrementalDatabase::default();
    let path = "/proj/a.R";
    let file = db.upsert_file(
        Path::new(path),
        "library(mysterypkgxyz)\nfoo <- function() across()\n".to_string(),
    );
    let manifest = db.set_library_index(IndexedProvider::empty());

    let project = project_one(&db, file, path);
    let res = external_resolution(&db, manifest, project, file);
    assert!(
        res.unresolved.is_empty(),
        "an unindexed attached package suppresses resolution: {:?}",
        res.unresolved
    );
}

#[test]
fn harvested_attach_sets_gate_and_resolve_external_resolution() {
    // A harvested meta-package (not in the static curated table) carries its
    // attach set in the library index; resolution and the conservative gate
    // must both honor it.
    let mut db = IncrementalDatabase::default();
    let path = "/proj/a.R";
    let file = db.upsert_file(
        Path::new(path),
        "library(metaverse)\nfoo <- function() {\n  helper_fn()\n  bogus()\n}\n".to_string(),
    );

    // Attached member indexed: its exports resolve, a genuine typo does not.
    let mut meta = index_pkg("metaverse", &[]);
    meta.attaches = vec!["helperpkg".into()];
    let manifest = db.set_library_index(IndexedProvider::from_indices([
        meta,
        index_pkg("helperpkg", &["helper_fn"]),
    ]));
    {
        let project = project_one(&db, file, path);
        let res = external_resolution(&db, manifest, project, file);
        assert!(
            !res.unresolved.contains("helper_fn"),
            "helper_fn resolves via the harvested attach set: {:?}",
            res.unresolved
        );
        assert!(
            res.unresolved.contains("bogus"),
            "bogus is genuinely undefined: {:?}",
            res.unresolved
        );
    }

    // Attach set naming an unknown member: that member could define any
    // unresolved name, so the whole file is suppressed.
    let mut meta = index_pkg("metaverse", &[]);
    meta.attaches = vec!["ghost_pkg_xyz".into()];
    let manifest = db.set_library_index(IndexedProvider::from_indices([meta]));
    let project = project_one(&db, file, path);
    let res = external_resolution(&db, manifest, project, file);
    assert!(
        res.unresolved.is_empty(),
        "an un-indexed harvested attach member suppresses resolution: {:?}",
        res.unresolved
    );
}

fn remote_exports(pkgs: &[(&str, &[&str])]) -> RemoteExports {
    let mut r = RemoteExports::new();
    for (pkg, names) in pkgs {
        r.insert_package(*pkg, names.iter().map(|n| smol_str::SmolStr::new(*n)));
    }
    r
}

#[test]
fn remote_sidecar_resolves_uninstalled_package() {
    // The downloadable sidecar lifts the conservative whole-file suppression of
    // `unindexed_attached_package_suppresses_resolution`: an attached package the
    // sidecar knows is fully resolvable, so a real export resolves while a genuine
    // non-export is reported undefined.
    let mut db = IncrementalDatabase::default();
    let path = "/proj/a.R";
    let file = db.upsert_file(
        Path::new(path),
        "library(tinytable)\nfoo <- function() {\n  tt()\n  bogus()\n}\n".to_string(),
    );

    // Without the sidecar, tinytable is neither indexed nor bundled: suppressed.
    let manifest = db.set_library_index(IndexedProvider::empty());
    {
        let project = project_one(&db, file, path);
        let res = external_resolution(&db, manifest, project, file);
        assert!(
            res.unresolved.is_empty(),
            "unindexed package suppresses the file: {:?}",
            res.unresolved
        );
    }

    // Install the sidecar: tinytable exports `tt` (but not `bogus`).
    let manifest = db.set_remote_exports(remote_exports(&[("tinytable", &["tt"])]));
    let project = project_one(&db, file, path);
    let res = external_resolution(&db, manifest, project, file);
    assert!(
        !res.unresolved.contains("tt"),
        "tt resolves via the sidecar: {:?}",
        res.unresolved
    );
    assert!(
        res.unresolved.contains("bogus"),
        "bogus is a genuine non-export: {:?}",
        res.unresolved
    );
}

#[test]
fn body_edit_does_not_rerun_resolution_with_remote_installed() {
    // The sidecar lives in the `remote` field of the HIGH-durability library
    // index, so — exactly like the harvested `data` field — a keystroke (a LOW
    // body edit) skips the resolution subgraph even when the sidecar is in play.
    let mut db = IncrementalDatabase::default();
    let path = "/proj/a.R";
    let file = db.upsert_file(
        Path::new(path),
        "library(tinytable)\nfoo <- function() {\n  tt(1)\n}\n".to_string(),
    );
    db.set_library_index(IndexedProvider::empty());
    let manifest = db.set_remote_exports(remote_exports(&[("tinytable", &["tt"])]));
    {
        let project = project_one(&db, file, path);
        let res = external_resolution(&db, manifest, project, file);
        assert!(
            !res.unresolved.contains("tt"),
            "tt resolves before the edit: {:?}",
            res.unresolved
        );
    }

    db.clear_query_log();

    // Body-only edit: free reads ({tt}) and loaded packages ({tinytable}) unchanged.
    db.set_file_text(
        file,
        "library(tinytable)\nfoo <- function() {\n  tt(1)\n  2\n}\n",
    );
    let project = project_one(&db, file, path);
    let _ = external_resolution(&db, manifest, project, file);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(counts.get(&QueryKind::SemanticModel), Some(&1));
    assert_eq!(
        counts.get(&QueryKind::ExternalResolution),
        None,
        "the remote sidecar is HIGH-durability; a body edit must not re-run resolution"
    );
}

/// Capture a handle to the first `CALL_EXPR` in `file`'s current parse.
fn first_call_ptr(db: &IncrementalDatabase, file: SourceFile) -> NodePtr {
    let snapshot = db.snapshot();
    let root = snapshot.parsed_tree(file);
    let call = root
        .descendants()
        .find(|node| node.kind() == SyntaxKind::CALL_EXPR)
        .expect("a call expression");
    NodePtr::from_node(&call)
}

#[test]
fn resolve_ptr_resolves_against_unchanged_text() {
    let db = IncrementalDatabase::default();
    let file = db.add_file("foo <- bar(2)\n");
    let text = db.snapshot().file_text(file).to_string();
    let ptr = first_call_ptr(&db, file);

    let snapshot = db.snapshot();
    let node = snapshot
        .resolve_ptr(file, ptr, &text, None)
        .expect("resolves on the same revision");
    assert_eq!(node.kind(), SyntaxKind::CALL_EXPR);
    assert_eq!(node.text(), "bar(2)");
}

#[test]
fn resolve_ptr_survives_edit_before_the_node() {
    let mut db = IncrementalDatabase::default();
    let old = "foo <- bar(2)\n";
    let file = db.add_file(old);
    let ptr = first_call_ptr(&db, file);
    let old = old.to_string();

    // Prepend a comment line: the call shifts down but is otherwise untouched.
    db.set_file_text(file, "# header\nfoo <- bar(2)\n");

    let snapshot = db.snapshot();
    let node = snapshot
        .resolve_ptr(file, ptr, &old, None)
        .expect("resolves after an edit before the node");
    assert_eq!(node.kind(), SyntaxKind::CALL_EXPR);
    assert_eq!(node.text(), "bar(2)");
}

#[test]
fn resolve_ptr_invalidates_when_the_node_is_edited() {
    let mut db = IncrementalDatabase::default();
    let old = "foo <- bar(2)\n";
    let file = db.add_file(old);
    let ptr = first_call_ptr(&db, file);
    let old = old.to_string();

    // Edit lands inside the call (`bar` -> `bazzz`): the handle is invalidated.
    db.set_file_text(file, "foo <- bazzz(2)\n");

    let snapshot = db.snapshot();
    assert!(snapshot.resolve_ptr(file, ptr, &old, None).is_none());
}

#[test]
fn resolve_ptr_precise_slice_survives_disjoint_edits_around_the_node() {
    // Two disjoint edits straddle the middle call: prepend a comment line and
    // append a trailing statement. A coalesced `diff_edit` spans from the first
    // change to the last — its replaced range swallows the call's interior and
    // the whole-text path invalidates the handle. The precise per-edit slice
    // keeps them disjoint, so the call survives.
    let mut db = IncrementalDatabase::default();
    let old = "foo <- bar(2)\n";
    let file = db.add_file(old);
    let ptr = first_call_ptr(&db, file);
    let old = old.to_string();

    db.set_file_text(file, "# header\nfoo <- bar(2)\nq <- 9\n");

    // Edits in application order, each against the text its predecessor left:
    // insert the header at the top, then append the statement at the new tail.
    let edits = [
        Edit {
            range: 0..0,
            insert: "# header\n".to_string(),
        },
        Edit {
            range: 23..23,
            insert: "q <- 9\n".to_string(),
        },
    ];

    let snapshot = db.snapshot();
    // The coalesced whole-text path cannot re-resolve this.
    assert!(
        snapshot.resolve_ptr(file, ptr, &old, None).is_none(),
        "a single spanning diff_edit straddles the node interior"
    );
    // The precise slice keeps the edits disjoint and re-anchors the call.
    let node = snapshot
        .resolve_ptr(file, ptr, &old, Some(&edits))
        .expect("precise slice re-anchors the call between the two edits");
    assert_eq!(node.kind(), SyntaxKind::CALL_EXPR);
    assert_eq!(node.text(), "bar(2)");
}

#[test]
fn resolve_ptr_stale_slice_falls_back_to_diff_edit() {
    // A slice that does not reconstruct the current text (here it omits the
    // trailing append) must be rejected by the apply-and-verify guard and the
    // resolver degrades to the whole-text `diff_edit` path — same result as
    // passing `None`, never a wrong node or a panic.
    let mut db = IncrementalDatabase::default();
    let old = "foo <- bar(2)\n";
    let file = db.add_file(old);
    let ptr = first_call_ptr(&db, file);
    let old = old.to_string();

    db.set_file_text(file, "# header\nfoo <- bar(2)\nq <- 9\n");

    // Only the prepend; the append is missing, so applying this to `old` does
    // not yield the current buffer.
    let stale = [Edit {
        range: 0..0,
        insert: "# header\n".to_string(),
    }];

    let snapshot = db.snapshot();
    assert_eq!(
        snapshot
            .resolve_ptr(file, ptr, &old, Some(&stale))
            .is_none(),
        snapshot.resolve_ptr(file, ptr, &old, None).is_none(),
        "a stale slice must behave exactly like the diff_edit fallback"
    );
}

#[test]
fn resolve_ptr_reuses_the_cached_parse() {
    // Tenet 2: re-resolution is a pure read — it must not trigger a reparse.
    let mut db = IncrementalDatabase::default();
    let file = db.add_file("foo <- bar(2)\nx <- 1\n");
    db.set_file_text(file, "foo <- bar(2)\nx <- 2\n");
    let text = "foo <- bar(2)\nx <- 2\n".to_string();
    let ptr = first_call_ptr(&db, file);
    let hits = db.reparse_hits();

    let snapshot = db.snapshot();
    let node = snapshot
        .resolve_ptr(file, ptr, &text, None)
        .expect("resolves on the warm tree");
    assert_eq!(node.text(), "bar(2)");
    drop(snapshot);

    assert_eq!(
        db.reparse_hits(),
        hits,
        "resolve_ptr must not perturb the reparse path"
    );
}

#[test]
fn workspace_project_is_pure_namespace_not_reread_on_keystroke() {
    // Model (b): `workspace_project` reads package metadata from the PackageGraph
    // salsa input, not from disk. A keystroke re-runs the query (parse-status
    // dependency) but must NOT pick up an on-disk NAMESPACE change; only an
    // explicit `refresh_package_graph` does.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir(root.join("R")).unwrap();
    std::fs::write(root.join("DESCRIPTION"), "Package: foo\n").unwrap();
    std::fs::write(root.join("NAMESPACE"), "export(foo)\n").unwrap();
    let member_path = root.join("R/foo.R");
    std::fs::write(&member_path, "foo <- function() 1\n").unwrap();

    let mut db = IncrementalDatabase::default();
    let m = db.upsert_file(&member_path, "foo <- function() 1\n".to_string());
    db.set_workspace_members(vec![m], vec![root.to_path_buf()]);

    let before: Vec<_> = workspace_project(&db).namespaces(&db).clone();
    assert_eq!(
        before,
        vec![(root.to_path_buf(), "export(foo)\n".to_string())],
        "the seeded NAMESPACE must be visible via the derived project"
    );

    // Mutate NAMESPACE on disk WITHOUT refreshing, then do a keystroke that
    // forces `workspace_project` to re-run.
    std::fs::write(root.join("NAMESPACE"), "export(bar)\n").unwrap();
    db.set_file_text(m, "foo <- function() 2\n");

    let after: Vec<_> = workspace_project(&db).namespaces(&db).clone();
    assert_eq!(
        before, after,
        "a keystroke must not re-read NAMESPACE from disk (query is pure)"
    );

    // An explicit refresh is the correct invalidation path: it re-reads disk and
    // re-runs the query with the new metadata.
    db.refresh_package_graph();
    let refreshed: Vec<_> = workspace_project(&db).namespaces(&db).clone();
    assert_eq!(
        refreshed,
        vec![(root.to_path_buf(), "export(bar)\n".to_string())],
        "refresh_package_graph must pick up the on-disk NAMESPACE change"
    );
}

/// A package on disk with a `DESCRIPTION`, one `R/` member, and a NAMESPACE,
/// seeded into a fresh database. Returns the db, the member input, and the root.
fn description_package(description: &str) -> (IncrementalDatabase, SourceFile, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir(root.join("R")).expect("R/");
    std::fs::write(root.join("DESCRIPTION"), description).expect("DESCRIPTION");
    std::fs::write(root.join("NAMESPACE"), "export(foo)\n").expect("NAMESPACE");
    let member_path = root.join("R/foo.R");
    std::fs::write(&member_path, "foo <- function() 1\n").expect("foo.R");

    let mut db = IncrementalDatabase::default();
    let m = db.upsert_file(&member_path, "foo <- function() 1\n".to_string());
    db.set_workspace_members(vec![m], vec![root.to_path_buf()]);
    (db, m, dir)
}

const DESCRIPTION_BASE: &str = "Package: foo\n\
     Title: A Package\n\
     Depends: R (>= 4.1), stats\n\
     Imports: dplyr\n\
     Collate: foo.R\n";

#[test]
fn description_prose_edit_backdates_and_spares_the_project() {
    // THE headline guarantee of making DESCRIPTION a text input with an `Eq`
    // facts projection: an edit that changes no *fact* re-runs the parse and
    // stops there. Storing facts directly in `PackageGraph` would instead make
    // this a full project-graph rebuild, on every prose keystroke.
    let (mut db, _m, dir) = description_package(DESCRIPTION_BASE);
    let _ = workspace_project(&db);

    db.clear_query_log();
    std::fs::write(
        dir.path().join("DESCRIPTION"),
        DESCRIPTION_BASE.replace("Title: A Package", "Title: A Rather Nice Package"),
    )
    .expect("DESCRIPTION");
    db.refresh_package_graph();
    let _ = workspace_project(&db);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(
        counts.get(&QueryKind::DescriptionFacts),
        Some(&1),
        "the text changed, so the facts must be re-derived"
    );
    assert_eq!(
        counts.get(&QueryKind::WorkspaceProject),
        None,
        "the facts compare equal, so they must backdate and spare the project"
    );
}

#[test]
fn description_buffer_prose_edit_does_not_invalidate_the_project_graph() {
    // The same guarantee as above, but for the path the language server takes
    // now that a `DESCRIPTION` can be an open document: the text arrives from
    // `upsert_description` (the live buffer) rather than `refresh_package_graph`
    // (disk). A keystroke in `Description:` prose must still stop at the facts.
    let (mut db, _m, dir) = description_package(DESCRIPTION_BASE);
    let root = dir.path().to_path_buf();
    // Seed the root's `DESCRIPTION` input, as the lint thread does on open.
    db.refresh_descriptions([root.clone()]);
    let _ = workspace_project(&db);

    db.clear_query_log();
    let (_, changed) = db.upsert_description(
        &root,
        DESCRIPTION_BASE.replace("Title: A Package", "Title: A Rather Nice Package"),
    );
    assert!(changed, "the buffer text really did change");
    let _ = workspace_project(&db);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(
        counts.get(&QueryKind::DescriptionFacts),
        Some(&1),
        "the text changed, so the facts must be re-derived"
    );
    assert_eq!(
        counts.get(&QueryKind::WorkspaceProject),
        None,
        "the facts compare equal, so they must backdate and spare the project"
    );
}

#[test]
fn description_collate_edit_reaches_the_project() {
    // The negative control for the test above: a `Collate:` edit changes a fact
    // the project graph consumes, so it must propagate. Without this, "nothing
    // re-runs" could just as well mean the dependency was never wired up.
    let (mut db, _m, dir) = description_package(DESCRIPTION_BASE);
    let before = workspace_project(&db).collations(&db).clone();
    assert!(
        before.iter().all(|c| c.complete),
        "`Collate: foo.R` matches the analyzed member, so the package is complete"
    );

    db.clear_query_log();
    std::fs::write(
        dir.path().join("DESCRIPTION"),
        DESCRIPTION_BASE.replace("Collate: foo.R", "Collate: foo.R generated.R"),
    )
    .expect("DESCRIPTION");
    db.refresh_package_graph();
    let after = workspace_project(&db).collations(&db).clone();

    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::WorkspaceProject),
        Some(&1),
        "a Collate change is a fact the project consumes and must propagate"
    );
    assert!(
        after.iter().all(|c| !c.complete),
        "an un-analyzed collated source makes the package incomplete"
    );
}

/// The backdating firewall under `unused-dependency`. That rule reports on
/// *absence*, so it folds every member's package references — and if that fold
/// re-ran on every keystroke, an opt-in audit rule would cost the LSP a project
/// walk per character.
#[test]
fn a_body_edit_spares_the_package_usage_fold() {
    let (mut db, m, _dir) = description_package(DESCRIPTION_BASE);
    let project = workspace_project(&db);
    let _ = package_usage(&db, project);

    db.clear_query_log();
    // A body edit: the file still names no package and still exports `foo`.
    db.set_file_text(
        m,
        "foo <- function() 1 + 1
",
    );
    let project = workspace_project(&db);
    let _ = package_usage(&db, project);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(
        counts.get(&QueryKind::PackageReferences),
        Some(&1),
        "the text changed, so the per-file references must be re-derived"
    );
    assert_eq!(
        counts.get(&QueryKind::PackageUsage),
        None,
        "the references compare equal, so they must backdate and spare the fold"
    );
}

/// The negative control: adding a `pkg::` does change the fold's input, so it
/// must propagate. Without this, "nothing re-runs" could mean nothing is wired.
#[test]
fn a_new_qualified_call_reaches_the_package_usage_fold() {
    let (mut db, m, _dir) = description_package(DESCRIPTION_BASE);
    let project = workspace_project(&db);
    assert!(
        !package_usage(&db, project)[&_dir.path().to_path_buf()]
            .used
            .contains("dplyr")
    );

    db.clear_query_log();
    db.set_file_text(
        m,
        "foo <- function() dplyr::filter(x)
",
    );
    let project = workspace_project(&db);
    let usage = package_usage(&db, project);

    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::PackageUsage),
        Some(&1),
        "a new package reference is a fact the fold consumes and must propagate"
    );
    assert!(usage[&_dir.path().to_path_buf()].used.contains("dplyr"));
}

#[test]
fn r_keystroke_does_not_revalidate_description_facts() {
    // The durability guard. `DescriptionFile.text` is written at MEDIUM and a
    // keystroke is a LOW write, so a body edit must not touch the DCF subgraph
    // at all — not even to revalidate it.
    let (mut db, m, _dir) = description_package(DESCRIPTION_BASE);
    let _ = workspace_project(&db);

    db.clear_query_log();
    db.set_file_text(m, "foo <- function() 2\n");
    let _ = workspace_project(&db);

    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::DescriptionFacts),
        None,
        "a body edit must never re-derive DESCRIPTION facts"
    );
}

#[test]
fn refresh_descriptions_skips_write_when_unchanged() {
    // The conditional setter, as for every other input: re-reading identical
    // bytes must not bump the revision.
    let (mut db, _m, _dir) = description_package(DESCRIPTION_BASE);
    let _ = workspace_project(&db);

    db.clear_query_log();
    db.refresh_package_graph();
    let _ = workspace_project(&db);

    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::DescriptionFacts),
        None,
        "an unchanged DESCRIPTION refresh must write nothing"
    );
}

#[test]
fn package_facts_resolve_for_a_member_without_disk() {
    // What the lint rules read instead of walking to the package root and
    // re-reading DESCRIPTION per file, per question. Deleting the file proves
    // the lookup goes through the tracked input, not disk.
    let (db, _m, dir) = description_package(DESCRIPTION_BASE);
    let member = dir.path().join("R/foo.R");
    std::fs::remove_file(dir.path().join("DESCRIPTION")).expect("remove");

    let facts = package_facts_for(&db, &member).expect("the member's package is tracked");
    assert_eq!(facts.package.as_deref(), Some("foo"));
    assert_eq!(facts.attached_packages(), ["stats".to_string()].into());

    // A file outside any package has none.
    assert!(package_facts_for(&db, Path::new("/elsewhere/loose.R")).is_none());
}

#[test]
fn an_untouched_metadata_refresh_reports_no_write() {
    // The watcher fires on any `DESCRIPTION`/`NAMESPACE` event, including saves
    // that changed nothing. The refreshers report whether they actually wrote,
    // so the lint thread can skip a re-lint that would find nothing new.
    let (mut db, _m, dir) = description_package(DESCRIPTION_BASE);

    assert!(
        !db.refresh_package_graph(),
        "identical on-disk metadata must not write"
    );
    assert!(
        !db.refresh_description(dir.path()),
        "an identical DESCRIPTION must not write"
    );

    std::fs::write(
        dir.path().join("DESCRIPTION"),
        DESCRIPTION_BASE.replace("Title: A Package", "Title: Renamed"),
    )
    .expect("DESCRIPTION");
    assert!(
        db.refresh_description(dir.path()),
        "a real edit must write, even when it changes no fact"
    );
}

#[test]
fn description_facts_are_tracked_per_root() {
    // The facts really are derived from the tracked input, and `R` never
    // reaches the dependency list.
    let (db, _m, dir) = description_package(DESCRIPTION_BASE);
    let file = db
        .lookup_description(dir.path())
        .expect("the package root has a tracked DESCRIPTION");
    let facts = description_facts(&db, file);

    assert_eq!(facts.package.as_deref(), Some("foo"));
    assert_eq!(facts.attached_packages(), ["stats".to_string()].into());
    assert_eq!(
        facts.declared_packages(),
        ["dplyr".to_string(), "stats".to_string()].into()
    );
    assert!(
        !facts.declared_packages().contains("R"),
        "`R` names the language; an attach set containing it would suppress \
         every diagnostic in the package"
    );
}

#[test]
fn depends_attaches_but_imports_does_not() {
    // The R semantics the whole stage hangs on. `Depends: pkg` puts pkg on the
    // search path, so a bare export of it resolves. `Imports: pkg` does not —
    // R reaches an imported package only through `pkg::` or a NAMESPACE
    // `importFrom`/`import`, so resolving it here would accept code that fails
    // under `R CMD check`.
    let describe = |field: &str| format!("Package: foo\n{field}: helperpkg\n");
    let (mut db, m, dir) = description_package(&describe("Depends"));
    std::fs::write(dir.path().join("R/foo.R"), "foo <- function() helper(1)\n").expect("foo.R");
    db.set_file_text(m, "foo <- function() helper(1)\n");
    let manifest = db.set_library_index(IndexedProvider::from_indices([index_pkg(
        "helperpkg",
        &["helper"],
    )]));

    {
        let project = workspace_project(&db);
        let res = external_resolution(&db, manifest, project, m);
        assert!(
            res.unresolved.is_empty(),
            "a Depends package is attached, so its bare export resolves: {:?}",
            res.unresolved
        );
    }

    // The same package under `Imports` must leave the bare name unresolved.
    std::fs::write(dir.path().join("DESCRIPTION"), describe("Imports")).expect("DESCRIPTION");
    db.refresh_package_graph();
    let project = workspace_project(&db);
    let res = external_resolution(&db, manifest, project, m);
    assert!(
        res.unresolved.contains("helper"),
        "an Imports package is not attached, so a bare name stays unresolved: {:?}",
        res.unresolved
    );
}

#[test]
fn an_unindexed_depends_suppresses_the_file() {
    // The other edge of the same gate, and a real behavior change: a `Depends`
    // we cannot enumerate could define any unresolved name, so the whole file
    // is suppressed — exactly as an unindexed `library()` call already does.
    let (mut db, m, dir) = description_package("Package: foo\nDepends: nosuchpkgzz\n");
    let body = "foo <- function() mystery(1)\n";
    std::fs::write(dir.path().join("R/foo.R"), body).expect("foo.R");
    db.set_file_text(m, body);
    let manifest = db.set_library_index(IndexedProvider::empty());

    {
        let project = workspace_project(&db);
        let res = external_resolution(&db, manifest, project, m);
        assert!(
            res.unresolved.is_empty(),
            "an unindexed Depends could define the name, so the file is suppressed: {:?}",
            res.unresolved
        );
    }

    // Control: with no `Depends` at all, the very same name is reported. Without
    // this the assertion above would also hold if nothing resolved anything.
    std::fs::write(dir.path().join("DESCRIPTION"), "Package: foo\n").expect("DESCRIPTION");
    db.refresh_package_graph();
    let project = workspace_project(&db);
    let res = external_resolution(&db, manifest, project, m);
    assert!(
        res.unresolved.contains("mystery"),
        "with nothing attached the undefined name must be reported: {:?}",
        res.unresolved
    );
}

#[test]
fn wildcard_import_clears_once_the_package_is_indexed() {
    // The roadmap item: `import(pkg)` used to suppress the whole file
    // unconditionally. It now goes through the same enumerability gate as an
    // attached package, so the suppression lifts as soon as the index (or the
    // sidecar) can enumerate pkg — and a genuine typo in that file is reported.
    let (mut db, m, dir) = description_package("Package: foo\n");
    std::fs::write(dir.path().join("NAMESPACE"), "import(helperpkg)\n").expect("NAMESPACE");
    let body = "foo <- function() helper(mystery)\n";
    std::fs::write(dir.path().join("R/foo.R"), body).expect("foo.R");
    db.set_file_text(m, body);
    db.refresh_package_graph();

    // Unindexed: we cannot enumerate helperpkg, so the file stays suppressed.
    {
        let manifest = db.set_library_index(IndexedProvider::empty());
        let project = workspace_project(&db);
        let res = external_resolution(&db, manifest, project, m);
        assert!(
            res.unresolved.is_empty(),
            "an unenumerable wildcard import must still suppress: {:?}",
            res.unresolved
        );
    }

    // Indexed: `helper` resolves through the import, and `mystery` — which the
    // whole-file suppression used to hide — is finally reported.
    let manifest = db.set_library_index(IndexedProvider::from_indices([index_pkg(
        "helperpkg",
        &["helper"],
    )]));
    let project = workspace_project(&db);
    let res = external_resolution(&db, manifest, project, m);
    assert!(
        !res.unresolved.contains("helper"),
        "import(helperpkg) puts its exports in scope: {:?}",
        res.unresolved
    );
    assert!(
        res.unresolved.contains("mystery"),
        "with the import enumerable, a real typo must surface: {:?}",
        res.unresolved
    );
}

#[test]
fn dynamic_source_still_suppresses() {
    // The control for the poison lift: `resolution_incomplete` keeps its one
    // remaining meaning, and a dynamic `source()` still suppresses the file.
    let mut db = IncrementalDatabase::default();
    let path = "/proj/a.R";
    let file = db.upsert_file(
        Path::new(path),
        "source(paste0(dir, '/x.R'))\nfoo <- function() mystery(1)\n".to_string(),
    );
    let manifest = db.set_library_index(IndexedProvider::empty());
    let project = project_one(&db, file, path);
    let res = external_resolution(&db, manifest, project, file);
    assert!(
        res.unresolved.is_empty(),
        "a dynamic source() could supply any name, so the file stays suppressed: {:?}",
        res.unresolved
    );
}

#[test]
fn refresh_package_graph_skips_write_when_unchanged() {
    // The conditional setter: refreshing with identical on-disk metadata must not
    // bump the revision, so `workspace_project` is not re-run.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir(root.join("R")).unwrap();
    std::fs::write(root.join("DESCRIPTION"), "Package: foo\n").unwrap();
    std::fs::write(root.join("NAMESPACE"), "export(foo)\n").unwrap();
    let member_path = root.join("R/foo.R");
    std::fs::write(&member_path, "foo <- function() 1\n").unwrap();

    let mut db = IncrementalDatabase::default();
    let m = db.upsert_file(&member_path, "foo <- function() 1\n".to_string());
    db.set_workspace_members(vec![m], vec![root.to_path_buf()]);
    let _ = workspace_project(&db);

    db.clear_query_log();
    db.refresh_package_graph();
    let _ = workspace_project(&db);

    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::WorkspaceProject),
        None,
        "an unchanged package-metadata refresh must not re-run workspace_project"
    );
}

/// The per-file roxygen markdown flag keys the salsa parse: a directive-less
/// block resolves markdown from it, flipping it invalidates the parse, and an
/// incremental reparse under the flag stays identical to a full parse.
#[test]
fn roxygen_markdown_flag_keys_parse_and_reparse() {
    use arity::incremental::parsed_tree_root;

    let text = "f <- function() {\n  #' *emph* text\n  1\n}\n";
    let mut db = IncrementalDatabase::default();
    let file = db.add_file(text);

    let kinds = |root: &arity::syntax::SyntaxNode| {
        root.descendants_with_tokens()
            .map(|el| el.kind())
            .collect::<Vec<_>>()
    };

    // Off by default (add_file tracks an in-memory doc with no package).
    assert!(!kinds(&parsed_tree_root(&db, file)).contains(&SyntaxKind::ROXYGEN_MD_EMPH));

    // Flipping the flag re-parses the same text in markdown mode.
    db.set_roxygen_markdown(file, true);
    assert!(kinds(&parsed_tree_root(&db, file)).contains(&SyntaxKind::ROXYGEN_MD_EMPH));

    // An edit inside the block reparses incrementally *under the flag*: the
    // spliced tree keeps the markdown interpretation.
    let before_hits = db.reparse_hits();
    db.set_file_text(file, "f <- function() {\n  #' *emph* text\n  12\n}\n");
    let root = parsed_tree_root(&db, file);
    assert!(kinds(&root).contains(&SyntaxKind::ROXYGEN_MD_EMPH));
    assert_eq!(
        db.reparse_hits(),
        before_hits + 1,
        "edit reparses incrementally"
    );

    // Setting the same value again is a no-op (no invalidation).
    db.clear_query_log();
    db.set_roxygen_markdown(file, true);
    let _ = parsed_tree_root(&db, file);
    assert!(
        db.query_log().is_empty(),
        "re-setting an unchanged flag must not re-run the parse"
    );

    // Flipping back re-parses Rd-first.
    db.set_roxygen_markdown(file, false);
    assert!(!kinds(&parsed_tree_root(&db, file)).contains(&SyntaxKind::ROXYGEN_MD_EMPH));
}

/// `upsert_file` resolves the flag from the file's package at creation.
#[test]
fn upsert_file_resolves_markdown_from_package() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join("R")).expect("R/");
    std::fs::write(
        dir.path().join("DESCRIPTION"),
        "Package: p\nRoxygen: list(markdown = TRUE)\n",
    )
    .expect("DESCRIPTION");
    let path = dir.path().join("R/doc.R");
    let text = "#' *emph* doc\nNULL\n";
    std::fs::write(&path, text).expect("doc.R");

    let mut db = IncrementalDatabase::default();
    let file = db.upsert_file(&path, text.to_string());
    let root = arity::incremental::parsed_tree_root(&db, file);
    assert!(
        root.descendants_with_tokens()
            .any(|el| el.kind() == SyntaxKind::ROXYGEN_MD_EMPH),
        "package markdown default reaches the salsa parse"
    );
}
