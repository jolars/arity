//! Direct list comparisons depend on coercion rather than a typed result.
//!
//! Only known, base-resolved producers are recognized. Choosing a result type
//! requires user intent, so the rule never rewrites the comparison.

use rowan::ast::AstNode as _;

use crate::ast::{CallExpr, ParenExpr};
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct ListComparison;

impl Rule for ListComparison {
    fn id(&self) -> &'static str {
        "list-comparison"
    }

    fn description(&self) -> &'static str {
        "Flag direct comparisons involving base `list()`, `pairlist()`, \
         `lapply()`, or `Map()` calls, including parenthesized operands. List \
         comparisons depend on coercion and may fail for non-scalar elements. \
         Prefer a typed iterator such as `vapply()` with an explicit result \
         template before comparing. Only simple base-resolved callees are \
         checked; variables, qualified calls, shadowed functions, and quoted \
         code are left alone. No automatic fix is offered because the intended \
         element type and comparison are unknown."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "Compare a typed result instead of a list:",
            source: "lapply(x, length) > 1\n",
        }]
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::BINARY_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some((lhs, op, rhs)) = el.as_node().and_then(matchers::binary_parts) else {
            return;
        };
        if !matches!(op.text(), "==" | "!=" | "<" | "<=" | ">" | ">=")
            || ![lhs, rhs]
                .into_iter()
                .any(|operand| is_list_call(operand, ctx))
        {
            return;
        }
        sink.push(Diagnostic {
            rule: self.id(),
            severity: Default::default(),
            path: Default::default(),
            range: op.text_range(),
            message: ViolationData::new(
                self.id(),
                "Direct list comparison relies on element coercion",
            )
            .with_suggestion("Use a typed iterator such as `vapply()` before comparing."),
            fix: None,
        });
    }
}

fn is_list_call(mut operand: SyntaxElement, ctx: &RuleContext<'_>) -> bool {
    loop {
        let Some(node) = operand.into_node() else {
            return false;
        };
        if let Some(paren) = ParenExpr::cast(node.clone()) {
            let Some(inner) = paren.inner() else {
                return false;
            };
            operand = inner;
            continue;
        }
        let Some(call) = CallExpr::cast(node) else {
            return false;
        };
        return matches!(
            matchers::callee_name(&call).as_deref(),
            Some("list" | "pairlist" | "lapply" | "Map")
        ) && ctx.resolves_to_base(&call)
            && call.callee_token().is_some_and(|callee| {
                ctx.model
                    .idents()
                    .iter()
                    .any(|ident| ident.range == callee.text_range() && !ident.data_masked)
            });
    }
}
