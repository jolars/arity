//! `browser`: a leftover `browser()` debugging call.
//!
//! `browser()` drops into R's interactive debugger; it is meant to be added
//! temporarily while debugging and removed before the code is committed or
//! shipped. A `browser()` left in source is almost always an oversight—in a
//! non-interactive session it silently does nothing, so it survives unnoticed.
//!
//! It is **namespace-confirmed** (`ns`): the callee must resolve to base R via
//! [`RuleContext::resolves_to_base`], so a user function that happens to be
//! named `browser` (or a `browser` shadowed by a local binding) is left alone.
//!
//! The fix deletes the call. Because `browser()` returns `NULL` and its only
//! effect is the debugger prompt, dropping a real base-R `browser()` is
//! behavior-preserving in any non-interactive run, so the fix is **safe**. It is
//! offered only when the call is a **direct statement** of a block or the file
//! top level—there, deleting the whole statement stays parseable (an emptied
//! block `{}` is valid R). In any other position (an `if` branch, an assignment
//! RHS, a call argument) deleting the call whole would break syntax, so the fix
//! is **withheld** (autofix-correctness discipline) while the finding is still
//! reported.

use rowan::ast::AstNode as _;

use crate::ast::CallExpr;
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct Browser;

const EXAMPLES: &[Example] = &[Example {
    caption: "A `browser()` call left in after debugging:",
    source: "f <- function(x) {\n  browser()\n  x + 1\n}\n",
}];

impl Rule for Browser {
    fn id(&self) -> &'static str {
        "browser"
    }

    fn description(&self) -> &'static str {
        "Flag a leftover `browser()` call. `browser()` opens R's interactive \
         debugger and is meant to be removed before code is committed; in a \
         non-interactive session it silently does nothing, so it lingers \
         unnoticed.\n\nOnly a call that resolves to base R's `browser` is \
         flagged—a same-named user function is left alone. The safe-delete fix \
         is offered only for a `browser()` that is a direct statement (of a block \
         or the file top level); in any other position it is withheld so the edit \
         can't break syntax."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(call) = el.as_node().cloned().and_then(CallExpr::cast) else {
            return;
        };
        if matchers::callee_name(&call).as_deref() != Some("browser") {
            return;
        }
        // Confirm the callee is base R's `browser`, not a user redefinition or a
        // locally shadowed name — otherwise this is not a debug call at all.
        if !ctx.resolves_to_base(&call) {
            return;
        }

        let range = call.syntax().text_range();

        // Offer the delete only where the call is a direct statement — the parent
        // is a block or the file root. Elsewhere (an `if` branch, an assignment
        // RHS, a call argument) deleting the call whole would break syntax.
        let is_statement = call
            .syntax()
            .parent()
            .is_some_and(|p| matches!(p.kind(), SyntaxKind::BLOCK_EXPR | SyntaxKind::ROOT));
        let fix = is_statement.then(|| {
            let src = ctx.root.text().to_string();
            let (start, end) = matchers::deletion_span(&src, range);
            Fix::safe(start, end, "", "Remove the `browser()` call")
        });

        sink.push(Diagnostic {
            rule: "browser",
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new("browser", "leftover `browser()` debugging call")
                .with_suggestion("Remove the `browser()` call."),
            fix,
        });
    }
}
