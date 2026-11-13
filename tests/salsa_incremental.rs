use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use arity::incremental::{
    IncrementalDatabase, QueryKind, SourceFile, description_facts, file_def_sites, line_index,
    top_level_events,
};
use arity::parser::Edit;
use arity::project::{
    ClassId, ClassIndex, DefKind, PackageInfo, external_resolution, package_facts_for,
    package_usage, project_classes, project_defs, project_graph, project_promises, project_reads,
    project_roxygen_topics, reverse_source_edges, visible_symbols, workspace_project,
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
fn upsert_of_equal_text_in_a_fresh_allocation_writes_nothing() {
    // The guard in `upsert_file` is the only thing standing between a no-op
    // upsert and a rebuild of the world: salsa's setter never compares, it just
    // bumps the revision. So the guard must answer on *content*, not on identity
    // — typing a character and deleting it again leaves the buffer holding equal
    // text in a brand-new allocation, and that must still count as unchanged.
    //
    // This is what fails if the `Arc::ptr_eq` fast path is ever put in place of
    // the content compare rather than in front of it.
    use std::path::Path;
    let mut db = IncrementalDatabase::default();
    let path = Path::new("/proj/a.R");

    let file = db.upsert_file(path, "x <- 1\n".to_string());
    let _ = db.parsed_tree(file);
    db.clear_query_log();

    // Built at runtime so it cannot share a `'static` allocation with the first.
    let same: String = "x <- 1\n".chars().collect();
    let again = db.upsert_file(path, same);
    let _ = db.parsed_tree(again);

    assert!(file == again, "same path must reuse the SourceFile input");
    assert!(
        db.query_log().is_empty(),
        "re-upserting equal text must not invalidate anything, got {} queries",
        db.query_log().len()
    );
}

#[test]
fn the_write_phase_hands_off_the_buffer_allocation_rather_than_a_copy() {
    // The language server's write phase is `upsert_file(path, buffer.text_arc())`
    // once per keystroke. It must move a handle, not a document: comparing the
    // data pointers is the only way to tell a refcount bump from an O(N) copy
    // that happens to compare equal.
    //
    // This is what fails if a `.text().to_string()` is ever reintroduced on the
    // dispatch path -- the assertion below, not a benchmark, is the tripwire.
    use arity::text::TextBuffer;
    use std::path::Path;
    let mut db = IncrementalDatabase::default();
    let path = Path::new("/proj/a.R");
    let buffer = TextBuffer::from("f <- function() 1\n");

    let file = db.upsert_file(path, buffer.text_arc());

    assert!(
        std::ptr::eq(db.file_text(file).as_ptr(), buffer.text().as_ptr()),
        "the tracked text must be the buffer's own allocation"
    );
    // And the read path's staleness gate settles on that shared handle, which is
    // what makes hover/completion/signature/format O(1) instead of O(N).
    assert!(
        db.snapshot().file_text_is(file, &buffer.text_arc()),
        "an unedited buffer must match the tracked text"
    );
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
fn single_staged_edit_uses_precise_reparse() {
    // The common case: one keystroke, one staged edit. It takes the precise path
    // like a multi-cursor batch does -- there is no two-edit floor -- so the
    // parse never re-derives an edit it was just handed.
    let mut db = IncrementalDatabase::default();
    let path = Path::new("/proj/a.R");
    let base = "x <- foo(alpha)\ny <- bar(b)\n";
    let file = db.upsert_file(path, base.to_string());

    let _ = db.parsed_tree(file);
    assert_eq!(db.precise_reparse_hits(), 0);

    let at = base.find("alpha").unwrap() + 1;
    db.stage_edits(
        file,
        vec![Edit {
            range: at..at,
            insert: "X".to_string(),
        }],
    );
    let edited = "x <- foo(aXlpha)\ny <- bar(b)\n";
    db.upsert_file(path, edited.to_string());

    assert_eq!(db.parsed_tree(file).text().to_string(), edited);
    assert_eq!(db.reparse_hits(), 1);
    assert_eq!(
        db.precise_reparse_hits(),
        1,
        "a single staged edit should be served by the precise path"
    );

    let fresh = arity::parser::parse(edited);
    assert_eq!(
        db.parsed_tree(file).text().to_string(),
        fresh.cst.text().to_string()
    );
    assert!(db.parse_diagnostics(file).is_empty());
}

#[test]
fn out_of_bounds_staged_edit_falls_back_to_diff_edit() {
    // Staged edits are caller data. A range the previous text cannot hold (a
    // coalescing gap, a misordered batch) must be rejected like any other stale
    // sequence -- the lint thread owns the only database, so a panic here would
    // take the whole language server's analysis down.
    let mut db = IncrementalDatabase::default();
    let path = Path::new("/proj/a.R");
    let base = "x <- foo(a)\n";
    let file = db.upsert_file(path, base.to_string());
    let _ = db.parsed_tree(file);

    db.stage_edits(
        file,
        vec![Edit {
            range: 9_999..9_999,
            insert: ", w".to_string(),
        }],
    );
    let edited = "x <- foo(a, w)\n";
    db.upsert_file(path, edited.to_string());

    assert_eq!(
        db.parsed_tree(file).text().to_string(),
        edited,
        "buffer still correct via diff_edit fallback"
    );
    assert_eq!(
        db.precise_reparse_hits(),
        0,
        "an unapplicable staged edit must not drive the tree"
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
/// body. Returns the db and the two tracked inputs. The synthetic `/pkg` root
/// does not exist on disk, so the seed's package discovery finds nothing; the
/// package metadata input is installed directly, exactly as
/// `discover_packages` would have from a real checkout.
fn package_ab(a_src: &str, b_src: &str) -> (IncrementalDatabase, SourceFile, SourceFile) {
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/pkg/R/a.R"), a_src.to_string());
    let b = db.upsert_file(Path::new("/pkg/R/b.R"), b_src.to_string());
    db.set_workspace_members(vec![a, b], vec![PathBuf::from("/pkg")]);
    db.set_package_metadata(vec![PackageInfo {
        root: PathBuf::from("/pkg"),
        namespace: None,
        dir_sources: BTreeSet::new(),
        native_routines: BTreeSet::new(),
    }]);
    (db, a, b)
}

#[test]
fn r6_runtime_reads_do_not_keep_sibling_lexical_names_alive() {
    use arity::incremental::file_free_reads;
    let (db, a, b) = package_ab(
        "self <- 1\nprivate <- 2\nsuper <- 3\nParent <- R6::R6Class(private = list(value = 1))\n",
        "Child <- R6::R6Class(inherit = Parent, public = list(read = function() list(self, private, super)))\n",
    );
    let reads = file_free_reads(&db, b);
    for name in ["self", "private", "super"] {
        assert!(!reads.contains(name), "{reads:?}");
        assert!(
            !project_graph(&db)
                .for_file(Path::new("/pkg/R/a.R"))
                .read_elsewhere(name)
        );
        assert!(db.snapshot().read_ranges_in(b, name).is_empty());
    }
    assert!(reads.contains("Parent"));
    assert!(
        db.semantic_model(a)
            .bindings()
            .iter()
            .any(|binding| binding.name == "private")
    );
}

#[test]
fn r6_environment_summaries_backdate_while_live_read_spans_follow_edits() {
    use arity::incremental::method_environments;
    let source = "A <- R6::R6Class(public = list(read = function() self))\n";
    let (mut db, file) = script_one(source);
    let before = method_environments(&db, file).clone();
    let model = db.runtime_semantic_model(file);
    let read = model
        .idents()
        .iter()
        .find(|ident| ident.name == "self")
        .unwrap();
    assert!(!model.is_free_read(read));
    db.clear_query_log();
    let edited = format!(
        "# Prose.\n{}",
        source.replace("function() self", "function() { print(1); self }")
    );
    db.set_file_text(file, edited.clone());
    assert_eq!(method_environments(&db, file), &before);
    let model = db.runtime_semantic_model(file);
    let read = model
        .idents()
        .iter()
        .find(|ident| ident.name == "self")
        .unwrap();
    assert_eq!(
        usize::from(read.range.start()),
        edited.find("self").unwrap()
    );
    assert!(!model.is_free_read(read));
    let counts = count_by_kind(&db.query_log());
    assert!(!counts.contains_key(&QueryKind::MethodEnvironments));
    assert!(!counts.contains_key(&QueryKind::ProjectClasses));
}

#[test]
fn r6_environments_invalidate_for_parent_members_inheritance_and_imports() {
    use arity::incremental::method_environments;
    use arity::semantic::enclosure::Availability;
    let (mut db, a, b) = package_ab(
        "Parent <- R6::R6Class()\n",
        "Child <- R6::R6Class(inherit = Parent, public = list(read = function() private))\n",
    );
    assert_eq!(
        method_environments(&db, b)[0].bindings[1],
        Availability::Absent
    );
    let model = db.runtime_semantic_model(b);
    assert!(
        model.is_free_read(
            model
                .idents()
                .iter()
                .find(|ident| ident.name == "private")
                .unwrap()
        )
    );
    db.set_file_text(a, "Parent <- R6::R6Class(private = list(value = 1))\n");
    assert_eq!(
        method_environments(&db, b)[0].bindings[1],
        Availability::Present
    );
    let model = db.runtime_semantic_model(b);
    assert!(
        !model.is_free_read(
            model
                .idents()
                .iter()
                .find(|ident| ident.name == "private")
                .unwrap()
        )
    );
    db.set_file_text(
        b,
        "Child <- R6::R6Class(public = list(read = function() super))\n",
    );
    assert_eq!(
        method_environments(&db, b)[0].bindings[2],
        Availability::Absent
    );
    db.set_file_text(
        b,
        "Child <- R6::R6Class(inherit = Parent, public = list(read = function() super))\n",
    );
    assert_eq!(
        method_environments(&db, b)[0].bindings[2],
        Availability::Present
    );

    db.set_file_text(
        b,
        "Child <- R6Class(public = list(read = function() self))\n",
    );
    let metadata = |namespace: &str| {
        vec![PackageInfo {
            root: PathBuf::from("/pkg"),
            namespace: Some(namespace.into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]
    };
    db.set_package_metadata(metadata("importFrom(R6, R6Class)\n"));
    assert_eq!(method_environments(&db, b).len(), 1);
    db.set_package_metadata(metadata("importFrom(other, R6Class)\n"));
    assert!(method_environments(&db, b).is_empty());
    let model = db.runtime_semantic_model(b);
    assert!(
        model.is_free_read(
            model
                .idents()
                .iter()
                .find(|ident| ident.name == "self")
                .unwrap()
        )
    );
}

#[test]
fn r6_implementation_aliases_preserve_possible_external_reads_and_refuse_rename() {
    let (db, _, b) = package_ab(
        "self <- 1\n",
        "helper <- function() self\nA <- R6::R6Class(public = list(read = helper))\nhelper()\n",
    );
    assert!(
        project_graph(&db)
            .for_file(Path::new("/pkg/R/a.R"))
            .read_elsewhere("self")
    );
    assert_eq!(db.snapshot().read_ranges_in(b, "self").len(), 1);
    assert!(
        db.snapshot()
            .reader_rename_ranges(
                Path::new("/pkg/R/b.R"),
                "self",
                &[PathBuf::from("/pkg/R/a.R")]
            )
            .is_none()
    );
}

/// A single loose script at `/proj/a.R`, seeded as the whole workspace. Seeding
/// happens once, here: re-seeding an unchanged membership before each query is
/// a no-op (see `reseeding_identical_membership_skips_write`).
fn script_one(src: &str) -> (IncrementalDatabase, SourceFile) {
    let mut db = IncrementalDatabase::default();
    let file = db.upsert_file(Path::new("/proj/a.R"), src.to_string());
    db.set_workspace_members(vec![file], Vec::new());
    (db, file)
}

/// A two-script project where `a.R` sources `b.R`. No package root, so the only
/// cross-file relation is the `source()` edge.
fn scripts_ab(a_src: &str, b_src: &str) -> (IncrementalDatabase, SourceFile, SourceFile) {
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/s/a.R"), a_src.to_string());
    let b = db.upsert_file(Path::new("/s/b.R"), b_src.to_string());
    db.set_workspace_members(vec![a, b], vec![PathBuf::from("/s")]);
    (db, a, b)
}

fn s3_probe(
    db: &IncrementalDatabase,
    file: SourceFile,
    path: &str,
) -> arity::project::classes::S3ClassEvidence {
    use arity::ast::CallExpr;
    use rowan::ast::AstNode as _;
    let root = db.parsed_tree(file);
    let model = db.semantic_model(file);
    let value = root
        .descendants()
        .filter_map(CallExpr::cast)
        .find(|call| call.callee_name().as_deref() == Some("probe"))
        .unwrap()
        .arg_list()
        .unwrap()
        .args()
        .next()
        .unwrap()
        .value()
        .unwrap();
    project_classes(db).s3_class_evidence(Path::new(path), &root, model, &value)
}

#[test]
fn s3_constructor_evidence_follows_project_visibility_and_helper_origins() {
    use arity::project::classes::S3ClassEvidence::{Explicit, Unknown};
    let constructor =
        "new_value <- function(x) structure(list(x = x), class = c('child', 'parent'))\n";
    let (db, a, _) = scripts_ab("source('b.R')\nprobe(new_value(1))\n", constructor);
    assert_eq!(
        s3_probe(&db, a, "/s/a.R"),
        Explicit(vec!["child".into(), "parent".into()])
    );
    let (db, a, _) = scripts_ab("probe(new_value(1))\n", constructor);
    assert_eq!(s3_probe(&db, a, "/s/a.R"), Unknown);
    let (db, a, _) = package_ab("f <- function() probe(new_value(1))\n", constructor);
    assert_eq!(
        s3_probe(&db, a, "/pkg/R/a.R"),
        Explicit(vec!["child".into(), "parent".into()])
    );
    let (db, a, _) = package_ab(
        "f <- function() probe(new_value(1))\nstructure <- function(...) NULL\n",
        constructor,
    );
    assert_eq!(s3_probe(&db, a, "/pkg/R/a.R"), Unknown);
    assert!(project_classes(&db).declarations.is_empty());
    assert!(project_classes(&db).supertypes.is_empty());
}

#[test]
fn s3_constructor_summaries_reject_embedded_control_flow() {
    use arity::project::classes::S3ClassEvidence::Unknown;
    for constructor in [
        "new_value <- function(flag) { flag && return(structure(list(), class = 'early')); structure(list(), class = 'late') }",
        "ret <- return; new_value <- function() { list(ret(structure(list(), class = 'early'))); structure(list(), class = 'late') }",
        "dispatch <- UseMethod; new_value <- function(x) { dispatch('generic'); structure(list(), class = 'late') }; generic.early <- function(x) structure(list(), class = 'early')",
    ] {
        let (db, a, _) = scripts_ab("source('b.R')\nprobe(new_value(TRUE))\n", constructor);
        assert_eq!(s3_probe(&db, a, "/s/a.R"), Unknown, "{constructor}");
    }
}

#[test]
fn s3_constructor_evidence_backdates_payloads_but_refreshes_class_order() {
    use arity::project::classes::S3ClassEvidence::Explicit;
    let (mut db, a, b) = package_ab(
        "f <- function() probe(new_value())\n",
        "new_value <- function() structure(list(value = 1), class = c('child', 'parent'))\n",
    );
    assert_eq!(
        s3_probe(&db, a, "/pkg/R/a.R"),
        Explicit(vec!["child".into(), "parent".into()])
    );
    let before = arity::incremental::file_class_defs(&db, b).clone();
    db.clear_query_log();
    db.set_file_text(b, "# moved\nnew_value <- function() structure(list(value = 20), class = c('child', 'parent'))\n");
    assert_eq!(arity::incremental::file_class_defs(&db, b), &before);
    assert_eq!(
        s3_probe(&db, a, "/pkg/R/a.R"),
        Explicit(vec!["child".into(), "parent".into()])
    );
    assert!(
        !db.query_log()
            .iter()
            .any(|entry| entry.kind == QueryKind::ProjectClasses)
    );
    db.set_file_text(
        b,
        "new_value <- function() structure(list(value = 20), class = c('parent', 'child'))\n",
    );
    assert_eq!(
        s3_probe(&db, a, "/pkg/R/a.R"),
        Explicit(vec!["parent".into(), "child".into()])
    );
    assert!(
        db.query_log()
            .iter()
            .any(|entry| entry.kind == QueryKind::ProjectClasses)
    );
}

#[test]
fn body_edit_does_not_rebuild_reverse_source_edges() {
    // The firewall: `a.R` sources `b.R`; editing `b.R`'s function body re-runs
    // its model but not its (empty) source edges, so the reverse map's memo —
    // built on the project snapshot + per-file source_edges — must be reused.
    let (mut db, _a, b) = scripts_ab("source(\"b.R\")\n", "bar <- function() {\n  baz()\n}\n");

    {
        let rev = reverse_source_edges(&db);
        assert!(
            rev.sourced_by
                .get(Path::new("/s/b.R"))
                .is_some_and(|s| s.contains(Path::new("/s/a.R"))),
            "a.R should be recorded as a sourcer of b.R"
        );
    }

    db.clear_query_log();
    db.set_file_text(b, "bar <- function() {\n  baz()\n  2\n}\n");

    let _ = reverse_source_edges(&db);

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
    let (mut db, a, _b) = scripts_ab("source(\"b.R\")\n", "bar <- 1\n");

    {
        let _ = reverse_source_edges(&db);
    }

    db.clear_query_log();
    db.set_file_text(a, "source(\"b.R\")\nsource(\"c.R\")\n");

    let rev = reverse_source_edges(&db);

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
    let (db, _a, _b) = package_ab("foo <- function() 1\n", "bar <- 2\nfoo <- 3\n");
    let defs = project_defs(&db);

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
    let (mut db, _a, b) = package_ab("foo <- function() 1\n", "bar <- function() {\n  foo()\n}\n");

    {
        let _ = project_defs(&db);
    }

    db.clear_query_log();
    db.set_file_text(b, "bar <- function() {\n  foo()\n  2\n}\n");

    let _ = project_defs(&db);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(counts.get(&QueryKind::SemanticModel), Some(&1));
    assert_eq!(
        counts.get(&QueryKind::ProjectDefs),
        None,
        "a body edit must not rebuild project_defs"
    );
}

#[test]
fn promise_summary_backdates_across_irrelevant_body_edit() {
    let (mut db, a, _b) = package_ab(
        "capture <- function(x) {\n  substitute(x)\n}\n",
        "capture(column)\n",
    );
    let before = project_promises(&db).clone();

    db.clear_query_log();
    db.set_file_text(
        a,
        "capture <- function(x) {\n  substitute(x)\n  invisible(NULL)\n}\n",
    );
    let after = project_promises(&db).clone();

    assert_eq!(before, after);
    let counts = count_by_kind(&db.query_log());
    assert_eq!(counts.get(&QueryKind::FilePromiseSeeds), Some(&1));
    assert_eq!(
        counts.get(&QueryKind::ProjectPromises),
        None,
        "an equal range-free seed must spare the package aggregate"
    );
}

#[test]
fn changed_promise_contract_rebuilds_package_summary() {
    let (mut db, a, _b) = package_ab("evaluate <- function(x) x + 1\n", "evaluate(value)\n");
    assert!(
        project_promises(&db).by_root[Path::new("/pkg")]["evaluate"]
            .eager
            .contains("x")
    );

    db.clear_query_log();
    db.set_file_text(a, "evaluate <- function(x) substitute(x)\n");
    let after = project_promises(&db);

    assert!(
        !after.by_root[Path::new("/pkg")]["evaluate"]
            .eager
            .contains("x")
    );
    let counts = count_by_kind(&db.query_log());
    assert_eq!(counts.get(&QueryKind::ProjectPromises), Some(&1));
}

fn class_named(index: &ClassIndex, name: &str) -> ClassId {
    index
        .declarations
        .iter()
        .find(|(_, class)| class.declaration.display_name() == name)
        .map(|(id, _)| id.clone())
        .expect("class")
}

#[test]
fn project_classes_aggregates_inheritance_edges() {
    let (db, _a, _b) = package_ab(
        "setClass(\"Animal\")\n",
        "setClass(\"Dog\", contains = \"Animal\")\n",
    );
    let classes = project_classes(&db);
    let animal = class_named(classes, "Animal");
    let dog = class_named(classes, "Dog");
    assert!(classes.supertypes[&dog].contains(&animal));
    assert!(classes.subtypes[&animal].contains(&dog));
    assert_ne!(animal, dog);
}

#[test]
fn s4_slot_and_relationship_edits_invalidate_the_shared_index() {
    let (mut db, a, b) = package_ab(
        "Make <- methods::setClass('Base', slots = c(value = 'numeric'), validity = function(object) TRUE)\nmethods::setClassUnion('Union', 'Base')\n",
        "methods::setClass('Child', contains = 'Base', slots = c(label = 'character'))\nmethods::setIs('Child', 'Union')\n",
    );
    let before = project_classes(&db);
    let child = class_named(before, "Child");
    let union = class_named(before, "Union");
    assert!(
        before
            .s4_slots(&child)
            .unwrap()
            .entries
            .contains_key("value")
    );
    assert!(before.supertypes[&child].contains(&union));

    db.clear_query_log();
    db.set_file_text(a, "# Prose.\nMake <- methods::setClass('Base', slots = c(value = 'numeric'), validity = function(object) FALSE)\nmethods::setClassUnion('Union', 'Base')\n");
    let _ = project_classes(&db);
    assert!(!count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectClasses));

    db.clear_query_log();
    db.set_file_text(a, "Make <- methods::setClass('Base', slots = c(renamed = 'character'), validity = function(object) FALSE)\nmethods::setClassUnion('Union', 'Base')\n");
    let after = project_classes(&db);
    let slots = after.s4_slots(&child).unwrap();
    assert!(slots.entries.contains_key("renamed"));
    assert!(!slots.entries.contains_key("value"));
    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::ProjectClasses),
        Some(&1)
    );

    db.set_file_text(b, "methods::setClass('Child', contains = 'Base', slots = c(label = 'character'))\nmethods::setIs('Child', 'Union', test = function(object) TRUE)\n");
    let after = project_classes(&db);
    assert!(!after.supertypes[&child].contains(&union));
    assert_eq!(after.s4_slots(&child).unwrap().entries.len(), 2);
}

#[test]
fn s4_opaque_coercion_edits_backdate_but_prototype_edits_do_not() {
    let source = "methods::setClass('A', slots = c(value = 'numeric'), prototype = list(value = 1))\nmethods::setClass('B', contains = 'VIRTUAL')\nmethods::setIs('A', 'B', coerce = function(from) from)\nmethods::setAs('B', 'A', function(from) stop('opaque'))\n";
    let (mut db, a, _) = package_ab(source, "");
    let _ = project_classes(&db);
    db.clear_query_log();
    db.set_file_text(a, source.replace("stop('opaque')", "stop('changed')"));
    let _ = project_classes(&db);
    assert!(!count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectClasses));

    db.clear_query_log();
    db.set_file_text(a, source.replace("value = 1", "value = 2"));
    let index = project_classes(&db);
    assert_eq!(
        index.s4_prototype(&class_named(index, "A")).unwrap().slots["value"],
        arity::project::classes::S4Default::Literal("2".into())
    );
    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::ProjectClasses),
        Some(&1)
    );
}

