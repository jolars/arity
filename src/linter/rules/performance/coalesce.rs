//! `coalesce`: `if (is.null(x)) y else x` is the null-coalescing `x %||% y`.
//!
//! `if (is.null(x)) y else x` (and its mirror `if (!is.null(x)) x else y`)
//! returns `x` unless it is `NULL`, in which case it falls back to `y`—exactly
//! what `x %||% y` expresses. The operator form is shorter, reads as
//! "`x`, or else `y`", and evaluates `x` once: the `if` spelling names `x`
//! twice, so a side-effecting `x` runs twice under the original.
//!
//! The rule is **namespace-confirmed** (`ns`): `is.null` must resolve to base R
//! (not a local redefinition, namespace-qualified, or package-masked name), or
//! the `if` is not a null check at all. It fires only on the clean shape—an
//! `if`/`else` whose tested value (the sole positional argument of `is.null`) is
//! syntactically the branch that survives when the value is non-`NULL`.
//!
//! The fix is **unsafe**: `%||%` is only base R since 4.4.0 (otherwise it needs
//! rlang), and collapsing the two evaluations of `x` into one changes behavior
//! when `x` has side effects. It is **correct by construction** where emitted:
//! `%||%` binds tighter than `* / + -` and the logical operators but looser than
//! indexing, `$`, `^`, `:`, and unary `+`/`-`, so both operands are guarded to be
//! atoms (`matchers::is_atom`) and the whole `if` is required to sit in a
//! splice-safe position (`matchers::is_safe_splice_context`); otherwise the fix
//! is withheld (the finding still reports). It is also withheld when a comment
//! anywhere in the `if` would be dropped by the rewrite.

use rowan::ast::AstNode as _;

use crate::ast::kinds::is_trivia;
use crate::ast::{CallExpr, IfExpr, UnaryExpr};
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct Coalesce;

const EXAMPLES: &[Example] = &[Example {
    caption: "Falling back to a default when a value is `NULL`:",
    source: "y <- if (is.null(x)) default else x\n",
}];

impl Rule for Coalesce {
    fn id(&self) -> &'static str {
        "coalesce"
    }

    fn description(&self) -> &'static str {
        "Flag `if (is.null(x)) y else x` (and its mirror `if (!is.null(x)) x \
         else y`), which is the null-coalescing `x %||% y`—shorter, and it \
         evaluates `x` once instead of twice.\n\nThe rule fires only when \
         `is.null` resolves to base R; a local redefinition is left alone. The \
         fix is unsafe: `%||%` needs R >= 4.4 (or rlang), and collapsing the two \
         evaluations of `x` changes behavior when `x` has side effects."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::IF_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else {
            return;
        };
        let Some(if_expr) = IfExpr::cast(node.clone()) else {
            return;
        };
        // Only an `if`/`else` can coalesce: a bare `if (is.null(x)) y` returns
        // `NULL` (not `x`) when the test fails, which `x %||% y` is not.
        if if_expr.else_keyword().is_none() {
            return;
        }
        let cond = if_expr.condition_elements().as_deref().and_then(sole_expr);
        let then = if_expr.then_elements().as_deref().and_then(sole_expr);
        let els = if_expr.else_elements().as_deref().and_then(sole_expr);
        let (Some(cond), Some(then), Some(els)) = (cond, then, els) else {
            return;
        };

        // The condition is `is.null(x)` or `!is.null(x)`; `negated` picks which
        // branch survives a non-`NULL` value.
        let Some((call, negated)) = is_null_target(&cond) else {
            return;
        };
        let Some(tested) = matchers::sole_positional(&call) else {
            return;
        };

        // `is.null` must be the base predicate, or this is not a null check.
        if !ctx.resolves_to_base(&call) {
            return;
        }

        // The value that survives when non-`NULL` (`preferred`) must be the same
        // expression as the tested value; the other branch is the fallback.
        // `if (is.null(x)) y else x` keeps the `else`; `if (!is.null(x)) x else y`
        // keeps the `then`.
        let (preferred, fallback) = if negated { (then, els) } else { (els, then) };
        if !text_eq(&preferred, &tested) {
            return;
        }

        let r = node.text_range();
        // Emit the `x %||% y` fix only where it is correct by construction: both
        // operands atoms (so `%||%`'s precedence can't misbind them), a
        // splice-safe outer position, and no comment that the rewrite would drop.
        let drops_comment = node
            .descendants_with_tokens()
            .any(|e| e.kind() == SyntaxKind::COMMENT);
        let fix = (matchers::is_atom(&preferred)
            && matchers::is_atom(&fallback)
            && matchers::is_safe_splice_context(node)
            && !drops_comment)
            .then(|| {
                Fix::unsafe_(
                    usize::from(r.start()),
                    usize::from(r.end()),
                    format!(
                        "{} %||% {}",
                        matchers::element_text(&preferred).trim(),
                        matchers::element_text(&fallback).trim()
                    ),
                    "Replace the `if`/`else` with `%||%`",
                )
            });

        sink.push(Diagnostic {
            rule: "coalesce",
            severity: Default::default(),
            path: Default::default(),
            range: r,
            message: ViolationData::new(
                "coalesce",
                "`if (is.null(x)) y else x` is the null-coalescing `x %||% y`",
            )
            .with_suggestion("Use `x %||% y`."),
            fix,
        });
    }
}

/// The `is.null(x)` call at the heart of the condition, plus whether it is
/// negated (`!is.null(x)`). `None` when the condition is neither shape.
fn is_null_target(cond: &SyntaxElement) -> Option<(CallExpr, bool)> {
    let node = cond.as_node()?;
    if let Some(call) = matchers::call_named(node, "is.null") {
        return Some((call, false));
    }
    // `!is.null(x)`: a `!` unary over the `is.null` call.
    let unary = UnaryExpr::cast(node.clone())?;
    if unary.op_kind() != Some(SyntaxKind::BANG) {
        return None;
    }
    let operand = unary.operand()?;
    let call = matchers::call_named(operand.as_node()?, "is.null")?;
    Some((call, true))
}

/// The sole non-trivia, non-comment element of a slice, or `None` when it holds
/// zero or several (so a condition or branch is unambiguous).
fn sole_expr(elements: &[SyntaxElement]) -> Option<SyntaxElement> {
    let mut it = elements
        .iter()
        .filter(|e| !is_trivia(e.kind()) && e.kind() != SyntaxKind::COMMENT);
    let first = it.next()?;
    it.next().is_none().then(|| first.clone())
}

/// Whether two operand elements are the same expression, compared by trimmed
/// source text. Conservative on spacing (`f(a,b)` vs `f(a, b)` are treated as
/// distinct), which only ever suppresses the rule—never a false rewrite.
fn text_eq(a: &SyntaxElement, b: &SyntaxElement) -> bool {
    matchers::element_text(a).trim() == matchers::element_text(b).trim()
}
