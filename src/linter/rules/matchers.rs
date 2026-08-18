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
use rowan::TextRange;
use rowan::ast::AstNode as _;
use smol_str::SmolStr;

use crate::ast::{
    Arg, AssignmentExpr, AstToken as _, BinaryExpr, CallExpr, Expr, ForExpr, HasArgList as _, Ident,
};
use crate::semantic::symbols::unbacktick;
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

// --- package loads ---------------------------------------------------------

/// The base callees whose `package` argument names a package, exactly the set
/// `tools:::.check_packages_used` matches. Owned by the semantic layer, which
/// records the same calls, so the two readings cannot drift.
pub use crate::semantic::symbols::PACKAGE_LOAD_CALLS;

/// The package `call` names and the token to span, when `call` is one of
/// [`PACKAGE_LOAD_CALLS`] and its `package` argument is a bare name or a string
/// literal.
///
/// `None` for a computed argument, for `character.only = TRUE` (where a symbol
/// names a variable holding the package name, not the package), and for R's
/// `common_names` placeholders — none of which name a package statically, so
/// matching them could only ever invent a finding.
pub fn package_load_arg(call: &CallExpr) -> Option<(SmolStr, SyntaxToken)> {
    if !PACKAGE_LOAD_CALLS.contains(&callee_name(call)?.as_str()) {
        return None;
    }
    // `character.only = TRUE` says the argument is a variable, by contract.
    if named_arg(call, "character.only").is_some_and(|el| is_true(&el)) {
        return None;
    }
    let value = named_arg(call, "package").or_else(|| nth_arg(call, 0))?;
    let token = value.as_token()?;
    let name = match token.kind() {
        SyntaxKind::IDENT => SmolStr::new(token.text()),
        SyntaxKind::STRING => SmolStr::new(string_literal(token)?.1),
        _ => return None,
    };
    if name.is_empty() || crate::semantic::symbols::is_package_name_placeholder(&name) {
        return None;
    }
    Some((name, token.clone()))
}

// --- binary expressions ----------------------------------------------------

/// Split a `BINARY_EXPR` into `(lhs, operator, rhs)` at its top-level operator
/// token. Operands are elements: they may be tokens (`x`, `TRUE`) or nodes
/// (`a + b`).
pub fn binary_parts(expr: &SyntaxNode) -> Option<(SyntaxElement, SyntaxToken, SyntaxElement)> {
    BinaryExpr::cast(expr.clone())?.parts()
}

/// Supported operators for comparisons against a distinguished constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstantComparisonOp {
    Equal,
    NotEqual,
    In,
}

