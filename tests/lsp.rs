use lsp_types::{Position, Range};
use ravel::formatter::{FormatStyle, format_with_style};
use ravel::lsp::{compute_format_edits, compute_format_range_edits};

#[test]
fn reformats_unformatted_input_with_full_document_edit() {
    let input = "x<-1\n";
    let style = FormatStyle::default();
    let expected = format_with_style(input, style).expect("formats");
    assert_ne!(expected, input, "fixture must require reformatting");

    let edits = compute_format_edits(input, style).expect("formatter accepts input");
    assert_eq!(edits.len(), 1, "expected a single whole-document edit");

    let edit = &edits[0];
    assert_eq!(edit.new_text, expected);
    assert_eq!(edit.range.start.line, 0);
    assert_eq!(edit.range.start.character, 0);
    assert_eq!(edit.range.end.line, 1);
    assert_eq!(edit.range.end.character, 0);
}

#[test]
fn no_edits_when_input_already_formatted() {
    let style = FormatStyle::default();
    let formatted = format_with_style("x <- 1\n", style).expect("formats");

    let edits = compute_format_edits(&formatted, style).expect("idempotent input");
    assert!(
        edits.is_empty(),
        "formatted input should produce no edits, got: {edits:?}"
    );
}

#[test]
fn returns_none_when_input_has_parse_errors() {
    let style = FormatStyle::default();
    // Unclosed parenthesis is a parser diagnostic; the formatter refuses.
    let result = compute_format_edits("function(x\n", style);
    assert!(result.is_none(), "expected None, got {result:?}");
}

#[test]
fn empty_document_produces_no_edits() {
    let style = FormatStyle::default();
    let edits = compute_format_edits("", style).expect("formatter accepts empty input");
    assert!(edits.is_empty(), "empty document should produce no edits");
}

#[test]
fn end_position_handles_input_without_trailing_newline() {
    let style = FormatStyle::default();
    // Force a reformat so we exercise the full-range computation. The result
    // must reach the last character of the trailing line when there's no `\n`.
    let input = "x<-1";
    let expected = format_with_style(input, style).expect("formats");
    if expected == input {
        // If a future formatter accepts this as already-canonical, the test
        // becomes uninteresting; fail loudly so we re-pick a fixture.
        panic!("fixture must require reformatting");
    }
    let edits = compute_format_edits(input, style).expect("formats");
    let edit = edits.first().expect("one edit");
    assert_eq!(edit.range.start.line, 0);
    assert_eq!(edit.range.end.line, 0);
    assert_eq!(edit.range.end.character, input.len() as u32);
}

fn line_range(start_line: u32, start_char: u32, end_line: u32, end_char: u32) -> Range {
    Range {
        start: Position::new(start_line, start_char),
        end: Position::new(end_line, end_char),
    }
}

#[test]
fn range_edit_is_scoped_to_the_selected_statement() {
    let style = FormatStyle::default();
    // Select the middle line `2+2`; only it should be reformatted.
    let input = "1+1\n2+2\n3+3\n";
    let range = line_range(1, 0, 1, 3);
    let edits = compute_format_range_edits(input, range, style).expect("formats");
    assert_eq!(edits.len(), 1, "expected a single scoped edit");
    let edit = &edits[0];
    assert_eq!(edit.new_text, "2 + 2");
    assert_eq!(edit.range.start, Position::new(1, 0));
    assert_eq!(edit.range.end, Position::new(1, 3));
}

#[test]
fn range_edit_preserves_first_line_indentation() {
    let style = FormatStyle::default();
    let input = "f <- function() {\n  1+1\n}\n";
    // Select the indented `1+1` (characters 2..5 of line 1).
    let range = line_range(1, 2, 1, 5);
    let edits = compute_format_range_edits(input, range, style).expect("formats");
    let edit = &edits[0];
    assert_eq!(edit.new_text, "1 + 1");
    // The edit starts after the existing indentation, which is left untouched.
    assert_eq!(edit.range.start, Position::new(1, 2));
}

#[test]
fn range_edit_is_empty_when_already_formatted() {
    let style = FormatStyle::default();
    let input = "1 + 1\n2 + 2\n";
    let range = line_range(0, 0, 0, 5);
    let edits = compute_format_range_edits(input, range, style).expect("accepts input");
    assert!(edits.is_empty(), "formatted region should produce no edits");
}

#[test]
fn range_edit_returns_none_on_parse_errors() {
    let style = FormatStyle::default();
    let range = line_range(0, 0, 1, 0);
    let result = compute_format_range_edits("function(x\n", range, style);
    assert!(result.is_none(), "parse errors must block range formatting");
}
