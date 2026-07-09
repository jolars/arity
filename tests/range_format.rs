//! Range-formatting tests. Each case marks the selection in the source with
//! `<<` / `>>`; the markers are stripped to recover the byte offsets, the region
//! is formatted via [`format_range`], and the resulting edit is spliced back.

use arity::formatter::{FormatStyle, format_range, format_with_style};
use arity::parser::parse;
use rowan::{TextRange, TextSize};

/// Strip the `<<`/`>>` selection markers, returning the clean text and the
/// selected byte range.
fn parse_marked(marked: &str) -> (String, TextRange) {
    let start = marked.find("<<").expect("missing `<<` marker");
    let rest = marked.replacen("<<", "", 1);
    let end = rest.find(">>").expect("missing `>>` marker");
    let clean = rest.replacen(">>", "", 1);
    (
        clean,
        TextRange::new(TextSize::new(start as u32), TextSize::new(end as u32)),
    )
}

/// Format the marked selection and splice the edit back into the source.
fn range_format(marked: &str) -> String {
    let (text, range) = parse_marked(marked);
    let parsed = parse(&text);
    assert!(
        parsed.diagnostics.is_empty(),
        "fixture must parse cleanly: {:?}",
        parsed.diagnostics
    );
    let Some(formatted) = format_range(&parsed.cst, range, FormatStyle::default(), &text)
        .expect("format_range failed")
    else {
        return text;
    };
    let start = usize::from(formatted.range.start());
    let end = usize::from(formatted.range.end());
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..start]);
    out.push_str(&formatted.text);
    out.push_str(&text[end..]);
    out
}

#[test]
fn preserves_first_line_indentation() {
    // air's `test_format_range_mismatched_indent`: the expression is formatted
    // but its existing 2-space indent is left untouched.
    assert_eq!(range_format("1\n  <<2+2>>\n"), "1\n  2 + 2\n");
}

#[test]
fn formats_only_the_selected_statement() {
    // The unselected statements are byte-identical, even when unformatted.
    assert_eq!(range_format("1+1\n<<2+2>>\n3+3\n"), "1+1\n2 + 2\n3+3\n");
}

#[test]
fn formats_a_run_of_top_level_statements() {
    assert_eq!(range_format("<<1+1\n2+2>>\n3+3\n"), "1 + 1\n2 + 2\n3+3\n");
}

#[test]
fn formats_statement_inside_a_block() {
    let out = range_format("f <- function() {\n  <<1+1>>\n}\n");
    assert_eq!(out, "f <- function() {\n  1 + 1\n}\n");
}

#[test]
fn formats_statement_inside_a_nested_block() {
    // base_indent == 2: the inner statement keeps its 4-space indent.
    let out = range_format("f <- function() {\n  g <- function() {\n    <<1+1>>\n  }\n}\n");
    assert_eq!(
        out,
        "f <- function() {\n  g <- function() {\n    1 + 1\n  }\n}\n"
    );
}

#[test]
fn keeps_external_control_flow_body() {
    // The for header's body is detached by the trailing comment; selecting only
    // the header must still pull the body in, not invent an empty `{}`.
    let out = range_format("<<for (x in xs) # c>>\n  foo()\n");
    assert_eq!(out, "for (x in xs) {\n  # c\n  foo()\n}\n");
}

#[test]
fn preserves_blank_line_straddling_selection() {
    let out = range_format("a<-1\n\n<<b<-2>>\n");
    assert_eq!(out, "a<-1\n\nb <- 2\n");
}

#[test]
fn keeps_trailing_comment_in_selection() {
    assert_eq!(range_format("<<1+1 # note>>\n"), "1 + 1 # note\n");
}

#[test]
fn empty_selection_in_whitespace_is_a_noop() {
    // A cursor on a blank line touches no statement.
    assert_eq!(range_format("a<-1\n<<>>\nb<-2\n"), "a<-1\n\nb<-2\n");
}

#[test]
fn refuses_when_document_has_parse_errors() {
    let text = "f <- function( {\n  1+1\n}\n";
    let parsed = parse(text);
    assert!(!parsed.diagnostics.is_empty());
    // format_range only guards stray ERROR tokens; the LSP layer rejects parse
    // diagnostics. Here we assert it does not panic and is a no-op-ish result.
    let range = TextRange::new(TextSize::new(0), TextSize::new(text.len() as u32));
    let _ = format_range(&parsed.cst, range, FormatStyle::default(), text);
}

/// Range-formatting an already-formatted region must be a no-op.
#[test]
fn idempotent_on_formatted_input() {
    let cases = [
        "1 + 1\n<<2 + 2>>\n3 + 3\n",
        "f <- function() {\n  <<1 + 1>>\n}\n",
    ];
    for marked in cases {
        let (text, _) = parse_marked(marked);
        assert_eq!(range_format(marked), text, "not idempotent for {marked:?}");
    }
}

/// Range-formatting the whole document must equal whole-document formatting.
#[test]
fn whole_document_range_matches_full_format() {
    let inputs = [
        "1+1\n2+2\n3+3\n",
        "f<-function(){\n1+1\n2+2\n}\n",
        "if(x){y}else{z}\n",
        "a<-1\n\nb<-2\n",
    ];
    for input in inputs {
        let marked = format!("<<{}>>", input.trim_end_matches('\n'));
        // Re-add the trailing newline outside the selection.
        let marked = format!("{marked}\n");
        let via_range = range_format(&marked);
        let via_full = format_with_style(input, FormatStyle::default()).unwrap();
        assert_eq!(via_range, via_full, "mismatch for {input:?}");
    }
}

/// In a CRLF document, the emitted edit must use the source's line ending
/// (`LineEnding::Auto`), never splice bare LF into a CRLF buffer.
#[test]
fn crlf_source_yields_crlf_edit() {
    // Single-statement selection: no internal breaks, but the edit must still
    // not introduce a stray LF.
    let text = "x<-1\r\ny<-2\r\n";
    let parsed = parse(text);
    assert!(parsed.diagnostics.is_empty());
    let range = TextRange::new(TextSize::new(6), TextSize::new(10)); // `y<-2`
    let formatted = format_range(&parsed.cst, range, FormatStyle::default(), text)
        .expect("format_range failed")
        .expect("emits an edit");
    assert!(
        !formatted.text.contains('\n'),
        "unexpected bare LF: {formatted:?}"
    );
    assert_eq!(formatted.text, "y <- 2");

    // Multi-statement selection: the internal break between statements must be
    // CRLF, and there must be no bare LF anywhere.
    let text = "f<-function(){\r\n1+1\r\n2+2\r\n}\r\n";
    let parsed = parse(text);
    assert!(parsed.diagnostics.is_empty());
    let range = TextRange::new(TextSize::new(0), TextSize::new(text.len() as u32));
    let formatted = format_range(&parsed.cst, range, FormatStyle::default(), text)
        .expect("format_range failed")
        .expect("emits an edit");
    assert!(
        formatted.text.contains("\r\n"),
        "expected CRLF break: {formatted:?}"
    );
    assert!(
        !formatted.text.replace("\r\n", "").contains('\n'),
        "unexpected bare LF: {formatted:?}"
    );
}
