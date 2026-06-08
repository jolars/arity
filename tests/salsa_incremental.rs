use std::collections::HashMap;

use ravel::incremental::{IncrementalDatabase, QueryKind};

fn count_by_kind(entries: &[ravel::incremental::QueryLogEntry]) -> HashMap<QueryKind, usize> {
    let mut counts = HashMap::new();
    for entry in entries {
        *counts.entry(entry.kind).or_insert(0) += 1;
    }
    counts
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
        .filter(|entry| entry.file == file_a)
        .collect();
    let file_b_entries: Vec<_> = log
        .iter()
        .copied()
        .filter(|entry| entry.file == file_b)
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
fn body_edit_keeps_model_in_sync() {
    // Editing a file's contents recomputes its semantic model so downstream
    // consumers see the new bindings.
    let mut db = IncrementalDatabase::default();
    let file = db.add_file("x <- 1\n");
    assert_eq!(db.semantic_model(file).bindings().len(), 1);

    db.set_file_text(file, "x <- 1\ny <- 2\n");
    assert_eq!(db.semantic_model(file).bindings().len(), 2);
}
