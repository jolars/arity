use arity::formatter::{FormatStyle, format_with_style};
use arity::lsp::{
    compute_color_presentations, compute_completions, compute_definition, compute_document_colors,
    compute_document_highlights, compute_document_links, compute_document_symbols,
    compute_folding_ranges, compute_format_edits, compute_format_range_edits,
    compute_prepare_rename, compute_references, compute_rename, compute_rename_with_anchor,
    compute_selection_ranges,
};
use arity::rindex::provider::IndexedProvider;
use arity::text::PositionEncoding;
use lsp_types::{
    Color, DocumentHighlightKind, DocumentSymbol, FoldingRange, FoldingRangeKind, Position, Range,
    SelectionRange, SymbolKind, TextEdit,
};

#[test]
fn completion_bare_includes_base_name() {
    use lsp_types::CompletionResponse;
    // With an empty index, base-R names still complete via the static layer.
    let resp = compute_completions("me", 2, &IndexedProvider::empty()).expect("completions");
    let labels: Vec<String> = match resp {
        CompletionResponse::Array(items) => items,
        CompletionResponse::List(list) => list.items,
    }
    .into_iter()
    .map(|i| i.label)
    .collect();
    assert!(labels.contains(&"mean".to_string()), "{labels:?}");
}

#[test]
fn reformats_unformatted_input_with_full_document_edit() {
    let input = "x<-1\n";
    let style = FormatStyle::default();
    let expected = format_with_style(input, style).expect("formats");
    assert_ne!(expected, input, "fixture must require reformatting");

    let edits = compute_format_edits(input, style, PositionEncoding::Utf16)
        .expect("formatter accepts input");
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

    let edits =
        compute_format_edits(&formatted, style, PositionEncoding::Utf16).expect("idempotent input");
    assert!(
        edits.is_empty(),
        "formatted input should produce no edits, got: {edits:?}"
    );
}

#[test]
fn returns_none_when_input_has_parse_errors() {
    let style = FormatStyle::default();
    // Unclosed parenthesis is a parser diagnostic; the formatter refuses.
    let result = compute_format_edits("function(x\n", style, PositionEncoding::Utf16);
    assert!(result.is_none(), "expected None, got {result:?}");
}

#[test]
fn empty_document_produces_no_edits() {
    let style = FormatStyle::default();
    let edits = compute_format_edits("", style, PositionEncoding::Utf16)
        .expect("formatter accepts empty input");
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
    let edits = compute_format_edits(input, style, PositionEncoding::Utf16).expect("formats");
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
    let edits =
        compute_format_range_edits(input, range, style, PositionEncoding::Utf16).expect("formats");
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
    let edits =
        compute_format_range_edits(input, range, style, PositionEncoding::Utf16).expect("formats");
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
    let edits = compute_format_range_edits(input, range, style, PositionEncoding::Utf16)
        .expect("accepts input");
    assert!(edits.is_empty(), "formatted region should produce no edits");
}

#[test]
fn range_edit_returns_none_on_parse_errors() {
    let style = FormatStyle::default();
    let range = line_range(0, 0, 1, 0);
    let result = compute_format_range_edits("function(x\n", range, style, PositionEncoding::Utf16);
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
    let prepared = compute_prepare_rename("value <- 1\nprint(value)\n", 0, PositionEncoding::Utf16)
        .expect("offers rename");
    assert_eq!(prepared.placeholder, "value");
    assert_eq!(prepared.range, line_range(0, 0, 0, 5));
}

#[test]
fn prepare_rename_declines_a_keyword() {
    // Cursor on `if`: not an identifier, so no rename is offered.
    assert!(compute_prepare_rename("if (x) y\n", 0, PositionEncoding::Utf16).is_none());
}

#[test]
fn prepare_rename_declines_a_nonlocal_name() {
    // `print` resolves to no local binding.
    assert!(compute_prepare_rename("print(1)\n", 0, PositionEncoding::Utf16).is_none());
}

#[test]
fn rename_rewrites_definition_and_reads() {
    let edits = compute_rename(
        "value <- 1\nprint(value)\n",
        0,
        "v2",
        PositionEncoding::Utf16,
    )
    .expect("renames");
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
    let edits = compute_rename(text, offset, "v2", PositionEncoding::Utf16).expect("renames");
    assert_eq!(edits, vec![edit_at(0, 0, 5, "v2"), edit_at(1, 6, 11, "v2")]);
}

