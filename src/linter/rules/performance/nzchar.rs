//! `nzchar`: comparing `nchar(x)` against zero is the purpose-built
//! `nzchar(x)`.
//!
//! `nzchar(x)` tests "is a non-empty string" directly: it is faster than
//! computing every element's length just to compare it to a bound, and states
//! the intent. All the emptiness spellings collapse — `nchar(x) > 0`,
//! `nchar(x) >= 1`, and `nchar(x) != 0` are `nzchar(x)`; `nchar(x) == 0`,
//! `nchar(x) <= 0`, and `nchar(x) < 1` are `!nzchar(x)` — the mirrored
//! literal-first forms (`0 < nchar(x)`, …) included.
//!
//! The rule fires only on the clean shape: `nchar` with a single positional
//! argument (a `type`, `allowNA`, or `keepNA` argument changes what the count
//! means) compared against a literal `0`/`1`, and is **namespace-confirmed**
//! (`ns`): the `nchar` callee must resolve to base R.
//!
//! **The fix is `Unsafe`.** On an `NA_character_` element `nchar` yields `NA`
//! under its default `keepNA`, so the comparison propagates `NA`, while
//! `nzchar` yields `TRUE` (exact equivalence would need
//! `nzchar(x, keepNA = TRUE)`). That is a real runtime difference, so the fix
//! is applied only under `--unsafe-fixes`; the finding is always reported. The
//! positive replacement is a call (a primary) replacing a comparison, so it
//! never needs a precedence guard; the negated `!nzchar(x)` binds looser than
//! the comparison it replaces, so that form is withheld in a context that binds
//! tighter than `!` (see [`matchers::is_safe_splice_context`]). Both forms are
//! withheld when a comment outside the preserved argument would be dropped.

use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct Nzchar;

const EXAMPLES: &[Example] = &[Example {
    caption: "Testing for non-empty strings:",
    source: "keep <- x[nchar(x) > 0]\n",
}];

impl Rule for Nzchar {
    fn id(&self) -> &'static str {
        "nzchar"
    }

    fn description(&self) -> &'static str {
        "Flag comparisons of `nchar(x)` against zero—`nchar(x) > 0`, \
         `nchar(x) >= 1`, `nchar(x) != 0`, and their mirrored and negated \
         spellings—which are the purpose-built `nzchar(x)` (or `!nzchar(x)` \
         for the empty test): faster, and it states the intent \
         directly.\n\nThe rule fires only on the clean single-argument shape \
         and only when `nchar` resolves to base R; a local redefinition is \
         left alone. The fix is **unsafe**: on `NA_character_` input `nzchar` \
         yields `TRUE` where the `nchar` comparison yields `NA` (exact \
         equivalence would need `nzchar(x, keepNA = TRUE)`)."
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
        // One side is the `nchar(...)` call, the other a literal `0`/`1`.
        let (call_el, lit_el, nchar_on_left) = if literal_bound(&rhs).is_some() {
            (&lhs, &rhs, true)
        } else if literal_bound(&lhs).is_some() {
            (&rhs, &lhs, false)
        } else {
            return;
        };
        let Some(call) = call_el
            .as_node()
            .and_then(|n| matchers::call_named(n, "nchar"))
        else {
            return;
        };
        // `nchar` must carry exactly one positional argument: `type`, `allowNA`,
        // and `keepNA` all change what the count means for the edge cases.
        let Some(arg) = matchers::sole_positional(&call) else {
            return;
        };
        let Some(negate) = classify(op.kind(), literal_bound(lit_el).unwrap(), nchar_on_left)
        else {
            return;
        };

        // Namespace-confirm `nchar` is base R; otherwise the comparison is not a
        // string-length test at all.
        if !ctx.resolves_to_base(&call) {
            return;
        }

        let r = node.text_range();
        // The fix preserves only the inner argument's text. A comment anywhere
        // else inside the comparison would be dropped, so withhold the fix there.
        let arg_range = arg.text_range();
        let drops_comment = node
            .descendants_with_tokens()
            .any(|e| e.kind() == SyntaxKind::COMMENT && !arg_range.contains_range(e.text_range()));
        // `nzchar(...)` is a primary and splices anywhere a comparison sat, but
        // the negated `!nzchar(...)` binds looser and needs a loose context.
        let context_ok = !negate || matchers::is_safe_splice_context(node);
        let fix = (!drops_comment && context_ok).then(|| {
            let bang = if negate { "!" } else { "" };
            Fix::unsafe_(
                usize::from(r.start()),
                usize::from(r.end()),
                format!("{bang}nzchar({})", matchers::element_text(&arg)),
                format!("Replace the `nchar()` comparison with `{bang}nzchar(...)`"),
            )
        });

        sink.push(Diagnostic {
            rule: "nzchar",
            severity: Default::default(),
            path: Default::default(),
            range: r,
            message: ViolationData::new(
                "nzchar",
                "comparing `nchar()` to zero is the faster, clearer `nzchar()`",
            )
            .with_suggestion("Use `nzchar(x)` for non-empty, `!nzchar(x)` for empty."),
            fix,
        });
    }
}

/// The numeric bound an emptiness comparison can use: `Some(0)` or `Some(1)`
/// for a bare integer/double literal spelling zero or one, else `None`.
fn literal_bound(el: &SyntaxElement) -> Option<u8> {
    let tok = el.as_token()?;
    if !matches!(tok.kind(), SyntaxKind::INT | SyntaxKind::FLOAT) {
        return None;
    }
    match tok.text() {
        "0" | "0L" => Some(0),
        "1" | "1L" => Some(1),
        _ => None,
    }
}

/// Whether the comparison is an emptiness test, and if so whether it tests
/// *empty* (`Some(true)`, rewrite negates) or *non-empty* (`Some(false)`).
/// `lit` is the literal bound; `nchar_on_left` orients the operator (the
/// mirrored `0 < nchar(x)` flips to `nchar(x) > 0`). `nchar()` is never
/// negative, so `!= 0` and `> 0` coincide, as do `== 0`, `<= 0`, and `< 1`.
fn classify(op: SyntaxKind, lit: u8, nchar_on_left: bool) -> Option<bool> {
    use SyntaxKind::*;
    let op = if nchar_on_left {
        op
    } else {
        match op {
            LESS_THAN => GREATER_THAN,
            LESS_THAN_OR_EQUAL => GREATER_THAN_OR_EQUAL,
            GREATER_THAN => LESS_THAN,
            GREATER_THAN_OR_EQUAL => LESS_THAN_OR_EQUAL,
            other => other,
        }
    };
    match (op, lit) {
        (GREATER_THAN, 0) | (GREATER_THAN_OR_EQUAL, 1) | (NOT_EQUAL, 0) => Some(false),
        (EQUAL2, 0) | (LESS_THAN_OR_EQUAL, 0) | (LESS_THAN, 1) => Some(true),
        _ => None,
    }
}
