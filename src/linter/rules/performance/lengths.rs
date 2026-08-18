//! `lengths`: `sapply(x, length)` is the purpose-built `lengths(x)`.
//!
//! `lengths(x)` computes every element's length in C without dispatching an R
//! call per element, so it is both faster and clearer. The two agree on the
//! result: both return an integer vector and both keep `x`'s names by default
//! (`sapply`'s `USE.NAMES = TRUE`, `lengths`' `use.names = TRUE`).
//!
//! The rule fires only on the clean shape — `sapply` with exactly two
//! positional arguments where the second is the bare name `length` — and is
//! **namespace-confirmed** (`ns`): `sapply` must resolve to base R and the
//! `length` read must not be locally rebound or package-masked, or the rewrite
//! would change which function runs. The replacement is a call (`lengths(...)`),
//! a primary like the `sapply(...)` it replaces, so no precedence guard is
//! needed and the fix is `Safe`. It is withheld when a comment outside the
//! preserved first argument would be dropped.

use rowan::ast::AstNode as _;

use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct Lengths;

const EXAMPLES: &[Example] = &[Example {
    caption: "Per-element lengths of a list:",
    source: "n <- sapply(x, length)\n",
}];

impl Rule for Lengths {
    fn id(&self) -> &'static str {
        "lengths"
    }

    fn description(&self) -> &'static str {
        "Flag `sapply(x, length)`, which is the purpose-built `lengths(x)`—\
         faster (a single C pass instead of an R call per element) and clearer. \
         Both return an integer vector and keep `x`'s names by default.\n\nThe \
         rule fires only on the clean two-positional-argument shape and only \
         when `sapply` resolves to base R and `length` is not locally rebound; \
         a redefinition of either is left alone."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else {
            return;
        };
        let Some(call) = matchers::call_named(node, "sapply") else {
            return;
        };
        let valued: Vec<_> = matchers::args(&call)
            .into_iter()
            .filter(|a| a.value.is_some())
            .collect();
        if valued.len() != 2 || valued.iter().any(|a| a.name.is_some()) {
            return;
        }
        let data = valued[0].value.clone().expect("value-bearing by filter");
        let fun = valued[1].value.clone().expect("value-bearing by filter");
        // …where FUN is the bare name `length` (not a string, call, or lambda).
        let Some(fun_token) = fun.as_token() else {
            return;
        };
        if fun_token.kind() != SyntaxKind::IDENT || fun_token.text() != "length" {
            return;
        }

        // Namespace-confirm both names are base R; otherwise the rewrite would
        // change which function runs.
        if !ctx.resolves_to_base(&call) || !ctx.read_resolves_to_base(fun_token) {
            return;
        }

        let r = call.syntax().text_range();
        // The fix preserves only the first argument's text. A comment anywhere
        // else inside `sapply(...)` would be dropped, so withhold the fix there.
        let data_range = data.text_range();
        let drops_comment = call
            .syntax()
            .descendants_with_tokens()
            .any(|e| e.kind() == SyntaxKind::COMMENT && !data_range.contains_range(e.text_range()));
        let fix = (!drops_comment).then(|| {
            Fix::safe(
                usize::from(r.start()),
                usize::from(r.end()),
                format!("lengths({})", matchers::element_text(&data)),
                "Replace `sapply(x, length)` with `lengths(x)`",
            )
        });

        sink.push(Diagnostic {
            rule: "lengths",
            severity: Default::default(),
            path: Default::default(),
            range: r,
            message: ViolationData::new(
                "lengths",
                "`sapply(x, length)` is the faster, clearer `lengths(x)`",
            )
            .with_suggestion("Use `lengths(x)`."),
            fix,
        });
    }
}
