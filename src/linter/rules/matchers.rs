//! Reusable CST/AST shape matchers for syntactic lint rules (Phase 0 §I1).
//!
//! These collapse the common "is this a call to `foo` / what is its nth
//! argument / is this operand a `TRUE` literal" patterns that otherwise get
//! rewritten ad hoc in every rule, reducing a typical syntactic rule to ~30
//! lines. They build on the typed AST wrappers (`CallExpr`, `ArgList`, `Arg`,
//! `BinaryExpr`) rather than re-walking raw CST wherever a wrapper exists.

use rowan::NodeOrToken;
use rowan::ast::AstNode as _;
use smol_str::SmolStr;

use crate::ast::CallExpr;
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

// --- calls & callees -------------------------------------------------------

/// The callee name of a call, when it is a simple name (`foo(…)`, `` `+`(…) ``).
/// `None` for a computed callee (`(g())(…)`, `x$f(…)`).
pub fn callee_name(call: &CallExpr) -> Option<SmolStr> {
    call.callee_token().map(|t| token_name(&t))
}

/// `node` cast to a [`CallExpr`] whose callee is exactly `name`.
pub fn call_named(node: &SyntaxNode, name: &str) -> Option<CallExpr> {
    let call = CallExpr::cast(node.clone())?;
    (callee_name(&call).as_deref() == Some(name)).then_some(call)
}

/// Whether the token covering `range` sits in the callee position of a call
/// (`name(…)`) — not a value read like `name[[i]]` or `name + 1`.
pub fn is_callee(root: &SyntaxNode, range: rowan::TextRange) -> bool {
    let NodeOrToken::Token(token) = root.covering_element(range) else {
        return false;
    };
    let Some(parent) = token.parent() else {
        return false;
    };
    parent.kind() == SyntaxKind::CALL_EXPR
        && CallExpr::cast(parent)
            .and_then(|call| call.callee_token())
            .is_some_and(|callee| callee.text_range() == range)
}

// --- arguments -------------------------------------------------------------

/// A single call argument, split into its optional name (for `name = value`)
/// and its value element.
pub struct ArgMatch {
    pub name: Option<SmolStr>,
    pub name_token: Option<SyntaxToken>,
    pub value: Option<SyntaxElement>,
}

/// The arguments of a call, each split into name and value.
pub fn args(call: &CallExpr) -> Vec<ArgMatch> {
    let Some(list) = call.arg_list() else {
        return Vec::new();
    };
    list.args().map(|arg| arg_parts(arg.syntax())).collect()
}

/// The value of the `n`th positional (unnamed) argument, 0-indexed.
pub fn nth_arg(call: &CallExpr, n: usize) -> Option<SyntaxElement> {
    args(call)
        .into_iter()
        .filter(|a| a.name.is_none())
        .nth(n)
        .and_then(|a| a.value)
}

/// The value of the argument named `name`, if present.
pub fn named_arg(call: &CallExpr, name: &str) -> Option<SyntaxElement> {
    args(call)
        .into_iter()
        .find(|a| a.name.as_deref() == Some(name))
        .and_then(|a| a.value)
}

fn arg_parts(arg: &SyntaxNode) -> ArgMatch {
    let elements: Vec<SyntaxElement> = arg.children_with_tokens().collect();
    match elements
        .iter()
        .position(|e| e.kind() == SyntaxKind::ASSIGN_EQ)
    {
        Some(eq) => {
            let name_token = elements[..eq].iter().rev().find_map(|e| match e {
                NodeOrToken::Token(t)
                    if matches!(t.kind(), SyntaxKind::IDENT | SyntaxKind::STRING) =>
                {
                    Some(t.clone())
                }
                _ => None,
            });
            let value = elements[eq + 1..]
                .iter()
                .find(|e| !is_trivia(e.kind()))
                .cloned();
            ArgMatch {
                name: name_token.as_ref().map(token_name),
                name_token,
                value,
            }
        }
        None => ArgMatch {
            name: None,
            name_token: None,
            value: elements.iter().find(|e| !is_trivia(e.kind())).cloned(),
        },
    }
}

// --- binary expressions ----------------------------------------------------

/// Split a `BINARY_EXPR` into `(lhs, operator, rhs)` at its top-level operator
/// token. Operands are elements: they may be tokens (`x`, `TRUE`) or nodes
/// (`a + b`).
pub fn binary_parts(expr: &SyntaxNode) -> Option<(SyntaxElement, SyntaxToken, SyntaxElement)> {
    if expr.kind() != SyntaxKind::BINARY_EXPR {
        return None;
    }
    let elements: Vec<SyntaxElement> = expr.children_with_tokens().collect();
    let op_idx = elements
        .iter()
        .position(|e| matches!(e, NodeOrToken::Token(t) if is_binary_operator(t.kind())))?;
    let op = elements[op_idx].as_token()?.clone();
    let lhs = elements[..op_idx]
        .iter()
        .rev()
        .find(|e| !is_trivia(e.kind()))?
        .clone();
    let rhs = elements[op_idx + 1..]
        .iter()
        .find(|e| !is_trivia(e.kind()))?
        .clone();
    Some((lhs, op, rhs))
}

// --- literal classifiers ---------------------------------------------------
//
// R's special constants (`TRUE`, `NA`, …) are all `IDENT` tokens; they are
// classified by text, mirroring `parser::expr::ident_is_special_constant`.

/// `TRUE`.
pub fn is_true(el: &SyntaxElement) -> bool {
    ident_text(el) == Some("TRUE")
}

