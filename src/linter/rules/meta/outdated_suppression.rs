//! `outdated-suppression`: a directive that no longer silences
//! anything.
//!
//! Suppressions outlive the code they were written for. The binding gets read
//! again, the call gets rewritten, the rule gets a better gate — and the
//! directive stays, now inert but still asserting that arity is wrong here. The
//! next person to touch the line has no way to know it is dead weight, and the
//! stale directive will happily suppress a *real* finding if the shape ever
//! comes back.
//!
//! This is the only rule that cannot run as an ordinary `check_file` pass:
//! "did this directive match anything" is a fact about the driver's filtering
//! step, which happens after every rule has emitted. It is implemented as
//! [`Rule::check_suppressions`], the post-suppression seam.
//!
//! ## Why the gates are narrow
//!
//! A directive that matched nothing is not necessarily stale — it may simply be
//! *dormant*, because the rule it names did not run in this pass. `select`,
//! `ignore`, and `default_enabled() == false` all produce that state, and
//! reporting it would make the finding a function of the invocation rather than
//! of the code. So a directive is only reported when the rule it names actually
//! ran (`ctx.enabled_rules`), or when it is *dangling* — no non-trivia sibling
//! follows it, so it can never match whatever the configuration.
//!
//! Blanket directives are left to `blanket-suppression`: whether "every rule"
//! found nothing is not answerable under a partial `select`. An unknown rule ID
//! is left to `misnamed-suppression`, which keeps the two rules from both
//! firing on one comment — and from offering a rename fix and a delete fix over
//! the same bytes.

use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers::comment_deletion_span;
use crate::linter::rules::{Example, Rule, RuleContext, is_known_rule};
use crate::linter::suppression::{Directive, DirectiveUsage};

pub struct OutdatedSuppression;

const EXAMPLES: &[Example] = &[Example {
    caption: "`x` is read, so `unused-binding` finds nothing and the directive is dead:",
    source: "# arity-lint skip unused-binding: no longer needed\nx <- 1\nprint(x)\n",
}];

impl Rule for OutdatedSuppression {
    fn id(&self) -> &'static str {
        "outdated-suppression"
    }

    fn description(&self) -> &'static str {
        "Flags an `# arity-lint` directive that suppressed nothing on this run \
— the code it was written for has changed, but the directive stayed. A stale \
suppression is misleading (it asserts arity is wrong at a spot where arity says \
nothing) and it is a trap: it will silence a real finding if the shape ever \
comes back. The fix deletes the directive.\
\n\nTo avoid reporting a directive that is merely *dormant*, the rule only \
fires when the rule the directive names actually ran — a rule excluded by \
`select`/`ignore`, or one that is off by default, leaves its directives alone \
— or when the directive is dangling, with no code after it to attach to. \
Directives naming no rule are left to `blanket-suppression`, and unknown rule \
IDs to `misnamed-suppression`."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    /// The example needs `unused-binding` to have *run* (and found nothing) for
    /// the directive to count as stale rather than dormant.
    fn doc_select(&self) -> &'static [&'static str] {
        &["unused-binding"]
    }

    fn check_suppressions(
        &self,
        ctx: &RuleContext<'_>,
        used: &DirectiveUsage,
        sink: &mut Vec<Diagnostic>,
    ) {
        let source = ctx.root.text().to_string();
        for (index, directive) in ctx.suppressions.directives().iter().enumerate() {
            // Names no rule -> `blanket-suppression`.
            let Some(rule) = directive.rule() else {
                continue;
            };
            // Names a rule that does not exist -> `misnamed-suppression`.
            if !is_known_rule(&rule.id) {
                continue;
            }
            if used.is_used(index) {
                continue;
            }
            // Dormant, not stale: the rule never ran, so "matched nothing"
            // carries no information. A dangling directive is dead either way.
            if !directive.is_dangling() && !ctx.enabled_rules.contains(&rule.id) {
                continue;
            }
            sink.push(report(&source, directive, &rule.id));
        }
    }
}

fn report(source: &str, directive: &Directive, rule: &str) -> Diagnostic {
    let body = if directive.is_dangling() {
        format!("`{rule}` is suppressed here, but no code follows this directive")
    } else {
        format!("`{rule}` reports nothing here; this suppression is no longer needed")
    };
    let (start, end) = comment_deletion_span(source, directive.comment);
    Diagnostic {
        rule: "outdated-suppression",
        severity: Default::default(),
        path: Default::default(),
        range: directive.comment,
        message: ViolationData::new("outdated-suppression", body),
        fix: Some(Fix::safe(start, end, "", "Remove the suppression")),
    }
}
