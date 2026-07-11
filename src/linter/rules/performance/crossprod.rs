//! `crossprod`: `t(x) %*% y` is `crossprod(x, y)`; `x %*% t(y)` is
//! `tcrossprod(x, y)`.
//!
//! `crossprod(x, y)` computes `t(x) %*% y` and `tcrossprod(x, y)` computes
//! `x %*% t(y)` directly through BLAS, without ever materializing the transpose
//! that `t()` builds. They are both faster and clearer, and exactly equivalent
//! to the explicit form. When both operands are the same symbol the
//! single-argument form (`crossprod(x)`, `tcrossprod(x)`) says the same thing
//! more idiomatically.
//!
//! The rule fires on a `%*%` binary expression where exactly one operand (the
//! left, preferentially) is a `t(...)` call with a single positional argument.
//! With `t()` on *both* sides the crossprod branch wins
//! (`t(x) %*% t(y)` -> `crossprod(x, t(y))`): still correct, and a partial win.
//!
//! It is **namespace-confirmed** (`ns`): the `t` callee must resolve to base R
//! (not a local redefinition, namespace-qualified, or package-masked name), or
//! the rewrite would be wrong. A local redefinition of `%*%` itself, or a
//! shadowed `crossprod`/`tcrossprod`, is the same accepted "operator/introduced
//! name" edge the other Phase 2 rules carry (see the Phase 4 hardening pass).
//!
//! The replacement is a call (a primary, binding tighter than everything), so
//! splicing it in place of the whole binary expression is safe in any parent
//! context and the fix is `Safe`. It is withheld when a comment inside the
//! expression but outside the preserved operands would be dropped.

use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct Crossprod;

const EXAMPLES: &[Example] = &[Example {
    caption: "Transposed matrix products:",
    source: "a <- t(x) %*% y\nb <- x %*% t(y)\n",
}];

/// The replacement for a matched shape: the base-R function, the human-readable
/// original shape, and the idiomatic replacement (both used only in messages).
struct Rewrite {
    fname: &'static str,
    shape: &'static str,
    replacement: &'static str,
}

const CROSSPROD: Rewrite = Rewrite {
    fname: "crossprod",
    shape: "t(x) %*% y",
    replacement: "crossprod(x, y)",
};

const TCROSSPROD: Rewrite = Rewrite {
    fname: "tcrossprod",
    shape: "x %*% t(y)",
    replacement: "tcrossprod(x, y)",
};

impl Rule for Crossprod {
    fn id(&self) -> &'static str {
        "crossprod"
    }

    fn description(&self) -> &'static str {
        "Flag `t(x) %*% y` and `x %*% t(y)`, which are the purpose-built \
         `crossprod(x, y)` and `tcrossprod(x, y)`—faster (they go straight to \
         BLAS and never materialize the transpose that `t()` builds) and \
         clearer.\n\nThe rule fires only when one operand is a single-argument \
         `t()` call and that `t` resolves to base R; a local redefinition is \
         left alone. When both operands are the same symbol the single-argument \
         form (`crossprod(x)`) is used."
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
        if op.kind() != SyntaxKind::USER_OP || op.text() != "%*%" {
            return;
        }

        // Left transpose -> `crossprod(x, y)` (== `t(x) %*% y`); else right
        // transpose -> `tcrossprod(x, y)` (== `x %*% t(y)`). `first`/`second` are
        // the replacement's arguments in order: the untransposed operand keeps
        // its side, `t()`'s argument drops the transpose.
        let (rewrite, t_call, first, second) = if let Some((call, arg)) = transposed(&lhs) {
            (&CROSSPROD, call, arg, rhs)
        } else if let Some((call, arg)) = transposed(&rhs) {
            (&TCROSSPROD, call, lhs, arg)
        } else {
            return;
        };

        // Namespace-confirm `t` is base R; otherwise the rewrite would change
        // which function runs.
        if !ctx.resolves_to_base(&t_call) {
            return;
        }

        let r = node.text_range();
        // The fix reconstructs from the two preserved operands only. A comment
        // anywhere else in the expression would be dropped, so withhold it there.
        let (first_range, second_range) = (first.text_range(), second.text_range());
        let drops_comment = node.descendants_with_tokens().any(|e| {
            e.kind() == SyntaxKind::COMMENT
                && !first_range.contains_range(e.text_range())
                && !second_range.contains_range(e.text_range())
        });

        // `t(x) %*% x` collapses to the single-argument `crossprod(x)`, but only
        // when both operands are the same bare symbol (fragile to compare beyond
        // a plain `IDENT`).
        let content = match (ident_text(&first), ident_text(&second)) {
            (Some(a), Some(b)) if a == b => format!("{}({a})", rewrite.fname),
            _ => format!(
                "{}({}, {})",
                rewrite.fname,
                matchers::element_text(&first),
                matchers::element_text(&second)
            ),
        };

        let fix = (!drops_comment).then(|| {
            Fix::safe(
                usize::from(r.start()),
                usize::from(r.end()),
                content,
                format!("Replace `{}` with `{}`", rewrite.shape, rewrite.replacement),
            )
        });

        sink.push(Diagnostic {
            rule: "crossprod",
            severity: Default::default(),
            path: Default::default(),
            range: r,
            message: ViolationData::new(
                "crossprod",
                format!(
                    "`{}` is the faster, clearer `{}`",
                    rewrite.shape, rewrite.replacement
                ),
            )
            .with_suggestion(format!("Use `{}`.", rewrite.replacement)),
            fix,
        });
    }
}

/// If `el` is a `t(...)` call with exactly one positional, value-bearing
/// argument, the call and that argument element.
fn transposed(el: &SyntaxElement) -> Option<(crate::ast::CallExpr, SyntaxElement)> {
    let node = el.as_node()?;
    let call = matchers::call_named(node, "t")?;
    let arg = matchers::sole_positional(&call)?;
    Some((call, arg))
}

/// The text of `el` when it is a bare symbol (`IDENT`), else `None`. Used to
/// decide the single-argument collapse without comparing whitespace-bearing
/// subtrees.
fn ident_text(el: &SyntaxElement) -> Option<String> {
    el.as_token()
        .filter(|t| t.kind() == SyntaxKind::IDENT)
        .map(|t| t.text().to_string())
}
