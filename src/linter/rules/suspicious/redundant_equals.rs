//! `redundant-equals`: comparing to a logical literal is redundant —
//! `x == TRUE` is just `x`, and `x == FALSE` is `!x`.

use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct RedundantEquals;

impl Rule for RedundantEquals {
    fn id(&self) -> &'static str {
        "redundant-equals"
    }

    fn description(&self) -> &'static str {
        "Flag comparison to a logical literal: `x == TRUE` is just `x`, and \
         `x == FALSE` is `!x`."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "Comparing to `TRUE`:",
            source: "if (ready == TRUE) go()\n",
        }]
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::BINARY_EXPR]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else {
            return;
        };
        let Some((lhs, op, rhs)) = matchers::binary_parts(node) else {
            return;
        };
        if op.kind() != SyntaxKind::EQUAL2 {
            return;
        }
        // Identify which side is the literal; the other is the operand we keep.
        // `negate` is set for the `== FALSE` form.
        let (operand, negate) = if matchers::is_true(&rhs) {
            (&lhs, false)
        } else if matchers::is_false(&rhs) {
            (&lhs, true)
        } else if matchers::is_true(&lhs) {
            (&rhs, false)
        } else if matchers::is_false(&lhs) {
            (&rhs, true)
        } else {
            return;
        };
        let r = node.text_range();
        let (start, end) = (usize::from(r.start()), usize::from(r.end()));
        // Dropping `== TRUE` is always parseable; the negating rewrite needs an
        // atom operand so `!x` can't misbind (e.g. `!a + b`). Withhold otherwise.
        let fix = match negate {
            false => Some(Fix::safe(
                start,
                end,
                matchers::element_text(operand),
                "Drop redundant `== TRUE`",
            )),
            true if matchers::is_atom(operand) => Some(Fix::safe(
                start,
                end,
                format!("!{}", matchers::element_text(operand)),
                "Replace `== FALSE` with `!`",
            )),
            true => None,
        };
        sink.push(Diagnostic {
            rule: "redundant-equals",
            severity: Default::default(),
            path: Default::default(),
            range: r,
            message: ViolationData::new(
                "redundant-equals",
                "comparison with a logical literal is redundant",
            )
            .with_suggestion("Use the expression directly, or negate it."),
            fix,
        });
    }
}
