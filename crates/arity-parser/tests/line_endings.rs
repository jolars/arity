use std::{fs, path::Path};

use arity_parser::parser::{parse, reconstruct};

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

#[test]
fn parser_round_trips_crlf_roxygen() {
    let input = fixture_text("roxygen_crlf");
    assert!(
        input.contains("\r\n"),
        "roxygen CRLF fixture should contain \\r\\n"
    );

    let parsed = parse(&input);
    assert!(
        parsed.diagnostics.is_empty(),
        "roxygen CRLF fixture should parse cleanly, got diagnostics: {:#?}",
        parsed.diagnostics
    );
    // The `\r\n` must stay a single NEWLINE token and never leak into roxygen
    // content (the sub-tokenizer leaves the carriage return to the main lexer).
    assert!(
        reconstruct(&input) == input,
        "roxygen CRLF fixture should round-trip losslessly"
    );
}

/// The DCF grammar owes the same guarantee. Its scanner deliberately avoids
/// [`str::lines`], which eats the `\r` of a CRLF pair; a leak would show up
/// here rather than in `folded_value`, which trims it away invisibly.
#[test]
fn dcf_round_trips_crlf_fixture() {
    let path = Path::new("tests")
        .join("fixtures")
        .join("dcf")
        .join("crlf")
        .join("input.dcf");
    let input = fs::read_to_string(&path).expect("DCF CRLF fixture");
    assert!(input.contains("\r\n"), "CRLF fixture should contain \\r\\n");

    let parsed = arity_parser::dcf::parse(&input);
    assert!(
        parsed.diagnostics.is_empty(),
        "DCF CRLF fixture should parse cleanly, got diagnostics: {:#?}",
        parsed.diagnostics
    );
    assert_eq!(
        arity_parser::dcf::reconstruct(&input),
        input,
        "DCF CRLF fixture should round-trip losslessly"
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
