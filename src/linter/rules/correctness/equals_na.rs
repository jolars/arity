//! `equals-na`: `x == NA` is always `NA`, never `TRUE`/`FALSE` — almost always
//! a mistake for `is.na(x)`. Only the `==` form is rewritten; `!=` is left for a
//! separate rule.

use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct EqualsNa;

impl Rule for EqualsNa {
    fn id(&self) -> &'static str {
        "equals-na"
    }

    fn description(&self) -> &'static str {
        "Flag `x == NA`, which is always `NA` rather than `TRUE`/`FALSE`—\
         almost always a mistake for `is.na(x)`, which is the autofix."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "Comparing to `NA` with `==`:",
            source: "x == NA\n",
        }]
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::BINARY_EXPR]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else {
            return;
        };
        let Some((other, op, _)) = matchers::constant_comparison(node, matchers::is_na) else {
            return;
        };
        if op != matchers::ConstantComparisonOp::Equal {
            return;
        }
        let r = node.text_range();
        // `is.na(<other>)` parenthesizes the operand, so no precedence guard.
        let fix = Fix::safe(
            usize::from(r.start()),
            usize::from(r.end()),
            format!("is.na({})", matchers::element_text(&other)),
            "Replace `== NA` with `is.na()`",
        );
        sink.push(Diagnostic {
            rule: "equals-na",
            severity: Default::default(),
            path: Default::default(),
            range: r,
            message: ViolationData::new(
                "equals-na",
                "comparison with `NA` is always `NA`; use `is.na()`",
            )
            .with_suggestion("Use `is.na(x)`."),
            fix: Some(fix),
        });
    }
}
