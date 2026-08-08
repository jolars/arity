//! Judging a parse against R's own acceptance rules.
//!
//! arity's parser is deliberately more lenient than R's: it recovers from
//! errors, and its lexer accepts name shapes R rejects. Consumers that need to
//! answer "would R have parsed this?" — rather than "did arity produce a usable
//! tree?" — go through the predicates here.

use crate::parser::core::ParseOutput;
use crate::syntax::{SyntaxKind, SyntaxNode};

/// Whether the parsed text is exactly one complete R expression, the way R's
/// own parser judges it — the predicate behind `rlang::parse_expr()` (and so
/// roxygen2's `can_parse()`).
///
/// This takes the whole [`ParseOutput`] rather than just the CST because a
/// recovered tree alone cannot answer the question: `f(` yields a well-shaped
/// `CALL_EXPR` and reports the missing `)` only as a diagnostic.
///
/// Three things have to hold:
///
/// - the parse is diagnostic-free;
/// - the root holds one expression, optionally followed by a single `;`. R's
///   `;` is a statement *terminator*, not a separator that may stand alone: it
///   must follow an expression on the same line, and a second one opens an
///   empty statement, which is a syntax error (`x;` parses, `;x`, `x;;`,
///   `x\n;` and `x; y` do not);
/// - no name in the tree is one R's lexer rejects but arity's accepts (see
///   [`has_r_invalid_name`]).
pub fn is_single_expression(out: &ParseOutput) -> bool {
    out.diagnostics.is_empty()
        && root_holds_one_expression(&out.cst)
        && !has_r_invalid_name(&out.cst)
}

/// The positional `expr` / `expr ;` rule over the root's children.
fn root_holds_one_expression(cst: &SyntaxNode) -> bool {
    let mut expr = false;
    let mut semicolon = false;
    let mut line_broke = false;
    for el in cst.children_with_tokens() {
        match el.kind() {
            SyntaxKind::WHITESPACE | SyntaxKind::COMMENT => {}
            SyntaxKind::NEWLINE => line_broke = true,
            SyntaxKind::SEMICOLON => {
                if !expr || semicolon || line_broke {
                    return false;
                }
                semicolon = true;
            }
            _ => {
                if expr {
                    return false;
                }
                expr = true;
                line_broke = false;
            }
        }
    }
    expr
}

/// Whether the tree contains a name R's lexer rejects but arity's more lenient
/// one accepts as an ordinary identifier.
///
/// A `_`-leading name errors in R's lexer, with one exception: a lone `_` used
/// as the native-pipe placeholder, valid only inside a `|>` pipeline (a
/// `_`-leading name of length ≥ 2 is never valid). A zero-length backquoted
/// name `` `` `` errors at parse time ("attempt to use zero-length variable
/// name"); any non-empty backquoted name is valid.
pub fn has_r_invalid_name(cst: &SyntaxNode) -> bool {
    let has_pipe = cst
        .descendants_with_tokens()
        .any(|el| el.kind() == SyntaxKind::PIPE);
    cst.descendants_with_tokens()
        .filter_map(|el| el.into_token())
        .filter(|t| t.kind() == SyntaxKind::IDENT)
        .any(|t| {
            let text = t.text();
            text == "``" || (text.starts_with('_') && (text.len() > 1 || !has_pipe))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::core::parse;

    fn single(text: &str) -> bool {
        is_single_expression(&parse(text))
    }

    #[test]
    fn accepts_one_expression() {
        assert!(single("x"));
        assert!(single("f(a, b)"));
        assert!(single("function(x) x + 1"));
        assert!(single("  x  "));
        assert!(single("\nx\n"));
        assert!(single("x # trailing comment"));
    }

    #[test]
    fn rejects_zero_or_several_expressions() {
        assert!(!single(""));
        assert!(!single("   "));
        assert!(!single("# just a comment"));
        assert!(!single("inline code"));
        assert!(!single("x\ny"));
    }

    #[test]
    fn rejects_an_incomplete_expression() {
        assert!(!single("f("));
        assert!(!single("x +"));
    }

    #[test]
    fn semicolon_terminates_but_never_stands_alone() {
        // Cross-checked against `rlang::parse_expr()` under R 4.6.1.
        assert!(single("x;"));
        assert!(single("x ;"));
        assert!(single("f(a);"));
        assert!(single("x;\n"));
        assert!(single("x; # trailing comment"));
        assert!(!single(";"));
        assert!(!single(";x"));
        assert!(!single("x;;"));
        assert!(!single("x; ;"));
        assert!(!single("x; y"));
        assert!(!single("x;\ny"));
        // A `;` on its own line opens an empty statement, so it is an error
        // even though the expression before it is complete.
        assert!(!single("x\n;"));
    }

    #[test]
    fn rejects_names_r_lexes_as_errors() {
        assert!(!single("_"));
        assert!(!single("_x"));
        assert!(!single("``"));
        assert!(single("x |> _$col"));
        assert!(single("a_b"));
        assert!(single("`x`"));
    }
}
