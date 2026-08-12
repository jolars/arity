//! The DCF parser against the `DESCRIPTION`s arity actually reads.
//!
//! The parser crate has its own committed copies of these files, snapshotted
//! and round-tripped. This suite guards them **in place**: it walks the rindex
//! fixture packages by directory listing, so a `DESCRIPTION` added for some
//! future harvest case is covered the moment it lands, with nothing to
//! register.

use std::fs;
use std::path::PathBuf;

use arity::dcf;

/// Every `tests/fixtures/rindex/*/DESCRIPTION` on disk.
fn fixture_descriptions() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("rindex");
    let mut found: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", root.display()))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join("DESCRIPTION"))
        .filter(|path| path.is_file())
        .collect();
    found.sort();
    found
}

#[test]
fn real_descriptions_round_trip_losslessly() {
    let paths = fixture_descriptions();
    assert!(
        !paths.is_empty(),
        "expected at least one rindex fixture DESCRIPTION"
    );

    for path in paths {
        let text = fs::read_to_string(&path).expect("fixture DESCRIPTION");
        assert_eq!(
            dcf::reconstruct(&text),
            text,
            "lossless round-trip failed for {}",
            path.display()
        );
    }
}

#[test]
fn real_descriptions_parse_without_diagnostics() {
    for path in fixture_descriptions() {
        let text = fs::read_to_string(&path).expect("fixture DESCRIPTION");
        let output = dcf::parse(&text);
        assert!(
            output.diagnostics.is_empty(),
            "{} should parse cleanly, got {:#?}",
            path.display(),
            output.diagnostics
        );
    }
}

/// The two fields the harvester refuses to proceed without.
#[test]
fn real_descriptions_declare_package_and_version() {
    for path in fixture_descriptions() {
        let text = fs::read_to_string(&path).expect("fixture DESCRIPTION");
        let document = dcf::parse(&text).document();
        for field in ["Package", "Version"] {
            let value = document
                .field(field)
                .map(|f| f.folded_value())
                .unwrap_or_default();
            assert!(
                !value.is_empty(),
                "{} should declare a non-empty {field}",
                path.display()
            );
        }
    }
}