#[test]
fn s4_body_local_class_lookups_do_not_rebuild_the_shared_index() {
    let source = "methods::setClass('A', slots = c(x = 'numeric'), validity = function(object) { TRUE })\nmethods::setClass('B', contains = 'VIRTUAL')\nmethods::setIs('A', 'B', coerce = function(from) { TRUE })\nmethods::setAs('B', 'A', function(from) { TRUE })\n";
    let (mut db, a, _) = package_ab(source, "");
    let before = project_classes(&db).clone();
    for body in [
        "methods::slot(object, 'x'); TRUE",
        "slot(object, 'x'); TRUE",
        "methods::new('A'); TRUE",
        "methods::as(object, 'A'); TRUE",
        "fetch <- methods::slot; fetch(object, 'x'); TRUE",
    ] {
        db.clear_query_log();
        db.set_file_text(a, source.replace("{ TRUE }", &format!("{{ {body} }}")));
        assert!(project_classes(&db) == &before, "{body}");
        assert!(
            !count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectClasses),
            "a body-local lookup must not rebuild project_classes: {body}"
        );
    }
}

#[test]
fn s4_slot_helpers_require_project_confirmed_origins() {
    let (db, _, _) = package_ab(
        "methods::setClass('A', slots = c(value = 'numeric'))\n",
        "c <- function(...) NULL\n",
    );
    let index = project_classes(&db);
    let slots = index.s4_slots(&class_named(index, "A")).unwrap();
    assert!(!slots.complete);
    assert!(slots.entries.is_empty());
}

#[test]
fn s4_old_class_bridges_require_a_confirmed_registration() {
    let (db, _, _) = package_ab(
        "methods::setOldClass('Ancestor')\nmethods::setClass('Base', slots = c(base = 'character'))\nmethods::setClass('A', contains = 'Base', slots = c(value = 'numeric'))\nsetOldClass(c('A', 'Ancestor'), S4Class = 'A')\n",
        "setOldClass <- function(...) NULL\n",
    );
    let index = project_classes(&db);
    let id = class_named(index, "A");
    let slots = index.s4_slots(&id).unwrap();
    assert!(!slots.complete);
    assert!(!slots.entries.contains_key(".S3Class"));
    assert!(slots.entries.contains_key("value"));
    assert!(slots.entries.contains_key("base"));
    assert!(index.supertypes[&id].contains(&class_named(index, "Base")));
    assert!(!index.supertypes[&id].contains(&class_named(index, "Ancestor")));
    let old = index.s4_class(&id).unwrap().old_class.unwrap();
    assert!(index.callee_identity(&id.path, &old.factory).is_none());
}

#[test]
fn old_class_prototype_helpers_require_confirmed_origins() {
    for (prototype, mask) in [
        (
            "structure(base::list(), class = 'Old')",
            "structure <- function(...) NULL\n",
        ),
        (
            "base::structure(list(), class = 'Old')",
            "list <- function(...) NULL\n",
        ),
    ] {
        let (db, _, _) = package_ab(
            &format!("methods::setOldClass('Old', prototype = {prototype})\n"),
            mask,
        );
        let index = project_classes(&db);
        assert_eq!(
            index
                .s4_class(&class_named(index, "Old"))
                .unwrap()
                .virtual_class,
            None,
            "{mask}"
        );
    }
}

#[test]
fn old_class_ancestor_reuse_requires_a_confirmed_predecessor() {
    for (first, mask) in [
        ("setOldClass('A')", "setOldClass <- function(...) NULL\n"),
        (
            "setClass('A', slots = c(x = 'numeric'))",
            "setClass <- function(...) NULL\n",
        ),
        ("setOldClass('A')", ""),
    ] {
        let (db, _, _) = package_ab(
            &format!("{first}\nmethods::setOldClass(c('B', 'A'))\n"),
            mask,
        );
        let index = project_classes(&db);
        assert_eq!(index.declarations.len(), 2, "{first}: {mask}");
        let a = class_named(index, "A");
        let b = class_named(index, "B");
        assert!(index.supertypes[&b].contains(&a), "{first}: {mask}");
        assert!(index.s4_slots(&a).unwrap().complete, "{first}: {mask}");
        assert!(index.s4_slots(&b).unwrap().complete, "{first}: {mask}");
    }
}

