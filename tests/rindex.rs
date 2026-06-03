//! Integration tests for the R-introspection harvester, run against real
//! installed-package bytes checked in under `tests/fixtures/rindex/`. These
//! require **no R** — the bytes are the ground truth.

use std::path::PathBuf;

use ravel::rindex::harvest::{HarvestOptions, harvest_package};
use ravel::rindex::lazyload::LazyLoadDb;
use ravel::rindex::rds::{self, Rkind};
use ravel::rindex::schema::{SymbolEntry, SymbolKind};

fn fixture(pkg: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/rindex")
        .join(pkg)
}

fn find<'a>(symbols: &'a [SymbolEntry], name: &str) -> Option<&'a SymbolEntry> {
    symbols.iter().find(|s| s.name == name)
}

#[test]
fn reads_lazyload_object_names() {
    let rdx = fixture("magrittr").join("R/magrittr.rdx");
    let db = LazyLoadDb::open(&rdx).expect("open magrittr lazy-load db");
    let names: Vec<&str> = db.names().collect();
    assert!(names.contains(&"%>%"), "expected pipe operator in objects");
    assert!(names.contains(&"freduce"));
    // The latin1/utf8 alias must round-trip.
    assert!(names.iter().any(|n| n.contains("est pas")));
}

#[test]
fn reads_rd_index_data_frame() {
    let bytes = std::fs::read(fixture("magrittr").join("Meta/Rd.rds")).unwrap();
    let rd = rds::read_rds(&bytes).expect("parse Rd.rds");
    let cols = rd.names().expect("data.frame columns");
    assert!(cols.contains(&Some("Name")));
    assert!(cols.contains(&Some("Title")));
    assert!(cols.contains(&Some("Aliases")));
}

#[test]
fn harvests_explicit_exports_with_titles() {
    let idx = harvest_package(&fixture("magrittr"), HarvestOptions::default(), 0)
        .expect("harvest magrittr");
    assert_eq!(idx.package, "magrittr");
    assert_eq!(idx.version, "2.0.4");

    // Explicit exports, including quoted operators.
    assert!(find(&idx.symbols, "%>%").is_some());
    assert!(find(&idx.symbols, "freduce").is_some());
    assert!(find(&idx.symbols, "n'est pas").is_some());

    // Help title resolved via the alias → title map.
    let pipe = find(&idx.symbols, "%<>%").expect("%<>% exported");
    let title = pipe.help.as_ref().and_then(|h| h.title.as_deref());
    assert_eq!(title, Some("Assignment pipe"));

    // Symbols are sorted.
    let mut sorted = idx.symbols.clone();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(idx.symbols, sorted);
}

#[test]
fn harvests_export_pattern_package() {
    let idx =
        harvest_package(&fixture("R.oo"), HarvestOptions::default(), 0).expect("harvest R.oo");
    assert_eq!(idx.package, "R.oo");

    // `exportPattern("^[^\\.]")` exports non-dotted objects.
    assert!(find(&idx.symbols, "Object").is_some());
    assert!(find(&idx.symbols, "throw").is_some());
    assert!(find(&idx.symbols, "Exception").is_some());

    // Dotted internals are excluded by the pattern.
    assert!(find(&idx.symbols, ".onLoad").is_none());
    assert!(find(&idx.symbols, ".__NAMESPACE__.").is_none());
}

#[test]
fn fetches_compiled_closure_formals() {
    // `freduce` is a byte-compiled magrittr closure: fetching it requires
    // consuming a BCODESXP body, and its formals must come back intact.
    let rdx = fixture("magrittr").join("R/magrittr.rdx");
    let db = LazyLoadDb::open(&rdx).expect("open magrittr lazy-load db");
    let obj = db.fetch("freduce").expect("fetch freduce");
    let Rkind::Closure { formals, .. } = &obj.kind else {
        panic!("freduce should be a closure, got {:?}", obj.kind);
    };
    let names: Vec<&str> = match &formals.kind {
        Rkind::Pairlist(items) => items.iter().filter_map(|i| i.tag.as_deref()).collect(),
        other => panic!("formals should be a pairlist, got {other:?}"),
    };
    assert_eq!(names, ["value", "function_list"]);

    // A formal carrying `...` round-trips as a named, default-less argument.
    let dots = db.fetch("[.fseq").expect("fetch [.fseq");
    let Rkind::Closure { formals, .. } = &dots.kind else {
        panic!("[.fseq should be a closure");
    };
    let names: Vec<&str> = match &formals.kind {
        Rkind::Pairlist(items) => items.iter().filter_map(|i| i.tag.as_deref()).collect(),
        other => panic!("formals pairlist, got {other:?}"),
    };
    assert!(
        names.contains(&"..."),
        "expected a `...` formal, got {names:?}"
    );
}

#[test]
fn harvest_fills_function_formals() {
    let idx = harvest_package(&fixture("magrittr"), HarvestOptions::default(), 0)
        .expect("harvest magrittr");

    let freduce = find(&idx.symbols, "freduce").expect("freduce exported");
    assert_eq!(freduce.kind, SymbolKind::Function);
    let formals = freduce.formals.as_ref().expect("freduce has formals");
    let names: Vec<&str> = formals.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["value", "function_list"]);
    // Both are required → no default text.
    assert!(formals.iter().all(|f| f.default.is_none()));

    // A primitive aliased into the package stays a callable Function with no
    // R-level formals (not misclassified as data).
    let add = find(&idx.symbols, "add").expect("add exported");
    assert_eq!(add.kind, SymbolKind::Function);
    assert!(add.formals.is_none());
}

#[test]
fn harvest_tolerates_missing_rdb() {
    // R.oo ships only a `.rdx` (no `.rdb`): harvesting must still succeed, just
    // without formals.
    let idx =
        harvest_package(&fixture("R.oo"), HarvestOptions::default(), 0).expect("harvest R.oo");
    assert!(idx.symbols.iter().all(|s| s.formals.is_none()));
}

#[test]
fn help_can_be_disabled() {
    let idx = harvest_package(&fixture("magrittr"), HarvestOptions { help: false }, 0)
        .expect("harvest magrittr");
    assert!(idx.symbols.iter().all(|s| s.help.is_none()));
}