#[test]
fn rename_rewrites_all_reassignments() {
    // `x` is one variable reassigned once: renaming from any occurrence rewrites
    // both defs and both reads (not just the ones tied to one binding record).
    let text = "x <- 1\nf(x)\nx <- 2\ng(x)\n";
    let def1 = text.find("x <- 1").expect("first def");
    let read1 = text.find("f(x)").expect("first read") + 2;
    let def2 = text.find("x <- 2").expect("second def");
    let read2 = text.find("g(x)").expect("second read") + 2;
    let expected = vec![
        edit_at(0, 0, 1, "y"), // first def
        edit_at(1, 2, 3, "y"), // read in f(x)
        edit_at(2, 0, 1, "y"), // second def
        edit_at(3, 2, 3, "y"), // read in g(x)
    ];
    for cursor in [def1, read1, def2, read2] {
        let edits = compute_rename(text, cursor, "y", PositionEncoding::Utf16).expect("renames");
        assert_eq!(edits, expected, "cursor at byte {cursor}");
    }
}

#[test]
fn rename_rejects_an_invalid_identifier() {
    assert!(compute_rename("value <- 1\n", 0, "1bad", PositionEncoding::Utf16).is_none());
    assert!(compute_rename("value <- 1\n", 0, "if", PositionEncoding::Utf16).is_none());
    assert!(compute_rename("value <- 1\n", 0, "has space", PositionEncoding::Utf16).is_none());
}

#[test]
fn rename_declines_a_nonlocal_name() {
    assert!(compute_rename("print(1)\n", 0, "p2", PositionEncoding::Utf16).is_none());
}