#[test]
fn old_class_ancestors_reuse_registrations_from_earlier_package_files() {
    let (db, _, _) = package_ab(
        "methods::setOldClass('A')\n",
        "methods::setOldClass(c('B', 'A'))\n",
    );
    let index = project_classes(&db);
    assert_eq!(index.declarations.len(), 2);
    let a = class_named(index, "A");
    let b = class_named(index, "B");
    assert!(index.supertypes[&b].contains(&a));
    assert!(index.s4_slots(&a).unwrap().complete);
    assert!(index.s4_slots(&b).unwrap().complete);
}

#[test]
fn body_edit_does_not_rebuild_project_classes() {
    // The firewall: editing a function body in b.R re-runs its model, but the
    // class-def set is unchanged, so the project-wide class aggregate is reused.
    let (mut db, _a, b) = package_ab(
        "setClass(\"Animal\")\n",
        "setClass(\"Dog\", contains = \"Animal\")\nf <- function() {\n  1\n}\n",
    );

    {
        let _ = project_classes(&db);
    }

    db.clear_query_log();
    db.set_file_text(
        b,
        "setClass(\"Dog\", contains = \"Animal\")\nf <- function() {\n  2\n}\n",
    );

    let _ = project_classes(&db);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(
        counts.get(&QueryKind::ProjectClasses),
        None,
        "a body edit must not rebuild project_classes"
    );
}

#[test]
fn adding_a_class_rebuilds_project_classes() {
    let (mut db, _a, b) = package_ab("setClass(\"Animal\")\n", "x <- 1\n");

    {
        let _ = project_classes(&db);
    }

    db.clear_query_log();
    db.set_file_text(b, "x <- 1\nsetClass(\"Dog\", contains = \"Animal\")\n");

    let classes = project_classes(&db);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(
        counts.get(&QueryKind::ProjectClasses),
        Some(&1),
        "a new class definition must rebuild project_classes"
    );
    assert!(
        classes.subtypes[&class_named(classes, "Animal")].contains(&class_named(classes, "Dog"))
    );
}

#[test]
fn class_identity_keeps_unrelated_scripts_and_packages_separate() {
    let (db, _, _) = scripts_ab(
        "Parent <- R6::R6Class(\"Same\")\n",
        "Child <- R6::R6Class(\"Child\", inherit = Parent)\n",
    );
    assert!(project_classes(&db).supertypes.is_empty());
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/one/R/a.R"), "setClass(\"Same\")\n");
    let b = db.upsert_file(
        Path::new("/two/R/b.R"),
        "setClass(\"Child\", contains = \"Same\")\n",
    );
    db.set_workspace_members(vec![a, b], vec![]);
    db.set_package_metadata(
        ["/one", "/two"]
            .into_iter()
            .map(|root| PackageInfo {
                root: root.into(),
                namespace: None,
                dir_sources: BTreeSet::new(),
                native_routines: BTreeSet::new(),
            })
            .collect(),
    );
    assert!(project_classes(&db).supertypes.is_empty());
}

#[test]
fn class_identity_resolves_sourced_generators_and_same_file_aliases() {
    let (db, _, _) = scripts_ab(
        "source(\"b.R\")\nChild <- R6::R6Class(\"Child\", inherit = Generator)\n",
        "Original <- R6::R6Class(\"Display\")\nGenerator <- Original\n",
    );
    let classes = project_classes(&db);
    assert!(
        classes.supertypes[&class_named(classes, "Child")]
            .contains(&class_named(classes, "Display"))
    );
    let (db, _, _) = scripts_ab(
        "Child <- R6::R6Class(\"Child\", inherit = Generator)\nsource(\"b.R\")\n",
        "Generator <- R6::R6Class(\"Display\")\n",
    );
    let classes = project_classes(&db);
    assert!(
        classes.supertypes[&class_named(classes, "Child")]
            .contains(&class_named(classes, "Display"))
    );
    let (db, _, _) = scripts_ab(
        "Child <- S7::new_class('Child', parent = Generator)\nsource('b.R')\n",
        "Generator <- S7::new_class('Display')\n",
    );
    assert!(project_classes(&db).supertypes.is_empty());
}

#[test]
fn class_identity_preserves_package_duplicates_without_picking_a_parent() {
    let (db, _, _) = package_ab(
        "setClass(\"Same\")\nsetClass(\"Child\", contains = \"Same\")\n",
        "setClass(\"Same\")\n",
    );
    let classes = project_classes(&db);
    assert_eq!(classes.declarations.len(), 3);
    assert!(classes.supertypes.is_empty());
    assert_eq!(
        classes.parent_candidates[&class_named(classes, "Child")][0].len(),
        2
    );
}

#[test]
fn formal_lookup_excludes_disproven_factories() {
    for source in [
        "setClass('Parent')\nmethods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\n",
        "f <- function() { setClass('Parent'); methods::setClass('Parent'); methods::setClass('Child', contains = 'Parent') }\n",
    ] {
        let (db, _, _) = package_ab(source, "setClass <- function(...) NULL\n");
        let index = project_classes(&db);
        assert_eq!(index.unresolved.len(), 1);
        let parent = class_named(index, "Parent");
        let child = class_named(index, "Child");
        assert_eq!(index.parent_candidates[&child], vec![vec![parent.clone()]]);
        assert_eq!(
            index.supertypes.get(&child),
            Some(&BTreeSet::from([parent]))
        );
    }
}

#[test]
fn formal_lookup_deduplicates_source_and_namespace_routes() {
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/one/R/a.R"), "methods::setClass('Parent')\n");
    let b = db.upsert_file(
        Path::new("/two/R/b.R"),
        "source('/one/R/a.R')\nmethods::setClass('Child', contains = 'Parent')\n",
    );
    db.set_workspace_members(vec![a, b], vec!["/one".into(), "/two".into()]);
    db.refresh_descriptions([PathBuf::from("/one"), PathBuf::from("/two")]);
    db.upsert_description(Path::new("/one"), "Package: one\n");
    db.upsert_description(Path::new("/two"), "Package: two\nImports: one\n");
    db.set_package_metadata(
        [
            ("/one", "exportClasses(Parent)\n"),
            ("/two", "importClassesFrom(one, Parent)\n"),
        ]
        .into_iter()
        .map(|(root, namespace)| PackageInfo {
            root: root.into(),
            namespace: Some(namespace.into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        })
        .collect(),
    );
    let index = project_classes(&db);
    let parent = class_named(index, "Parent");
    let child = class_named(index, "Child");
    assert_eq!(index.parent_candidates[&child], vec![vec![parent.clone()]]);
    assert_eq!(
        index.supertypes.get(&child),
        Some(&BTreeSet::from([parent]))
    );
}

#[test]
fn class_identity_respects_namespace_imports_and_project_shadowing() {
    let (mut db, _, b) = package_ab("A <- R6Class(classname = \"A\")\n", "");
    assert!(project_classes(&db).declarations.is_empty());
    let metadata = |namespace: &str| {
        vec![PackageInfo {
            root: "/pkg".into(),
            namespace: Some(namespace.into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]
    };
    db.set_package_metadata(metadata("importFrom(R6, R6Class)\n"));
    assert_eq!(project_classes(&db).declarations.len(), 1);
    db.set_package_metadata(metadata("importFrom(other, R6Class)\n"));
    assert!(project_classes(&db).declarations.is_empty());
    db.set_package_metadata(metadata("import(R6)\n"));
    assert_eq!(project_classes(&db).declarations.len(), 1);
    db.set_file_text(b, "R6Class <- function(...) NULL\n");
    assert!(project_classes(&db).declarations.is_empty());
}

#[test]
fn oop_callees_use_namespace_origins_and_reject_project_masks() {
    let (mut db, _, b) = package_ab(
        "define <- setGeneric\ninstall <- setMethod\ndefine('g', function(x) x)\ninstall('g', 'C', function(x) x)\nmethod(g, signature = C) <- function(x) x\n",
        "",
    );
    let metadata = |namespace: &str| {
        vec![PackageInfo {
            root: "/pkg".into(),
            namespace: Some(namespace.into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]
    };
    let resolved = |db: &IncrementalDatabase| {
        let index = project_classes(db);
        let file = index.file(Path::new("/pkg/R/a.R")).unwrap();
        file.facts
            .oop
            .generics
            .iter()
            .map(|def| &def.factory)
            .chain(
                file.facts
                    .oop
                    .methods
                    .iter()
                    .filter_map(|def| def.registration.as_ref()),
            )
            .map(|reference| index.callee_identity(&file.path, reference).cloned())
            .collect::<Vec<_>>()
    };
    db.set_package_metadata(metadata(
        "importFrom(methods, setGeneric, setMethod)\nimportFrom(S7, `method<-`)\n",
    ));
    assert_eq!(
        resolved(&db)
            .iter()
            .map(|origin| origin.as_ref().map(|id| id.package.as_str()))
            .collect::<Vec<_>>(),
        vec![Some("methods"), Some("methods"), Some("S7")]
    );
    db.set_package_metadata(metadata(
        "importFrom(other, setGeneric, setMethod, `method<-`)\n",
    ));
    assert!(resolved(&db).iter().all(Option::is_none));
    db.set_package_metadata(metadata(
        "importFrom(methods, setGeneric, setMethod)\nimportFrom(S7, `method<-`)\n",
    ));
    db.set_file_text(b, "setGeneric <- function(...) NULL\nsetMethod <- function(...) NULL\n`method<-` <- function(...) NULL\n");
    assert!(resolved(&db).iter().all(Option::is_none));
}

#[test]
fn oop_aliases_capture_the_attachment_order_at_their_definition() {
    let (mut db, _) = script_one(
        "library(S7)\nmake <- new_generic\nsetter <- `method<-`\nlibrary(other)\ng <- make('g', 'x')\nsetter(g, C, function(x) x)\nnew_generic('other', 'x')\n",
    );
    db.set_library_index(IndexedProvider::from_indices([
        index_pkg("S7", &["new_generic", "method<-"]),
        index_pkg("other", &["new_generic", "method<-"]),
    ]));
    let index = project_classes(&db);
    let file = index.file(Path::new("/proj/a.R")).unwrap();
    assert_eq!(file.facts.oop.generics.len(), 2);
    assert_eq!(
        index
            .callee_identity(&file.path, &file.facts.oop.generics[0].factory)
            .unwrap()
            .package,
        "S7"
    );
    assert!(
        index
            .callee_identity(&file.path, &file.facts.oop.generics[1].factory)
            .is_none()
    );
    let reference = file.facts.oop.methods[0].registration.as_ref().unwrap();
    assert_eq!(
        index
            .callee_identity(&file.path, reference)
            .unwrap()
            .package,
        "S7"
    );
}

#[test]
fn class_identity_parent_and_alias_edits_invalidate_the_graph() {
    let (mut db, a, b) = package_ab(
        "A <- R6::R6Class(\"First\")\nB <- R6::R6Class(\"Second\")\n",
        "Child <- R6::R6Class(\"Child\", inherit = A)\n",
    );
    let first = project_classes(&db).clone();
    assert!(
        first.supertypes[&class_named(&first, "Child")].contains(&class_named(&first, "First"))
    );
    db.clear_query_log();
    db.set_file_text(b, "Child <- R6::R6Class(\"Child\", inherit = B)\n");
    let second = project_classes(&db).clone();
    assert!(
        second.supertypes[&class_named(&second, "Child")].contains(&class_named(&second, "Second"))
    );
    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::ProjectClasses),
        Some(&1)
    );
    db.clear_query_log();
    db.set_file_text(
        a,
        "A <- R6::R6Class(\"First\")\nOriginal <- R6::R6Class(\"Second\")\nB <- Original\n",
    );
    let third = project_classes(&db);
    assert!(third.supertypes[&class_named(third, "Child")].contains(&class_named(third, "Second")));
    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::ProjectClasses),
        Some(&1)
    );
}

#[test]
fn class_identity_rejects_wildcard_masks_and_conditional_imports() {
    let (mut db, _, _) = package_ab("setClass(\"Phantom\")\n", "");
    db.set_library_index(IndexedProvider::from_indices([index_pkg(
        "other",
        &["setClass"],
    )]));
    db.set_package_metadata(vec![PackageInfo {
        root: "/pkg".into(),
        namespace: Some("import(other)\n".into()),
        dir_sources: BTreeSet::new(),
        native_routines: BTreeSet::new(),
    }]);
    assert!(project_classes(&db).declarations.is_empty());
    let (mut db, _, _) = package_ab("A <- R6Class(\"A\")\n", "");
    db.set_package_metadata(vec![PackageInfo {
        root: "/pkg".into(),
        namespace: Some("if (flag) importFrom(R6, R6Class)\n".into()),
        dir_sources: BTreeSet::new(),
        native_routines: BTreeSet::new(),
    }]);
    assert!(project_classes(&db).declarations.is_empty());
}

#[test]
fn class_identity_distinguishes_depends_from_imports_and_later_library_masks() {
    let (mut db, a, _) = package_ab("A <- R6Class(\"A\")\n", "");
    db.refresh_descriptions([PathBuf::from("/pkg")]);
    db.upsert_description(Path::new("/pkg"), "Package: pkg\nImports: R6\n");
    assert!(project_classes(&db).declarations.is_empty());
    db.upsert_description(Path::new("/pkg"), "Package: pkg\nDepends: R6\n");
    assert_eq!(project_classes(&db).declarations.len(), 1);
    db.set_library_index(IndexedProvider::from_indices([index_pkg(
        "other",
        &["R6Class"],
    )]));
    db.set_file_text(a, "library(other)\nA <- R6Class(\"A\")\n");
    assert!(project_classes(&db).declarations.is_empty());
}

#[test]
fn class_body_and_prose_edits_backdate_despite_unrelated_project_reads() {
    let (mut db, a, _) = package_ab(
        "A <- R6::R6Class(\"A\", public = list(read = function() 1))\n",
        "",
    );
    let before = project_classes(&db).clone();
    db.clear_query_log();
    db.set_file_text(
        a,
        "# Prose.\nA <- R6::R6Class(\"A\", public = list(read = function() print(2)))\n",
    );
    assert_eq!(&before, project_classes(&db));
    assert!(!count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectClasses));
}

