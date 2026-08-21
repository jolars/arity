//! Zero-cost typed wrappers over the DCF CST, in the same mould as
//! [`crate::ast`].
//!
//! These are a read-only **navigation** view: they add no structure the CST
//! does not already carry, and casting one costs nothing.
//!
//! **Never store a wrapper in salsa.** Every type here holds a red
//! [`SyntaxNode`], which is neither `Send` nor `Eq`; store the green node and
//! re-derive.

use rowan::TextRange;
use rowan::ast::AstNode;
use smol_str::SmolStr;

use crate::dcf::syntax::{DcfLanguage, SyntaxKind, SyntaxNode, SyntaxToken};

macro_rules! ast_node {
    ($name:ident, $kind:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(SyntaxNode);

        impl AstNode for $name {
            type Language = DcfLanguage;

            fn can_cast(kind: SyntaxKind) -> bool {
                kind == $kind
            }

            fn cast(syntax: SyntaxNode) -> Option<Self> {
                Self::can_cast(syntax.kind()).then(|| Self(syntax))
            }

            fn syntax(&self) -> &SyntaxNode {
                &self.0
            }
        }
    };
}

ast_node!(Document, SyntaxKind::ROOT);
ast_node!(Record, SyntaxKind::RECORD);
ast_node!(Field, SyntaxKind::FIELD);
ast_node!(ValueLine, SyntaxKind::VALUE_LINE);
ast_node!(CommentLine, SyntaxKind::COMMENT_LINE);
ast_node!(MalformedLine, SyntaxKind::MALFORMED_LINE);

/// The first direct-child token of `node` with the given kind.
fn token(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|el| el.into_token())
        .find(|tok| tok.kind() == kind)
}

impl Document {
    /// The document's records, in order. A `DESCRIPTION` has exactly one.
    pub fn records(&self) -> impl Iterator<Item = Record> + use<> {
        self.0.children().filter_map(Record::cast)
    }

    /// The first record, if any.
    pub fn first_record(&self) -> Option<Record> {
        self.records().next()
    }

    /// Every field of every record, in document order.
    ///
    /// This is the record-**blind** view, and it is deliberately what the
    /// `DESCRIPTION` readers use: a stray blank line splits the file into two
    /// records, and must not hide the fields after it from a caller that only
    /// wants "the `Version` of this package".
    pub fn fields(&self) -> impl Iterator<Item = Field> + use<> {
        self.records().flat_map(|record| record.fields())
    }

    /// The last field named `name`, across all records.
    ///
    /// Last occurrence wins, matching [`read.dcf`](https://stat.ethz.ch/R-manual/R-devel/library/base/html/read.dcf.html).
    pub fn field(&self, name: &str) -> Option<Field> {
        self.fields().filter(|field| field.name() == name).last()
    }
}

impl Record {
    /// This record's fields, in order.
    pub fn fields(&self) -> impl Iterator<Item = Field> + use<> {
        self.0.children().filter_map(Field::cast)
    }

    /// The last field of this record named `name`.
    pub fn field(&self, name: &str) -> Option<Field> {
        self.fields().filter(|field| field.name() == name).last()
    }
}

impl Field {
    /// The field's name, excluding any whitespace before the colon. Empty when
    /// the source had no name at all (`: value`), which the parser also
    /// diagnoses.
    pub fn name(&self) -> SmolStr {
        self.name_token()
            .map(|tok| SmolStr::new(tok.text()))
            .unwrap_or_default()
    }

    /// The `FIELD_NAME` token, absent for a nameless `: value`.
    pub fn name_token(&self) -> Option<SyntaxToken> {
        token(&self.0, SyntaxKind::FIELD_NAME)
    }

    /// The name's source range, collapsed to the colon's start when there is no
    /// name token — so a diagnostic always has somewhere to point.
    pub fn name_range(&self) -> TextRange {
        match self.name_token() {
            Some(tok) => tok.text_range(),
            None => {
                let start = self.0.text_range().start();
                TextRange::empty(start)
            }
        }
    }

    /// The range spanning every value line, or an empty range at the field's
    /// end when the field has no value at all (`Package:` at EOF).
    pub fn value_range(&self) -> TextRange {
        let mut lines = self.value_lines();
        let Some(first) = lines.next() else {
            return TextRange::empty(self.0.text_range().end());
        };
        let last = lines.last().unwrap_or_else(|| first.clone());
        TextRange::new(
            first.syntax().text_range().start(),
            last.syntax().text_range().end(),
        )
    }

    /// The field's value lines, starting with the field's own line (everything
    /// after the colon) and continuing through its continuation lines.
    pub fn value_lines(&self) -> impl Iterator<Item = ValueLine> + use<> {
        self.0.children().filter_map(ValueLine::cast)
    }

    /// Comment lines interleaved into this field's continuation block. R's
    /// `read.dcf` skips them and resumes the field.
    pub fn comment_lines(&self) -> impl Iterator<Item = CommentLine> + use<> {
        self.0.children().filter_map(CommentLine::cast)
    }

    /// The field's **logical** value: each nonempty value line trimmed, joined
    /// with `\n`. This is what a caller wanting `Depends` or `Roxygen` reads.
    /// Empty value lines contribute no segment, matching R's `read.dcf`.
    pub fn folded_value(&self) -> String {
        self.value_lines()
            .map(|line| line.trimmed_text())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The exact source bytes of everything after the colon, newlines and
    /// interleaved comments included. For the formatter, which rewrites rather
    /// than reads.
    pub fn raw_value_text(&self) -> String {
        let mut out = String::new();
        let mut past_colon = false;
        for tok in self
            .0
            .descendants_with_tokens()
            .filter_map(|el| el.into_token())
        {
            if past_colon {
                out.push_str(tok.text());
            } else if tok.kind() == SyntaxKind::COLON {
                past_colon = true;
            }
        }
        out
    }
}

impl ValueLine {
    /// The line's leading whitespace — a continuation's indent, or the space
    /// after the colon on a field's own line. The formatter's re-indent handle.
    pub fn indent(&self) -> Option<SyntaxToken> {
        self.0
            .children_with_tokens()
            .filter_map(|el| el.into_token())
            .next()
            .filter(|tok| tok.kind() == SyntaxKind::WHITESPACE)
    }

    /// The line's content run, absent on a line that is only whitespace.
    pub fn content(&self) -> Option<SyntaxToken> {
        token(&self.0, SyntaxKind::VALUE_TEXT)
    }

    /// The line's content, trimmed — the unit [`Field::folded_value`] joins.
    pub fn trimmed_text(&self) -> String {
        self.content()
            .map(|tok| tok.text().trim().to_string())
            .unwrap_or_default()
    }

    /// The content run's source range, collapsed to the line's start when the
    /// line has no content.
    pub fn content_range(&self) -> TextRange {
        match self.content() {
            Some(tok) => tok.text_range(),
            None => TextRange::empty(self.0.text_range().start()),
        }
    }
}

impl CommentLine {
    /// The comment's text, excluding the newline.
    pub fn text(&self) -> Option<SyntaxToken> {
        token(&self.0, SyntaxKind::COMMENT)
    }
}

impl MalformedLine {
    /// The line's raw content, excluding the newline.
    pub fn text(&self) -> Option<SyntaxToken> {
        token(&self.0, SyntaxKind::TEXT)
    }
}
