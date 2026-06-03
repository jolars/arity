//! Integration tests for the R-introspection harvester, run against real
//! installed-package bytes checked in under `tests/fixtures/rindex/`. These
//! require **no R** — the bytes are the ground truth.

use std::path::PathBuf;

use ravel::rindex::harvest::{HarvestOptions, harvest_package};
use ravel::rindex::lazyload::LazyLoadDb;
use ravel::rindex::rds;
use ravel::rindex::schema::SymbolEntry;

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
fn help_can_be_disabled() {
    let idx = harvest_package(&fixture("magrittr"), HarvestOptions { help: false }, 0)
        .expect("harvest magrittr");
    assert!(idx.symbols.iter().all(|s| s.help.is_none()));
}