#[test]
fn rename_via_anchor_survives_an_edit_since_prepare() {
    // Prepare a rename of `value` against text A, then a keystroke inserts a
    // comment line above. The stored anchor must still drive the rename at the
    // shifted positions in text B.
    let text_a = "value <- 1\nprint(value)\n";
    let prepared =
        compute_prepare_rename(text_a, 0, PositionEncoding::Utf16).expect("offers rename");

    let text_b = "# note\nvalue <- 1\nprint(value)\n";
    let edits = compute_rename_with_anchor(text_b, &prepared.anchor, "v2", PositionEncoding::Utf16)
        .expect("renames");
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
fn references_group_reassignments_of_one_variable() {
    // Both defs and both reads of the reassigned `x` are references to one
    // variable, from a cursor on any occurrence.
    let text = "x <- 1\nf(x)\nx <- 2\ng(x)\n";
    let def1 = text.find("x <- 1").expect("first def");
    let read1 = text.find("f(x)").expect("first read") + 2;
    let def2 = text.find("x <- 2").expect("second def");
    let read2 = text.find("g(x)").expect("second read") + 2;
    let expected = vec![
        (def1, "x".to_string()),
        (read1, "x".to_string()),
        (def2, "x".to_string()),
        (read2, "x".to_string()),
    ];
    for cursor in [def1, read1, def2, read2] {
        assert_eq!(
            refs_at(text, cursor, true),
            expected,
            "cursor at byte {cursor}"
        );
    }
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
fn document_highlight_marks_each_reassignment_as_write() {
    // Every reassignment of `x` is a WRITE; every read is a READ.
    let text = "x <- 1\nf(x)\nx <- 2\ng(x)\n";
    let def1 = text.find("x <- 1").expect("first def");
    let read1 = text.find("f(x)").expect("first read") + 2;
    let def2 = text.find("x <- 2").expect("second def");
    let read2 = text.find("g(x)").expect("second read") + 2;
    assert_eq!(
        highlights_at(text, def1),
        vec![
            (def1, "x".to_string(), DocumentHighlightKind::WRITE),
            (read1, "x".to_string(), DocumentHighlightKind::READ),
            (def2, "x".to_string(), DocumentHighlightKind::WRITE),
            (read2, "x".to_string(), DocumentHighlightKind::READ),
        ]
    );
}

#[test]
fn document_highlight_declines_a_nonlocal_name() {
    assert!(compute_document_highlights("print(1)\n", 0).is_none());
}

// --- document symbols -------------------------------------------------------

/// `(name, kind)` pairs for a flat list of symbols, in order.
#[track_caller]
fn symbol_names(symbols: &[DocumentSymbol]) -> Vec<(&str, SymbolKind)> {
    symbols.iter().map(|s| (s.name.as_str(), s.kind)).collect()
}

#[test]
fn document_symbols_list_top_level_functions_and_variables() {
    let text = "f <- function(x) x + 1\ny <- 42\n";
    let symbols = compute_document_symbols(text, PositionEncoding::Utf16);
    assert_eq!(
        symbol_names(&symbols),
        vec![("f", SymbolKind::FUNCTION), ("y", SymbolKind::VARIABLE)]
    );
}

#[test]
fn document_symbols_nest_bindings_inside_a_function_body() {
    let text = "f <- function() {\n  g <- function() 1\n  h <- 2\n}\n";
    let symbols = compute_document_symbols(text, PositionEncoding::Utf16);
    assert_eq!(symbol_names(&symbols), vec![("f", SymbolKind::FUNCTION)]);
    let children = symbols[0].children.as_ref().expect("f has nested symbols");
    assert_eq!(
        symbol_names(children),
        vec![("g", SymbolKind::FUNCTION), ("h", SymbolKind::VARIABLE)]
    );
    // The leaf function has no further children.
    assert!(children[0].children.is_none());
}

#[test]
fn document_symbols_are_empty_for_a_bare_call() {
    assert!(compute_document_symbols("print(1)\n", PositionEncoding::Utf16).is_empty());
}

#[test]
fn document_symbols_surface_a_binding_inside_a_control_flow_block() {
    // An `if` block introduces no scope, so `x` is a file-level binding: it must
    // still surface (here flattened to the top level), never be lost.
    let text = "if (cond) {\n  x <- 1\n}\n";
    let symbols = compute_document_symbols(text, PositionEncoding::Utf16);
    assert_eq!(symbol_names(&symbols), vec![("x", SymbolKind::VARIABLE)]);
}

#[test]
fn document_symbols_handle_right_assignment() {
    let text = "42 -> z\n";
    let symbols = compute_document_symbols(text, PositionEncoding::Utf16);
    assert_eq!(symbol_names(&symbols), vec![("z", SymbolKind::VARIABLE)]);
}

#[test]
fn document_symbol_ranges_target_the_name_and_enclose_the_statement() {
    let text = "value <- 1\n";
    let symbols = compute_document_symbols(text, PositionEncoding::Utf16);
    let sym = &symbols[0];
    // The selection range is the identifier `value` (columns 0..5).
    assert_eq!(sym.selection_range.start, Position::new(0, 0));
    assert_eq!(sym.selection_range.end, Position::new(0, 5));
    // The full range encloses the whole `value <- 1` statement.
    assert_eq!(sym.range.start, Position::new(0, 0));
    assert_eq!(sym.range.end, Position::new(0, 10));
}

#[track_caller]
fn fold_lines(ranges: &[FoldingRange]) -> Vec<(u32, u32)> {
    ranges.iter().map(|r| (r.start_line, r.end_line)).collect()
}

#[test]
fn folding_ranges_fold_a_multiline_function_body() {
    let text = "f <- function() {\n  x + 1\n}\n";
    let ranges = compute_folding_ranges(text);
    // The `{` is on line 0, the `}` on line 2: a single block fold. The
    // single-line parameter list `()` does not fold.
    assert_eq!(fold_lines(&ranges), vec![(0, 2)]);
    assert!(ranges[0].kind.is_none());
}

#[test]
fn folding_ranges_skip_single_line_constructs() {
    assert!(compute_folding_ranges("f <- function() 1\n").is_empty());
    assert!(compute_folding_ranges("g(1, 2, 3)\n").is_empty());
}

#[test]
fn folding_ranges_fold_a_multiline_call_argument_list() {
    let text = "g(\n  1,\n  2\n)\n";
    let ranges = compute_folding_ranges(text);
    assert_eq!(fold_lines(&ranges), vec![(0, 3)]);
}

#[test]
fn folding_ranges_fold_nested_blocks_independently() {
    let text = "f <- function() {\n  if (a) {\n    b\n  }\n}\n";
    let ranges = compute_folding_ranges(text);
    let mut lines = fold_lines(&ranges);
    lines.sort_unstable();
    assert_eq!(lines, vec![(0, 4), (1, 3)]);
}

#[test]
fn folding_ranges_fold_a_run_of_comments() {
    let text = "# one\n# two\n# three\nx <- 1\n";
    let ranges = compute_folding_ranges(text);
    assert_eq!(fold_lines(&ranges), vec![(0, 2)]);
    assert_eq!(ranges[0].kind, Some(FoldingRangeKind::Comment));
}

#[test]
fn folding_ranges_ignore_a_lone_comment() {
    assert!(compute_folding_ranges("# solo\nx <- 1\n").is_empty());
}

// --- document links -------------------------------------------------------

const NO_LIMIT: u64 = u64::MAX;

#[test]
fn document_links_link_existing_relative_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("helpers.R"), "f <- function() 1\n").unwrap();
    // A source() arg and a bare literal both name the existing file.
    let text = "source(\"helpers.R\")\nx <- \"helpers.R\"\n";
    let links = compute_document_links(text, Some(dir.path()), NO_LIMIT, PositionEncoding::Utf16);
    assert_eq!(links.len(), 2, "both literals resolve to the file");
    // Each link targets a file:// URI naming the resolved file.
    for link in &links {
        let target = link.target.as_ref().expect("link has a target");
        let target = target.to_string();
        assert!(
            target.starts_with("file://"),
            "target is a file URI: {target}"
        );
        assert!(
            target.ends_with("helpers.R"),
            "target names the file: {target}"
        );
        // The link stays on a single line (one quoted token).
        assert_eq!(link.range.start.line, link.range.end.line);
    }
    // First link is on line 0 (the source() call), second on line 1.
    assert_eq!(links[0].range.start.line, 0);
    assert_eq!(links[1].range.start.line, 1);
}