/// `FALSE`.
pub fn is_false(el: &SyntaxElement) -> bool {
    ident_text(el) == Some("FALSE")
}

/// The rebindable boolean symbols `T` / `F`.
pub fn is_bool_symbol(el: &SyntaxElement) -> bool {
    matches!(ident_text(el), Some("T" | "F"))
}

/// `NA` or one of its typed variants (`NA_integer_`, …).
pub fn is_na(el: &SyntaxElement) -> bool {
    matches!(
        ident_text(el),
        Some("NA" | "NA_integer_" | "NA_real_" | "NA_complex_" | "NA_character_")
    )
}

/// `NULL`.
pub fn is_null(el: &SyntaxElement) -> bool {
    ident_text(el) == Some("NULL")
}

/// `NaN`.
pub fn is_nan(el: &SyntaxElement) -> bool {
    ident_text(el) == Some("NaN")
}

// --- shared helpers --------------------------------------------------------

/// The source text of an element: a token's text, or a node's full text.
pub fn element_text(el: &SyntaxElement) -> String {
    match el {
        NodeOrToken::Token(t) => t.text().to_string(),
        NodeOrToken::Node(n) => n.text().to_string(),
    }
}

/// Whether an operand is a primary/atomic expression that can be prefixed with
/// `!` (or dropped) without changing how the result parses — the guard a
/// negating rewrite like `x == FALSE` → `!x` needs to stay correct.
pub fn is_atom(el: &SyntaxElement) -> bool {
    match el {
        NodeOrToken::Token(t) => matches!(
            t.kind(),
            SyntaxKind::IDENT
                | SyntaxKind::INT
                | SyntaxKind::FLOAT
                | SyntaxKind::STRING
                | SyntaxKind::COMPLEX
        ),
        NodeOrToken::Node(n) => matches!(
            n.kind(),
            SyntaxKind::CALL_EXPR
                | SyntaxKind::PAREN_EXPR
                | SyntaxKind::SUBSET_EXPR
                | SyntaxKind::SUBSET2_EXPR
        ),
    }
}

fn ident_text(el: &SyntaxElement) -> Option<&str> {
    match el {
        NodeOrToken::Token(t) if t.kind() == SyntaxKind::IDENT => Some(t.text()),
        _ => None,
    }
}

fn is_binary_operator(kind: SyntaxKind) -> bool {
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

fn is_trivia(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
    )
}

/// The name a token denotes: raw text for an `IDENT`, the unquoted contents for
/// a backtick/quoted `STRING`.
fn token_name(token: &SyntaxToken) -> SmolStr {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn first_call(src: &str) -> CallExpr {
        parse(src)
            .cst
            .descendants()
            .find_map(CallExpr::cast)
            .expect("a call")
    }

    fn first_binary(src: &str) -> SyntaxNode {
        parse(src)
            .cst
            .descendants()
            .find(|n| n.kind() == SyntaxKind::BINARY_EXPR)
            .expect("a binary expr")
    }

    #[test]
    fn callee_name_reads_simple_names() {
        assert_eq!(callee_name(&first_call("foo(1)")).as_deref(), Some("foo"));
        assert!(call_named(first_call("foo(1)").syntax(), "foo").is_some());
        assert!(call_named(first_call("foo(1)").syntax(), "bar").is_none());
    }

    #[test]
    fn callee_name_none_for_computed_callee() {
        assert!(callee_name(&first_call("(g())(1)")).is_none());
    }

    #[test]
    fn nth_and_named_args() {
        let call = first_call("f(1, b = 2, 3)");
        assert_eq!(element_text(&nth_arg(&call, 0).unwrap()), "1");
        // `b = 2` is named, so it is skipped by positional indexing.
        assert_eq!(element_text(&nth_arg(&call, 1).unwrap()), "3");
        assert_eq!(element_text(&named_arg(&call, "b").unwrap()), "2");
        assert!(named_arg(&call, "z").is_none());
    }

    #[test]
    fn binary_parts_splits_comparison() {
        let (lhs, op, rhs) = binary_parts(&first_binary("x == TRUE")).unwrap();
        assert_eq!(element_text(&lhs), "x");
        assert_eq!(op.kind(), SyntaxKind::EQUAL2);
        assert!(is_true(&rhs));
    }

    #[test]
    fn literal_classifiers() {
        let (_, _, rhs) = binary_parts(&first_binary("x == FALSE")).unwrap();
        assert!(is_false(&rhs));
        let (_, _, rhs) = binary_parts(&first_binary("x == NA")).unwrap();
        assert!(is_na(&rhs));
        let (_, _, rhs) = binary_parts(&first_binary("x == NA_integer_")).unwrap();
        assert!(is_na(&rhs));
        let (_, _, rhs) = binary_parts(&first_binary("x == NULL")).unwrap();
        assert!(is_null(&rhs));
        let (lhs, _, _) = binary_parts(&first_binary("T == x")).unwrap();
        assert!(is_bool_symbol(&lhs));
    }

    #[test]
    fn is_atom_guards_negation() {
        let (lhs, _, _) = binary_parts(&first_binary("x == FALSE")).unwrap();
        assert!(is_atom(&lhs));
        let (lhs, _, _) = binary_parts(&first_binary("a > b == FALSE")).unwrap();
        // `a > b` is a binary expr, not a primary — negation would misparse.
        assert!(!is_atom(&lhs));
    }
}
