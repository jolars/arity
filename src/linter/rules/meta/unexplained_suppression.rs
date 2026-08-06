//! `unexplained-suppression`: a `# arity-ignore` directive with no reason.
//!
//! A suppression is a claim that the linter is wrong here, and that claim
//! outlives the person who made it. `# arity-ignore vector-logic` says nothing
//! about *why* — whether the operands are known scalars, whether it is a
//! deliberate vectorized comparison, or whether someone was silencing noise on
//! a deadline. The next reader cannot tell a considered exception from a
//! papered-over bug, so the suppression becomes permanent by default.
//!
//! Off by default: requiring a reason is a house style, not a defect, and a
//! codebase that has adopted `# arity-ignore <rule>` without reasons would see
//! a finding per directive on the first run. Enable it with `select`.
//!
//! Report-only, and not merely for convention — writing the reason *is* the
//! fix, and inventing one would fabricate a justification nobody stands behind.

use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext};

pub struct UnexplainedSuppression;

const EXAMPLES: &[Example] = &[Example {
    caption: "The directive says what to silence, but not why:",
    source: "# arity-ignore unused-binding\nx <- 1\n",
}];

impl Rule for UnexplainedSuppression {
    fn id(&self) -> &'static str {
        "unexplained-suppression"
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn description(&self) -> &'static str {
        "Flags a `# arity-ignore` directive that carries no reason — the text \
after the `:`. A suppression is a standing claim that the linter is wrong at \
this spot, and without a reason the next reader cannot tell a considered \
exception from noise someone silenced under deadline, so it becomes permanent \
by default. Disabled by default, since requiring reasons is a house style \
rather than a defect; enable it with `select`. Report-only: writing the reason \
is the fix, and inventing one would fabricate a justification."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        for directive in ctx.suppressions.directives() {
            if directive.has_reason() {
                continue;
            }
            sink.push(Diagnostic {
                rule: "unexplained-suppression",
                severity: Default::default(),
                path: Default::default(),
                range: directive.comment,
                message: ViolationData::new(
                    "unexplained-suppression",
                    "this suppression gives no reason",
                )
                .with_suggestion("add one after the rule: `# arity-ignore <rule>: <reason>`"),
                fix: None,
            });
        }
    }
}
