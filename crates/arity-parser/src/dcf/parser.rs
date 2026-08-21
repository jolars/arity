//! The DCF parser: one forward pass over physical lines.
//!
//! Unlike the R grammar, this needs no event pipeline. Events exist there
//! because the Pratt parser must retroactively wrap tokens it has already
//! passed; DCF never backtracks and nests no deeper than
//! `ROOT > RECORD > FIELD > VALUE_LINE`, so a direct [`GreenNodeBuilder`] walk
//! is the whole parser.
//!
//! Losslessness is **structural**: the scanner carries one byte cursor and
//! every emitted token is a slice `text[a..b]` with `a` the previous token's
//! end. Note the deliberate absence of [`str::lines`] — it eats the `\r` of a
//! CRLF pair and erases the difference between a final line with and without a
//! trailing newline.

use rowan::GreenNodeBuilder;

use crate::dcf::ast::Document;
use crate::dcf::syntax::{SyntaxKind, SyntaxNode};
use crate::parser::diagnostics::push_diagnostic;

pub use crate::parser::ParseDiagnostic;

/// Reported for a line that is neither a field, a continuation, a comment, nor
/// blank. R's `read.dcf` errors here ("Line starting ... is malformed!").
pub(crate) const MALFORMED_LINE: &str =
    "malformed line: expected 'Field: value' or an indented continuation line";

/// Reported for an indented line with no field to continue. R's `read.dcf`
/// errors here ("Found continuation line ... at begin of record.").
pub(crate) const ORPHAN_CONTINUATION: &str = "continuation line at the start of a record";

/// Reported for `: value` — a colon with nothing before it. R's `read.dcf`
/// treats this as malformed too.
pub(crate) const EMPTY_FIELD_NAME: &str = "empty field name";

#[derive(Debug, Clone)]
pub struct ParseOutput {
    pub cst: SyntaxNode,
    pub diagnostics: Vec<ParseDiagnostic>,
}

impl ParseOutput {
    /// The typed view of the parsed document.
    pub fn document(&self) -> Document {
        use rowan::ast::AstNode as _;
        Document::cast(self.cst.clone()).expect("the DCF parser always roots the tree at ROOT")
    }
}

/// Parse DCF text into a lossless CST plus any diagnostics.
///
/// The parse never fails and never drops a byte: a malformed line is kept as a
/// [`MALFORMED_LINE`](SyntaxKind::MALFORMED_LINE) node and reported on the
/// side channel.
pub fn parse(text: &str) -> ParseOutput {
    Scanner::new(text).run()
}

/// Round-trip `text` through the parser. Always equal to `text`.
pub fn reconstruct(text: &str) -> String {
    parse(text)
        .cst
        .descendants_with_tokens()
        .filter_map(|el| el.into_token())
        .map(|tok| tok.text().to_string())
        .collect::<String>()
}

struct Scanner<'a> {
    text: &'a str,
    builder: GreenNodeBuilder<'static>,
    diagnostics: Vec<ParseDiagnostic>,
    record_open: bool,
    field_open: bool,
}

