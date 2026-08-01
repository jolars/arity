//! Typed AST wrappers over CST *tokens* (leaves), mirroring rowan's `AstNode`
//! for nodes. rowan 0.16 ships only `AstNode`; arity defines its own
//! [`AstToken`] (as rust-analyzer does).
//!
//! R's atomic operands are **bare tokens**, not `LITERAL` nodes: `1 + 2` is
//! `BINARY_EXPR { INT, PLUS, INT }`, and every special constant (`TRUE`, `NA`,
//! `NULL`, …) is an `IDENT` classified by text. These wrappers type those
//! leaves so consumers can navigate them without re-inspecting [`SyntaxKind`].

use rowan::TextRange;
use smol_str::SmolStr;

use crate::syntax::{SyntaxKind, SyntaxToken};

/// A typed, zero-cost wrapper over a single CST token of a fixed
/// [`SyntaxKind`]. The token analogue of rowan's `AstNode`.
pub trait AstToken {
    fn can_cast(kind: SyntaxKind) -> bool
    where
        Self: Sized;

    fn cast(syntax: SyntaxToken) -> Option<Self>
    where
        Self: Sized;

    fn syntax(&self) -> &SyntaxToken;

    fn text(&self) -> &str {
        self.syntax().text()
    }

    fn text_range(&self) -> TextRange {
        self.syntax().text_range()
    }
}

macro_rules! ast_token {
    ($name:ident, $kind:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(SyntaxToken);

        impl AstToken for $name {
            fn can_cast(kind: SyntaxKind) -> bool {
                kind == $kind
            }

            fn cast(syntax: SyntaxToken) -> Option<Self> {
                Self::can_cast(syntax.kind()).then(|| Self(syntax))
            }

            fn syntax(&self) -> &SyntaxToken {
                &self.0
            }
        }
    };
}

ast_token!(Ident, SyntaxKind::IDENT);
ast_token!(IntLit, SyntaxKind::INT);
ast_token!(FloatLit, SyntaxKind::FLOAT);
ast_token!(ComplexLit, SyntaxKind::COMPLEX);
ast_token!(StringLit, SyntaxKind::STRING);
ast_token!(Comment, SyntaxKind::COMMENT);

/// One of R's special constant symbols. All are lexed as `IDENT` and
/// distinguished by text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RConstant {
    True,
    False,
    Na,
    NaInteger,
    NaReal,
    NaComplex,
    NaCharacter,
    Null,
    NaN,
    Inf,
    /// The rebindable boolean symbol `T`.
    TSymbol,
    /// The rebindable boolean symbol `F`.
    FSymbol,
}

impl Ident {
    /// The identifier text.
    pub fn name(&self) -> &str {
        self.text()
    }

    /// Classify the identifier as one of R's special constant symbols, if it is
    /// one. `T`/`F` classify as [`RConstant::TSymbol`]/[`RConstant::FSymbol`]
    /// even though they are rebindable base bindings — callers that must exclude
    /// them use [`Ident::is_reserved_constant`].
    pub fn constant(&self) -> Option<RConstant> {
        Some(match self.text() {
            "TRUE" => RConstant::True,
            "FALSE" => RConstant::False,
            "NA" => RConstant::Na,
            "NA_integer_" => RConstant::NaInteger,
            "NA_real_" => RConstant::NaReal,
            "NA_complex_" => RConstant::NaComplex,
            "NA_character_" => RConstant::NaCharacter,
            "NULL" => RConstant::Null,
            "NaN" => RConstant::NaN,
            "Inf" => RConstant::Inf,
            "T" => RConstant::TSymbol,
            "F" => RConstant::FSymbol,
            _ => return None,
        })
    }

    /// `TRUE`.
    pub fn is_true(&self) -> bool {
        self.text() == "TRUE"
    }

    /// `FALSE`.
    pub fn is_false(&self) -> bool {
        self.text() == "FALSE"
    }

    /// `NA` or one of its typed variants (`NA_integer_`, …).
    pub fn is_na(&self) -> bool {
        matches!(
            self.text(),
            "NA" | "NA_integer_" | "NA_real_" | "NA_complex_" | "NA_character_"
        )
    }