#[test]
fn document_links_skip_nonexistent_paths() {
    let dir = tempfile::tempdir().expect("tempdir");
    let links = compute_document_links(
        "x <- \"missing.R\"\n",
        Some(dir.path()),
        NO_LIMIT,
        PositionEncoding::Utf16,
    );
    assert!(links.is_empty(), "a path with no file is not linked");
}

#[test]
fn document_links_skip_directories() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join("subdir")).unwrap();
    let links = compute_document_links(
        "x <- \"subdir\"\n",
        Some(dir.path()),
        NO_LIMIT,
        PositionEncoding::Utf16,
    );
    assert!(links.is_empty(), "a directory is not a linkable file");
}

#[test]
fn document_links_respect_size_limit() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("helpers.R"), "f <- function() 1\n").unwrap();
    let text = "x <- \"helpers.R\"\n";
    // A limit below the document size skips scanning entirely.
    let limit = (text.len() - 1) as u64;
    assert!(
        compute_document_links(text, Some(dir.path()), limit, PositionEncoding::Utf16).is_empty()
    );
}

fn pos(line: u32, character: u32) -> Position {
    Position { line, character }
}

fn rng(sl: u32, sc: u32, el: u32, ec: u32) -> Range {
    Range {
        start: pos(sl, sc),
        end: pos(el, ec),
    }
}

/// Flatten a selection-range chain into its ranges, innermost first.
fn chain(sr: &SelectionRange) -> Vec<Range> {
    let mut out = vec![sr.range];
    let mut cur = sr.parent.as_deref();
    while let Some(p) = cur {
        out.push(p.range);
        cur = p.parent.as_deref();
    }
    out
}

fn le(a: Position, b: Position) -> bool {
    (a.line, a.character) <= (b.line, b.character)
}

/// `outer` strictly contains `inner`: covers it and is not identical.
fn contains_strictly(outer: Range, inner: Range) -> bool {
    le(outer.start, inner.start) && le(inner.end, outer.end) && outer != inner
}

