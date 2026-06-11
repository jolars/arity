use lsp_types::{DocumentHighlightKind, Position, Range, TextEdit};
use ravel::formatter::{FormatStyle, format_with_style};
use ravel::lsp::{
    compute_definition, compute_document_highlights, compute_format_edits,
    compute_format_range_edits, compute_prepare_rename, compute_references, compute_rename,
    compute_rename_with_anchor,
};

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

// --- rename ----------------------------------------------------------------

fn edit_at(line: u32, start: u32, end: u32, new_text: &str) -> TextEdit {
    TextEdit {
        range: line_range(line, start, line, end),
        new_text: new_text.to_string(),
    }
}

#[test]
fn prepare_rename_offers_a_local_identifier() {
    // Cursor on the definition of `value`.
    let prepared = compute_prepare_rename("value <- 1\nprint(value)\n", 0).expect("offers rename");
    assert_eq!(prepared.placeholder, "value");
    assert_eq!(prepared.range, line_range(0, 0, 0, 5));
}

#[test]
fn prepare_rename_declines_a_keyword() {
    // Cursor on `if`: not an identifier, so no rename is offered.
    assert!(compute_prepare_rename("if (x) y\n", 0).is_none());
}

#[test]
fn prepare_rename_declines_a_nonlocal_name() {
    // `print` resolves to no local binding.
    assert!(compute_prepare_rename("print(1)\n", 0).is_none());
}

#[test]
fn rename_rewrites_definition_and_reads() {
    let edits = compute_rename("value <- 1\nprint(value)\n", 0, "v2").expect("renames");
    assert_eq!(
        edits,
        vec![
            edit_at(0, 0, 5, "v2"),  // definition
            edit_at(1, 6, 11, "v2"), // read inside print()
        ]
    );
}

#[test]
fn rename_from_a_read_site_finds_the_binding() {
    // Cursor on the read inside `print(value)` (byte offset 17).
    let text = "value <- 1\nprint(value)\n";
    let offset = text.find("value)").expect("read site");
    let edits = compute_rename(text, offset, "v2").expect("renames");
    assert_eq!(edits, vec![edit_at(0, 0, 5, "v2"), edit_at(1, 6, 11, "v2")]);
}

#[test]
fn rename_rejects_an_invalid_identifier() {
    assert!(compute_rename("value <- 1\n", 0, "1bad").is_none());
    assert!(compute_rename("value <- 1\n", 0, "if").is_none());
    assert!(compute_rename("value <- 1\n", 0, "has space").is_none());
}

#[test]
fn rename_declines_a_nonlocal_name() {
    assert!(compute_rename("print(1)\n", 0, "p2").is_none());
}

#[test]
fn rename_via_anchor_survives_an_edit_since_prepare() {
    // Prepare a rename of `value` against text A, then a keystroke inserts a
    // comment line above. The stored anchor must still drive the rename at the
    // shifted positions in text B.
    let text_a = "value <- 1\nprint(value)\n";
    let prepared = compute_prepare_rename(text_a, 0).expect("offers rename");

    let text_b = "# note\nvalue <- 1\nprint(value)\n";
    let edits = compute_rename_with_anchor(text_b, &prepared.anchor, "v2").expect("renames");
    assert_eq!(
        edits,
        vec![
            edit_at(1, 0, 5, "v2"),  // definition, shifted down a line
            edit_at(2, 6, 11, "v2"), // read inside print()
        ]
    );
}

/// Assert `compute_definition` at `offset` jumps to the byte span spelling
/// `expected` at `expected_start`.
#[track_caller]
fn assert_def(text: &str, offset: usize, expected: &str, expected_start: usize) {
    let range = compute_definition(text, offset).expect("resolves a definition");
    let start = usize::from(range.start());
    let end = usize::from(range.end());
    assert_eq!(&text[start..end], expected);
    assert_eq!(start, expected_start, "definition start");
}

#[test]
fn definition_from_read_jumps_to_the_assignment() {
    // Cursor on the read inside `print(value)` jumps to the LHS of the assignment.
    let text = "value <- 1\nprint(value)\n";
    let offset = text.find("value)").expect("read site");
    assert_def(text, offset, "value", 0);
}