#[test]
fn shared_oop_facts_reach_the_project_with_package_ownership() {
    let (db, _, _) = package_ab(
        "A <- R6::R6Class('A', public = list(read = function(scale = 1) scale))\n",
        "methods::setGeneric('read', function(x) standardGeneric('read'))\n\
         methods::setMethod('read', 'A', function(x) x)\n\
         methods::setMethod('read', 'A', function(x) 2)\n",
    );
    let classes = project_classes(&db);
    assert_eq!(classes.oop_files().count(), 2);
    let a = classes.file(Path::new("/pkg/R/a.R")).unwrap();
    let b = classes.file(Path::new("/pkg/R/b.R")).unwrap();
    assert_eq!(a.package_root, Some(PathBuf::from("/pkg")));
    assert_eq!(a.facts.oop.members.len(), 1);
    assert_eq!(a.facts.oop.methods.len(), 1);
    assert_eq!(b.package_root, a.package_root);
    assert_eq!(b.facts.oop.generics.len(), 1);
    assert_eq!(b.facts.oop.methods.len(), 2);
    assert_ne!(b.facts.oop.methods[0].key, b.facts.oop.methods[1].key);
}

#[test]
fn project_replacement_generic_objects_do_not_select_the_getter() {
    let (db, _, _) = package_ab(
        "gname <- methods::setGeneric('value', function(object) object)\n\
         methods::setGeneric('value<-', function(object, value) object)\n\
         methods::setReplaceMethod(gname, 'C', function(object, value) object)\n",
        "",
    );
    let index = project_classes(&db);
    let file = index.file(Path::new("/pkg/R/a.R")).unwrap();
    let id = |n: usize| arity::project::classes::OopId {
        path: file.path.clone(),
        key: file.facts.oop.generics[n].key.clone(),
    };
    assert_eq!(
        index.generic_candidates(
            &file.path,
            file.facts.oop.methods[0].generic.as_ref().unwrap(),
            false
        ),
        vec![id(1)]
    );
    assert!(index.methods_for_generic(&id(0)).is_empty());
    assert_eq!(index.methods_for_generic(&id(1)).len(), 1);
}

#[test]
fn project_dispatch_rejects_masked_signature_helpers() {
    use arity::project::classes::{DispatchClass, OopId, OopUncertainty};

    for (helper, expression) in [
        ("c", "c(x = 'numeric')"),
        ("signature", "signature(x = 'numeric')"),
        ("list", "list(C)"),
    ] {
        let registration = if helper == "list" {
            format!("S7::method(g, {expression}) <- function(x) x\n")
        } else {
            format!("methods::setMethod('g', {expression}, function(x) x)\n")
        };
        let source = format!("source('b.R')\n{registration}");
        let (db, _, _) = scripts_ab(
            &source,
            &format!(
                "C <- S7::new_class('C')\n{helper} <- function(...) base::c(x = 'character')\n"
            ),
        );
        let index = project_classes(&db);
        let file = index.file(Path::new("/s/a.R")).unwrap();
        let method = index
            .method(&OopId {
                path: file.path.clone(),
                key: file.facts.oop.methods[0].key.clone(),
            })
            .unwrap();
        assert!(
            method
                .uncertainty
                .contains(&OopUncertainty::DynamicSignature),
            "{source}"
        );
        assert_eq!(method.dispatch.len(), 1);
        assert_eq!(method.dispatch[0].class, DispatchClass::Unknown, "{source}");
    }
}

#[test]
fn project_dispatch_confirms_unmasked_and_qualified_helpers() {
    use arity::project::classes::{DispatchClass, OopId, OopUncertainty};

    for (helper, expression, qualified) in [
        ("c", "c(x = 'numeric')", "base::c(x = 'numeric')"),
        (
            "signature",
            "signature(x = 'numeric')",
            "methods::signature(x = 'numeric')",
        ),
        ("list", "list(C)", "base::list(C)"),
    ] {
        for (expression, mask) in [(expression, false), (qualified, true)] {
            let registration = if helper == "list" {
                format!("S7::method(g, {expression}) <- function(x) x\n")
            } else {
                format!("methods::setMethod('g', {expression}, function(x) x)\n")
            };
            let source = format!("source('b.R')\n{registration}");
            let (db, _, _) = scripts_ab(
                &source,
                &if mask {
                    format!("C <- S7::new_class('C')\n{helper} <- function(...) NULL\n")
                } else {
                    "C <- S7::new_class('C')\n".into()
                },
            );
            let index = project_classes(&db);
            let file = index.file(Path::new("/s/a.R")).unwrap();
            let method = index
                .method(&OopId {
                    path: file.path.clone(),
                    key: file.facts.oop.methods[0].key.clone(),
                })
                .unwrap();
            assert!(method.dispatch_factory.is_some(), "{source}");
            assert!(
                !method
                    .uncertainty
                    .contains(&OopUncertainty::DynamicSignature),
                "{source}"
            );
            assert_eq!(method.dispatch.len(), 1);
            assert!(
                if helper == "list" {
                    matches!(method.dispatch[0].class, DispatchClass::Generator(_))
                } else {
                    method.dispatch[0].class == DispatchClass::Formal("numeric".into())
                },
                "{source}"
            );
        }
    }
}

