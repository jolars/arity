//! A terminal `close()` does not protect cleanup from errors or early returns.
//!
//! Only direct function tails are checked, through transparent blocks and
//! parentheses. Connection ownership and the intended return value are unknown,
//! so this rule is opt-in and never moves cleanup automatically.

use rowan::ast::AstNode as _;

use crate::ast::{BlockExpr, CallExpr, FunctionExpr, ParenExpr};
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::semantic::symbols::unbacktick;
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct TerminalClose;

const EXAMPLES: &[Example] = &[Example {
    caption: "An error before the last statement skips cleanup:",
    source: "read_file <- function(path) {\n  con <- file(path)\n  print(readLines(con))\n  close(con)\n}\n",
}];

impl Rule for TerminalClose {
    fn id(&self) -> &'static str {
        "terminal-close"
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn description(&self) -> &'static str {
        "Flag a direct base `close()` call at the end of a function, including \
         an unbraced body or a tail wrapped in blocks or parentheses. Errors or \
         early returns before that call can skip cleanup. Register \
         `on.exit(close(con), add = TRUE)` immediately after acquiring the \
         connection so cleanup runs when the function exits; see \
         [R's `on.exit()` documentation](https://stat.ethz.ch/R-manual/R-devel/library/base/html/on.exit.html).\n\n\
         This rule is opt-in because a function may intentionally take ownership \
         of a caller's connection or expose the result of `close()`. No automatic \
         fix is offered: moving cleanup can change ownership, timing, and the \
         return value. Only simple calls that resolve to base R are checked; \
         shadowed or qualified callees, quoted code, conditional cleanup, and \
         calls nested inside other operations are left alone."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::FUNCTION_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(call) = el
            .as_node()
            .cloned()
            .and_then(FunctionExpr::cast)
            .and_then(|function| function.body())
            .and_then(terminal_call)
        else {
            return;
        };
        if !matchers::callee_name(&call).is_some_and(|name| unbacktick(&name) == "close")
            || !ctx.resolves_to_base(&call)
        {
            return;
        }
        let Some(callee) = call.callee_token() else {
            return;
        };
        // Namespace confirmation alone does not exclude quoted or data-masked code.
        if !ctx
            .model
            .idents()
            .iter()
            .any(|ident| ident.range == callee.text_range() && !ident.data_masked)
        {
            return;
        }

        sink.push(Diagnostic {
            rule: self.id(),
            severity: Default::default(),
            path: Default::default(),
            range: call.syntax().text_range(),
            message: ViolationData::new(
                self.id(),
                "`close()` at the end of a function can be skipped by errors or early returns",
            )
            .with_suggestion(
                "Register cleanup with `on.exit(..., add = TRUE)` immediately after acquiring the connection.",
            ),
            fix: None,
        });
    }
}

fn terminal_call(mut tail: SyntaxElement) -> Option<CallExpr> {
    loop {
        let node = tail.into_node()?;
        tail = match node.kind() {
            SyntaxKind::BLOCK_EXPR => BlockExpr::cast(node)?.statements().last()?,
            SyntaxKind::PAREN_EXPR => ParenExpr::cast(node)?.inner()?,
            _ => return CallExpr::cast(node),
        };
    }
}
