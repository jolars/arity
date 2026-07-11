//! Reusable CST/AST shape matchers for syntactic lint rules (Phase 0 §I1).
//!
//! These collapse the common "is this a call to `foo` / what is its nth
//! argument / is this operand a `TRUE` literal" patterns that otherwise get
//! rewritten ad hoc in every rule, reducing a typical syntactic rule to ~30
//! lines. They are thin adapters over the typed AST layer (`CallExpr`, `Arg`,
//! `BinaryExpr`, `Ident`, `StringLit`, `Expr`) — the structural navigation
//! lives on the wrappers, not here. What remains local is the root/range,
//! string-predicate, and byte-span logic that has no single node/token home.

use rowan::NodeOrToken;
use rowan::ast::AstNode as _;
use smol_str::SmolStr;

use crate::ast::{Arg, AstToken as _, BinaryExpr, CallExpr, Expr, HasArgList as _, Ident};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

// --- calls & callees -------------------------------------------------------

/// The callee name of a call, when it is a simple name (`foo(…)`, `` `+`(…) ``).
/// `None` for a computed callee (`(g())(…)`, `x$f(…)`).
pub fn callee_name(call: &CallExpr) -> Option<SmolStr> {
    call.callee_name()
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
    call.args().map(arg_match).collect()
}

/// The value of the `n`th positional (unnamed) argument, 0-indexed.
pub fn nth_arg(call: &CallExpr, n: usize) -> Option<SyntaxElement> {
    call.nth_positional(n)
}

/// The value of the argument named `name`, if present.
pub fn named_arg(call: &CallExpr, name: &str) -> Option<SyntaxElement> {
    call.named_arg(name)
}

/// Split a typed [`Arg`] into the flat [`ArgMatch`] shape the rules consume.
fn arg_match(arg: Arg) -> ArgMatch {
    ArgMatch {
        name: arg.name(),
        name_token: arg.name_token(),
        value: arg.value(),
    }
}

/// The value of `call`'s sole positional argument, or `None` unless it has
/// exactly one value-bearing argument and that argument is positional. A stray
/// comment parses as a value-less `ARG`, so it is ignored here (the caller
/// withholds the fix on a comment that would be dropped) rather than counted as
/// a second argument.
pub fn sole_positional(call: &CallExpr) -> Option<SyntaxElement> {
    let mut valued = args(call).into_iter().filter(|a| a.value.is_some());
    let only = valued.next()?;
    if valued.next().is_some() || only.name.is_some() {
        return None;
    }
    only.value
}

// --- binary expressions ----------------------------------------------------

/// Split a `BINARY_EXPR` into `(lhs, operator, rhs)` at its top-level operator
/// token. Operands are elements: they may be tokens (`x`, `TRUE`) or nodes
/// (`a + b`).
pub fn binary_parts(expr: &SyntaxNode) -> Option<(SyntaxElement, SyntaxToken, SyntaxElement)> {
    BinaryExpr::cast(expr.clone())?.parts()
}

// --- literal classifiers ---------------------------------------------------
//
// R's special constants (`TRUE`, `NA`, …) are all `IDENT` tokens classified by
// text. These element-level helpers cast an operand to an [`Ident`] and defer
// to its classifiers, the single source of truth.

/// Cast an operand element to an [`Ident`] token, if it is one.
fn as_ident(el: &SyntaxElement) -> Option<Ident> {
    el.as_token().cloned().and_then(Ident::cast)
}

/// `TRUE`.
pub fn is_true(el: &SyntaxElement) -> bool {
    as_ident(el).is_some_and(|i| i.is_true())
}

/// `FALSE`.
pub fn is_false(el: &SyntaxElement) -> bool {
    as_ident(el).is_some_and(|i| i.is_false())
}

/// The rebindable boolean symbols `T` / `F`.
pub fn is_bool_symbol(el: &SyntaxElement) -> bool {
    as_ident(el).is_some_and(|i| i.is_bool_symbol())
}

/// `NA` or one of its typed variants (`NA_integer_`, …).
pub fn is_na(el: &SyntaxElement) -> bool {
    as_ident(el).is_some_and(|i| i.is_na())
}

/// `NULL`.
pub fn is_null(el: &SyntaxElement) -> bool {
    as_ident(el).is_some_and(|i| i.is_null())
}

/// `NaN`.
pub fn is_nan(el: &SyntaxElement) -> bool {
    as_ident(el).is_some_and(|i| i.is_nan())
}

// --- string literals -------------------------------------------------------

/// The `(quote, inner)` of a single string-literal token, when it is quoted with
/// `"` or `'` (not a backtick name). `inner` is the raw text between the quotes,
/// escapes and all. `None` for a non-string token or a backtick-quoted name.
pub fn string_literal(token: &SyntaxToken) -> Option<(char, &str)> {
    if token.kind() != SyntaxKind::STRING {
        return None;
    }
    let text = token.text();
    let bytes = text.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let quote = bytes[0];
    if !matches!(quote, b'"' | b'\'') || bytes[bytes.len() - 1] != quote {
        return None;
    }
    Some((quote as char, &text[1..text.len() - 1]))
}

