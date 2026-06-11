use std::{fs, path::Path};

use arity::parser::{parse, reconstruct};

#[test]
fn parser_round_trips_crlf_fixture() {
    let input = fixture_text("crlf_line_ending");
    assert!(input.contains("\r\n"), "CRLF fixture should contain \\r\\n");

    let parsed = parse(&input);
    assert!(
        parsed.diagnostics.is_empty(),
        "CRLF fixture should parse cleanly, got diagnostics: {:#?}",
        parsed.diagnostics
    );

    let reconstructed = reconstruct(&input);
    assert_eq!(
        reconstructed, input,
        "CRLF fixture should round-trip losslessly"
    );
}

#[test]
fn parser_round_trips_lf_fixture() {
    let input = fixture_text("lf_line_ending");
    assert!(!input.contains('\r'), "LF fixture should not contain \\r");

    let parsed = parse(&input);
    assert!(
        parsed.diagnostics.is_empty(),
        "LF fixture should parse cleanly, got diagnostics: {:#?}",
        parsed.diagnostics
    );

    let reconstructed = reconstruct(&input);
    assert_eq!(
        reconstructed, input,
        "LF fixture should round-trip losslessly"
    );
}

fn fixture_text(name: &str) -> String {
    let path = Path::new("tests")
        .join("fixtures")
        .join("parser")
        .join(name)
        .join("input.R");
    fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("failed to read fixture {}: {err}", path.display());
    })
}