#[test]
fn project_dispatch_rejects_imported_helper_masks() {
    use arity::project::classes::{ClassName, DispatchClass, OopId, OopUncertainty};

    for namespace in ["importFrom(mask, c)\n", "import(mask)\n"] {
        let (mut db, _, _) = package_ab(
            "methods::setGeneric('g', function(x) x, signature = c('x'))\n\
             methods::setMethod('g', c(x = 'numeric'), function(x) x)\n",
            "",
        );
        db.set_package_metadata(vec![PackageInfo {
            root: "/pkg".into(),
            namespace: Some(namespace.into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        db.set_library_index(IndexedProvider::from_indices([index_pkg("mask", &["c"])]));
        let index = project_classes(&db);
        let file = index.file(Path::new("/pkg/R/a.R")).unwrap();
        let method = index
            .method(&OopId {
                path: file.path.clone(),
                key: file.facts.oop.methods[0].key.clone(),
            })
            .unwrap();
        let generic = index
            .generic(&OopId {
                path: file.path.clone(),
                key: file.facts.oop.generics[0].key.clone(),
            })
            .unwrap();
        assert_eq!(
            method.dispatch[0].class,
            DispatchClass::Unknown,
            "{namespace}"
        );
        assert!(
            method
                .uncertainty
                .contains(&OopUncertainty::DynamicSignature)
        );
        assert_eq!(
            generic.dispatch_arguments,
            vec![ClassName::Unknown],
            "{namespace}"
        );
        assert!(
            generic
                .uncertainty
                .contains(&OopUncertainty::DynamicSignature)
        );
        assert!(file.facts.oop.methods[0].dispatch_factory.is_some());
        assert!(file.facts.oop.generics[0].dispatch_factory.is_some());
    }
}

#[test]
fn bare_generic_factories_retain_bindings_for_project_confirmation() {
    for (declaration, namespace) in [
        ("library(S7)\ng <- new_generic('g', 'x')\n", ""),
        (
            "g <- new_generic('g', 'x')\n",
            "importFrom(S7, new_generic)\n",
        ),
    ] {
        let source = format!(
            "{declaration}alias <- g\nC <- S7::new_class('C')\nS7::method(g, C) <- function(x) x\nS7::method(alias, C) <- function(x) x\n"
        );
        let (mut db, _, _) = package_ab(&source, "");
        db.set_package_metadata(vec![PackageInfo {
            root: "/pkg".into(),
            namespace: Some(namespace.into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        db.set_library_index(IndexedProvider::from_indices([index_pkg(
            "S7",
            &["new_generic"],
        )]));
        let index = project_classes(&db);
        let file = index.file(Path::new("/pkg/R/a.R")).unwrap();
        let generic = arity::project::classes::OopId {
            path: file.path.clone(),
            key: file.facts.oop.generics[0].key.clone(),
        };
        for method in &file.facts.oop.methods {
            assert_eq!(
                index.generic_candidates(&file.path, method.generic.as_ref().unwrap(), false),
                vec![generic.clone()],
                "{source}"
            );
        }
        assert_eq!(index.methods_for_generic(&generic).len(), 2);
        db.set_package_metadata(vec![PackageInfo {
            root: "/pkg".into(),
            namespace: Some("importFrom(other, new_generic)\n".into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        assert!(
            project_classes(&db)
                .methods_for_generic(&generic)
                .is_empty()
        );
    }
}

#[test]
fn generic_object_lookup_requires_stable_qualified_bindings() {
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/one/R/a.R"), "");
    let b = db.upsert_file(
        Path::new("/two/R/b.R"),
        "S7::method(one::g, C) <- function(x) x\nS7::method(one::alias, C) <- function(x) x\n",
    );
    db.set_workspace_members(vec![a, b], vec!["/one".into(), "/two".into()]);
    db.refresh_descriptions([PathBuf::from("/one"), PathBuf::from("/two")]);
    db.upsert_description(Path::new("/one"), "Package: one\n");
    db.upsert_description(Path::new("/two"), "Package: two\nImports: one\n");
    db.set_package_metadata(
        [("/one", "export(g, alias)\n"), ("/two", "")]
            .into_iter()
            .map(|(root, namespace)| PackageInfo {
                root: root.into(),
                namespace: Some(namespace.into()),
                dir_sources: BTreeSet::new(),
                native_routines: BTreeSet::new(),
            })
            .collect(),
    );
    for (source, expected) in [
        ("g <- S7::new_generic('g', 'x')\nalias <- g\n", 1),
        (
            "g <- S7::new_generic('g', 'x')\nalias <- g\ng <- function(x) x\n",
            0,
        ),
    ] {
        db.set_file_text(a, source);
        let index = project_classes(&db);
        let file = index.file(Path::new("/two/R/b.R")).unwrap();
        for method in &file.facts.oop.methods {
            assert_eq!(
                index
                    .generic_candidates(&file.path, method.generic.as_ref().unwrap(), false)
                    .len(),
                expected,
                "{source}"
            );
        }
    }
}

#[test]
fn function_local_generic_aliases_do_not_escape_or_invalidate_the_index() {
    let source = "g <- S7::new_generic('g', 'x')\nf <- function() 1\nS7::method(alias, C) <- function(x) x\n";
    let (mut db, a, _) = package_ab(source, "");
    let before = project_classes(&db).clone();
    db.clear_query_log();
    db.set_file_text(
        a,
        source.replace("function() 1", "function() { alias <- g }"),
    );
    let index = project_classes(&db);
    let file = index.file(Path::new("/pkg/R/a.R")).unwrap();
    assert!(
        index
            .generic_candidates(
                &file.path,
                file.facts.oop.methods[0].generic.as_ref().unwrap(),
                false
            )
            .is_empty()
    );
    assert_eq!(&before, index);
    assert!(!count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectClasses));
}

#[test]
fn generic_object_lookup_requires_stable_visible_bindings() {
    for (declaration, expected) in [
        ("g <- S7::new_generic('g', 'x')\n", 1),
        ("g <- S7::new_generic('g', 'x')\ng <- function(x) x\n", 0),
        ("if (flag) g <- S7::new_generic('g', 'x')\n", 0),
        (
            "g <- S7::new_generic('g', 'x')\nalias <- g\ng <- function(x) x\n",
            0,
        ),
    ] {
        let (db, _, _) = package_ab(declaration, "S7::method(g, C) <- function(x) x\n");
        let index = project_classes(&db);
        let file = index.file(Path::new("/pkg/R/b.R")).unwrap();
        let candidates = index.generic_candidates(
            &file.path,
            file.facts.oop.methods[0].generic.as_ref().unwrap(),
            false,
        );
        assert_eq!(candidates.len(), expected, "{declaration}");
    }
}

#[test]
fn callable_alias_body_edits_backdate_but_signature_edits_invalidate() {
    let source =
        "helper <- function(x = 1) x\nA <- R6::R6Class('A', public = base::list(read = helper))\n";
    let (mut db, a, _) = package_ab(source, "");
    let before = project_classes(&db).clone();
    db.clear_query_log();
    db.set_file_text(a, source.replace(") x", ") { print(x); x }"));
    assert_eq!(&before, project_classes(&db));
    assert!(!count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectClasses));
    db.clear_query_log();
    db.set_file_text(a, source.replace("x = 1", "x = 2"));
    assert_ne!(&before, project_classes(&db));
    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::ProjectClasses),
        Some(&1)
    );
}

#[test]
fn r6_api_tracks_flags_members_and_parent_signatures_without_body_ranges() {
    let source = "Parent <- R6::R6Class('Display', public = list(initialize = function(x = 1) x, read = function(scale = 2) scale))\n";
    let child = "Child <- R6::R6Class(inherit = Parent)\n";
    let (mut db, a, b) = package_ab(source, child);
    let before = project_classes(&db).clone();
    let child_id = class_named(&before, "Child");
    assert_eq!(
        before
            .r6_api(&child_id)
            .unwrap()
            .initializer
            .unwrap()
            .parameters[0]
            .default
            .as_deref(),
        Some("1")
    );
    db.clear_query_log();
    db.set_file_text(a, source.replace(") scale", ") { print(scale); scale }"));
    assert_eq!(&before, project_classes(&db));
    assert!(!count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectClasses));
    db.set_file_text(a, source.replace("x = 1", "x = 9"));
    assert_eq!(
        project_classes(&db)
            .r6_api(&child_id)
            .unwrap()
            .initializer
            .unwrap()
            .parameters[0]
            .default
            .as_deref(),
        Some("9")
    );
    db.set_file_text(
        b,
        "Child <- R6::R6Class(inherit = Parent, cloneable = FALSE, class = FALSE)\n",
    );
    let api = project_classes(&db).r6_api(&child_id).unwrap();
    assert!(!api.public.entries.contains_key("clone"));
    assert_eq!(api.class_attribute, Some(vec![]));
}

#[test]
fn member_mutations_backdate_bodies_and_invalidate_for_signatures_and_order() {
    let definitions = "G <- R6::R6Class('Mutation')\n";
    let mutations = "G$set('public', 'read', function(scale = 2) self$value * scale)\n";
    let (mut db, a, b) = package_ab(definitions, mutations);
    db.refresh_descriptions([PathBuf::from("/pkg")]);
    db.upsert_description(Path::new("/pkg"), "Package: pkg\nCollate: a.R b.R\n");
    let before = project_classes(&db).clone();
    let id = class_named(&before, "Mutation");
    assert_eq!(
        before.r6_api(&id).unwrap().public.entries["read"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .default
            .as_deref(),
        Some("2")
    );
    db.clear_query_log();
    db.set_file_text(
        b,
        mutations.replace("self$value * scale", "{ print(scale); self$value * scale }"),
    );
    assert_eq!(&before, project_classes(&db));
    assert!(!count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectClasses));
    db.set_file_text(b, mutations.replace("scale = 2", "scale = 8"));
    assert_eq!(
        project_classes(&db).r6_api(&id).unwrap().public.entries["read"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .default
            .as_deref(),
        Some("8")
    );
    db.upsert_description(Path::new("/pkg"), "Package: pkg\nCollate: b.R a.R\n");
    assert!(
        !project_classes(&db)
            .r6_api(&id)
            .unwrap()
            .public
            .entries
            .contains_key("read")
    );
    db.set_file_text(
        a,
        definitions.replace(
            "R6Class('Mutation')",
            "R6Class('Mutation', lock_class = TRUE)",
        ),
    );
    db.upsert_description(Path::new("/pkg"), "Package: pkg\nCollate: a.R b.R\n");
    assert!(
        !project_classes(&db)
            .r6_api(&id)
            .unwrap()
            .public
            .entries
            .contains_key("read")
    );
}

#[test]
fn reference_apis_backdate_bodies_and_refresh_fields_initializers_and_parents() {
    use arity::project::classes::DispatchClass;
    let source = "Parent <- methods::setRefClass('Display', fields = list(value = 'numeric', active = function(new_value) value), methods = list(initialize = function(x = 1) initFields(value = x), read = function(scale = 2) value * scale))\n";
    let child = "Child <- methods::setRefClass('Child', contains = 'Display', methods = list(read = function(scale = 3) callSuper(scale)))\nobj <- getRefClass('Child')()\n";
    let (mut db, a, b) = package_ab(source, child);
    let before = project_classes(&db).clone();
    let child_id = class_named(&before, "Child");
    let api = before.reference_api(&child_id).unwrap();
    assert!(api.instance.complete);
    assert_eq!(
        api.initializer.unwrap().parameters[0].default.as_deref(),
        Some("1")
    );
    assert_eq!(
        before
            .reference_super_method(&child_id, "read")
            .unwrap()
            .signature
            .unwrap()
            .parameters[0]
            .default
            .as_deref(),
        Some("2")
    );
    db.clear_query_log();
    db.set_file_text(
        a,
        format!(
            "# Prose.\n{}",
            source
                .replace(") value * scale", ") { print(scale); value * scale }")
                .replace(") value)", ") { value + 1 })")
        ),
    );
    assert_eq!(&before, project_classes(&db));
    assert!(!count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectClasses));
    db.set_file_text(
        a,
        source
            .replace("x = 1", "x = 9")
            .replace("'numeric'", "'character'")
            .replace("scale = 2", "scale = 8"),
    );
    let index = project_classes(&db);
    let api = index.reference_api(&child_id).unwrap();
    assert_eq!(
        api.initializer.unwrap().parameters[0].default.as_deref(),
        Some("9")
    );
    assert_eq!(
        api.instance.entries["value"][0].declared_type,
        Some(DispatchClass::Formal("character".into()))
    );
    assert_eq!(
        index
            .reference_super_method(&child_id, "read")
            .unwrap()
            .signature
            .unwrap()
            .parameters[0]
            .default
            .as_deref(),
        Some("8")
    );
    db.set_file_text(
        b,
        child.replace("contains = 'Display'", "contains = 'Unknown'"),
    );
    assert!(
        !project_classes(&db)
            .reference_api(&child_id)
            .unwrap()
            .instance
            .complete
    );
}

#[test]
fn r6_inherits_through_aliases_in_another_file() {
    let (mut db, a, b) = package_ab(
        "Parent <- R6::R6Class('Display', public = list(read = function(x = 1) x))\n",
        "Alias <- Parent\n",
    );
    let c = db.upsert_file(
        Path::new("/pkg/R/c.R"),
        "Child <- R6::R6Class(inherit = Alias)\n",
    );
    db.set_workspace_members(vec![a, b, c], vec!["/pkg".into()]);
    db.set_package_metadata(vec![PackageInfo {
        root: "/pkg".into(),
        namespace: None,
        dir_sources: BTreeSet::new(),
        native_routines: BTreeSet::new(),
    }]);
    let index = project_classes(&db);
    let child = class_named(index, "Child");
    let api = index.r6_api(&child).unwrap();
    assert!(api.public.complete);
    assert!(api.public.entries.contains_key("read"));
    db.set_file_text(b, "Alias <- Parent\nAlias <- unknown\n");
    let api = project_classes(&db).r6_api(&child).unwrap();
    assert!(!api.public.complete);
    assert!(!api.public.entries.contains_key("read"));
}

#[test]
fn member_and_signature_edits_invalidate_the_shared_project_projection() {
    let source = "A <- R6::R6Class('A', public = list(read = function(scale = 1) scale))\n";
    let (mut db, a, _) = package_ab(source, "");
    let before = project_classes(&db).clone();
    for updated in [
        source.replace("read =", "write ="),
        source.replace("scale = 1", "scale = 2"),
    ] {
        db.clear_query_log();
        db.set_file_text(a, updated);
        let after = project_classes(&db);
        assert_ne!(&before, after);
        assert_eq!(
            count_by_kind(&db.query_log()).get(&QueryKind::ProjectClasses),
            Some(&1)
        );
    }
}

#[test]
fn generic_method_body_edits_backdate_but_dispatch_edits_invalidate() {
    let source = "methods::setGeneric('read', function(x) standardGeneric('read'))\n\
                  methods::setMethod('read', 'A', function(x) x)\n";
    let (mut db, a, _) = package_ab(source, "");
    let before = project_classes(&db).clone();
    db.clear_query_log();
    db.set_file_text(
        a,
        source.replace("function(x) x", "function(x) { print(x); x }"),
    );
    assert_eq!(&before, project_classes(&db));
    assert!(!count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectClasses));
    db.clear_query_log();
    db.set_file_text(a, source.replace("'A'", "'B'"));
    assert_ne!(&before, project_classes(&db));
    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::ProjectClasses),
        Some(&1)
    );
}

#[test]
fn shared_oop_resolution_joins_visible_generics_methods_and_members() {
    let (db, _, _) = package_ab(
        "Parent <- R6::R6Class('Parent', public = list(read = function(x = 1) x))\n\
         methods::setGeneric('g', function(x) standardGeneric('g'))\n",
        "Child <- R6::R6Class('Child', inherit = Parent)\n\
         methods::setMethod('g', 'Parent', function(x) x)\n",
    );
    let index = project_classes(&db);
    let method = &index
        .file(Path::new("/pkg/R/b.R"))
        .unwrap()
        .facts
        .oop
        .methods[0];
    let generic = index.generic_candidates(
        Path::new("/pkg/R/b.R"),
        method.generic.as_ref().unwrap(),
        false,
    );
    assert_eq!(generic.len(), 1);
    assert_eq!(generic[0].path, Path::new("/pkg/R/a.R"));
    let methods = index.methods_for_generic(&generic[0]);
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].path, Path::new("/pkg/R/b.R"));
    let members = index.member_candidates(&class_named(index, "Child"), "read");
    assert_eq!(members.len(), 1);
    let member = index.member(&members[0]).unwrap();
    let signature = index
        .method(&arity::project::classes::OopId {
            path: members[0].path.clone(),
            key: member.method.clone().unwrap(),
        })
        .unwrap()
        .signature
        .as_ref()
        .unwrap();
    assert_eq!(signature.parameters[0].name, "x");
    assert_eq!(signature.parameters[0].default.as_deref(), Some("1"));
}

#[test]
fn shared_oop_resolution_keeps_source_closures_directional() {
    for (source, expected) in [("source('b.R')\n", 1), ("", 0)] {
        let (db, _, _) = scripts_ab(
            &format!("{source}methods::setMethod('g', 'C', function(x) x)\n"),
            "methods::setGeneric('g', function(x) standardGeneric('g'))\n",
        );
        let index = project_classes(&db);
        let file = index.file(Path::new("/s/a.R")).unwrap();
        let candidates = index.generic_candidates(
            &file.path,
            file.facts.oop.methods[0].generic.as_ref().unwrap(),
            false,
        );
        assert_eq!(candidates.len(), expected);
        if let Some(generic) = candidates.first() {
            assert_eq!(index.methods_for_generic(generic).len(), 1);
        }
    }
}

#[test]
fn oop_namespace_imports_join_only_exported_workspace_declarations() {
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(
        Path::new("/one/R/a.R"),
        "methods::setClass('Parent')\nmethods::setGeneric('g', function(x) standardGeneric('g'))\n",
    );
    let b = db.upsert_file(Path::new("/two/R/b.R"),
        "methods::setClass('Child', contains = 'Parent')\nmethods::setMethod('g', 'Parent', function(x) x)\n");
    db.set_workspace_members(vec![a, b], vec!["/one".into(), "/two".into()]);
    db.refresh_descriptions([PathBuf::from("/one"), PathBuf::from("/two")]);
    db.upsert_description(Path::new("/one"), "Package: one\n");
    db.upsert_description(Path::new("/two"), "Package: two\nImports: one\n");
    let metadata = |exports: &str, imports: &str| {
        [("/one", exports), ("/two", imports)]
            .into_iter()
            .map(|(root, namespace)| PackageInfo {
                root: root.into(),
                namespace: Some(namespace.into()),
                dir_sources: BTreeSet::new(),
                native_routines: BTreeSet::new(),
            })
            .collect()
    };
    for (exports, imports, expected) in [
        ("exportClasses(Parent)\nexportMethods(g)\n", "", 0),
        (
            "",
            "importClassesFrom(one, Parent)\nimportMethodsFrom(one, g)\n",
            0,
        ),
        (
            "exportClasses(Parent)\nexportMethods(g)\n",
            "importClassesFrom(one, Parent)\nimportMethodsFrom(one, g)\n",
            1,
        ),
        (
            "exportClasses(Parent)\nexportMethods(g)\n",
            "import(one)\nif (flag) import(other)\n",
            0,
        ),
        (
            "exportClasses(Parent)\nexportMethods(g)\n",
            "if (flag) import(one)\n",
            0,
        ),
    ] {
        db.clear_query_log();
        db.set_package_metadata(metadata(exports, imports));
        let index = project_classes(&db);
        let file = index.file(Path::new("/two/R/b.R")).unwrap();
        let candidates = index.generic_candidates(
            &file.path,
            file.facts.oop.methods[0].generic.as_ref().unwrap(),
            false,
        );
        assert_eq!(candidates.len(), expected);
        assert_eq!(index.supertypes.len(), expected);
        if let Some(generic) = candidates.first() {
            assert_eq!(index.methods_for_generic(generic).len(), 1);
        }
        assert_eq!(
            count_by_kind(&db.query_log()).get(&QueryKind::ProjectClasses),
            Some(&1)
        );
    }
}

#[test]
fn oop_qualified_and_imported_bindings_resolve_in_named_packages() {
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(
        Path::new("/one/R/a.R"),
        "Parent <- R6::R6Class('Parent', public = list(read = function() 1))\n\
         g <- S7::new_generic('display', 'x')\nalias <- g\n",
    );
    let b = db.upsert_file(
        Path::new("/two/R/b.R"),
        "Child <- R6::R6Class('Child', inherit = one::Parent)\n\
         S7::method(one::alias, signature = Child) <- function(x) x\n",
    );
    db.set_workspace_members(vec![a, b], vec!["/one".into(), "/two".into()]);
    db.refresh_descriptions([PathBuf::from("/one"), PathBuf::from("/two")]);
    db.upsert_description(Path::new("/one"), "Package: one\n");
    db.upsert_description(Path::new("/two"), "Package: two\nImports: one\n");
    db.set_package_metadata(
        [
            ("/one", "export(Parent, alias)\n"),
            ("/two", "importFrom(one, Parent, alias)\n"),
        ]
        .into_iter()
        .map(|(root, namespace)| PackageInfo {
            root: root.into(),
            namespace: Some(namespace.into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        })
        .collect(),
    );
    for source in [
        "Child <- R6::R6Class('Child', inherit = one::Parent)\nS7::method(one::alias, signature = Child) <- function(x) x\n",
        "Child <- R6::R6Class('Child', inherit = Parent)\nS7::method(alias, signature = Child) <- function(x) x\n",
    ] {
        db.set_file_text(b, source);
        let index = project_classes(&db);
        assert_eq!(index.supertypes[&class_named(index, "Child")].len(), 1);
        assert_eq!(
            index
                .member_candidates(&class_named(index, "Child"), "read")
                .len(),
            1
        );
        let file = index.file(Path::new("/two/R/b.R")).unwrap();
        let generics = index.generic_candidates(
            &file.path,
            file.facts.oop.methods[0].generic.as_ref().unwrap(),
            false,
        );
        assert_eq!(generics.len(), 1);
        assert_eq!(
            index.generic(&generics[0]).unwrap().name,
            arity::project::classes::ClassName::Known("display".into())
        );
        assert_eq!(index.methods_for_generic(&generics[0]).len(), 1);
    }
}

#[test]
fn shared_oop_sites_follow_current_spans_without_rebuilding_the_index() {
    use arity::project::classes::{OopSiteRole, OopTarget, oop_sites};
    let source = "A <- R6::R6Class('A', public = list(read = function(x = 1) x))\n";
    let (mut db, a, _) = package_ab(source, "");
    let before = project_classes(&db).clone();
    let root = db.parsed_tree(a);
    let sites = oop_sites(db.semantic_model(a), &root);
    let member = sites.iter().find(|site| site.role == OopSiteRole::Declaration
        && matches!(&site.target, OopTarget::Declaration(key) if key.kind == arity::project::classes::OopKind::Member)).unwrap();
    let old_range = member.range;
    let target = member.target.clone();
    db.clear_query_log();
    db.set_file_text(a, format!("# Prose shifts every site.\n{source}"));
    assert_eq!(&before, project_classes(&db));
    assert!(!count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectClasses));
    let root = db.parsed_tree(a);
    let sites = oop_sites(db.semantic_model(a), &root);
    let member = sites
        .iter()
        .find(|site| site.role == OopSiteRole::Declaration && site.target == target)
        .unwrap();
    assert_eq!(
        member.range.start() - old_range.start(),
        rowan::TextSize::of("# Prose shifts every site.\n")
    );
    assert_eq!(&db.file_text(a)[member.range], "read");
}

#[test]
fn oop_binding_resolution_respects_source_order_and_masks() {
    for (source, expected) in [
        (
            "source('b.R')\nS7::method(g, signature = C) <- function(x) x\n",
            1,
        ),
        (
            "S7::method(g, signature = C) <- function(x) x\nsource('b.R')\n",
            0,
        ),
        (
            "source('b.R')\ng <- function(x) x\nS7::method(g, signature = C) <- function(x) x\n",
            0,
        ),
        (
            "source(path)\nS7::method(g, signature = C) <- function(x) x\n",
            0,
        ),
    ] {
        let (db, _, _) = scripts_ab(source, "g <- S7::new_generic('g', 'x')\n");
        let index = project_classes(&db);
        let file = index.file(Path::new("/s/a.R")).unwrap();
        assert_eq!(
            index
                .generic_candidates(
                    &file.path,
                    file.facts.oop.methods[0].generic.as_ref().unwrap(),
                    false
                )
                .len(),
            expected,
            "{source}"
        );
    }
}

#[test]
fn oop_collate_changes_invalidate_resolution_but_description_prose_backdates() {
    let (mut db, _, _) = package_ab(
        "methods::setGeneric('g', function(x) standardGeneric('g'))\n",
        "methods::setMethod('g', 'C', function(x) x)\n",
    );
    db.refresh_descriptions([PathBuf::from("/pkg")]);
    db.upsert_description(
        Path::new("/pkg"),
        "Package: pkg\nDescription: First prose.\nCollate: a.R b.R\n",
    );
    let before = project_classes(&db).clone();
    db.clear_query_log();
    db.upsert_description(
        Path::new("/pkg"),
        "Package: pkg\nDescription: Different prose.\nCollate: a.R b.R\n",
    );
    assert_eq!(&before, project_classes(&db));
    assert!(!count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectClasses));
    db.clear_query_log();
    db.upsert_description(
        Path::new("/pkg"),
        "Package: pkg\nCollate: a.R b.R missing.R\n",
    );
    let index = project_classes(&db);
    let file = index.file(Path::new("/pkg/R/b.R")).unwrap();
    assert!(
        index
            .generic_candidates(
                &file.path,
                file.facts.oop.methods[0].generic.as_ref().unwrap(),
                false
            )
            .is_empty()
    );
    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::ProjectClasses),
        Some(&1)
    );
}