#[test]
fn selection_range_expands_from_identifier_outward() {
    let text = "f <- function() g(x + 1)\n";
    let x = text.find('x').unwrap() as u32;
    let ranges = compute_selection_ranges(text, &[pos(0, x)], PositionEncoding::Utf16);
    assert_eq!(ranges.len(), 1);
    let chain = chain(&ranges[0]);

    // Innermost is the identifier under the cursor.
    assert_eq!(chain[0], rng(0, 18, 0, 19));
    // The binary expression `x + 1` is a step out.
    assert!(chain.contains(&rng(0, 18, 0, 23)));
    // Each step strictly contains the previous one (no degenerate links).
    for w in chain.windows(2) {
        assert!(
            contains_strictly(w[1], w[0]),
            "{:?} should strictly contain {:?}",
            w[1],
            w[0]
        );
    }
    // The outermost range covers the whole file.
    assert_eq!(chain.last().unwrap().start, pos(0, 0));
}

#[test]
fn selection_range_on_whitespace_expands_from_enclosing_node() {
    // Cursor on the indentation before `x`, not on any real token.
    let text = "foo(\n  x\n)\n";
    let ranges = compute_selection_ranges(text, &[pos(1, 1)], PositionEncoding::Utf16);
    assert_eq!(ranges.len(), 1);
    let chain = chain(&ranges[0]);
    // The innermost range is a real (non-empty) enclosing node, never zero-width.
    assert_ne!(chain[0].start, chain[0].end);
    for w in chain.windows(2) {
        assert!(contains_strictly(w[1], w[0]));
    }
}

#[test]
fn selection_range_returns_one_chain_per_position() {
    let text = "a <- 1\nb <- 2\n";
    let ranges = compute_selection_ranges(text, &[pos(0, 0), pos(1, 0)], PositionEncoding::Utf16);
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0].range, rng(0, 0, 0, 1));
    assert_eq!(ranges[1].range, rng(1, 0, 1, 1));
}

#[test]
fn selection_range_bare_identifier_expands_to_file() {
    let text = "x\n";
    let ranges = compute_selection_ranges(text, &[pos(0, 0)], PositionEncoding::Utf16);
    let chain = chain(&ranges[0]);
    assert_eq!(chain[0], rng(0, 0, 0, 1));
    assert_eq!(chain.last().unwrap().start, pos(0, 0));
}

#[test]
fn selection_range_empty_input_does_not_panic() {
    let ranges = compute_selection_ranges("", &[pos(0, 0)], PositionEncoding::Utf16);
    assert_eq!(ranges.len(), 1);
    // A whole-file (empty) range, and no parent.
    assert_eq!(ranges[0].range, rng(0, 0, 0, 0));
    assert!(ranges[0].parent.is_none());
}

// --- document color ---------------------------------------------------------

fn color(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color {
        red: r,
        green: g,
        blue: b,
        alpha: a,
    }
}

#[test]
fn document_color_recognizes_six_digit_hex() {
    let text = "x <- \"#ff0000\"\n";
    let colors = compute_document_colors(text, PositionEncoding::Utf16);
    assert_eq!(colors.len(), 1);
    assert_eq!(colors[0].color, color(1.0, 0.0, 0.0, 1.0));
    // The swatch range slices the whole quoted token and stays on one line.
    assert_eq!(colors[0].range, rng(0, 5, 0, 14));
}

#[test]
fn document_color_recognizes_eight_digit_hex_alpha() {
    let colors = compute_document_colors("x <- \"#ff000080\"\n", PositionEncoding::Utf16);
    assert_eq!(colors.len(), 1);
    assert_eq!(colors[0].color, color(1.0, 0.0, 0.0, 128.0 / 255.0));
}

#[test]
fn document_color_hex_is_case_insensitive() {
    let upper = compute_document_colors("x <- \"#FF0000\"\n", PositionEncoding::Utf16);
    let lower = compute_document_colors("x <- \"#ff0000\"\n", PositionEncoding::Utf16);
    assert_eq!(upper[0].color, lower[0].color);
}

#[test]
fn document_color_recognizes_named_colors() {
    let colors = compute_document_colors("x <- \"red\"\n", PositionEncoding::Utf16);
    assert_eq!(colors.len(), 1);
    assert_eq!(colors[0].color, color(1.0, 0.0, 0.0, 1.0));
}

