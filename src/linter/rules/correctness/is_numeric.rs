//! `is-numeric`: `is.numeric(x) || is.integer(x)` is just `is.numeric(x)`.
//!
//! `is.numeric()` already returns `TRUE` for integer vectors (its default
//! method tests whether the *type* is double or integer), so `||`-ing it with
//! `is.integer()` adds nothing — the disjunction betrays the common
//! misreading that `is.numeric()` means "double only". The `|` spelling is the
//! same redundancy, vectorized.
//!
//! The rule fires only on the clean shape: both operands are
//! single-positional-argument calls to `is.numeric`/`is.integer` (one each,
//! either order) whose arguments are textually identical, and is
//! **namespace-confirmed** (`ns`): both callees must resolve to base R.
//!
//! The fix is `Safe`: for every input where the two tests are the base-R type
//! tests, `is.integer(x)` implies `is.numeric(x)`, except classed integer
//! vectors such as factors — where `is.integer()` answering `TRUE` is itself
//! the trap, i.e. the very confusion this rule exists to flag. The replacement
//! is a call (a primary) replacing a binary expression, so no precedence guard
//! is needed; the fix is withheld when a comment outside the preserved
//! argument would be dropped.

use crate::ast::CallExpr;
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct IsNumeric;

const EXAMPLES: &[Example] = &[Example {
    caption: "Testing for a numeric vector:",
    source: "if (is.numeric(x) || is.integer(x)) mean(x)\n",
}];

impl Rule for IsNumeric {
    fn id(&self) -> &'static str {
        "is-numeric"
    }

    fn description(&self) -> &'static str {
        "Flag `is.numeric(x) || is.integer(x)` (and the vectorized `|` \
         spelling), which is just `is.numeric(x)`: `is.numeric()` already \
         returns `TRUE` for integer vectors, so the disjunction adds nothing \
         and suggests a misreading of what `is.numeric()` tests.\n\nThe rule \
         fires only when both operands are single-argument calls on the same \
         argument and both callees resolve to base R; a local redefinition of \
         either is left alone."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::BINARY_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else {
            return;
        };
        let Some((lhs, op, rhs)) = matchers::binary_parts(node) else {
            return;
        };
        if !matches!(op.kind(), SyntaxKind::OR | SyntaxKind::OR2) {
            return;
        }
        // One operand is `is.numeric(...)`, the other `is.integer(...)`, in
        // either order — each with exactly one positional argument.
        let (numeric, integer) = match (type_test(&lhs), type_test(&rhs)) {
            (Some((a, a_arg)), Some((b, b_arg))) if a.0 != b.0 => {
                if a.0 == "is.numeric" {
                    ((a.1, a_arg), (b.1, b_arg))
                } else {
                    ((b.1, b_arg), (a.1, a_arg))
                }
            }
            _ => return,
        };
        // Both tests must inspect the same value; textual identity is the
        // conservative stand-in (a whitespace difference simply doesn't fire).
        if matchers::element_text(&numeric.1) != matchers::element_text(&integer.1) {
            return;
        }

        // Namespace-confirm both callees are base R; otherwise the disjunction
        // is not the redundant base-R type test.
        if !ctx.resolves_to_base(&numeric.0) || !ctx.resolves_to_base(&integer.0) {
            return;
        }

        let r = node.text_range();
        // The fix preserves only the `is.numeric` call's argument. A comment
        // anywhere else inside the disjunction would be dropped, so withhold
        // the fix there.
        let arg_range = numeric.1.text_range();
        let drops_comment = node
            .descendants_with_tokens()
            .any(|e| e.kind() == SyntaxKind::COMMENT && !arg_range.contains_range(e.text_range()));
        let fix = (!drops_comment).then(|| {
            Fix::safe(
                usize::from(r.start()),
                usize::from(r.end()),
                format!("is.numeric({})", matchers::element_text(&numeric.1)),
                "Replace the disjunction with `is.numeric(x)`",
            )
        });

        sink.push(Diagnostic {
            rule: "is-numeric",
            severity: Default::default(),
            path: Default::default(),
            range: r,
            message: ViolationData::new(
                "is-numeric",
                "`is.numeric(x) || is.integer(x)` is redundant—`is.numeric()` is already \
                 `TRUE` for integer vectors",
            )
            .with_suggestion("Use `is.numeric(x)`."),
            fix,
        });
    }
}

/// If `el` is an `is.numeric(...)` or `is.integer(...)` call with exactly one
/// positional, value-bearing argument: its name, the call, and that argument.
fn type_test(el: &SyntaxElement) -> Option<((&'static str, CallExpr), SyntaxElement)> {
    let node = el.as_node()?;
    for name in ["is.numeric", "is.integer"] {
        if let Some(call) = matchers::call_named(node, name) {
            let arg = matchers::sole_positional(&call)?;
            return Some(((name, call), arg));
        }
    }
    None
}
