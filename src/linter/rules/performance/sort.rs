//! `sort`: taking the first element of a full sort is the purpose-built
//! `min(x)` / `max(x)`.
//!
//! `sort(x)[1]` sorts the entire vector — `O(n log n)` and a full copy — just
//! to read one extreme, which `min(x)` finds in a single `O(n)` scan; with
//! `decreasing = TRUE` the first element is the maximum, so the rewrite is
//! `max(x)`. The named function also states the intent directly.
//!
//! The rule fires only on the clean shape: a `[`-subset whose sole subscript is
//! the literal `1`/`1L` over a `sort` call carrying one positional argument and
//! at most a literal `decreasing = TRUE`/`FALSE` (any other argument —
//! `na.last`, `method`, `partial`, a positional or computed `decreasing` —
//! changes what the first element means). It is **namespace-confirmed** (`ns`):
//! the `sort` callee must resolve to base R.
//!
//! **The fix is `Unsafe`.** `sort` drops `NA`s under its default
//! `na.last = NA`, so `sort(x)[1]` skips them, while `min(x)` propagates `NA`
//! (equivalence would need `min(x, na.rm = TRUE)`); on a zero-length vector
//! `sort(x)[1]` is `NA` while `min(x)` warns and yields `Inf`. Those are real
//! runtime differences, so the fix is applied only under `--unsafe-fixes`; the
//! finding is always reported. The replacement is a call (a primary) replacing
//! a subset (also a primary), so it never needs a precedence guard; the fix is
//! withheld when a comment outside the preserved argument would be dropped.

use rowan::ast::AstNode as _;

use crate::ast::SubsetExpr;
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct Sort;

const EXAMPLES: &[Example] = &[Example {
    caption: "Reading one extreme off a full sort:",
    source: "smallest <- sort(x)[1]\nlargest <- sort(x, decreasing = TRUE)[1]\n",
}];

impl Rule for Sort {
    fn id(&self) -> &'static str {
        "sort"
    }

    fn description(&self) -> &'static str {
        "Flag `sort(x)[1]`, which sorts the whole vector just to read one \
         extreme — the purpose-built `min(x)` (or `max(x)` for \
         `decreasing = TRUE`) finds it in a single pass and states the intent \
         directly.\n\nThe rule fires only on the clean shape — a `[1]` subset \
         of a `sort` call with one positional argument and at most a literal \
         `decreasing` flag — and only when `sort` resolves to base R; a local \
         redefinition is left alone. The fix is **unsafe**: `sort` drops `NA`s \
         by default while `min`/`max` propagate them (exact equivalence would \
         need `na.rm = TRUE`), and on an empty vector `sort(x)[1]` is `NA` \
         while `min(x)` warns and yields `Inf`."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::SUBSET_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else {
            return;
        };
        let Some(subset) = SubsetExpr::cast(node.clone()) else {
            return;
        };
        // The sole subscript is the literal `1`/`1L` — the first element.
        if !sole_subscript_is_one(&subset) {
            return;
        }
        // The base is a `sort(...)` call…
        let Some(call) = subset
            .base()
            .and_then(|b| b.into_node())
            .and_then(|n| matchers::call_named(&n, "sort"))
        else {
            return;
        };
        // …with one positional argument and at most a literal `decreasing`
        // flag. Anything else (`na.last`, `method`, `partial`, a positional or
        // computed `decreasing`) changes what the first element means.
        let Some((arg, decreasing)) = sort_shape(&call) else {
            return;
        };

        // Namespace-confirm `sort` is base R; otherwise the subset is not a
        // minimum at all.
        if !ctx.resolves_to_base(&call) {
            return;
        }

        let replacement = if decreasing { "max" } else { "min" };
        let r = subset.syntax().text_range();
        // The fix preserves only the sorted argument's text. A comment anywhere
        // else inside the subset would be dropped, so withhold the fix there.
        let arg_range = arg.text_range();
        let drops_comment = subset
            .syntax()
            .descendants_with_tokens()
            .any(|e| e.kind() == SyntaxKind::COMMENT && !arg_range.contains_range(e.text_range()));
        let fix = (!drops_comment).then(|| {
            Fix::unsafe_(
                usize::from(r.start()),
                usize::from(r.end()),
                format!("{replacement}({})", matchers::element_text(&arg)),
                format!("Replace the `sort(...)[1]` subset with `{replacement}(...)`"),
            )
        });

        sink.push(Diagnostic {
            rule: "sort",
            severity: Default::default(),
            path: Default::default(),
            range: r,
            message: ViolationData::new(
                "sort",
                format!(
                    "`sort(x)[1]` sorts everything to read one extreme — use `{replacement}(x)`"
                ),
            )
            .with_suggestion(format!("Use `{replacement}(x)`.")),
            fix,
        });
    }
}

/// Whether the subset carries exactly one value-bearing, positional subscript
/// that is the literal `1`/`1L`.
fn sole_subscript_is_one(subset: &SubsetExpr) -> bool {
    let mut valued = subset.args().filter(|a| a.value().is_some());
    let Some(only) = valued.next() else {
        return false;
    };
    if valued.next().is_some() || only.is_named() {
        return false;
    }
    only.value().and_then(|v| v.into_token()).is_some_and(|t| {
        matches!(t.kind(), SyntaxKind::INT | SyntaxKind::FLOAT) && matches!(t.text(), "1" | "1L")
    })
}

/// Match the `sort` call's argument list: one positional argument (returned),
/// plus optionally a literal `decreasing = TRUE`/`FALSE` (returned as the
/// flag). `None` for any other shape.
fn sort_shape(call: &crate::ast::CallExpr) -> Option<(SyntaxElement, bool)> {
    let mut positional = None;
    let mut decreasing = false;
    for arg in matchers::args(call) {
        let Some(value) = arg.value else {
            continue; // a stray comment parses as a value-less `ARG`
        };
        match arg.name.as_deref() {
            None => {
                if positional.replace(value).is_some() {
                    return None; // a second positional — not the clean shape
                }
            }
            Some("decreasing") => {
                if matchers::is_true(&value) {
                    decreasing = true;
                } else if !matchers::is_false(&value) {
                    return None; // computed `decreasing` — direction unknown
                }
            }
            Some(_) => return None, // `na.last`, `method`, `partial`, …
        }
    }
    positional.map(|arg| (arg, decreasing))
}
