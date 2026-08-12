use std::{fs, path::Path};

use insta::assert_snapshot;

use arity_parser::dcf::{parse, reconstruct};

/// The DCF suite's counterpart to `parser_fixtures_snapshots_and_losslessness`.
///
/// Note the round-trip assertion is **unconditional**: unlike the R grammar,
/// DCF has no input that the parser is allowed to lose, so there is no
/// `requires_lossless_round_trip` escape hatch here.
#[test]
fn dcf_fixtures_snapshots_and_losslessness() {
    for name in fixture_names() {
        let input = fixture_input(name);
        let output = parse(&input);

        assert_snapshot!(format!("{name}_cst"), format!("{:#?}", output.cst));
        assert_snapshot!(
            format!("{name}_diagnostics"),
            format!("{:#?}", output.diagnostics)
        );

        assert_eq!(
            reconstruct(&input),
            input,
            "lossless round-trip failed for {name}"
        );
    }
}

/// The four real `DESCRIPTION`s must parse without a single diagnostic — a
/// diagnostic on an ordinary CRAN package would mean the grammar is wrong, not
/// the package.
#[test]
fn real_descriptions_parse_cleanly() {
    for name in [
        "desc_magrittr",
        "desc_r_oo",
        "desc_metatoy",
        "desc_lazydata",
    ] {
        let input = fixture_input(name);
        let output = parse(&input);
        assert!(
            output.diagnostics.is_empty(),
            "{name} should parse cleanly, got {:#?}",
            output.diagnostics
        );
        assert!(
            output.document().field("Package").is_some(),
            "{name} should declare a Package field"
        );
    }
}

fn fixture_input(name: &str) -> String {
    let path = Path::new("tests")
        .join("fixtures")
        .join("dcf")
        .join(name)
        .join("input.dcf");
    fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("failed to read fixture {}: {err}", path.display());
    })
}

/// Fixtures are **hand-registered**, exactly as on the R side: a case only runs
/// once its name is listed here.
fn fixture_names() -> &'static [&'static str] {
    &[
        "simple",
        "continuation",
        "empty_own_line_value",
        "multi_record",
        "comment_lines",
        "comment_inside_continuation",
        "crlf",
        "no_trailing_newline",
        "malformed_line",
        "orphan_continuation",
        "empty_field_name",
        "whitespace_only_separator",
        "duplicate_fields",
        "field_name_spaced",
        "desc_magrittr",
        "desc_r_oo",
        "desc_metatoy",
        "desc_lazydata",
    ]
}
