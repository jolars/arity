use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ravel::incremental::{IncrementalDatabase, QueryKind, SourceFile};
use ravel::project::{Project, ProjectMember, external_resolution, visible_symbols};
use ravel::rindex::provider::IndexedProvider;
use ravel::rindex::schema::{PackageIndex, SCHEMA_VERSION, SymbolEntry, SymbolKind};

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
    let fresh = ravel::parser::parse("f <- function() {\n  a + b + c\n}\n");
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
    Project::new(db, members, Vec::new())
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
    Project::new(db, members, Vec::new())
}

/// A harvested package index for `name` exporting `exports`.
fn index_pkg(name: &str, exports: &[&str]) -> PackageIndex {
    PackageIndex {
        schema_version: SCHEMA_VERSION,
        package: name.into(),
        version: "1.0".into(),
        lib_path: "/lib".into(),
        r_version: None,
        harvested_at: 0,
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
