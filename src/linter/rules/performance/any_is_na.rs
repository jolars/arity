//! `any-is-na`: `any(is.na(x))` is the purpose-built `anyNA(x)`.
//!
//! `anyNA(x)` short-circuits on the first `NA` and never materializes the full
//! logical vector that `is.na(x)` builds, so it is both faster and clearer. The
//! two are equivalent: `is.na()` returns a logical vector that itself contains
//! no `NA`, so `any(is.na(x))` is always `TRUE`/`FALSE` (never `NA`), exactly
//! like `anyNA(x)`.
//!
//! The rule fires only on the clean shape — `any` with a single positional
//! argument that is `is.na` with a single positional argument — and is
//! **namespace-confirmed** (`ns`): both callees must resolve to base R (not a
//! local redefinition, namespace-qualified, or package-masked name), or the
//! rewrite would be wrong. The replacement is a call (`anyNA(...)`), a primary
//! like the `any(...)` it replaces, so no precedence guard is needed and the fix
//! is `Safe`. It is withheld when a comment outside the preserved inner argument
//! would be dropped.

use rowan::ast::AstNode as _;

use crate::ast::CallExpr;
use crate::linter::diagnostic::{Diagnostic, Fix, Severity, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct AnyIsNa;

const EXAMPLES: &[Example] = &[Example {
    caption: "Testing for any missing value:",
    source: "if (any(is.na(x))) stop()\n",
}];

impl Rule for AnyIsNa {
    fn id(&self) -> &'static str {
        "any-is-na"
    }

    fn description(&self) -> &'static str {
        "Flag `any(is.na(x))`, which is the purpose-built `anyNA(x)`—faster \
         (it short-circuits and builds no intermediate logical vector) and \
         clearer.\n\nThe rule fires only on the clean single-argument shape and \
         only when both `any` and `is.na` resolve to base R; a local \
         redefinition of either is left alone."
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
        let Some(call) = matchers::call_named(node, "any") else {
            return;
        };
        // `any` must carry exactly one positional, value-bearing argument (a
        // stray comment parses as a value-less `ARG`, so match on value-bearing
        // args and let the comment-withholding check below handle it)…
        let Some(outer_arg) = sole_positional(&call) else {
            return;
        };
        // …which is a call to `is.na`…
        let Some(inner_node) = outer_arg.as_node() else {
            return;
        };
        let Some(inner) = matchers::call_named(inner_node, "is.na") else {
            return;
        };
        // …with exactly one positional argument of its own.
        let Some(arg) = sole_positional(&inner) else {
            return;
        };

        // Namespace-confirm both callees are base R; otherwise the rewrite would
        // change which function runs.
        if !ctx.resolves_to_base(&call) || !ctx.resolves_to_base(&inner) {
            return;
        }

        let r = call.syntax().text_range();
        // The fix preserves only the inner argument's text. A comment anywhere
        // else inside `any(...)` would be dropped, so withhold the fix there.
        let arg_range = arg.text_range();
        let drops_comment = call
            .syntax()
            .descendants_with_tokens()
            .any(|e| e.kind() == SyntaxKind::COMMENT && !arg_range.contains_range(e.text_range()));
        let fix = (!drops_comment).then(|| {
            Fix::safe(
                usize::from(r.start()),
                usize::from(r.end()),
                format!("anyNA({})", matchers::element_text(&arg)),
                "Replace `any(is.na(x))` with `anyNA(x)`",
            )
        });

        sink.push(Diagnostic {
            rule: "any-is-na",
            severity: Severity::Warning,
            path: Default::default(),
            range: r,
            message: ViolationData::new(
                "any-is-na",
                "`any(is.na(x))` is the faster, clearer `anyNA(x)`",
            )
            .with_suggestion("Use `anyNA(x)`."),
            fix,
        });
    }
}

/// The value of `call`'s sole positional argument, or `None` unless it has
/// exactly one value-bearing argument and that argument is positional. A stray
/// comment parses as a value-less `ARG`, so it is ignored here (the caller
/// withholds the fix on a comment that would be dropped) rather than counted as
/// a second argument.
fn sole_positional(call: &CallExpr) -> Option<SyntaxElement> {
    let mut valued = matchers::args(call)
        .into_iter()
        .filter(|a| a.value.is_some());
    let only = valued.next()?;
    if valued.next().is_some() || only.name.is_some() {
        return None;
    }
    only.value
}