/// Whether `s` contains no regex metacharacter (and no backslash escape), so it
/// matches literally and identically under both regex and `fixed = TRUE`
/// semantics. All metacharacters are ASCII, so a byte scan is sufficient.
pub fn is_plain_regex_literal(s: &str) -> bool {
    !s.bytes().any(|b| {
        matches!(
            b,
            b'.' | b'\\'
                | b'|'
                | b'('
                | b')'
                | b'['
                | b']'
                | b'{'
                | b'}'
                | b'^'
                | b'$'
                | b'*'
                | b'+'
                | b'?'
        )
    })
}

// --- splice contexts -------------------------------------------------------

/// Whether an expression that binds as loosely as `!` (a negation, or the
/// comparison it is at least as tight as) is safe to splice in unparenthesized
/// at `node`'s position. Safe when the parent does not bind tighter than `!`: a
/// statement position, a delimited clause/argument, an assignment, an outer `!`,
/// or a looser logical/formula operator. Anything tighter (arithmetic, another
/// comparison, indexing, `$`/`@`, a call) would capture the rewrite, so it is
/// unsafe — the caller withholds the fix there. Unknown parents are unsafe by
/// default, keeping the guard conservative.
pub fn is_safe_splice_context(node: &SyntaxNode) -> bool {
    let Some(parent) = node.parent() else {
        return true;
    };
    match parent.kind() {
        SyntaxKind::ROOT
        | SyntaxKind::BLOCK_EXPR
        | SyntaxKind::PAREN_EXPR
        | SyntaxKind::ARG
        | SyntaxKind::IF_EXPR
        | SyntaxKind::WHILE_EXPR
        | SyntaxKind::FOR_EXPR
        | SyntaxKind::REPEAT_EXPR
        | SyntaxKind::ASSIGNMENT_EXPR => true,
        SyntaxKind::BINARY_EXPR => binary_parts(&parent).is_some_and(|(_, op, _)| {
            matches!(
                op.kind(),
                SyntaxKind::AND
                    | SyntaxKind::AND2
                    | SyntaxKind::OR
                    | SyntaxKind::OR2
                    | SyntaxKind::TILDE
            )
        }),
        SyntaxKind::UNARY_EXPR => parent
            .children_with_tokens()
            .find_map(|e| e.into_token())
            .is_some_and(|t| t.kind() == SyntaxKind::BANG),
        _ => false,
    }
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
    Expr::cast(el.clone()).is_some_and(|e| e.is_atom())
}

// --- deletion spans --------------------------------------------------------

/// Widen a statement's (or run of statements') range to swallow its leading
/// indentation, its own line terminator, and any wholly-blank lines that follow
/// — but not the next content line's indentation. When the range is the last
/// content in the file, preceding blank lines are absorbed too. This keeps a
/// whole-statement deletion format-clean by construction (the autofix-correctness
/// discipline): no orphaned blank line, no stranded indentation.
pub fn deletion_span(src: &str, range: rowan::TextRange) -> (usize, usize) {
    let bytes = src.as_bytes();

    // Leading indentation on the statement's line.
    let mut start = usize::from(range.start());
    while start > 0 && matches!(bytes[start - 1], b' ' | b'\t') {
        start -= 1;
    }

    // Trailing horizontal whitespace, then the statement's own newline.
    let mut end = usize::from(range.end());
    while end < bytes.len() && matches!(bytes[end], b' ' | b'\t') {
        end += 1;
    }
    end = consume_newline(bytes, end);

    // Absorb any fully-blank lines that follow, stopping before the next
    // line that carries content (so its indentation survives).
    loop {
        let mut probe = end;
        while probe < bytes.len() && matches!(bytes[probe], b' ' | b'\t') {
            probe += 1;
        }
        let after_nl = consume_newline(bytes, probe);
        if after_nl == probe {
            break; // line has content (or EOF) — keep it intact
        }
        end = after_nl;
    }

    // If nothing but whitespace follows (the statement was the last content),
    // also pull back over preceding blank lines so we don't leave a trailing
    // blank, keeping the previous content line's own terminator.
    if end == bytes.len() {
        let mut prev = start;
        while prev > 0 && matches!(bytes[prev - 1], b' ' | b'\t' | b'\n' | b'\r') {
            prev -= 1;
        }
        start = if prev > 0 {
            consume_newline(bytes, prev)
        } else {
            0
        };
    }

    (start, end)
}

/// Advance past a single `\n` or `\r\n` at `i`, else return `i` unchanged.
fn consume_newline(bytes: &[u8], i: usize) -> usize {
    match bytes.get(i) {
        Some(b'\n') => i + 1,
        Some(b'\r') if bytes.get(i + 1) == Some(&b'\n') => i + 2,
        _ => i,
    }
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
    fn string_literal_extracts_quote_and_inner() {
        let call = first_call("grepl(\"^abc\", x)");
        let tok = nth_arg(&call, 0).unwrap().into_token().unwrap();
        assert_eq!(string_literal(&tok), Some(('"', "^abc")));
        let call = first_call("grepl('a.b', x)");
        let tok = nth_arg(&call, 0).unwrap().into_token().unwrap();
        assert_eq!(string_literal(&tok), Some(('\'', "a.b")));
    }

    #[test]
    fn plain_regex_literal_rejects_metacharacters() {
        assert!(is_plain_regex_literal("abc"));
        assert!(is_plain_regex_literal("hello world"));
        assert!(!is_plain_regex_literal("a.b"));
        assert!(!is_plain_regex_literal("a\\.b"));
        assert!(!is_plain_regex_literal("^abc"));
        assert!(!is_plain_regex_literal("a+b"));
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
