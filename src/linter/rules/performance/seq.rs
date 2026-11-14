//! `seq`: `1:length(x)` is `seq_along(x)`; `1:n` is `seq_len(n)`.
//!
//! A colon range from `1` up to a length silently counts *down* when that
//! length is zero — `1:0` is `c(1L, 0L)`, not an empty sequence — so a loop
//! over `1:length(x)` runs twice on an empty `x`, with out-of-bounds indices.
//! `seq_along(x)` and `seq_len(n)` return a zero-length sequence instead. The
//! two forms agree exactly whenever the range is well-defined (length at least
//! one) and diverge only on the empty case that makes `1:length(x)` a bug, so
//! the fix is `Safe`: the divergence *is* the fix.
//!
//! The rule fires on a `:` binary expression whose left operand is the literal
//! `1` (or `1L`) and whose right operand is either
//!
//! - a single-positional-argument call to `length` (-> `seq_along(x)`) or to
//!   one of the dimension counts `nrow`/`ncol`/`NROW`/`NCOL`
//!   (-> `seq_len(nrow(x))`), **namespace-confirmed** (`ns`) to resolve to base
//!   R — a redefinition is left alone; or
//! - a bare name (-> `seq_len(n)`), excluding the special constants (`NA`,
//!   `TRUE`, `T`, ...) and dot-dot placeholders, which are not length
//!   variables.
//!
//! Literal ranges (`1:10`), ranges not starting at `1`, and computed right
//! operands (`1:(n - 1)`, `1:foo(x)`) are left alone — nothing says they are
//! lengths. The replacement is a call (a primary, binding tighter than the `:`
//! expression it replaces), so no precedence guard is needed. The fix is
//! withheld when a comment outside the preserved operand would be dropped.
//!
//! Also flag `seq(length(x))` when both calls resolve to base R and each has
//! exactly one positional argument. This fix is `Unsafe`: `length` can return
//! a classed count that dispatches a `seq` method, unlike `seq_along(x)`.
//! Withhold it if the introduced `seq_along` could be shadowed. Arbitrary
//! `seq(x)`, qualified/named calls, and pipe stages are left alone.
//!
//! The message quotes the range and its rewrite as they appear in the file, so
//! it reads as advice about *this* code rather than about a stand-in shape.

use rowan::ast::AstNode as _;

use crate::ast::{AstToken as _, CallExpr, Ident};
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct Seq;

const EXAMPLES: &[Example] = &[
    Example {
        caption: "Ranges over a vector's indices and up to a count:",
        source: "for (i in 1:length(x)) print(x[i])\nfor (j in 1:n) f(j)\n",
    },
    Example {
        caption: "A seq call over a possibly-empty vector's length:",
        source: "seq(length(numeric(0)))\n",
    },
];

/// The callees whose count the range runs up to. `length` gets the dedicated
/// `seq_along(x)`; the dimension counts keep the call inside `seq_len(...)`.
const DIM_CALLEES: &[&str] = &["nrow", "ncol", "NROW", "NCOL"];