#[test]
fn class_identity_omits_cross_file_edges_when_package_sources_are_incomplete() {
    let (mut db, _, _) = package_ab(
        "A <- R6::R6Class(\"A\")\n",
        "B <- R6::R6Class(\"B\", inherit = A)\n",
    );
    db.set_package_metadata(vec![PackageInfo {
        root: "/pkg".into(),
        namespace: None,
        dir_sources: ["a.R", "b.R", "missing.R"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        native_routines: BTreeSet::new(),
    }]);
    assert!(project_classes(&db).supertypes.is_empty());
}

/// A documented `add(x, y)` in a.R whose `@param y` lives on a `@rdname add`
/// joiner in b.R — the shape the package-wide topic index exists for.
const TOPIC_A: &str =
    "#' Add\n#' @param x A number.\n#' @export\nadd <- function(x, y) {\n  x + y\n}\n";
const TOPIC_B: &str =
    "#' @rdname add\n#' @param y The other number.\nadd2 <- function(x, y) x + y\n";

const ALIAS_TOPIC: &str = "#' Manual\n#' @name manual\n#' @aliases g\n#' @usage g(x)\n#' @param x Value.\n#' @return Value.\nNULL\n";

#[test]
fn roxygen_alias_projection_backdates_across_prose_offsets_and_bodies() {
    let exported = "#' @export\ng <- function(x) x\n";
    let (mut db, a, b) = package_ab(ALIAS_TOPIC, exported);
    let before = project_roxygen_topics(&db).clone();
    for docs in [
        ALIAS_TOPIC.replace("Manual", "A longer title"),
        format!("\n\n{ALIAS_TOPIC}"),
        ALIAS_TOPIC.replace("Value.", "A different description."),
    ] {
        db.clear_query_log();
        db.set_file_text(a, docs.as_str());
        assert_eq!(project_roxygen_topics(&db), &before);
        assert!(!count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectRoxygenTopics));
    }
    db.clear_query_log();
    db.set_file_text(b, "#' @export\ng <- function(x) { x + 1 }\n");
    assert_eq!(project_roxygen_topics(&db), &before);
    assert!(!count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectRoxygenTopics));
}

#[test]
fn roxygen_alias_and_usage_edits_rebuild_the_topic_index() {
    for docs in [
        ALIAS_TOPIC.replace("@aliases g", "@aliases h"),
        ALIAS_TOPIC.replace("g(x)", "g(x, extra)"),
        ALIAS_TOPIC.replace("@usage g(x)", "@usage NULL"),
        ALIAS_TOPIC.replace("@aliases g", "@aliases NULL g"),
    ] {
        let (mut db, a, _b) = package_ab(ALIAS_TOPIC, "#' @export\ng <- function(x) x\n");
        let before = project_roxygen_topics(&db).clone();
        db.clear_query_log();
        db.set_file_text(a, docs.as_str());
        assert_ne!(project_roxygen_topics(&db), &before, "{docs}");
        assert_eq!(
            count_by_kind(&db.query_log()).get(&QueryKind::ProjectRoxygenTopics),
            Some(&1),
            "{docs}",
        );
    }
}

#[test]
fn body_edit_does_not_rebuild_project_roxygen_topics() {
    // The firewall: the topic projection is range-free and turns only on the
    // documentation and the formals, so editing a body leaves it equal.
    let (mut db, a, _b) = package_ab(TOPIC_A, TOPIC_B);

    {
        let _ = project_roxygen_topics(&db);
    }

    db.clear_query_log();
    db.set_file_text(
        a,
        "#' Add\n#' @param x A number.\n#' @export\nadd <- function(x, y) {\n  x + y + 0\n}\n",
    );

    let _ = project_roxygen_topics(&db);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(
        counts.get(&QueryKind::ProjectRoxygenTopics),
        None,
        "a body edit must not rebuild project_roxygen_topics"
    );
}

#[test]
fn adding_a_param_tag_rebuilds_project_roxygen_topics() {
    let (mut db, _a, b) = package_ab(TOPIC_A, "#' @rdname add\nadd2 <- function(x, y) x + y\n");

    {
        let _ = project_roxygen_topics(&db);
    }

    db.clear_query_log();
    db.set_file_text(b, TOPIC_B);

    let topics = project_roxygen_topics(&db);

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
    let (mut db, _a, b) = package_ab("foo <- function() 1\n", "bar <- function() foo()\n");

    {
        let _ = project_defs(&db);
    }

    db.clear_query_log();
    db.set_file_text(b, "bar <- function() foo()\nqux <- 2\n");

    let defs = project_defs(&db);

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
    let (mut db, a, _b) = package_ab("foo <- function() 1\n", "bar <- 2\n");
    {
        let _ = project_defs(&db);
    }

    db.set_file_text(a, "# header\nfoo <- function() 1\n");

    let defs = project_defs(&db);
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

    let paths: Vec<_> = project.members.iter().map(|m| m.path.clone()).collect();
    assert_eq!(
        paths,
        vec![PathBuf::from("/s/a.R")],
        "a file with parse errors must be dropped from the derived membership"
    );
}

#[test]
fn keystroke_backdates_workspace_project_and_spares_graph() {
    // The membership firewall: a body edit re-runs workspace_project (it reads
    // the edited file's parse status), but it backdates to an *equal*
    // Project, so the cross-file project graph derived from it is not rebuilt.
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/s/a.R"), "foo <- function() 1\n".to_string());
    let b = db.upsert_file(
        Path::new("/s/b.R"),
        "bar <- function() {\n  foo()\n}\n".to_string(),
    );
    db.set_workspace_members(vec![a, b], vec![PathBuf::from("/s")]);

    {
        let _ = workspace_project(&db);
        let _ = visible_symbols(&db, a);
        let _ = visible_symbols(&db, b);
    }

    db.clear_query_log();
    db.set_file_text(b, "bar <- function() {\n  foo()\n  2\n}\n");

    let _ = workspace_project(&db);
    let _ = visible_symbols(&db, a);
    let _ = visible_symbols(&db, b);

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
    assert_eq!(project.members.len(), 2);
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
    let (db, _a, _b) = package_ab("foo <- function() 1\n", "bar <- function() {\n  foo()\n}\n");
    let reads = project_reads(&db);

    let foo = reads.by_name.get("foo").expect("foo is free-read");
    assert!(foo.contains(&PathBuf::from("/pkg/R/b.R")));
    assert!(
        !foo.contains(&PathBuf::from("/pkg/R/a.R")),
        "a.R binds foo, so it is not a free read there"
    );
}

#[test]
fn unchanged_use_only_names_spare_the_project_graph() {
    let (mut db, _a, b) = package_ab("label <- \"value\"\n", "glue::glue(\"Label: {label}\")\n");
    {
        let _ = project_graph(&db);
    }

    db.clear_query_log();
    db.set_file_text(b, "glue::glue(\"Longer label: {label}\")\n");
    let _ = project_graph(&db);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(counts.get(&QueryKind::FileUseOnlyReads), Some(&1));
    assert_eq!(
        counts.get(&QueryKind::ProjectGraph),
        None,
        "the range-free use-only set must backdate and spare the project graph"
    );
}

#[test]
fn changed_use_only_name_rebuilds_the_project_graph() {
    let (mut db, _a, b) = package_ab("label <- \"value\"\n", "glue::glue(\"{label}\")\n");
    {
        let _ = project_graph(&db);
    }

    db.clear_query_log();
    db.set_file_text(b, "glue::glue(\"{other}\")\n");
    let _ = project_graph(&db);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(counts.get(&QueryKind::ProjectGraph), Some(&1));
}

#[test]
fn body_edit_does_not_rebuild_project_reads() {
    // The firewall: editing b.R's body re-runs its model and file_free_reads, but
    // the free-read name set is unchanged, so the project-wide aggregate is reused.
    let (mut db, _a, b) = package_ab("foo <- function() 1\n", "bar <- function() {\n  foo()\n}\n");

    {
        let _ = project_reads(&db);
    }

    db.clear_query_log();
    db.set_file_text(b, "bar <- function() {\n  foo()\n  2\n}\n");

    let _ = project_reads(&db);

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
        let _ = visible_symbols(&db, a);
        let _ = visible_symbols(&db, b);
        assert!(visible_symbols(&db, b).visible.contains("foo"));
    }

    db.clear_query_log();

    // Edit b's body only: still defines `bar`, still reads `foo`.
    db.set_file_text(b, "bar <- function() {\n  foo()\n  2\n}\n");

    // Re-lint: requery, as production does after each edit.
    let _ = visible_symbols(&db, a);
    let _ = visible_symbols(&db, b);

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
        let _ = visible_symbols(&db, a);
        let _ = visible_symbols(&db, b);
    }

    db.clear_query_log();

    // Add a new top-level binding: b's exports change.
    db.set_file_text(b, "bar <- function() {\n  foo()\n}\nqux <- 2\n");

    let _ = visible_symbols(&db, a);
    let _ = visible_symbols(&db, b);

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
    let (mut db, _a, b) = package_ab("foo <- function() 1\n", "bar <- function() {\n  foo()\n}\n");
    {
        let _ = top_level_events(&db, b);
        let _ = visible_symbols(&db, b);
    }

    db.clear_query_log();
    db.set_file_text(b, "bar <- function() {\n  foo()\n  2\n}\n");

    let _ = top_level_events(&db, b);
    let _ = visible_symbols(&db, b);

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
    let (mut db, _a, b) = package_ab("foo <- function() 1\n", "foo\nbar\n");
    {
        let _ = visible_symbols(&db, b);
    }

    db.clear_query_log();
    db.set_file_text(b, "bar\nfoo\n");

    let _ = visible_symbols(&db, b);

    let counts = count_by_kind(&db.query_log());
    assert_eq!(
        counts.get(&QueryKind::ProjectGraph),
        Some(&1),
        "reordered top-level events must rebuild the graph even though the sets are unchanged"
    );
}