    /// `NULL`.
    pub fn is_null(&self) -> bool {
        self.text() == "NULL"
    }

    /// `NaN`.
    pub fn is_nan(&self) -> bool {
        self.text() == "NaN"
    }

    /// The rebindable boolean symbols `T` / `F`.
    pub fn is_bool_symbol(&self) -> bool {
        matches!(self.text(), "T" | "F")
    }

    /// An implicit method-dispatch variable (`.Generic`, `.Method`, `.Class`),
    /// bound automatically by R inside an S3/S4 (group-)generic method body. It
    /// is always defined there, so the semantic model treats it as never a
    /// resolvable read rather than an undefined symbol.
    pub fn is_implicit_method_var(&self) -> bool {
        matches!(self.text(), ".Generic" | ".Method" | ".Class")
    }

    /// A dot-dot placeholder (`...`, `..1`), lexed as `IDENT` but not
    /// scope-resolvable.
    pub fn is_dots(&self) -> bool {
        let name = self.text();
        name.starts_with('.') && name.chars().all(|c| c == '.' || c.is_ascii_digit())
    }

    /// A reserved constant that is a *value*, not a symbol reference. Mirrors
    /// [`crate::parser::expr::ident_is_special_constant`] (`NA*`, `NULL`,
    /// `TRUE`, `FALSE`, `Inf`, `NaN`) and excludes the rebindable `T`/`F`; this
    /// is the set the semantic model skips when recording identifier reads.
    pub fn is_reserved_constant(&self) -> bool {
        matches!(
            self.constant(),
            Some(
                RConstant::Na
                    | RConstant::NaInteger
                    | RConstant::NaReal
                    | RConstant::NaComplex
                    | RConstant::NaCharacter
                    | RConstant::Null
                    | RConstant::True
                    | RConstant::False
                    | RConstant::Inf
                    | RConstant::NaN
            )
        )
    }
}

impl StringLit {
    /// The opening quote character if this is a `"`- or `'`-quoted string
    /// literal (not a backtick-quoted name). `None` otherwise.
    pub fn quote(&self) -> Option<char> {
        let bytes = self.text().as_bytes();
        if bytes.len() < 2 {
            return None;
        }
        let q = bytes[0];
        (matches!(q, b'"' | b'\'') && bytes[bytes.len() - 1] == q).then_some(q as char)
    }

    /// Raw text between the quotes (escapes intact) for a `"`/`'`-quoted string;
    /// `None` for a backtick-quoted name. Subsumes the inner half of the old
    /// `matchers::string_literal`.
    pub fn inner(&self) -> Option<&str> {
        self.quote()?;
        let text = self.text();
        Some(&text[1..text.len() - 1])
    }

    /// Whether this is a backtick-quoted name (`` `x` ``).
    pub fn is_backtick(&self) -> bool {
        let bytes = self.text().as_bytes();
        bytes.len() >= 2 && bytes[0] == b'`' && bytes[bytes.len() - 1] == b'`'
    }

    /// The unquoted contents for any of the `"` `'` `` ` `` delimiters — the
    /// name a string denotes. `None` if the token is not delimiter-wrapped.
    pub fn unquote(&self) -> Option<&str> {
        let text = self.text();
        let bytes = text.as_bytes();
        if bytes.len() >= 2 {
            let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
            if matches!(first, b'"' | b'\'' | b'`') && first == last {
                return Some(&text[1..text.len() - 1]);
            }
        }
        None
    }
}

/// The textual name a token denotes: the raw text for an `IDENT` (or any other
/// token), or the unquoted contents for a backtick/quoted `STRING`.
pub fn token_name(token: &SyntaxToken) -> SmolStr {
    if token.kind() == SyntaxKind::STRING {
        let text = token.text();
        let bytes = text.as_bytes();
        if bytes.len() >= 2 {
            let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
            if matches!(first, b'"' | b'\'' | b'`') && first == last {
                return SmolStr::new(&text[1..text.len() - 1]);
            }
        }
    }
    SmolStr::new(token.text())
}
