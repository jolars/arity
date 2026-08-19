//! `all-equal`: using `all.equal()` itself as a logical test.
//!
//! Base R's `all.equal()` returns `TRUE` for equality, but a character vector
//! describing the differences otherwise. Testing that value directly does not
//! test equality. The intended predicate is `isTRUE(all.equal(...))`.
//!
//! This is namespace-confirmed (`ns`): both `all.equal()` and an enclosing
//! `isFALSE()` must resolve to base R. Rewrites are unsafe because they repair
//! behavior that existing code may, however unusually, rely on.

use rowan::ast::AstNode as _;

use crate::ast::{CallExpr, IfExpr, ParenExpr, UnaryExpr, WhileExpr};
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct AllEqual;

const EXAMPLES: &[Example] = &[Example {
    caption: "Testing the return value of `all.equal()` directly:",
    source: "if (all.equal(actual, expected)) pass()\n",
}];

impl Rule for AllEqual {
    fn id(&self) -> &'static str {
        "all-equal"
    }

    fn description(&self) -> &'static str {
        "Flag `all.equal()` used directly as a condition, negated, or passed to \
         `isFALSE()`. A disagreement returns a character vector rather than \
         `FALSE`, so these forms do not reliably test equality. Use \
         `isTRUE(all.equal(...))` instead. Only base-R callees are flagged. The \
         fix is unsafe because it deliberately changes existing behavior."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[
            SyntaxKind::IF_EXPR,
            SyntaxKind::WHILE_EXPR,
            SyntaxKind::UNARY_EXPR,
            SyntaxKind::CALL_EXPR,
        ]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some((tested, range, replacement)) = tested_all_equal(el, ctx) else {
            return;
        };
        if !ctx.resolves_to_base(&tested) {
            return;
        }

        let fix = replacement.map(|content| {
            Fix::unsafe_(
                usize::from(range.start()),
                usize::from(range.end()),
                content,
                "Test `all.equal()` with `isTRUE()`",
            )
        });
        sink.push(Diagnostic {
            rule: "all-equal",
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new(
                "all-equal",
                "`all.equal()` does not return `FALSE` for unequal objects",
            )
            .with_suggestion("Use `isTRUE(all.equal(...))` to test equality."),
            fix,
        });
    }
}

fn tested_all_equal(
    el: &SyntaxElement,
    ctx: &RuleContext<'_>,
) -> Option<(CallExpr, rowan::TextRange, Option<String>)> {
    let node = el.as_node()?.clone();
    match node.kind() {
        SyntaxKind::IF_EXPR | SyntaxKind::WHILE_EXPR => {
            let elements = IfExpr::cast(node.clone())
                .and_then(|expr| expr.condition_elements())
                .or_else(|| WhileExpr::cast(node)?.condition_elements())?;
            let condition = elements.into_iter().find(|part| !is_trivia(part.kind()))?;
            let call = all_equal_call(&condition)?;
            let range = condition.text_range();
            Some((call, range, Some(format!("isTRUE({condition})"))))
        }
        SyntaxKind::UNARY_EXPR => {
            let unary = UnaryExpr::cast(node)?;
            if unary.op_kind() != Some(SyntaxKind::BANG) {
                return None;
            }
            let operand = unary.operand()?;
            let call = all_equal_call(&operand)?;
            let range = unary.syntax().text_range();
            Some((call, range, Some(format!("!isTRUE({operand})"))))
        }
        SyntaxKind::CALL_EXPR => {
            let outer = CallExpr::cast(node)?;
            if matchers::callee_name(&outer).as_deref() != Some("isFALSE")
                || !ctx.resolves_to_base(&outer)
            {
                return None;
            }
            let argument = matchers::sole_positional(&outer)?;
            let call = all_equal_call(&argument)?;
            let range = outer.syntax().text_range();
            let replacement = (!outer
                .syntax()
                .descendants_with_tokens()
                .any(|part| part.kind() == SyntaxKind::COMMENT))
            .then(|| format!("!isTRUE({argument})"));
            Some((call, range, replacement))
        }
        _ => None,
    }
}

fn all_equal_call(element: &SyntaxElement) -> Option<CallExpr> {
    if let Some(node) = element.as_node() {
        if let Some(call) = matchers::call_named(node, "all.equal") {
            return Some(call);
        }
        if node.kind() == SyntaxKind::PAREN_EXPR {
            return ParenExpr::cast(node.clone())
                .and_then(|paren| paren.inner())
                .and_then(|inner| all_equal_call(&inner));
        }
    }
    None
}

fn is_trivia(kind: SyntaxKind) -> bool {
    matches!(kind, SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE)
}
