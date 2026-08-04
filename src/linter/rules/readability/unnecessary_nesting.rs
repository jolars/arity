//! `unnecessary-nesting`: an `if` whose entire body is a second `if`, where both
//! could be a single `if` with the conditions joined by `&&`.
//!
//! `if (a) { if (b) body }` (and its braceless form `if (a) if (b) body`) run
//! `body` exactly when `a && b` holds, so the outer nesting adds a level of
//! indentation for nothing—`if (a && b) body` says the same thing. The rule
//! fires only on the unambiguous collapsible shape: the **outer** `if` has no
//! `else`, its then-branch is *only* an inner `if` (directly, or as the sole
//! statement of a `{ }` block), and the **inner** `if` has no `else`. An `else`
//! on either side changes which code runs, so neither collapses.
//!
//! The fix rewrites the pair to `if (<a> && <b>) <body>`. It is **unsafe**:
//! collapsing dedents the body, so the result may need a reformat (layout is the
//! formatter's job, Tenet 1—the intended pipeline is fix-then-format). It stays
//! *correct* by construction: each condition that is not a primary is wrapped in
//! parentheses so the combined `&&` preserves the original grouping (`a || c`
//! binds looser than `&&`, so it must not inline bare), and the fix is
//! **withheld** whenever collapsing would drop a comment—the finding still
//! stands.

use rowan::TextRange;
use rowan::ast::AstNode as _;

use crate::ast::IfExpr;
use crate::ast::kinds::is_trivia;
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

pub struct UnnecessaryNesting;

const EXAMPLES: &[Example] = &[Example {
    caption: "An `if` whose only body is another `if` can be a single `if`:",
    source: "if (a) {\n  if (b) {\n    do_thing()\n  }\n}\n",
}];

impl Rule for UnnecessaryNesting {
    fn id(&self) -> &'static str {
        "unnecessary-nesting"
    }

    fn description(&self) -> &'static str {
        "Flag an `if` whose entire body is a second `if`—the two could be a \
         single `if` with the conditions joined by `&&`, dropping a needless \
         level of nesting. It fires only when neither `if` has an `else` (an \
         `else` on either side changes what runs) and the inner `if` is the sole \
         statement of the outer one.\n\nThe fix joins the conditions with `&&`, \
         parenthesizing each non-primary condition so the grouping is preserved. \
         It is unsafe (collapsing dedents the body, so a reformat may follow) and \
         withheld when it would drop a comment."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::IF_EXPR]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else {
            return;
        };
        let Some(outer) = IfExpr::cast(node.clone()) else {
            return;
        };
        // The outer `if` must have no `else` (an `else` would run when the inner
        // condition fails, which the collapsed form can't express).
        if outer.else_keyword().is_some() {
            return;
        }
        // The outer body, reduced to its sole meaningful element, must *be* an
        // inner `if` — directly, or the lone statement of a `{ }` block.
        let then = outer.then_elements().as_deref().and_then(sole_expr);
        let Some(inner) = then.and_then(|t| inner_if(&t)) else {
            return;
        };
        // A collapsible inner `if` must itself have no `else`.
        if inner.else_keyword().is_some() {
            return;
        }

        let (Some(outer_cond), Some(inner_cond), Some(inner_body)) = (
            outer.condition_elements().as_deref().and_then(sole_expr),
            inner.condition_elements().as_deref().and_then(sole_expr),
            inner.then_elements().as_deref().and_then(sole_expr),
        ) else {
            return;
        };

        let full = node.text_range();
        let (start, end) = (usize::from(full.start()), usize::from(full.end()));

        // Withhold the fix if collapsing would drop a comment sitting outside the
        // three retained fragments (the two conditions and the inner body); the
        // finding is still reported.
        let keep = [
            outer_cond.text_range(),
            inner_cond.text_range(),
            inner_body.text_range(),
        ];
        let fix = (!drops_comment(node, &keep)).then(|| {
            let combined = format!("{} && {}", wrap(&outer_cond), wrap(&inner_cond));
            let body = matchers::element_text(&inner_body);
            Fix::unsafe_(
                start,
                end,
                format!("if ({combined}) {body}"),
                "Combine the nested `if` conditions with `&&`",
            )
        });

        let range = inner
            .if_keyword()
            .map_or_else(|| inner.syntax().text_range(), |k| k.text_range());
        sink.push(Diagnostic {
            rule: "unnecessary-nesting",
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new(
                "unnecessary-nesting",
                "this `if` is nested in another `if` with no `else`",
            )
            .with_suggestion("Combine the two conditions with `&&` to drop a level of nesting."),
            fix,
        });
    }
}

