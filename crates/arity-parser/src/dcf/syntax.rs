//! The DCF grammar's node kinds and its rowan language definition.
//!
//! This is a **second** [`rowan::Language`] alongside the R grammar's
//! [`RLanguage`](crate::syntax::RLanguage). The two `SyntaxKind` enums are
//! distinct types that happen to share a name: the module path is the
//! disambiguator, so never glob-import both.

use rowan::Language;

/// A DCF node or token kind.
///
/// Nodes come first, tokens second, and the discriminants are load-bearing —
/// [`DcfLanguage::kind_from_raw`] maps them back by number, and
/// [`SyntaxKind::ALL`] plus `syntax_kind_round_trips` guard that table.
#[allow(non_camel_case_types)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[repr(u16)]
pub enum SyntaxKind {
    // --- nodes ---
    /// The whole document.
    ROOT,
    /// One blank-line-delimited record. A `DESCRIPTION` has exactly one.
    RECORD,
    /// `Name: value` plus every continuation line that follows it.
    FIELD,
    /// One physical line of a field's value. The field's own line (everything
    /// after the colon) is a `VALUE_LINE` too, which is what makes folding
    /// uniform.
    VALUE_LINE,
    /// A `#`-at-column-zero line. Legal inside a [`FIELD`](Self::FIELD): R's
    /// `read.dcf` skips such a line and *resumes* the field's continuation.
    COMMENT_LINE,
    /// An empty or whitespace-only line — a record separator.
    BLANK_LINE,
    /// A line that is neither a field, a continuation, a comment, nor blank.
    /// `read.dcf` errors here; arity keeps the bytes and reports a diagnostic.
    MALFORMED_LINE,

    // --- tokens ---
    /// A field's name, excluding any whitespace before the colon.
    FIELD_NAME,
    /// The `:` separating a field's name from its value.
    COLON,
    /// A value line's content run, from its first to its last non-space byte.
    VALUE_TEXT,
    /// A comment line's `#...`, up to but excluding the newline.
    COMMENT,
    /// A malformed line's raw content.
    TEXT,
    /// Indentation, the space(s) after a colon, and trailing spaces.
    WHITESPACE,
    /// `\n` or `\r\n` — always one token, never split.
    NEWLINE,
    /// The `_` arm of [`DcfLanguage::kind_from_raw`]. Never emitted by the
    /// parser.
    ERROR,
}

impl SyntaxKind {
    /// Every kind, in discriminant order.
    pub const ALL: [SyntaxKind; Self::COUNT] = [
        SyntaxKind::ROOT,
        SyntaxKind::RECORD,
        SyntaxKind::FIELD,
        SyntaxKind::VALUE_LINE,
        SyntaxKind::COMMENT_LINE,
        SyntaxKind::BLANK_LINE,
        SyntaxKind::MALFORMED_LINE,
        SyntaxKind::FIELD_NAME,
        SyntaxKind::COLON,
        SyntaxKind::VALUE_TEXT,
        SyntaxKind::COMMENT,
        SyntaxKind::TEXT,
        SyntaxKind::WHITESPACE,
        SyntaxKind::NEWLINE,
        SyntaxKind::ERROR,
    ];

    /// How many kinds there are.
    pub const COUNT: usize = 15;

    /// Whether this kind is a node (rather than a token).
    pub fn is_node(self) -> bool {
        matches!(
            self,
            SyntaxKind::ROOT
                | SyntaxKind::RECORD
                | SyntaxKind::FIELD
                | SyntaxKind::VALUE_LINE
                | SyntaxKind::COMMENT_LINE
                | SyntaxKind::BLANK_LINE
                | SyntaxKind::MALFORMED_LINE
        )
    }

    /// Whether this kind is one of the four line nodes. Every physical line of
    /// the source belongs to exactly one of these, or is a
    /// [`FIELD`](Self::FIELD)'s own first line.
    pub fn is_line(self) -> bool {
        matches!(
            self,
            SyntaxKind::VALUE_LINE
                | SyntaxKind::COMMENT_LINE
                | SyntaxKind::BLANK_LINE
                | SyntaxKind::MALFORMED_LINE
        )
    }
}

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        Self(kind as u16)
    }
}

/// The DCF language, for rowan's generic tree types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DcfLanguage {}

impl Language for DcfLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        match raw.0 {
            0 => SyntaxKind::ROOT,
            1 => SyntaxKind::RECORD,
            2 => SyntaxKind::FIELD,
            3 => SyntaxKind::VALUE_LINE,
            4 => SyntaxKind::COMMENT_LINE,
            5 => SyntaxKind::BLANK_LINE,
            6 => SyntaxKind::MALFORMED_LINE,
            7 => SyntaxKind::FIELD_NAME,
            8 => SyntaxKind::COLON,
            9 => SyntaxKind::VALUE_TEXT,
            10 => SyntaxKind::COMMENT,
            11 => SyntaxKind::TEXT,
            12 => SyntaxKind::WHITESPACE,
            13 => SyntaxKind::NEWLINE,
            _ => SyntaxKind::ERROR,
        }
    }

    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        kind.into()
    }
}

pub type SyntaxNode = rowan::SyntaxNode<DcfLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<DcfLanguage>;
pub type SyntaxElement = rowan::SyntaxElement<DcfLanguage>;

#[cfg(test)]
mod tests {
    use super::*;

    /// The numeric `kind_from_raw` table is hand-written, so it is exactly the
    /// thing that rots silently when a kind is inserted mid-enum.
    #[test]
    fn syntax_kind_round_trips() {
        for kind in SyntaxKind::ALL {
            assert_eq!(
                DcfLanguage::kind_from_raw(kind.into()),
                kind,
                "{kind:?} does not round-trip through kind_from_raw"
            );
        }
    }

    #[test]
    fn all_is_in_discriminant_order() {
        for (i, kind) in SyntaxKind::ALL.iter().enumerate() {
            assert_eq!(rowan::SyntaxKind::from(*kind).0 as usize, i, "{kind:?}");
        }
    }
}