impl<'a> Scanner<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            builder: GreenNodeBuilder::new(),
            diagnostics: Vec::new(),
            record_open: false,
            field_open: false,
        }
    }

    fn run(mut self) -> ParseOutput {
        self.builder.start_node(SyntaxKind::ROOT.into());

        let mut pos = 0;
        while pos < self.text.len() {
            pos = self.line(pos);
        }

        self.close_record();
        self.builder.finish_node();

        let green = self.builder.finish();
        ParseOutput {
            cst: SyntaxNode::new_root(green),
            diagnostics: self.diagnostics,
        }
    }

    // --- node stack -------------------------------------------------------

    fn close_field(&mut self) {
        if self.field_open {
            self.builder.finish_node();
            self.field_open = false;
        }
    }

    fn close_record(&mut self) {
        self.close_field();
        if self.record_open {
            self.builder.finish_node();
            self.record_open = false;
        }
    }

    fn open_record(&mut self) {
        if !self.record_open {
            self.builder.start_node(SyntaxKind::RECORD.into());
            self.record_open = true;
        }
    }

    fn token(&mut self, kind: SyntaxKind, start: usize, end: usize) {
        // Empty tokens are a rowan footgun and would break the "every leaf is
        // non-empty" invariant the tiling test asserts.
        if start < end {
            self.builder.token(kind.into(), &self.text[start..end]);
        }
    }

    // --- lines ------------------------------------------------------------

    /// Consume the physical line starting at `pos`; return the next position.
    fn line(&mut self, pos: usize) -> usize {
        let (content_end, line_end) = self.line_bounds(pos);
        let content = &self.text[pos..content_end];

        if content.trim().is_empty() {
            // Empty *or* whitespace-only: a record separator, faithful to
            // `read.dcf`, which starts a new record on either.
            self.close_record();
            self.builder.start_node(SyntaxKind::BLANK_LINE.into());
            self.token(SyntaxKind::WHITESPACE, pos, content_end);
            self.token(SyntaxKind::NEWLINE, content_end, line_end);
            self.builder.finish_node();
        } else if content.starts_with('#') {
            // A comment at column zero attaches to whatever is open and closes
            // nothing: `read.dcf` skips it and the enclosing field's
            // continuation *resumes* on the next line. It never opens a record
            // either, so a comment cannot bridge two records.
            self.builder.start_node(SyntaxKind::COMMENT_LINE.into());
            self.token(SyntaxKind::COMMENT, pos, content_end);
            self.token(SyntaxKind::NEWLINE, content_end, line_end);
            self.builder.finish_node();
        } else if content.starts_with([' ', '\t']) {
            if self.field_open {
                self.value_line(pos, content_end, line_end);
            } else {
                push_diagnostic(&mut self.diagnostics, ORPHAN_CONTINUATION, pos, content_end);
                self.open_record();
                self.malformed_line(pos, content_end, line_end);
            }
        } else {
            match content.find(':') {
                Some(offset) => self.field(pos, pos + offset, content_end, line_end),
                None => {
                    push_diagnostic(&mut self.diagnostics, MALFORMED_LINE, pos, content_end);
                    // A column-zero line ends any open field, malformed or not.
                    self.close_field();
                    self.open_record();
                    self.malformed_line(pos, content_end, line_end);
                }
            }
        }

        line_end
    }

    /// `(content_end, line_end)` for the line starting at `pos`: the byte
    /// offset where the line's content stops and where the line (newline
    /// included) stops. Equal when the final line has no trailing newline.
    fn line_bounds(&self, pos: usize) -> (usize, usize) {
        match self.text[pos..].find('\n') {
            Some(offset) => {
                let line_end = pos + offset + 1;
                let mut content_end = line_end - 1;
                if content_end > pos && self.text.as_bytes()[content_end - 1] == b'\r' {
                    content_end -= 1;
                }
                (content_end, line_end)
            }
            None => (self.text.len(), self.text.len()),
        }
    }

    fn malformed_line(&mut self, start: usize, content_end: usize, line_end: usize) {
        self.builder.start_node(SyntaxKind::MALFORMED_LINE.into());
        self.token(SyntaxKind::TEXT, start, content_end);
        self.token(SyntaxKind::NEWLINE, content_end, line_end);
        self.builder.finish_node();
    }

    /// A field's first line: `Name`, optional whitespace, `:`, then the
    /// remainder as this field's first [`VALUE_LINE`](SyntaxKind::VALUE_LINE).
    fn field(&mut self, start: usize, colon: usize, content_end: usize, line_end: usize) {
        let name = &self.text[start..colon];
        let name_end = start + name.trim_end().len();
        if name_end == start {
            push_diagnostic(&mut self.diagnostics, EMPTY_FIELD_NAME, colon, colon + 1);
        }

        self.close_field();
        self.open_record();
        self.builder.start_node(SyntaxKind::FIELD.into());
        self.field_open = true;

        self.token(SyntaxKind::FIELD_NAME, start, name_end);
        // Whitespace between the name and the colon (`Package : p`) must be its
        // own token: a `FIELD_NAME` that swallowed it would still satisfy
        // `starts_with("Collate")` while silently breaking `== "Package"`.
        self.token(SyntaxKind::WHITESPACE, name_end, colon);
        self.token(SyntaxKind::COLON, colon, colon + 1);

        self.value_line(colon + 1, content_end, line_end);
    }

    /// One value line, spanning `start..line_end`. Emits nothing at all when
    /// the span is empty — that is `Package:` at EOF, a field with no value.
    fn value_line(&mut self, start: usize, content_end: usize, line_end: usize) {
        if start == line_end {
            return;
        }
        let slice = &self.text[start..content_end];
        let trimmed = slice.trim();
        // An all-whitespace value is one leading-whitespace run, not a leading
        // and a trailing run — computing the two independently would overlap
        // them and emit those bytes twice.
        let (lead, trail) = if trimmed.is_empty() {
            (content_end, content_end)
        } else {
            let lead = start + (slice.len() - slice.trim_start().len());
            (lead, lead + trimmed.len())
        };

        self.builder.start_node(SyntaxKind::VALUE_LINE.into());
        self.token(SyntaxKind::WHITESPACE, start, lead);
        self.token(SyntaxKind::VALUE_TEXT, lead, trail);
        self.token(SyntaxKind::WHITESPACE, trail, content_end);
        self.token(SyntaxKind::NEWLINE, content_end, line_end);
        self.builder.finish_node();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dcf::syntax::SyntaxKind;

    fn doc(text: &str) -> ParseOutput {
        parse(text)
    }

    fn folded(text: &str, name: &str) -> Option<String> {
        doc(text).document().field(name).map(|f| f.folded_value())
    }

    /// Every structural guarantee the tree owes its consumers, checked at once:
    /// no empty leaves, the leaves tile the source exactly, every newline lands
    /// in a line node, and no line node holds more than one.
    fn assert_tiles(text: &str) {
        let output = parse(text);
        let mut rebuilt = String::new();
        let mut newlines = 0;

        for token in output
            .cst
            .descendants_with_tokens()
            .filter_map(|el| el.into_token())
        {
            assert!(
                !token.text().is_empty(),
                "empty {:?} token in {text:?}",
                token.kind()
            );
            rebuilt.push_str(token.text());

            if token.kind() == SyntaxKind::NEWLINE {
                newlines += 1;
                let parent = token.parent().expect("a token always has a parent");
                assert!(
                    parent.kind().is_line(),
                    "NEWLINE parented by {:?}, not a line node, in {text:?}",
                    parent.kind()
                );
            }
        }

        assert_eq!(rebuilt, text, "leaves do not tile the source");
        assert_eq!(
            newlines,
            text.matches('\n').count(),
            "newline count mismatch in {text:?}"
        );

        for node in output.cst.descendants().filter(|n| n.kind().is_line()) {
            let count = node
                .children_with_tokens()
                .filter_map(|el| el.into_token())
                .filter(|t| t.kind() == SyntaxKind::NEWLINE)
                .count();
            assert!(
                count <= 1,
                "{:?} holds {count} newlines in {text:?}",
                node.kind()
            );
        }
    }

    /// Adversarial inputs, every one of which must round-trip and tile. This is
    /// the crate's losslessness bar (Tenet 4), stated as a table rather than a
    /// property crate so the parser crate stays dependency-thin.
    const ADVERSARIAL: &[&str] = &[
        "",
        "\n",
        "\r\n",
        "\n\n\n",
        "Package: p",
        "Package: p\n",
        "Package:\n",
        "Package:",
        "Package: ",
        ":",
        ": v\n",
        "   \n",
        "\t\n",
        "   ",
        "Package: p\r\nVersion: 1\r\n",
        "Package: p\r\n  cont\r\n",
        "a\rb\n",
        "\u{FEFF}Package: p\n",
        "# c",
        "# c\n",
        "#\n",
        "   # c\n",
        "Collate:\n a.R\n# c\n b.R\n",
        "garbage\n",
        "garbage",
        "  orphan\n",
        "Package: p\n\n  orphan\n",
        "Package: p\n\nVersion: 1\n",
        "Date/Publication: 2025-09-12 07:20:14 UTC\n",
        "Authors@R: c(person(\"A\", \"B\", role = c(\"aut\", \"cre\")))\n",
        "Collate:\n    'a.R'\n    'b.R'\n",
        "Package: p\n   \nVersion: 1\n",
        "Package : p\n",
        "Built: R 4.5.3; ; 2025-01-01 00:00:00 UTC; unix\n",
    ];

    #[test]
    fn round_trips_every_adversarial_input() {
        for input in ADVERSARIAL {
            assert_eq!(
                &reconstruct(input),
                input,
                "round-trip failed for {input:?}"
            );
            assert_tiles(input);
        }
    }

    #[test]
    fn folds_continuation_lines() {
        assert_eq!(folded("Package: p\n", "Package").as_deref(), Some("p"));
        assert_eq!(
            folded("Depends:\n    a,\n    b\n", "Depends").as_deref(),
            Some("a,\nb")
        );
    }

    /// A field whose own line is empty folds without a leading `\n`, matching
    /// R's `read.dcf`.
    #[test]
    fn folded_value_drops_empty_first_line() {
        let text = "Package: testpkg\nCollate:\n    a.R\n    b.R\nVersion: 1.0\n";
        assert_eq!(folded(text, "Collate").as_deref(), Some("a.R\nb.R"));
        assert_eq!(folded(text, "Package").as_deref(), Some("testpkg"));
        assert_eq!(folded(text, "Version").as_deref(), Some("1.0"));
    }

    /// The `Roxygen` field is fed straight to the *R* parser downstream, so the
    /// folded text has to stay valid R. This crate owns both grammars, so it
    /// can assert the cross-grammar contract itself.
    #[test]
    fn folded_roxygen_field_parses_as_r() {
        let text = "Package: p\nRoxygen: list(load = \"installed\",\n    markdown = TRUE)\n";
        let value = folded(text, "Roxygen").expect("Roxygen field");
        assert_eq!(value, "list(load = \"installed\",\nmarkdown = TRUE)");
        let r = crate::parser::parse(&value);
        assert!(r.diagnostics.is_empty(), "folded value is not valid R");
    }

    /// The migration's load-bearing case: a blank line splits records, but the
    /// `DESCRIPTION` readers are record-blind and must still see every field.
    #[test]
    fn blank_line_starts_a_new_record_without_hiding_fields() {
        let output = doc("Package: p\n\nVersion: 1\n");
        assert_eq!(output.document().records().count(), 2);
        assert_eq!(
            folded("Package: p\n\nVersion: 1\n", "Version").as_deref(),
            Some("1")
        );
    }

    /// A whitespace-only line is a separator, not a continuation. Before this
    /// parser it appended a bare `"\n"` to the preceding value — and that value
    /// is spliced into a filesystem path by the harvester.
    #[test]
    fn whitespace_only_line_is_blank_not_a_continuation() {
        let text = "Package: p\n   \nVersion: 1\n";
        assert_eq!(folded(text, "Package").as_deref(), Some("p"));
        assert_eq!(doc(text).document().records().count(), 2);
    }

    #[test]
    fn comment_at_column_zero_is_skipped() {
        let text = "Package: p\n# a comment\nVersion: 1\n";
        assert_eq!(doc(text).document().fields().count(), 2);
        assert_eq!(folded(text, "Version").as_deref(), Some("1"));
    }

    /// A `#` at column zero inside a continuation block does not end the field:
    /// `read.dcf` skips the line and the continuation resumes.
    #[test]
    fn comment_between_continuations_keeps_the_field() {
        let text = "Collate:\n a.R\n# skip me\n b.R\n";
        assert_eq!(folded(text, "Collate").as_deref(), Some("a.R\nb.R"));
        assert_eq!(doc(text).document().fields().count(), 1);
    }

    #[test]
    fn indented_hash_is_a_continuation() {
        let text = "Description: one\n  # two\n";
        assert_eq!(folded(text, "Description").as_deref(), Some("one\n# two"));
    }

    /// A commented-out field is a comment, not a field named `# Depends`.
    #[test]
    fn commented_field_is_not_a_field() {
        let text = "Package: p\n# Depends: R (>= 4.0)\n";
        assert!(folded(text, "Depends").is_none());
        assert!(folded(text, "# Depends").is_none());
    }

    #[test]
    fn malformed_line_is_diagnosed_and_preserved() {
        let output = doc("Package: p\ngarbage\nVersion: 1\n");
        assert_eq!(output.diagnostics.len(), 1);
        assert_eq!(output.diagnostics[0].message, MALFORMED_LINE);
        assert_eq!(
            &"Package: p\ngarbage\nVersion: 1\n"
                [output.diagnostics[0].start..output.diagnostics[0].end],
            "garbage"
        );
        // The surrounding fields are unaffected.
        assert_eq!(output.document().fields().count(), 2);
    }

    #[test]
    fn orphan_continuation_is_diagnosed() {
        let output = doc("Package: p\n\n  orphan\n");
        assert_eq!(output.diagnostics.len(), 1);
        assert_eq!(output.diagnostics[0].message, ORPHAN_CONTINUATION);
        // It does not silently join the previous record's last field.
        assert_eq!(
            folded("Package: p\n\n  orphan\n", "Package").as_deref(),
            Some("p")
        );
    }

    #[test]
    fn empty_field_name_is_diagnosed() {
        let output = doc(": value\n");
        assert_eq!(output.diagnostics.len(), 1);
        assert_eq!(output.diagnostics[0].message, EMPTY_FIELD_NAME);
        let field = output.document().fields().next().expect("a field");
        assert_eq!(field.name(), "");
        assert_eq!(field.folded_value(), "value");
    }

    /// Like `read.dcf`, document-wide lookup resolves a duplicate to its last
    /// occurrence, including when a blank line put it in a later record.
    #[test]
    fn last_duplicate_wins() {
        assert_eq!(
            folded("Package: first\n\nPackage: second\n", "Package").as_deref(),
            Some("second")
        );
    }

    /// Record-local lookup follows the same last-wins rule.
    #[test]
    fn last_duplicate_in_record_wins() {
        let output = doc("Package: first\nPackage: second\n");
        let record = output.document().first_record().expect("a record");
        assert_eq!(
            record.field("Package").map(|field| field.folded_value()),
            Some("second".to_string())
        );
    }

    /// arity trims the name. R's `read.dcf` does **not**: `Package : p`
    /// declares a field literally named `"Package "`, so R sees no `Package`
    /// at all. Trimming is the lenient direction, and the whitespace survives
    /// as its own token, so a lint can still flag the typo precisely.
    #[test]
    fn field_name_excludes_trailing_space() {
        let text = "Package : p\n";
        let field = doc(text).document().fields().next().expect("a field");
        assert_eq!(field.name(), "Package");
        assert_eq!(field.folded_value(), "p");
    }

    /// `folded_value` would *hide* a stray `\r` (`str::trim` eats it), so CRLF
    /// correctness has to be asserted on the tree.
    #[test]
    fn crlf_newline_is_one_token() {
        let output = doc("Package: p\r\n  cont\r\n");
        let newlines: Vec<String> = output
            .cst
            .descendants_with_tokens()
            .filter_map(|el| el.into_token())
            .filter(|t| t.kind() == SyntaxKind::NEWLINE)
            .map(|t| t.text().to_string())
            .collect();
        assert_eq!(newlines, vec!["\r\n", "\r\n"]);

        for token in output
            .cst
            .descendants_with_tokens()
            .filter_map(|el| el.into_token())
            .filter(|t| t.kind() == SyntaxKind::VALUE_TEXT)
        {
            assert!(!token.text().contains('\r'), "\\r leaked into VALUE_TEXT");
        }
    }

    #[test]
    fn value_keeps_colons_after_the_first() {
        assert_eq!(
            folded("Built: R 4.5.3; ; 2025-01-01 00:00:00 UTC; unix\n", "Built").as_deref(),
            Some("R 4.5.3; ; 2025-01-01 00:00:00 UTC; unix")
        );
        assert_eq!(
            folded(
                "Date/Publication: 2025-09-12 07:20:14 UTC\n",
                "Date/Publication"
            )
            .as_deref(),
            Some("2025-09-12 07:20:14 UTC")
        );
    }

    /// A field with no value at all emits no value line, and folds to `""`.
    #[test]
    fn field_without_a_value() {
        let field = doc("Package:").document().fields().next().expect("a field");
        assert_eq!(field.value_lines().count(), 0);
        assert_eq!(field.folded_value(), "");
    }

    #[test]
    fn raw_value_text_keeps_every_byte_after_the_colon() {
        let field = doc("Collate:\n    a.R\n    b.R\n")
            .document()
            .fields()
            .next()
            .expect("a field");
        assert_eq!(field.raw_value_text(), "\n    a.R\n    b.R\n");
    }

    #[test]
    fn clean_input_reports_no_diagnostics() {
        for input in [
            "Package: p\n",
            "Package: p\n\nVersion: 1\n",
            "Collate:\n a.R\n# c\n b.R\n",
            "   \n",
            "# only a comment\n",
        ] {
            assert!(
                doc(input).diagnostics.is_empty(),
                "unexpected diagnostics for {input:?}: {:?}",
                doc(input).diagnostics
            );
        }
    }
}
