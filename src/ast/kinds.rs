//! Shared [`SyntaxKind`] predicates for the AST-wrapper layer and its
//! consumers. Kept in one place so node/token accessors and the linter's
//! matchers agree on what counts as a binary operator, a unary operator, or
//! trivia.

use crate::syntax::SyntaxKind;

/// Whitespace trivia. Does **not** include comments — callers that also want to
/// skip comments test `is_trivia(k) || k == SyntaxKind::COMMENT`, the idiom used
/// throughout the wrapper accessors.
pub fn is_trivia(kind: SyntaxKind) -> bool {
    matches!(kind, SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE)
}

/// A binary-operator token that can head a `BINARY_EXPR` (including the
/// namespace operators `::`/`:::` and the access operators `$`/`@`).
pub fn is_binary_operator(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::PLUS
            | SyntaxKind::MINUS
            | SyntaxKind::STAR
            | SyntaxKind::SLASH
            | SyntaxKind::CARET
            | SyntaxKind::PIPE
            | SyntaxKind::COLON
            | SyntaxKind::COLON2
            | SyntaxKind::COLON3
            | SyntaxKind::DOLLAR
            | SyntaxKind::AT
            | SyntaxKind::OR
            | SyntaxKind::OR2
            | SyntaxKind::AND
            | SyntaxKind::AND2
            | SyntaxKind::EQUAL2
            | SyntaxKind::NOT_EQUAL
            | SyntaxKind::LESS_THAN
            | SyntaxKind::LESS_THAN_OR_EQUAL
            | SyntaxKind::GREATER_THAN
            | SyntaxKind::GREATER_THAN_OR_EQUAL
            | SyntaxKind::USER_OP
            | SyntaxKind::TILDE
            | SyntaxKind::QUESTION
    )
}

/// A prefix unary-operator token that can head a `UNARY_EXPR`
/// (`-`, `+`, `!`, `~`, `?`).
pub fn is_unary_operator(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::MINUS
            | SyntaxKind::PLUS
            | SyntaxKind::BANG
            | SyntaxKind::TILDE
            | SyntaxKind::QUESTION
    )
}
