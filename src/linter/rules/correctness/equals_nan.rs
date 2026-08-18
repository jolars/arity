//! `equals-nan`: comparisons with `NaN` do not test for NaN values. Reports
//! `==`, `!=`, and `%in%`; fixes only equivalent directions and only when the
//! introduced `is.nan` call is confirmed to resolve to base R.

use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers::{self, ConstantComparisonOp};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct EqualsNan;

impl Rule for EqualsNan {
    fn id(&self) -> &'static str {
        "equals-nan"
    }

    fn description(&self) -> &'static str {
        "Flag `==`, `!=`, and `%in%` comparisons with `NaN`; use `is.nan()` to test for NaN values."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "Comparing a value with `NaN`:",
            source: "if (x == NaN) handle_nan()\n",
        }]
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::BINARY_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else { return };
        let Some((operand, op, constant_on_right)) =
            matchers::constant_comparison(node, matchers::is_nan)
        else {
            return;
        };
        let range = node.text_range();
        let fixable = ctx.introduced_call_resolves_to_base("is.nan")
            && match op {
                ConstantComparisonOp::Equal => true,
                ConstantComparisonOp::NotEqual => matchers::is_safe_splice_context(node),
                ConstantComparisonOp::In => constant_on_right,
            };
        let fix = fixable.then(|| {
            let test = format!("is.nan({})", matchers::element_text(&operand));
            let replacement = if op == ConstantComparisonOp::NotEqual {
                format!("!{test}")
            } else {
                test
            };
            Fix::safe(
                range.start().into(),
                range.end().into(),
                replacement,
                "Use `is.nan()`",
            )
        });
        sink.push(Diagnostic {
            rule: "equals-nan",
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new(
                "equals-nan",
                "comparison with `NaN` does not reliably test for NaN values",
            )
            .with_suggestion("Use `is.nan(x)`."),
            fix,
        });
    }
}
