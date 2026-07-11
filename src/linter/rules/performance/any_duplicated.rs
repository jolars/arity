//! `any-duplicated`: `any(duplicated(x))` is the purpose-built
//! `anyDuplicated(x) > 0`.
//!
//! `anyDuplicated(x)` returns the index of the first duplicated element (`0` when
//! there are none) and stops scanning as soon as it finds one, so it never
//! materializes the full logical vector that `duplicated(x)` builds. Comparing it
//! to `0` recovers the boolean: `anyDuplicated(x) > 0` is `TRUE` exactly when
//! `any(duplicated(x))` is — both are plain `TRUE`/`FALSE` (`duplicated()` yields
//! a logical vector free of `NA`).
//!
//! The rule fires only on the clean shape — `any` with a single positional
//! argument that is `duplicated` with a single positional argument — and is
//! **namespace-confirmed** (`ns`): both callees must resolve to base R (not a
//! local redefinition, namespace-qualified, or package-masked name), or the
//! rewrite would be wrong.
//!
//! Unlike `anyNA(...)`, the replacement `anyDuplicated(...) > 0` is a
//! *comparison*, which binds looser than arithmetic, indexing, `$`/`@`, and the
//! like. Spliced bare into a parent that binds tighter than a comparison it would
//! misparse (`any(duplicated(x)) + 1` → `anyDuplicated(x) > 0 + 1`), so the fix
//! is withheld in such a context (see [`matchers::is_safe_splice_context`]); the
//! finding is still reported. The fix is likewise withheld when a comment
//! outside the preserved inner argument would be dropped.

use rowan::ast::AstNode as _;

use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct AnyDuplicated;

const EXAMPLES: &[Example] = &[Example {
    caption: "Testing for any duplicate value:",
    source: "if (any(duplicated(x))) stop()\n",
}];

impl Rule for AnyDuplicated {
    fn id(&self) -> &'static str {
        "any-duplicated"
    }

    fn description(&self) -> &'static str {
        "Flag `any(duplicated(x))`, which is the purpose-built `anyDuplicated(x) \
         > 0`—faster (it short-circuits and builds no intermediate logical \
         vector) and clearer.\n\nThe rule fires only on the clean \
         single-argument shape and only when both `any` and `duplicated` resolve \
         to base R; a local redefinition of either is left alone. Because the \
         replacement is a comparison, the fix is withheld in a context that binds \
         tighter than a comparison, where the bare rewrite would need parentheses."
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
        let Some(outer_arg) = matchers::sole_positional(&call) else {
            return;
        };
        // …which is a call to `duplicated`…
        let Some(inner_node) = outer_arg.as_node() else {
            return;
        };
        let Some(inner) = matchers::call_named(inner_node, "duplicated") else {
            return;
        };
        // …with exactly one positional argument of its own.
        let Some(arg) = matchers::sole_positional(&inner) else {
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
        // The replacement is a comparison, which binds looser than the call it
        // replaces; only splice it bare where a comparison can sit unparenthesized.
        let fix = (!drops_comment && matchers::is_safe_splice_context(call.syntax())).then(|| {
            Fix::safe(
                usize::from(r.start()),
                usize::from(r.end()),
                format!("anyDuplicated({}) > 0", matchers::element_text(&arg)),
                "Replace `any(duplicated(x))` with `anyDuplicated(x) > 0`",
            )
        });

        sink.push(Diagnostic {
            rule: "any-duplicated",
            severity: Default::default(),
            path: Default::default(),
            range: r,
            message: ViolationData::new(
                "any-duplicated",
                "`any(duplicated(x))` is the faster, clearer `anyDuplicated(x) > 0`",
            )
            .with_suggestion("Use `anyDuplicated(x) > 0`."),
            fix,
        });
    }
}