#[test]
fn reseeding_membership_preserves_seeded_metadata_and_graph_memo() {
    // Re-seeding an unchanged membership — what a lint does before querying —
    // must skip disk discovery entirely: discovery over the synthetic `/pkg`
    // root (which exists only in salsa) finds nothing and would replace the
    // seeded package metadata with an empty graph, silently unlinking a from
    // b. The graph memo must also survive, keeping the scope warm across
    // lints over the same set of files.
    let (mut db, a, b) = package_ab("foo <- function() 1\n", "bar <- function() foo()\n");
    let _ = visible_symbols(&db, a);
    assert!(visible_symbols(&db, b).visible.contains("foo"));

    db.clear_query_log();
    db.set_workspace_members(vec![a, b], vec![PathBuf::from("/pkg")]);

    assert!(
        visible_symbols(&db, b).visible.contains("foo"),
        "the seeded package metadata must survive an unchanged re-seed"
    );
    assert_eq!(
        count_by_kind(&db.query_log()).get(&QueryKind::ProjectGraph),
        None,
        "an unchanged membership must not rebuild the graph"
    );
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
    let (mut db, file) = script_one("foo <- function() {\n  bar(1)\n}\n");
    let manifest = db.set_library_index(IndexedProvider::empty());

    {
        let res = external_resolution(&db, manifest, file);
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

    let _ = external_resolution(&db, manifest, file);

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
    let (mut db, file) = script_one("library(somepkg)\nfoo <- function() across()\n");

    // somepkg is indexed (so the rule's gate passes) but does not yet export
    // `across`, so `across` is unresolved.
    let manifest = db.set_library_index(IndexedProvider::from_indices([index_pkg("somepkg", &[])]));
    {
        let res = external_resolution(&db, manifest, file);
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

    let res = external_resolution(&db, manifest, file);
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
fn spelling_candidates_refresh_even_when_unresolved_names_stay_equal() {
    let (mut db, file) = script_one("library(suggestpkg)\ncustmer_total\n");
    let manifest = db.set_library_index(IndexedProvider::from_indices([index_pkg(
        "suggestpkg",
        &["customer_total"],
    )]));
    let before = external_resolution(&db, manifest, file).clone();
    assert!(
        before
            .suggestion_exports
            .as_ref()
            .unwrap()
            .contains("customer_total")
    );
    db.set_library_index(IndexedProvider::from_indices([index_pkg(
        "suggestpkg",
        &[],
    )]));
    let after = external_resolution(&db, manifest, file);
    assert_eq!(before.unresolved, after.unresolved);
    assert!(
        !after
            .suggestion_exports
            .as_ref()
            .unwrap()
            .contains("customer_total")
    );
    let manifest = db.set_remote_exports(remote_exports(&[("suggestpkg", &["customer_total"])]));
    // The installed tier remains authoritative, including an empty export list.
    assert!(
        !external_resolution(&db, manifest, file)
            .suggestion_exports
            .as_ref()
            .unwrap()
            .contains("customer_total")
    );
    db.set_library_index(IndexedProvider::empty());
    assert!(
        external_resolution(&db, manifest, file)
            .suggestion_exports
            .as_ref()
            .unwrap()
            .contains("customer_total")
    );
}

#[test]
fn unindexed_attached_package_suppresses_resolution() {
    // Conservative gate: when an attached package's exports are unknown (not
    // base, not indexed, not bundled), it could define any unresolved name, so
    // resolution yields nothing for the whole file.
    let (mut db, file) = script_one("library(mysterypkgxyz)\nfoo <- function() across()\n");
    let manifest = db.set_library_index(IndexedProvider::empty());

    let res = external_resolution(&db, manifest, file);
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
    let (mut db, file) =
        script_one("library(metaverse)\nfoo <- function() {\n  helper_fn()\n  bogus()\n}\n");

    // Attached member indexed: its exports resolve, a genuine typo does not.
    let mut meta = index_pkg("metaverse", &[]);
    meta.attaches = vec!["helperpkg".into()];
    let manifest = db.set_library_index(IndexedProvider::from_indices([
        meta,
        index_pkg("helperpkg", &["helper_fn"]),
    ]));
    {
        let res = external_resolution(&db, manifest, file);
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
    let res = external_resolution(&db, manifest, file);
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
    let (mut db, file) =
        script_one("library(tinytable)\nfoo <- function() {\n  tt()\n  bogus()\n}\n");

    // Without the sidecar, tinytable is neither indexed nor bundled: suppressed.
    let manifest = db.set_library_index(IndexedProvider::empty());
    {
        let res = external_resolution(&db, manifest, file);
        assert!(
            res.unresolved.is_empty(),
            "unindexed package suppresses the file: {:?}",
            res.unresolved
        );
    }

    // Install the sidecar: tinytable exports `tt` (but not `bogus`).
    let manifest = db.set_remote_exports(remote_exports(&[("tinytable", &["tt"])]));
    let res = external_resolution(&db, manifest, file);
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
    let (mut db, file) = script_one("library(tinytable)\nfoo <- function() {\n  tt(1)\n}\n");
    db.set_library_index(IndexedProvider::empty());
    let manifest = db.set_remote_exports(remote_exports(&[("tinytable", &["tt"])]));
    {
        let res = external_resolution(&db, manifest, file);
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
    let _ = external_resolution(&db, manifest, file);

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

    let before: Vec<_> = workspace_project(&db).namespaces.clone();
    assert_eq!(
        before,
        vec![(root.to_path_buf(), "export(foo)\n".to_string())],
        "the seeded NAMESPACE must be visible via the derived project"
    );

    // Mutate NAMESPACE on disk WITHOUT refreshing, then do a keystroke that
    // forces `workspace_project` to re-run.
    std::fs::write(root.join("NAMESPACE"), "export(bar)\n").unwrap();
    db.set_file_text(m, "foo <- function() 2\n");

    let after: Vec<_> = workspace_project(&db).namespaces.clone();
    assert_eq!(
        before, after,
        "a keystroke must not re-read NAMESPACE from disk (query is pure)"
    );

    // An explicit refresh is the correct invalidation path: it re-reads disk and
    // re-runs the query with the new metadata.
    db.refresh_package_graph();
    let refreshed: Vec<_> = workspace_project(&db).namespaces.clone();
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
    let before = workspace_project(&db).collations.clone();
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
    let after = workspace_project(&db).collations.clone();

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
    let _ = workspace_project(&db);
    let _ = package_usage(&db);

    db.clear_query_log();
    // A body edit: the file still names no package and still exports `foo`.
    db.set_file_text(
        m,
        "foo <- function() 1 + 1
",
    );
    let _ = workspace_project(&db);
    let _ = package_usage(&db);

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
    let _ = workspace_project(&db);
    assert!(
        !package_usage(&db)[&_dir.path().to_path_buf()]
            .used
            .contains("dplyr")
    );

    db.clear_query_log();
    db.set_file_text(
        m,
        "foo <- function() dplyr::filter(x)
",
    );
    let _ = workspace_project(&db);
    let usage = package_usage(&db);

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
        let _ = workspace_project(&db);
        let res = external_resolution(&db, manifest, m);
        assert!(
            res.unresolved.is_empty(),
            "a Depends package is attached, so its bare export resolves: {:?}",
            res.unresolved
        );
    }

    // The same package under `Imports` must leave the bare name unresolved.
    std::fs::write(dir.path().join("DESCRIPTION"), describe("Imports")).expect("DESCRIPTION");
    db.refresh_package_graph();
    let _ = workspace_project(&db);
    let res = external_resolution(&db, manifest, m);
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
        let _ = workspace_project(&db);
        let res = external_resolution(&db, manifest, m);
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
    let _ = workspace_project(&db);
    let res = external_resolution(&db, manifest, m);
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
        let _ = workspace_project(&db);
        let res = external_resolution(&db, manifest, m);
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
    let _ = workspace_project(&db);
    let res = external_resolution(&db, manifest, m);
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
    let (mut db, file) = script_one("source(paste0(dir, '/x.R'))\nfoo <- function() mystery(1)\n");
    let manifest = db.set_library_index(IndexedProvider::empty());
    let res = external_resolution(&db, manifest, file);
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

#[test]
fn control_flow_expression_decisions_backdate_and_refresh() {
    let mut db = IncrementalDatabase::default();
    let before = "f <- function() { value <- if (aaa && b) 1 else 2; value }\n";
    let file = db.add_file(before);
    let graph = db.control_flow(file).clone();
    assert_eq!(graph.functions()[0].1.cyclomatic_complexity(&|_| true), 3);
    db.set_file_text(file, before.replace("aaa", "bbb"));
    assert_eq!(*db.control_flow(file), graph);
    db.clear_query_log();
    let _ = db.control_flow(file);
    assert!(db.query_log().is_empty());
    db.set_file_text(file, before.replace("aaa && b", "aaa &  b"));
    assert_eq!(
        db.control_flow(file).functions()[0]
            .1
            .cyclomatic_complexity(&|_| true),
        2
    );
}

#[test]
fn reference_environments_share_project_inheritance_and_keep_writes_out_of_exports() {
    use arity::incremental::{file_exports, file_free_reads, method_environments};
    let (mut db, a, b) = package_ab(
        "value <- 1\nBase <- methods::setRefClass('Base', fields = list(value = 'numeric'))\n",
        "Child <- methods::setRefClass('Child', contains = 'Base', methods = list(read = function() { value <<- value + 1; value }))\n",
    );
    assert!(!file_exports(&db, b).contains("value"));
    assert!(!arity::incremental::file_def_sites(&db, b).contains_key("value"));
    assert!(!file_free_reads(&db, b).contains("value"));
    assert!(
        !project_graph(&db)
            .for_file(Path::new("/pkg/R/a.R"))
            .read_elsewhere("value")
    );
    let environments = method_environments(&db, b).clone();
    assert_eq!(environments.len(), 1);
    db.set_file_text(b, "# Prose.\nChild <- methods::setRefClass('Child', contains = 'Base', methods = list(read = function() { value <<- value + 2; value }))\n");
    assert_eq!(method_environments(&db, b), &environments);
    assert!(!file_free_reads(&db, b).contains("value"));
    db.set_file_text(a, "value <- 1\nBase <- methods::setRefClass('Base')\n");
    assert_ne!(method_environments(&db, b), &environments);
    assert!(file_free_reads(&db, b).contains("value"));
}

#[test]
fn reference_alias_writes_remain_available_to_other_files() {
    let (db, a, b) = package_ab(
        "helper <- function() { value <<- 1 }\nA <- methods::setRefClass('A', fields = list(value = 'numeric'), methods = list(update = helper))\n",
        "helper()\nprint(value)\n",
    );
    assert!(arity::incremental::file_exports(&db, a).contains("value"));
    assert!(file_def_sites(&db, a).contains_key("value"));
    assert!(visible_symbols(&db, b).visible.contains("value"));
}

#[test]
fn dispatch_contexts_backdate_while_current_reads_follow_edits() {
    use arity::incremental::dispatch_contexts;
    let source = "g <- function(x) UseMethod('g')\ng.C <- function(x) .Generic\n";
    let (mut db, file) = script_one(source);
    let contexts = dispatch_contexts(&db, file).clone();
    assert!(!contexts.methods.is_empty());
    let _ = db.runtime_semantic_model(file);
    db.clear_query_log();
    let edited = format!(
        "# Prose.\n{}",
        source.replace("function(x) .Generic", "function(x) { print(x); .Generic }")
    );
    db.set_file_text(file, edited.clone());
    assert_eq!(dispatch_contexts(&db, file), &contexts);
    let model = db.runtime_semantic_model(file);
    let ident = model
        .idents()
        .iter()
        .find(|ident| ident.name == ".Generic")
        .unwrap();
    assert!(!model.is_free_read(ident));
    assert_eq!(
        usize::from(ident.range.start()),
        edited.find(".Generic").unwrap()
    );
    let counts = count_by_kind(&db.query_log());
    assert!(!counts.contains_key(&QueryKind::DispatchContexts));
    assert!(!counts.contains_key(&QueryKind::ProjectClasses));
}

#[test]
fn namespace_registration_supplies_s3_metadata_to_a_nonconventional_name() {
    use arity::incremental::dispatch_contexts;
    let (mut db, _, file) = package_ab(
        "g <- function(x) UseMethod('g')\n",
        "handler <- function(x) list(.Generic, .Class, .Group)\n",
    );
    let metadata = |namespace: &str| {
        vec![PackageInfo {
            root: PathBuf::from("/pkg"),
            namespace: Some(namespace.into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]
    };
    db.set_package_metadata(metadata("S3method(g, C, handler)\n"));
    assert_eq!(dispatch_contexts(&db, file).registered_s3, ["handler"]);
    let model = db.runtime_semantic_model(file);
    assert!(
        model
            .idents()
            .iter()
            .filter(|ident| ident.name.starts_with('.'))
            .all(|ident| !model.is_free_read(ident))
    );
    db.set_package_metadata(metadata(""));
    let model = db.runtime_semantic_model(file);
    assert!(
        model
            .idents()
            .iter()
            .filter(|ident| ident.name.starts_with('.'))
            .all(|ident| model.is_free_read(ident))
    );
}

#[test]
fn generic_effects_reach_sibling_files_without_creating_source_definitions() {
    use arity::incremental::{file_def_sites, file_exports};
    let (mut db, a, _b) = package_ab(
        "methods::setGeneric('generated', function(x) standardGeneric('generated'))\nmethods::setClass('HiddenClass')\n",
        "f <- function() generated(1)\n",
    );
    assert!(file_exports(&db, a).contains("generated"));
    assert!(!file_exports(&db, a).contains("HiddenClass"));
    assert!(!file_def_sites(&db, a).contains_key("generated"));
    assert!(
        project_graph(&db)
            .for_file(Path::new("/pkg/R/b.R"))
            .resolves("generated")
    );
    let before = arity::incremental::dispatch_contexts(&db, a).clone();
    db.set_file_text(a, "# Prose.\nmethods::setGeneric('generated', function(x) { standardGeneric('generated') })\nmethods::setClass('HiddenClass')\n");
    assert_eq!(arity::incremental::dispatch_contexts(&db, a), &before);
    assert!(db.snapshot().workspace_def_sites("generated").is_empty());
}

#[test]
fn dispatch_metadata_rejects_a_disproven_imported_registration_api() {
    let (mut db, _, file) = package_ab("", "setMethod('g', 'ANY', function(x) .Generic)\n");
    db.set_package_metadata(vec![PackageInfo {
        root: PathBuf::from("/pkg"),
        namespace: Some("importFrom(other, setMethod)\n".into()),
        dir_sources: BTreeSet::new(),
        native_routines: BTreeSet::new(),
    }]);
    let model = db.runtime_semantic_model(file);
    let ident = model
        .idents()
        .iter()
        .find(|ident| ident.name == ".Generic")
        .unwrap();
    assert!(model.is_free_read(ident));
}

#[test]
fn dispatch_registrations_supply_context_to_a_sibling_implementation() {
    let (mut db, implementation, registration) = package_ab(
        "implementation <- function(x) list(.Generic, .target, .Class)\n",
        "methods::setMethod('show', 'ANY', implementation)\n",
    );
    let assert_context = |db: &IncrementalDatabase, expected: bool| {
        let model = db.runtime_semantic_model(implementation);
        for ident in model
            .idents()
            .iter()
            .filter(|ident| matches!(ident.name.as_str(), ".Generic" | ".target"))
        {
            assert_eq!(!model.is_free_read(ident), expected, "{}", ident.name);
        }
        assert!(
            model.is_free_read(
                model
                    .idents()
                    .iter()
                    .find(|ident| ident.name == ".Class")
                    .unwrap()
            )
        );
    };
    assert_context(&db, true);
    db.set_file_text(registration, "");
    assert_context(&db, false);
}

#[test]
fn sourced_dispatch_implementations_require_visibility_and_backdate_body_edits() {
    use arity::incremental::dispatch_contexts;
    let registration_source = "methods::setMethod('show', 'ANY', implementation)\n";
    let (mut db, registration, implementation) = scripts_ab(
        registration_source,
        "implementation <- function(x) .Generic\n",
    );
    assert!(dispatch_contexts(&db, implementation).named.is_empty());
    db.set_file_text(
        registration,
        format!("source('b.R')\n{registration_source}"),
    );
    let before = dispatch_contexts(&db, implementation).clone();
    assert_eq!(before.named.len(), 1);
    let _ = db.runtime_semantic_model(implementation);
    db.clear_query_log();
    let edited = "# Prose.\nimplementation <- function(x) { print(x); .Generic }\n";
    db.set_file_text(implementation, edited);
    assert_eq!(dispatch_contexts(&db, implementation), &before);
    let model = db.runtime_semantic_model(implementation);
    let ident = model
        .idents()
        .iter()
        .find(|ident| ident.name == ".Generic")
        .unwrap();
    assert!(!model.is_free_read(ident));
    assert_eq!(
        usize::from(ident.range.start()),
        edited.find(".Generic").unwrap()
    );
    let counts = count_by_kind(&db.query_log());
    assert!(!counts.contains_key(&QueryKind::DispatchContexts));
    assert!(!counts.contains_key(&QueryKind::ProjectClasses));
}

#[test]
fn generated_replacement_names_resolve_from_sibling_files() {
    let (mut db, _, file) = package_ab(
        "methods::setGeneric('value<-', function(x, value) standardGeneric('value<-'))\n",
        "f <- function(x) `value<-`(x, 1)\n",
    );
    let library = db.set_library_index(IndexedProvider::empty());
    assert!(
        !external_resolution(&db, library, file)
            .unresolved
            .contains("`value<-`")
    );
}

#[test]
fn s3_registry_tracks_namespace_identity_and_backdates_method_bodies() {
    use arity::project::classes::{ClassName, S3MethodSource};
    let (mut db, _, b) = package_ab(
        "g <- function(x) UseMethod('g')\n",
        "handler <- function(x) x\ng.dotted.class <- function(x) x\n",
    );
    let metadata = |text: &str| {
        vec![PackageInfo {
            root: PathBuf::from("/pkg"),
            namespace: Some(text.into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]
    };
    db.set_package_metadata(metadata(
        "S3method(g, C, handler)\nS3method(other::g, D, handler)\n",
    ));
    let before = project_classes(&db).s3.clone();
    let registrations: Vec<_> = before
        .methods
        .iter()
        .filter(|method| matches!(method.source, S3MethodSource::Namespace { .. }))
        .collect();
    assert_eq!(registrations.len(), 2);
    assert_eq!(registrations[0].generics.len(), 1);
    assert!(!registrations[0].exported);
    assert!(registrations[1].delayed);
    assert_eq!(registrations[1].generic.package.as_deref(), Some("other"));
    assert!(registrations[1].generics.is_empty());
    assert!(
        before
            .methods
            .iter()
            .any(|method| method.generic.name == ClassName::Known("g".into())
                && method.class == ClassName::Known("dotted.class".into())
                && method.generics.len() == 1)
    );
    let visibility = visible_symbols(&db, b).scope();
    assert!(!visibility.exported_by_namespace("handler"));
    assert!(visibility.is_s3_method("handler"));
    assert!(!visibility.used_elsewhere("handler"));
    db.clear_query_log();
    db.set_file_text(
        b,
        "# Prose.\nhandler <- function(x) { print(x); x }\ng.dotted.class <- function(x) x\n",
    );
    assert_eq!(project_classes(&db).s3, before);
    assert!(!count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectClasses));
    db.set_package_metadata(metadata("S3method(g, Changed, handler)\nexport(handler)\n"));
    let after = &project_classes(&db).s3;
    assert_ne!(after, &before);
    let registration = after
        .methods
        .iter()
        .find(|method| matches!(method.source, S3MethodSource::Namespace { .. }))
        .unwrap();
    assert_eq!(registration.class, ClassName::Known("Changed".into()));
    assert!(registration.exported);
}

#[test]
fn s3_registry_tracks_script_registration_edits() {
    let (mut db, _, b) = package_ab(
        "g <- function(x) UseMethod('g')\nhandler <- function(x) x\n",
        ".onLoad <- function(lib, pkg) base::registerS3method('g', 'C', 'handler')\n",
    );
    let before = project_classes(&db).s3.clone();
    let registration = before
        .methods
        .iter()
        .find(|method| method.registered)
        .unwrap();
    assert_eq!(registration.generics.len(), 1);
    assert!(!registration.owner.lexical.0.is_empty());
    db.set_file_text(
        b,
        ".onLoad <- function(lib, pkg) base::registerS3method('g', 'D', 'handler')\n",
    );
    assert_ne!(project_classes(&db).s3, before);
}

#[test]
fn s3_registry_tracks_generic_lookup_context_changes() {
    let (mut db, _, b) = package_ab(
        "handler <- function(x) x\n",
        "install <- function() base::registerS3method('print', 'C', handler)\n",
    );
    db.set_library_index(IndexedProvider::empty());
    let before = project_classes(&db).s3.clone();
    assert_eq!(
        before
            .methods
            .iter()
            .find(|method| method.registered)
            .unwrap()
            .generics
            .len(),
        1
    );
    for source in [
        "install <- function() { print <- function(x) x; base::registerS3method('print', 'C', handler) }\n",
        "library(maskpkg)\ninstall <- function() base::registerS3method('print', 'C', handler)\n",
    ] {
        db.set_file_text(b, source);
        let after = &project_classes(&db).s3;
        assert_ne!(after, &before);
        assert!(
            after
                .methods
                .iter()
                .find(|method| method.registered)
                .unwrap()
                .generics
                .is_empty()
        );
    }
    db.set_library_index(IndexedProvider::from_indices([index_pkg(
        "maskpkg",
        &["print"],
    )]));
    assert!(
        project_classes(&db)
            .s3
            .methods
            .iter()
            .find(|method| method.registered)
            .unwrap()
            .generics
            .is_empty()
    );
    db.set_library_index(IndexedProvider::from_indices([index_pkg("maskpkg", &[])]));
    assert_eq!(
        project_classes(&db)
            .s3
            .methods
            .iter()
            .find(|method| method.registered)
            .unwrap()
            .generics
            .len(),
        1
    );
}

#[test]
fn s3_registry_tracks_implementation_binding_ownership_and_backdates_bodies() {
    let (mut db, _, b) = package_ab(
        "g <- function(x) UseMethod('g')\nhandler <- function(x) x\n",
        ".onLoad <- function(lib, pkg) { handler <- function(x) x + 1; base::registerS3method('g', 'C', handler) }\n",
    );
    db.set_package_metadata(vec![PackageInfo {
        root: "/pkg".into(),
        namespace: Some("export(handler)\n".into()),
        dir_sources: BTreeSet::new(),
        native_routines: BTreeSet::new(),
    }]);
    let before = project_classes(&db).s3.clone();
    assert!(
        !before
            .methods
            .iter()
            .find(|method| method.registered)
            .unwrap()
            .exported
    );
    db.clear_query_log();
    db.set_file_text(b, ".onLoad <- function(lib, pkg) { handler <- function(x) { print(x); x + 2 }; base::registerS3method('g', 'C', handler) }\n");
    assert_eq!(project_classes(&db).s3, before);
    assert!(!count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectClasses));
    db.set_file_text(
        b,
        ".onLoad <- function(lib, pkg) base::registerS3method('g', 'C', handler)\n",
    );
    let after = &project_classes(&db).s3;
    assert_ne!(after, &before);
    assert!(
        after
            .methods
            .iter()
            .find(|method| method.registered)
            .unwrap()
            .exported
    );
}

#[test]
fn s3_dispatch_refreshes_class_order_and_registrations_but_backdates_method_bodies() {
    use arity::ast::CallExpr;
    use arity::project::classes::{MethodImplementationSource, S3Dispatch, file_class_defs};
    use rowan::ast::AstNode as _;
    let (mut db, a, b) = package_ab(
        "g <- function(x) UseMethod('g')\nfirst <- function(x) x\nsecond <- function(x) x\n",
        "f <- function() probe(g(structure(list(), class = c('C', 'D'))))\n",
    );
    let metadata = |class: &str| {
        vec![PackageInfo {
            root: "/pkg".into(),
            namespace: Some(format!(
                "S3method(g, {class}, first)\nS3method(g, D, second)\n"
            )),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]
    };
    db.set_package_metadata(metadata("C"));
    let probe = |db: &IncrementalDatabase| {
        let root = db.parsed_tree(b);
        let model = db.semantic_model(b);
        let value = root
            .descendants()
            .filter_map(CallExpr::cast)
            .find(|call| call.callee_name().as_deref() == Some("g"))
            .unwrap()
            .syntax()
            .clone()
            .into();
        let path = Path::new("/pkg/R/b.R");
        let tracked = project_classes(db)
            .s3_dispatch_at(path, &root, model, &value)
            .unwrap();
        let live = db
            .snapshot()
            .class_index_in(path, &file_class_defs(model, &root), &root, model)
            .s3_dispatch_at(path, &root, model, &value)
            .unwrap();
        assert_eq!(tracked, live);
        tracked
    };
    let selected = |result: S3Dispatch| match result.selected.unwrap().implementation.unwrap() {
        MethodImplementationSource::Binding { name, .. } => name,
        other => panic!("{other:?}"),
    };
    assert_eq!(selected(probe(&db)), "first");
    db.clear_query_log();
    db.set_file_text(a, "# Shift every declaration.\ng <- function(x) UseMethod('g')\nfirst <- function(x) { print(x); x }\nsecond <- function(x) x\n");
    let _ = project_classes(&db);
    assert!(!count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectClasses));
    assert_eq!(selected(probe(&db)), "first");
    db.set_file_text(
        b,
        "f <- function() probe(g(structure(list(), class = c('D', 'C'))))\n",
    );
    assert_eq!(selected(probe(&db)), "second");
    db.set_file_text(
        b,
        "f <- function() probe(g(structure(list(), class = c('C', 'D'))))\n",
    );
    db.set_package_metadata(metadata("Changed"));
    assert_eq!(selected(probe(&db)), "second");
    db.set_file_text(a, "g <- function(x) { object <- structure(list(), class = 'Changed'); UseMethod('g', object) }\nfirst <- function(x) x\nsecond <- function(x) x\n");
    assert_eq!(selected(probe(&db)), "first");
}

#[test]
fn s3_implicit_dispatch_refreshes_reference_effects_across_files() {
    use arity::ast::CallExpr;
    use arity::project::classes::{MethodImplementationSource, S3DispatchUncertainty};
    use rowan::ast::AstNode as _;

    let methods = "g.A <- function(x) x; g.B <- function(x) x";
    let (mut db, a, b) = package_ab(
        &format!("g <- function(x) UseMethod('g'); {methods}"),
        "f <- function() probe(g(structure(new.env(), class = 'A')))",
    );
    let probe = |db: &IncrementalDatabase| {
        let root = db.parsed_tree(b);
        let model = db.semantic_model(b);
        let value = root
            .descendants()
            .filter_map(CallExpr::cast)
            .find(|call| call.callee_name().as_deref() == Some("g"))
            .unwrap()
            .syntax()
            .clone()
            .into();
        project_classes(db)
            .s3_dispatch_at(Path::new("/pkg/R/b.R"), &root, model, &value)
            .unwrap()
    };
    let first = probe(&db);
    assert!(matches!(first.selected.unwrap().implementation,
        Some(MethodImplementationSource::Binding { name, .. }) if name == "g.A"));

    db.clear_query_log();
    db.set_file_text(
        a,
        format!("g <- function(x) {{ class(x) <- 'B'; UseMethod('g') }}; {methods}"),
    );
    let mutated = probe(&db);
    assert!(mutated.selected.is_none(), "{mutated:#?}");
    assert!(
        mutated
            .uncertainty
            .contains(&S3DispatchUncertainty::UnknownClass)
    );
    assert!(count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectClasses));

    db.clear_query_log();
    db.set_file_text(a, format!("# Shift the generic.\ng <- function(x) {{ class(x) <- 'B'; UseMethod('g') }}; {methods}"));
    assert_eq!(probe(&db), mutated);
    assert!(!count_by_kind(&db.query_log()).contains_key(&QueryKind::ProjectClasses));

    db.set_file_text(
        a,
        format!(
            "g <- function(x) {{ x <- structure(list(), class = 'B'); UseMethod('g') }}; {methods}"
        ),
    );
    let rebound = probe(&db);
    assert!(matches!(rebound.selected.unwrap().implementation,
        Some(MethodImplementationSource::Binding { name, .. }) if name == "g.A"));
}