#[test]
fn document_color_named_lookup_is_case_insensitive() {
    let colors = compute_document_colors("x <- \"Red\"\n", PositionEncoding::Utf16);
    assert_eq!(colors.len(), 1);
    assert_eq!(colors[0].color, color(1.0, 0.0, 0.0, 1.0));
}

#[test]
fn document_color_grey_and_gray_agree() {
    let grey = compute_document_colors("x <- \"grey40\"\n", PositionEncoding::Utf16);
    let gray = compute_document_colors("x <- \"gray40\"\n", PositionEncoding::Utf16);
    assert_eq!(grey[0].color, gray[0].color);
}

#[test]
fn document_color_skips_non_colors() {
    // A plain word, the 3/4-digit short form, and trailing junk are not colors.
    for src in [
        "x <- \"hello\"\n",
        "x <- \"#fff\"\n",
        "x <- \"#ff00\"\n",
        "x <- \"#ff0000 \"\n",
        "x <- \"\"\n",
    ] {
        assert!(
            compute_document_colors(src, PositionEncoding::Utf16).is_empty(),
            "expected no color for {src:?}"
        );
    }
}

#[test]
fn document_color_recognizes_single_line_raw_string() {
    let colors = compute_document_colors("x <- r\"(#ff0000)\"\n", PositionEncoding::Utf16);
    assert_eq!(colors.len(), 1);
    assert_eq!(colors[0].color, color(1.0, 0.0, 0.0, 1.0));
}

#[test]
fn document_color_reports_each_literal() {
    let colors =
        compute_document_colors("a <- \"red\"\nb <- \"#0000ff\"\n", PositionEncoding::Utf16);
    assert_eq!(colors.len(), 2);
    assert_eq!(colors[0].color, color(1.0, 0.0, 0.0, 1.0));
    assert_eq!(colors[0].range, rng(0, 5, 0, 10));
    assert_eq!(colors[1].color, color(0.0, 0.0, 1.0, 1.0));
    assert_eq!(colors[1].range, rng(1, 5, 1, 14));
}

// --- color presentation -----------------------------------------------------

#[test]
fn color_presentation_round_trips_and_preserves_double_quote() {
    let text = "x <- \"#ff0000\"\n";
    let range = rng(0, 5, 0, 14); // the token range, quotes included
    let out = compute_color_presentations(
        text,
        &color(0.0, 1.0, 0.0, 1.0),
        range,
        PositionEncoding::Utf16,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].label, "#00ff00");
    let edit = out[0].text_edit.as_ref().expect("text edit");
    assert_eq!(edit.range, range);
    assert_eq!(edit.new_text, "\"#00ff00\"");
}

#[test]
fn color_presentation_preserves_single_quote() {
    let text = "x <- '#ff0000'\n";
    let range = rng(0, 5, 0, 14);
    let out = compute_color_presentations(
        text,
        &color(0.0, 1.0, 0.0, 1.0),
        range,
        PositionEncoding::Utf16,
    );
    assert_eq!(out[0].text_edit.as_ref().unwrap().new_text, "'#00ff00'");
}

#[test]
fn color_presentation_emits_eight_digits_for_alpha() {
    let text = "x <- \"#ff0000\"\n";
    let range = rng(0, 5, 0, 14);
    let out = compute_color_presentations(
        text,
        &color(1.0, 0.0, 0.0, 0.5),
        range,
        PositionEncoding::Utf16,
    );
    // 0.5 rounds to 0x80; alpha < 1 keeps the RRGGBBAA form.
    assert_eq!(out[0].label, "#ff000080");
    assert_eq!(out[0].text_edit.as_ref().unwrap().new_text, "\"#ff000080\"");
}

#[test]
fn color_presentation_defaults_to_double_quote_for_raw_string() {
    let text = "x <- r\"(#ff0000)\"\n";
    // The token starts at the `r`; the peek isn't a quote, so we default to `"`.
    let range = rng(0, 5, 0, 17);
    let out = compute_color_presentations(
        text,
        &color(0.0, 1.0, 0.0, 1.0),
        range,
        PositionEncoding::Utf16,
    );
    assert_eq!(out[0].text_edit.as_ref().unwrap().new_text, "\"#00ff00\"");
}