/// A `==`, `!=`, or `%in%` expression with exactly one operand selected by
/// `is_constant`. The other operand is returned along with whether the constant
/// was written on the right; `%in%` callers need that direction because it is
/// not commutative.
pub fn constant_comparison(
    expr: &SyntaxNode,
    is_constant: fn(&SyntaxElement) -> bool,
) -> Option<(SyntaxElement, ConstantComparisonOp, bool)> {
    let (lhs, op, rhs) = binary_parts(expr)?;
    let op = match op.kind() {
        SyntaxKind::EQUAL2 => ConstantComparisonOp::Equal,
        SyntaxKind::NOT_EQUAL => ConstantComparisonOp::NotEqual,
        SyntaxKind::USER_OP if op.text() == "%in%" => ConstantComparisonOp::In,
        _ => return None,
    };
    match (is_constant(&lhs), is_constant(&rhs)) {
        (true, false) => Some((rhs, op, false)),
        (false, true) => Some((lhs, op, true)),
        _ => None,
    }
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

// --- loops -----------------------------------------------------------------

/// A `for` loop's clause, split into the pieces the loop-index rules match on.
pub struct ForClause {
    /// The loop-index identifier token (`i` in `for (i in xs)`).
    pub index: SyntaxToken,
    /// Range of the sequence expression (`xs`), spanning all of its elements.
    pub sequence: TextRange,
}

impl ForClause {
    /// The whole `i in xs` clause range — the span a finding about the clause
    /// should point at (tighter than the `FOR_EXPR`, which swallows the body).
    pub fn range(&self) -> TextRange {
        TextRange::new(self.index.text_range().start(), self.sequence.end())
    }
}

/// Split a `for` loop's clause into its index token and sequence range. `None`
/// unless the index is a simple name and the sequence is non-empty — a
/// recovered or malformed clause matches nothing, keeping callers conservative.
pub fn for_clause(for_expr: &ForExpr) -> Option<ForClause> {
    let parts = for_expr.parts()?;

    // The index must be exactly one `IDENT` token: anything else (an empty
    // clause, an error node) is not a plain loop variable.
    let [NodeOrToken::Token(index)] = parts.variable_elements.as_slice() else {
        return None;
    };
    if index.kind() != SyntaxKind::IDENT {
        return None;
    }

    let first = parts.sequence_elements.first()?;
    let last = parts.sequence_elements.last()?;
    Some(ForClause {
        index: index.clone(),
        sequence: TextRange::new(first.text_range().start(), last.text_range().end()),
    })
}

// --- bindings and dispatch -------------------------------------------------

/// Whether the binding whose defining identifier spans `def_range` is assigned a
/// function literal (`f <- function(x) …`, `f <- \(x) …`). Conservative: any
/// other shape — a call's return value, a constant, a nested or chained
/// assignment — is not a function *definition* as far as a rule asking this is
/// concerned.
pub fn binds_a_function(root: &SyntaxNode, def_range: TextRange) -> bool {
    let Some(token) = root.covering_element(def_range).into_token() else {
        return false;
    };
    let Some(assign) = token.parent().and_then(AssignmentExpr::cast) else {
        return false;
    };
    assign
        .value_element()
        .and_then(|el| el.into_node())
        .is_some_and(|value| value.kind() == SyntaxKind::FUNCTION_EXPR)
}

/// Whether `name` has the shape of an S3 method, `generic.class`.
///
/// Dispatch reaches a method without any read of its name, so "nothing reads
/// it" is no evidence that it is dead. A `NAMESPACE` lookup
/// ([`crate::project::FileScope::is_s3_method`]) answers a *different*
/// question — `S3method()` decides what is registered for outside callers,
/// while R dispatches to a method defined in the namespace either way — so for
/// reachability the name's shape is all there is. Whether the prefix really
/// names a generic is a run-time fact, and arity never evaluates R; roxygen2
/// hits the same wall and consults the build-time generic set. Deliberately
/// blunt in the safe direction: a genuinely dead `my.util` goes unreported
/// rather than a live method being reported (and, under `--unsafe-fixes`,
/// deleted).
///
/// Backticks are stripped first, so operator methods (`` `$.cls` ``,
/// `` `[.cls` ``, `` `==.cls` ``) are recognized. The dot must sit strictly
/// inside the name: a leading one is R's convention for an internal name, and a
/// trailing one names no class.
pub fn looks_like_s3_method(name: &str) -> bool {
    let bare = unbacktick(name);
    bare.char_indices()
        .any(|(i, c)| c == '.' && i > 0 && i + 1 < bare.len())
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

/// The byte span to delete in order to remove a comment.
///
/// Deliberately *not* [`deletion_span`]: that widens over following blank lines,
/// which would swallow a separator the author put there and — worse — make the
/// spans of two comments on consecutive lines overlap, so `apply_fixes` would
/// drop one of them.
///
/// Two shapes, distinguished by what precedes the comment on its line:
///
/// - **own line** (only indentation before it): the line goes, terminator and
///   all, leaving the next line's indentation untouched.
/// - **trailing** (code before it): from the end of that code to the end of the
///   comment, so the separating whitespace goes too but the newline stays.
///
/// Both leave parseable R by construction — a comment is trivia, and neither
/// shape removes any non-comment text.
pub fn comment_deletion_span(src: &str, comment: rowan::TextRange) -> (usize, usize) {
    let bytes = src.as_bytes();

    // Walk back over this line's horizontal whitespace.
    let mut start = usize::from(comment.start());
    while start > 0 && matches!(bytes[start - 1], b' ' | b'\t') {
        start -= 1;
    }

    let own_line = start == 0 || matches!(bytes[start - 1], b'\n' | b'\r');
    if own_line {
        (start, consume_newline(bytes, usize::from(comment.end())))
    } else {
        // Code precedes it: keep the newline, drop the separating whitespace.
        (start, usize::from(comment.end()))
    }
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
    fn looks_like_s3_method_needs_an_interior_dot() {
        assert!(looks_like_s3_method("print.foo"));
        assert!(looks_like_s3_method("as.data.frame.myclass"));
        // Operator and replacement methods, which only appear backticked.
        assert!(looks_like_s3_method("`$.cls`"));
        assert!(looks_like_s3_method("`[.cls`"));
        assert!(looks_like_s3_method("`length<-.cls`"));
        // A generic may itself carry a dot.
        assert!(looks_like_s3_method(".subset.cls"));

        assert!(!looks_like_s3_method("helper"));
        assert!(!looks_like_s3_method("`%||%`"));
        // A leading dot is R's internal-name convention, not a generic; a
        // trailing one names no class.
        assert!(!looks_like_s3_method(".internal"));
        assert!(!looks_like_s3_method("trailing."));
        assert!(!looks_like_s3_method("."));
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
