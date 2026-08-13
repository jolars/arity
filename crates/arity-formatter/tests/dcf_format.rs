//! `DESCRIPTION` formatting fixtures.
//!
//! Two registries. `fixture_names` holds cases the formatter rewrites, each
//! gated on the five properties the R suite asserts plus meaning and comment
//! preservation. `declined_fixture_names` holds valid-or-invalid input the
//! formatter must **refuse**, where the only correct output is no output at all.
//!
//! Both are hand-registered: a fixture directory not listed here does not run.

use arity_formatter::formatter::{
    DescriptionFormatError, FormatStyle, format_description, format_description_with_style,
};
use arity_parser::dcf;
use insta::assert_snapshot;
use std::{fs, path::Path};

#[path = "support/dcf_meaning.rs"]
mod meaning;

fn fixture_text(name: &str, file: &str) -> String {
    let path = Path::new("tests")
        .join("fixtures")
        .join("dcf")
        .join(name)
        .join(file);
    fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("failed to read fixture {}: {err}", path.display());
    })
}

fn fixture_names() -> &'static [&'static str] {
    &[
        "authors_at_r_inline",
        "authors_at_r_unparseable",
        "collate_comment_line_stays_at_column_zero",
        "collate_quoted_order_preserved",
        "comment_before_field_moves_with_it",
        "comment_between_fields_attaches_forward",
        "comment_indented_is_value",
        "comment_trailing_has_no_anchor",
        "crlf",
        "deps_constraint_spacing",
        "deps_empty_value",
        "deps_malformed_constraint_verbatim",
        "deps_sorted_r_first",
        "no_trailing_newline",
        "opaque_field_preserves_lines",
        "order_shuffled",
        "roxygen_field_inline",
        "utf8_value",
        "wrap_description",
        "wrap_tab_continuation",
        "wrap_unbreakable_url",
    ]
}

fn declined_fixture_names() -> &'static [&'static str] {
    &[
        "declined_duplicate_fields",
        "declined_encoding_latin1",
        "declined_field_name_spaced",
        "declined_malformed_line",
        "declined_multi_record",
    ]
}

#[test]
fn dcf_fixtures_match_expected_and_snapshots() {
    for name in fixture_names() {
        let input = fixture_text(name, "input.dcf");
        let expected = fixture_text(name, "expected.dcf");
        let formatted = format_description(&input).unwrap_or_else(|err| {
            panic!("failed to format fixture {name}: {err}");
        });

        assert_eq!(formatted, expected, "formatted output mismatch for {name}");
        assert_snapshot!(format!("{name}_formatted"), formatted);
    }
}

#[test]
fn dcf_fixtures_are_stable_parseable_and_lossless() {
    for name in fixture_names() {
        let input = fixture_text(name, "input.dcf");

        let parsed_input = dcf::parse(&input);
        assert!(
            parsed_input.diagnostics.is_empty(),
            "fixture {name} input should parse cleanly, got: {:#?}",
            parsed_input.diagnostics
        );

        let formatted = format_description(&input).unwrap_or_else(|err| {
            panic!("failed to format fixture {name}: {err}");
        });

        let reparsed = dcf::parse(&formatted);
        assert!(
            reparsed.diagnostics.is_empty(),
            "fixture {name} output should parse cleanly, got: {:#?}",
            reparsed.diagnostics
        );
        assert_eq!(
            dcf::reconstruct(&formatted),
            formatted,
            "fixture {name} output should round-trip losslessly"
        );

        let reformatted = format_description(&formatted).unwrap_or_else(|err| {
            panic!("failed to reformat fixture {name}: {err}");
        });
        assert_eq!(
            reformatted, formatted,
            "fixture {name} formatting should be idempotent"
        );
    }
}

#[test]
fn dcf_fixtures_preserve_meaning_and_comments() {
    for name in fixture_names() {
        let input = fixture_text(name, "input.dcf");
        let formatted = format_description(&input).unwrap_or_else(|err| {
            panic!("failed to format fixture {name}: {err}");
        });

        let before = meaning::meaning(&input);
        let after = meaning::meaning(&formatted);
        assert_eq!(
            before,
            after,
            "fixture {name} changed meaning: {}",
            meaning::describe_difference(&before, &after)
        );

        assert_eq!(
            meaning::comments(&input),
            meaning::comments(&formatted),
            "fixture {name} changed the comment multiset"
        );
    }
}

#[test]
fn dcf_output_never_carries_trailing_whitespace() {
    for name in fixture_names() {
        let input = fixture_text(name, "input.dcf");
        let formatted = format_description(&input).unwrap_or_else(|err| {
            panic!("failed to format fixture {name}: {err}");
        });
        for (number, line) in formatted.lines().enumerate() {
            assert_eq!(
                line.trim_end(),
                line,
                "fixture {name} line {} has trailing whitespace",
                number + 1
            );
        }
        assert!(
            formatted.ends_with('\n') && !formatted.ends_with("\n\n"),
            "fixture {name} should end in exactly one newline"
        );
    }
}

#[test]
fn declined_fixtures_are_refused() {
    for name in declined_fixture_names() {
        let input = fixture_text(name, "input.dcf");
        let err = format_description(&input)
            .expect_err(&format!("fixture {name} should have been refused"));
        assert_snapshot!(format!("{name}_refusal"), err.to_string());
    }
}

#[test]
fn a_narrow_line_width_still_wraps_prose() {
    let input = "Package: p\nDescription: alpha beta gamma delta epsilon zeta\n";
    let style = FormatStyle {
        line_width: 30,
        ..FormatStyle::default()
    };
    let formatted = format_description_with_style(input, style).expect("formats");
    assert_eq!(
        formatted,
        "Package: p\nDescription: alpha beta gamma\n    delta epsilon zeta\n"
    );
}

#[test]
fn a_decline_is_distinguishable_from_a_parse_error() {
    let declined = format_description("Package: p\n\nPackage: q\n").expect_err("refused");
    assert!(declined.is_decline());
    assert!(matches!(declined, DescriptionFormatError::Declined(_)));

    let broken = format_description("Package: p\ngarbage\n").expect_err("refused");
    assert!(!broken.is_decline());
    assert!(matches!(broken, DescriptionFormatError::ParseErrors { .. }));
}

#[test]
fn a_comments_only_file_keeps_its_comments() {
    let formatted = format_description("# just a note\n").expect("formats");
    assert_eq!(formatted, "# just a note\n");
}

#[test]
fn an_empty_file_formats_to_nothing() {
    assert_eq!(format_description("").expect("formats"), "");
}
