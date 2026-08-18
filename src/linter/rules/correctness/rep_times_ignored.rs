//! `rep-times-ignored`: in base `rep()`, a supplied `length.out` normally
//! determines the result length and makes `times` ineffective. Both arguments
//! must be explicitly named, and the callee must resolve to base R. The rule is
//! report-only: when `length.out` is invalid or `NA`, `times` can still affect
//! behavior, so deleting it is not safe for arbitrary static expressions.

use rowan::ast::AstNode as _;

use crate::ast::CallExpr;
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct RepTimesIgnored;

impl Rule for RepTimesIgnored {
    fn id(&self) -> &'static str {
        "rep-times-ignored"
    }

    fn description(&self) -> &'static str {
        "Flag a base `rep()` call that supplies both `times` and `length.out`. `length.out` normally determines the result length, making `times` ineffective. There is no autofix because `times` can still matter when `length.out` is invalid or `NA`."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "`length.out` normally overrides `times`:",
            source: "rep(x, times = 2, length.out = 10)\n",
        }]
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(call) = el.as_node().cloned().and_then(CallExpr::cast) else {
            return;
        };
        if matchers::callee_name(&call).as_deref() != Some("rep")
            || !ctx.resolves_to_base(&call)
            || matchers::named_arg(&call, "length.out").is_none()
        {
            return;
        }
        let Some(times) = matchers::args(&call)
            .into_iter()
            .find(|arg| arg.name.as_deref() == Some("times"))
            .and_then(|arg| arg.name_token)
        else {
            return;
        };

        sink.push(Diagnostic {
            rule: "rep-times-ignored",
            severity: Default::default(),
            path: Default::default(),
            range: times.text_range(),
            message: ViolationData::new(
                "rep-times-ignored",
                "`times` is normally ignored when `length.out` is supplied",
            )
            .with_suggestion("Remove `times` after confirming `length.out` is always valid."),
            fix: None,
        });
    }
}
