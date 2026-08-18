//! `equals-null`: comparisons with `NULL` produce a zero-length logical value,
//! not a scalar null test. Membership is reported but never fixed because
//! `%in% NULL` is not equivalent to `is.null()`.

use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers::{self, ConstantComparisonOp};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct EqualsNull;

impl Rule for EqualsNull {
    fn id(&self) -> &'static str {
        "equals-null"
    }

    fn description(&self) -> &'static str {
        "Flag `==`, `!=`, and `%in%` comparisons with `NULL`; use `is.null()` for a scalar null test."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "Comparing a value with `NULL`:",
            source: "if (x == NULL) handle_null()\n",
        }]
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::BINARY_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else { return };
        let Some((operand, op, _)) = matchers::constant_comparison(node, matchers::is_null) else {
            return;
        };
        let range = node.text_range();
        let fixable = ctx.introduced_call_resolves_to_base("is.null")
            && match op {
                ConstantComparisonOp::Equal => true,
                ConstantComparisonOp::NotEqual => matchers::is_safe_splice_context(node),
                ConstantComparisonOp::In => false,
            };
        let fix = fixable.then(|| {
            let test = format!("is.null({})", matchers::element_text(&operand));
            let replacement = if op == ConstantComparisonOp::NotEqual {
                format!("!{test}")
            } else {
                test
            };
            Fix::safe(
                range.start().into(),
                range.end().into(),
                replacement,
                "Use `is.null()`",
            )
        });
        sink.push(Diagnostic {
            rule: "equals-null",
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new(
                "equals-null",
                "comparison with `NULL` does not produce a scalar null test",
            )
            .with_suggestion("Use `is.null(x)`."),
            fix,
        });
    }
}