/// The inner `if` collapsible into `then`: `then` cast as an `IfExpr` (the
/// braceless `if (a) if (b) …` form), or the sole statement of a `{ }` block cast
/// as an `IfExpr` (the `if (a) { if (b) … }` form). `None` for any other shape.
fn inner_if(then: &SyntaxElement) -> Option<IfExpr> {
    let node = then.as_node()?;
    if let Some(inner) = IfExpr::cast(node.clone()) {
        return Some(inner);
    }
    if node.kind() == SyntaxKind::BLOCK_EXPR {
        return block_sole_stmt(node).and_then(IfExpr::cast);
    }
    None
}

/// The sole statement node of a block (its children minus the braces, statement
/// separators, and trivia/comments), or `None` when it holds zero, several, or a
/// bare-token statement.
fn block_sole_stmt(block: &SyntaxNode) -> Option<SyntaxNode> {
    let mut it = block.children_with_tokens().filter(|e| {
        !matches!(
            e.kind(),
            SyntaxKind::LBRACE
                | SyntaxKind::RBRACE
                | SyntaxKind::SEMICOLON
                | SyntaxKind::WHITESPACE
                | SyntaxKind::NEWLINE
                | SyntaxKind::COMMENT
        )
    });
    let first = it.next()?;
    it.next().is_none().then_some(())?;
    first.into_node()
}

/// The sole non-trivia, non-comment element of a slice, or `None` when it holds
/// zero or several (so a condition or branch is unambiguous). Mirrors the helper
/// in `if_always_true`.
fn sole_expr(elements: &[SyntaxElement]) -> Option<SyntaxElement> {
    let mut it = elements
        .iter()
        .filter(|e| !is_trivia(e.kind()) && e.kind() != SyntaxKind::COMMENT);
    let first = it.next()?;
    it.next().is_none().then(|| first.clone())
}

/// A condition's source, parenthesized when it is not a primary expression, so
/// inlining it as an operand of `&&` keeps its original grouping. Only atoms,
/// calls, subscripts, and already-parenthesized expressions bind at least as
/// tightly as `&&`; everything else (any binary/unary operator) is wrapped —
/// over-wrapping is always correct.
fn wrap(el: &SyntaxElement) -> String {
    let text = matchers::element_text(el);
    if needs_paren(el) {
        format!("({text})")
    } else {
        text
    }
}

fn needs_paren(el: &SyntaxElement) -> bool {
    if matchers::is_atom(el) {
        return false;
    }
    !matches!(
        el.as_node().map(|n| n.kind()),
        Some(
            SyntaxKind::CALL_EXPR
                | SyntaxKind::SUBSET_EXPR
                | SyntaxKind::SUBSET2_EXPR
                | SyntaxKind::PAREN_EXPR
        )
    )
}

/// Whether collapsing would drop a comment: any `COMMENT` in the outer `if`
/// subtree that is not contained in one of the retained fragments (`keep`).
fn drops_comment(outer: &SyntaxNode, keep: &[TextRange]) -> bool {
    outer.descendants_with_tokens().any(|el| {
        el.kind() == SyntaxKind::COMMENT && !keep.iter().any(|k| k.contains_range(el.text_range()))
    })
}
