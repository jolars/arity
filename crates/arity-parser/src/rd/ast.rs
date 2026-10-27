//! Read-only navigation over the source CST, without semantic interpretation.

use rowan::ast::AstNode;
use smol_str::SmolStr;

use super::syntax::{RdLanguage, SyntaxKind, SyntaxNode, SyntaxToken};

macro_rules! wrapper {
    ($name:ident, $kind:ident) => {
        #[doc = concat!("A typed view of an Rd `", stringify!($kind), "` node.")]
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(SyntaxNode);

        impl AstNode for $name {
            type Language = RdLanguage;
            fn can_cast(kind: SyntaxKind) -> bool {
                kind == SyntaxKind::$kind
            }
            fn cast(node: SyntaxNode) -> std::option::Option<Self> {
                Self::can_cast(node.kind()).then_some(Self(node))
            }
            fn syntax(&self) -> &SyntaxNode {
                &self.0
            }
        }
    };
}

wrapper!(Document, ROOT);
wrapper!(Macro, MACRO);
wrapper!(Argument, ARGUMENT);
wrapper!(Group, GROUP);
wrapper!(Option, OPTION);
wrapper!(Conditional, CONDITIONAL);

fn token(node: &SyntaxNode, kind: SyntaxKind) -> std::option::Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind() == kind)
}

impl Document {
    /// Top-level macros in source order, without flattening conditional bodies.
    pub fn macros(&self) -> impl Iterator<Item = Macro> + use<> {
        self.0.children().filter_map(Macro::cast)
    }
}

impl Macro {
    /// The original macro name, including its backslash.
    pub fn name_token(&self) -> std::option::Option<SyntaxToken> {
        token(&self.0, SyntaxKind::MACRO_NAME)
    }

    /// The macro name without its leading backslash.
    pub fn name(&self) -> SmolStr {
        self.name_token()
            .map(|token| SmolStr::new(token.text().trim_start_matches('\\')))
            .unwrap_or_default()
    }

    /// Supplied arguments, including recovered incomplete arguments.
    pub fn arguments(&self) -> impl Iterator<Item = Argument> + use<> {
        self.0.children().filter_map(Argument::cast)
    }

    /// The supplied bracketed option, if any.
    pub fn option(&self) -> std::option::Option<Option> {
        self.0.children().find_map(Option::cast)
    }
}

impl Option {
    /// Original option contents, excluding its opening and closing brackets.
    pub fn text(&self) -> String {
        self.0
            .children_with_tokens()
            .filter(|element| {
                !matches!(element.kind(), SyntaxKind::LBRACKET | SyntaxKind::RBRACKET)
            })
            .map(|element| element.to_string())
            .collect()
    }
}

impl Conditional {
    /// The opening `#ifdef` or `#ifndef` token.
    pub fn directive(&self) -> std::option::Option<SyntaxToken> {
        token(&self.0, SyntaxKind::DIRECTIVE)
    }

    /// The source target, including its surrounding whitespace and line ending.
    pub fn target(&self) -> std::option::Option<SyntaxToken> {
        token(&self.0, SyntaxKind::TARGET)
    }
}
