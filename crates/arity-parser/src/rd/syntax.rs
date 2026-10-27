//! Rd-specific rowan kinds. These are independent of the R and DCF kinds.

macro_rules! syntax_kinds {
    ($($kind:ident),+ $(,)?) => {
        /// A node or token in the lossless Rd tree.
        #[allow(non_camel_case_types)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
                 serde::Serialize, serde::Deserialize)]
        #[repr(u16)]
        pub enum SyntaxKind { $($kind),+ }

        impl SyntaxKind {
            /// Every kind in discriminant order.
            pub const ALL: &'static [Self] = &[$(Self::$kind),+];
        }
    };
}

syntax_kinds! {
    ROOT, MACRO, ARGUMENT, GROUP, OPTION, CONDITIONAL,
    MACRO_NAME, LBRACE, RBRACE, LBRACKET, RBRACKET,
    TEXT, RCODE, VERBATIM, ESCAPED_TEXT, ESCAPED_RCODE, ESCAPED_VERBATIM, R_STRING_ESCAPE,
    COMMENT, DIRECTIVE, TARGET, ERROR,
}

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        Self(kind as u16)
    }
}

/// The Rd grammar's rowan language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RdLanguage {}

impl rowan::Language for RdLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        SyntaxKind::ALL
            .get(usize::from(raw.0))
            .copied()
            .unwrap_or(SyntaxKind::ERROR)
    }

    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        kind.into()
    }
}

pub type SyntaxNode = rowan::SyntaxNode<RdLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<RdLanguage>;
pub type SyntaxElement = rowan::SyntaxElement<RdLanguage>;

#[cfg(test)]
mod tests {
    use super::*;
    use rowan::Language;

    #[test]
    fn kinds_round_trip() {
        for &kind in SyntaxKind::ALL {
            assert_eq!(
                RdLanguage::kind_from_raw(RdLanguage::kind_to_raw(kind)),
                kind
            );
        }
        assert_eq!(
            RdLanguage::kind_from_raw(rowan::SyntaxKind(u16::MAX)),
            SyntaxKind::ERROR
        );
    }
}
