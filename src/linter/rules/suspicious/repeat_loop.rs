//! `repeat`: `while (TRUE)` is an unconditional loop better written as
//! `repeat`.
//!
//! `while (TRUE) <body>` loops forever (short of a `break`/`return`); `repeat
//! <body>` says exactly that without the dummy condition. The rewrite replaces
//! the `while (TRUE)` header with `repeat`, leaving the body untouched — a
//! same-shape, behavior-preserving edit, so the fix is `Safe`. It is withheld
//! when the clause carries a comment (which the rewrite would drop). Only the
//! reserved literal `TRUE` is matched: the rebindable `T` could be a local
//! binding, so it is left to `true-false-symbol` rather than assumed here.

use rowan::ast::AstNode as _;

use crate::ast::WhileExpr;
use crate::linter::diagnostic::{Diagnostic, Fix, Severity, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct Repeat;

const EXAMPLES: &[Example] = &[Example {
    caption: "An unconditional `while` loop:",
    source: "while (TRUE) {\n  poll()\n}\n",
}];

impl Rule for Repeat {
    fn id(&self) -> &'static str {
        "repeat"
    }

    fn description(&self) -> &'static str {
        "Flag `while (TRUE)`, an unconditional loop better written as `repeat`.\
         \n\n`repeat` states the intent — loop until a `break`/`return` — \
         without the dummy `TRUE` condition. Only the reserved literal `TRUE` is \
         matched; the rebindable `T` is left to `true-false-symbol`."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::WHILE_EXPR]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(while_expr) = el.as_node().cloned().and_then(WhileExpr::cast) else {
            return;
        };
        // The condition must be exactly the reserved literal `TRUE`.
        let Some(condition) = while_expr.condition_elements() else {
            return;
        };
        if condition.len() != 1 || !matchers::is_true(&condition[0]) {
            return;
        }
        // The reported/replaced span is the `while (TRUE)` header; the body stays.
        let elements = while_expr.elements();
        let (Some(while_kw), Some((_, rparen_idx))) =
            (while_expr.while_keyword(), while_expr.clause_bounds())
        else {
            return;
        };
        let Some(rparen) = elements[rparen_idx].as_token() else {
            return;
        };
        let start = while_kw.text_range().start();
        let end = rparen.text_range().end();
        let range = rowan::TextRange::new(start, end);

        // A comment inside the clause would be dropped by the rewrite, so withhold
        // the fix (the finding is still reported).
        let has_comment = while_expr.leading_comments().is_some_and(|c| !c.is_empty());
        let fix = (!has_comment).then(|| {
            Fix::safe(
                usize::from(start),
                usize::from(end),
                "repeat",
                "Replace `while (TRUE)` with `repeat`",
            )
        });

        sink.push(Diagnostic {
            rule: "repeat",
            severity: Severity::Warning,
            path: Default::default(),
            range,
            message: ViolationData::new(
                "repeat",
                "`while (TRUE)` is an unconditional loop; use `repeat`",
            )
            .with_suggestion("Write `repeat` for a loop with no exit condition."),
            fix,
        });
    }
}