#[test]
fn definition_on_the_definition_returns_itself() {
    let text = "value <- 1\nprint(value)\n";
    assert_def(text, 0, "value", 0);
}

#[test]
fn definition_of_a_parameter_read_jumps_to_the_param() {
    // The read of `x` in the body jumps to the parameter binding.
    let text = "f <- function(x) {\n  x + 1\n}\n";
    let offset = text.find("x + 1").expect("param read");
    let param_start = text.find("x)").expect("param def");
    assert_def(text, offset, "x", param_start);
}

#[test]
fn definition_declines_a_nonlocal_name() {
    // `print` resolves to no local binding; intra-file definition yields nothing.
    assert!(compute_definition("print(1)\n", 0).is_none());
}

#[test]
fn definition_declines_a_keyword() {
    assert!(compute_definition("if (x) y\n", 0).is_none());
}

// --- references -------------------------------------------------------------

/// The `(start, text)` pairs for `compute_references` at `offset`, in order.
#[track_caller]
fn refs_at(text: &str, offset: usize, include_declaration: bool) -> Vec<(usize, String)> {
    compute_references(text, offset, include_declaration)
        .expect("resolves references")
        .into_iter()
        .map(|range| {
            let start = usize::from(range.start());
            let end = usize::from(range.end());
            (start, text[start..end].to_string())
        })
        .collect()
}

#[test]
fn references_with_declaration_returns_definition_and_all_reads() {
    let text = "value <- 1\nprint(value)\nlog(value)\n";
    let def = text.find("value").expect("definition");
    let read1 = text.find("value)").expect("first read");
    let read2 = text.rfind("value)").expect("second read");
    assert_eq!(
        refs_at(text, def, true),
        vec![
            (def, "value".to_string()),
            (read1, "value".to_string()),
            (read2, "value".to_string()),
        ]
    );
}

#[test]
fn references_without_declaration_drops_the_definition() {
    let text = "value <- 1\nprint(value)\n";
    let read = text.find("value)").expect("read site");
    assert_eq!(refs_at(text, 0, false), vec![(read, "value".to_string())]);
}

#[test]
fn references_from_a_read_site_finds_the_full_set() {
    // Starting on a read resolves the same binding the definition does.
    let text = "value <- 1\nprint(value)\n";
    let read = text.find("value)").expect("read site");
    assert_eq!(
        refs_at(text, read, true),
        vec![(0, "value".to_string()), (read, "value".to_string())]
    );
}

#[test]
fn references_of_a_parameter_are_collected_in_file() {
    // A nested local (a parameter) still resolves its in-file def + reads.
    let text = "f <- function(x) {\n  x + 1\n}\n";
    let param = text.find("x)").expect("param def");
    let read = text.find("x + 1").expect("param read");
    assert_eq!(
        refs_at(text, read, true),
        vec![(param, "x".to_string()), (read, "x".to_string())]
    );
}

#[test]
fn references_decline_a_nonlocal_name() {
    assert!(compute_references("print(1)\n", 0, true).is_none());
}

#[test]
fn references_decline_a_keyword() {
    assert!(compute_references("if (x) y\n", 0, true).is_none());
}

// --- document highlight -----------------------------------------------------

/// The `(start, text, kind)` triples for `compute_document_highlights` at
/// `offset`, in order.
#[track_caller]
fn highlights_at(text: &str, offset: usize) -> Vec<(usize, String, DocumentHighlightKind)> {
    compute_document_highlights(text, offset)
        .expect("resolves highlights")
        .into_iter()
        .map(|(range, kind)| {
            let start = usize::from(range.start());
            let end = usize::from(range.end());
            (start, text[start..end].to_string(), kind)
        })
        .collect()
}

#[test]
fn document_highlight_marks_definition_write_and_reads_read() {
    let text = "value <- 1\nprint(value)\n";
    let read = text.find("value)").expect("read site");
    assert_eq!(
        highlights_at(text, 0),
        vec![
            (0, "value".to_string(), DocumentHighlightKind::WRITE),
            (read, "value".to_string(), DocumentHighlightKind::READ),
        ]
    );
}

#[test]
fn document_highlight_declines_a_nonlocal_name() {
    assert!(compute_document_highlights("print(1)\n", 0).is_none());
}
