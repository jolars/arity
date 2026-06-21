//! `comparison-negation`: negating a comparison is clearer written as the
//! opposite comparison — `!(a == b)` is `a != b`, `!(x < y)` is `x >= y`.
//!
//! Both the parenthesized form `!(<lhs> <cmp> <rhs>)` and the bare `!a == b`
//! are matched: R binds `!` looser than the comparison operators, so `!a == b`
//! already means `!(a == b)` (the operand of the `!` is the whole comparison).
//! The rewrite swaps the comparison operator and drops the `!` (and the parens,
//! when present). The result (a comparison) binds *tighter* than the `!` it
//! replaces, so it never needs new parens in any parent context — no parent
//! guard is required. The fix is withheld when a comment in the operand would
//! be dropped.

use crate::linter::diagnostic::{Diagnostic, Fix, Severity, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

pub struct ComparisonNegation;

const EXAMPLES: &[Example] = &[Example {
    caption: "Negating an equality test:",
    source: "if (!(a == b)) stop()\n",
}];

impl Rule for ComparisonNegation {
    fn id(&self) -> &'static str {
        "comparison-negation"
    }

    fn description(&self) -> &'static str {
        "Flag a negated comparison — `!(a == b)`, `!x < y` — which reads more \
         clearly as the opposite comparison (`a != b`, `x >= y`).\n\nThe fix is \
         withheld when a comment in the operand would otherwise be lost."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::UNARY_EXPR]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(unary) = el.as_node() else {
            return;
        };
        // The operator must be `!`.
        let Some(op) = unary.children_with_tokens().find_map(|e| e.into_token()) else {
            return;
        };
        if op.kind() != SyntaxKind::BANG {
            return;
        }
        // The operand is the comparison — either bare (`!a == b`) or wrapped in
        // parens (`!(a == b)`).
        let Some(operand) = unary.children().next() else {
            return;
        };
        let inner = match operand.kind() {
            SyntaxKind::BINARY_EXPR => operand,
            SyntaxKind::PAREN_EXPR => {
                let Some(b) = operand
                    .children()
                    .find(|n| n.kind() == SyntaxKind::BINARY_EXPR)
                else {
                    return;
                };
                b
            }
            _ => return,
        };
        let Some((lhs, cmp, rhs)) = matchers::binary_parts(&inner) else {
            return;
        };
        let Some(negated) = negate_comparison(cmp.kind()) else {
            return;
        };

        let r = unary.text_range();
        // The rewrite reconstructs from the operands' text only, so a comment
        // anywhere in the subtree would be dropped — withhold the fix then.
        let fix = if has_comment(unary) {
            None
        } else {
            Some(Fix::safe(
                usize::from(r.start()),
                usize::from(r.end()),
                format!(
                    "{} {negated} {}",
                    matchers::element_text(&lhs),
                    matchers::element_text(&rhs)
                ),
                format!("Rewrite as `{negated}`"),
            ))
        };
        sink.push(Diagnostic {
            rule: "comparison-negation",
            severity: Severity::Warning,
            path: Default::default(),
            range: r,
            message: ViolationData::new(
                "comparison-negation",
                "negated comparison is clearer as the opposite operator",
            )
            .with_suggestion("Flip the comparison instead of negating it."),
            fix,
        });
    }
}

/// The opposite of a comparison operator, or `None` for a non-comparison.
fn negate_comparison(kind: SyntaxKind) -> Option<&'static str> {
    Some(match kind {
        SyntaxKind::EQUAL2 => "!=",
        SyntaxKind::NOT_EQUAL => "==",
        SyntaxKind::LESS_THAN => ">=",
        SyntaxKind::LESS_THAN_OR_EQUAL => ">",
        SyntaxKind::GREATER_THAN => "<=",
        SyntaxKind::GREATER_THAN_OR_EQUAL => "<",
        _ => return None,
    })
}

/// Whether `node`'s subtree carries any comment token.
fn has_comment(node: &SyntaxNode) -> bool {
    node.descendants_with_tokens()
        .any(|e| e.kind() == SyntaxKind::COMMENT)
}