/// Render a snippet for the diagnostic text. The operands are arbitrary code,
/// so a bound wrapped across lines would otherwise put a newline in the
/// message; the fix keeps its own copy verbatim.
fn compact(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Rule for Seq {
    fn id(&self) -> &'static str {
        "seq"
    }

    fn description(&self) -> &'static str {
        "Flag colon ranges from `1` up to a length—`1:length(x)`, `1:nrow(x)`, \
         or `1:n`—which silently count *down* when that length is zero \
         (`1:0` is `c(1L, 0L)`, not an empty sequence), a classic off-by-one \
         bug for loops over possibly-empty input. `seq_along(x)` and \
         `seq_len(n)` return a zero-length sequence instead, and agree with \
         the colon form everywhere else.\n\nThe `length`/`nrow`/`ncol`/`NROW`/\
         `NCOL` forms fire only when the callee resolves to base R; a \
         redefinition is left alone. Literal ranges (`1:10`) and computed \
         bounds (`1:(n - 1)`) are not flagged.\n\n\
         Also flag `seq(length(x))` when both callees resolve to base R and \
         each call has exactly one positional argument. For ordinary vectors, \
         `seq_along(x)` agrees on nonempty input and returns `integer(0)` on \
         empty input instead of `c(1L, 0L)`. This fix is unsafe because a \
         custom `length` method can return a classed count that dispatches a \
         `seq` method. It requires `--unsafe-fixes` and is withheld when \
         `seq_along` could be shadowed or comments would be lost. Qualified \
         calls, named arguments, extra or missing arguments, pipe stages, and \
         arbitrary `seq(x)` are left alone."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::BINARY_EXPR, SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else {
            return;
        };
        if let Some(call) = matchers::call_named(node, "seq") {
            check_length_call(&call, ctx, sink);
            return;
        }
        let Some((lhs, op, rhs)) = matchers::binary_parts(node) else {
            return;
        };
        if op.kind() != SyntaxKind::COLON {
            return;
        }
        // The range must start at the literal `1` (or `1L`).
        let starts_at_one = lhs
            .as_token()
            .is_some_and(|t| t.kind() == SyntaxKind::INT && matches!(t.text(), "1" | "1L"));
        if !starts_at_one {
            return;
        }

        // Classify the upper bound: a bare name, or a base length-like call.
        // `preserved` is the sole operand the fix carries over verbatim.
        let (content, preserved) = if let Some(token) = rhs.as_token() {
            let is_name =
                Ident::cast(token.clone()).is_some_and(|i| i.constant().is_none() && !i.is_dots());
            if !is_name {
                return;
            }
            (format!("seq_len({})", token.text()), rhs.text_range())
        } else if let Some(call) = rhs.as_node().and_then(|n| CallExpr::cast(n.clone())) {
            let Some(name) = call.callee_name() else {
                return;
            };
            let Some(arg) = matchers::sole_positional(&call) else {
                return;
            };
            // Namespace-confirm the bound really is base R's count; otherwise
            // the rewrite would change which function runs.
            if !ctx.resolves_to_base(&call) {
                return;
            }
            match name.as_str() {
                "length" => (
                    format!("seq_along({})", matchers::element_text(&arg)),
                    arg.text_range(),
                ),
                _ if DIM_CALLEES.contains(&name.as_str()) => (
                    format!("seq_len({})", call.syntax().text()),
                    call.syntax().text_range(),
                ),
                _ => return,
            }
        } else {
            return;
        };

        // Name the range that is actually there — a stand-in shape would tell a
        // `1:NCOL(a)` reader about `1:nrow(x)`.
        let shape = compact(&format!(
            "{}:{}",
            matchers::element_text(&lhs),
            matchers::element_text(&rhs)
        ));
        let replacement = compact(&content);

        let r = node.text_range();
        // The fix preserves only the upper bound's operand. A comment anywhere
        // else in the range expression would be dropped, so withhold it there.
        let drops_comment = node
            .descendants_with_tokens()
            .any(|e| e.kind() == SyntaxKind::COMMENT && !preserved.contains_range(e.text_range()));
        let fix = (!drops_comment).then(|| {
            Fix::safe(
                usize::from(r.start()),
                usize::from(r.end()),
                content,
                format!("Replace `{shape}` with `{replacement}`"),
            )
        });

        sink.push(Diagnostic {
            rule: "seq",
            severity: Default::default(),
            path: Default::default(),
            range: r,
            message: ViolationData::new(
                "seq",
                format!(
                    "`{shape}` counts down (`1:0`) when the length is zero; `{replacement}` handles empty input"
                ),
            )
            .with_suggestion(format!("Use `{replacement}`.")),
            fix,
        });
    }
}

/// Keep the single-argument match strict: `sole_positional` ignores empty
/// slots, but `seq(length(x),)` is outside this rule's narrow call shape.
/// Comments can create value-less ARG nodes, so count separators, not nodes.
fn single_argument(call: &CallExpr) -> Option<SyntaxElement> {
    if call
        .arg_list()?
        .syntax()
        .children_with_tokens()
        .any(|el| el.kind() == SyntaxKind::COMMA)
    {
        return None;
    }
    let arg = matchers::sole_positional(call)?;
    if arg
        .as_token()
        .and_then(|token| Ident::cast(token.clone()))
        .is_some_and(|ident| ident.is_dots())
    {
        return None;
    }
    Some(arg)
}

fn check_length_call(call: &CallExpr, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
    // A pipe stage receives an implicit argument, so the written arguments do
    // not establish that seq is called with just a length.
    if call.syntax().parent().is_some_and(|parent| {
        matchers::binary_parts(&parent).is_some_and(|(_, op, rhs)| {
            rhs.as_node() == Some(call.syntax())
                && (op.kind() == SyntaxKind::PIPE
                    || (op.kind() == SyntaxKind::USER_OP
                        && matches!(op.text(), "%>%" | "%T>%" | "%<>%" | "%$%")))
        })
    }) {
        return;
    }
    let Some(inner) = single_argument(call)
        .and_then(SyntaxElement::into_node)
        .and_then(|node| matchers::call_named(&node, "length"))
    else {
        return;
    };
    let Some(arg) = single_argument(&inner) else {
        return;
    };
    if !ctx.resolves_to_base(call) || !ctx.resolves_to_base(&inner) {
        return;
    }

    let content = format!("seq_along({})", matchers::element_text(&arg));
    let shape = compact(&call.syntax().text().to_string());
    let replacement = compact(&content);
    let range = call.syntax().text_range();
    let drops_comment = call.syntax().descendants_with_tokens().any(|el| {
        el.kind() == SyntaxKind::COMMENT && !arg.text_range().contains_range(el.text_range())
    });
    let fix = (!drops_comment && ctx.introduced_call_resolves_to_base("seq_along")).then(|| {
        Fix::unsafe_(
            usize::from(range.start()),
            usize::from(range.end()),
            content,
            format!("Replace `{shape}` with `{replacement}`"),
        )
    });
    sink.push(Diagnostic {
        rule: "seq",
        severity: Default::default(),
        path: Default::default(),
        range,
        message: ViolationData::new(
            "seq",
            format!(
                "`{shape}` counts down (`1:0`) when the length is zero; `{replacement}` handles empty input"
            ),
        )
        .with_suggestion(format!("Use `{replacement}`.")),
        fix,
    });
}
