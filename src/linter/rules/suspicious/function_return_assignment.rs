//! `function-return-assignment`: an assignment passed directly to `return()`.
//!
//! The assignment's value is returned, but the binding remains as a side
//! effect. That combination is easy to write accidentally and hard to infer an
//! intended rewrite for, so the rule reports it without offering a fix.

use rowan::ast::AstNode as _;

use crate::ast::AssignmentExpr;
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct FunctionReturnAssignment;

const EXAMPLES: &[Example] = &[Example {
    caption: "Assigning while returning a value:",
    source: "f <- function() return(result <- compute())\n",
}];

impl Rule for FunctionReturnAssignment {
    fn id(&self) -> &'static str {
        "function-return-assignment"
    }

    fn description(&self) -> &'static str {
        "Flag an assignment passed directly to base `return()`. The assigned \
         value is returned, but the binding remains as a side effect. Move the \
         assignment before `return()` or return the value directly. No fix is \
         offered because the intended behavior cannot be inferred."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(call) = el
            .as_node()
            .and_then(|node| matchers::call_named(node, "return"))
        else {
            return;
        };
        if !ctx.resolves_to_base(&call) {
            return;
        }
        let Some(assignment) = matchers::sole_positional(&call)
            .and_then(|argument| argument.into_node())
            .and_then(AssignmentExpr::cast)
        else {
            return;
        };

        sink.push(Diagnostic {
            rule: "function-return-assignment",
            severity: Default::default(),
            path: Default::default(),
            range: assignment.syntax().text_range(),
            message: ViolationData::new(
                "function-return-assignment",
                "assignment inside `return()` has a side effect",
            )
            .with_suggestion("Move the assignment before `return()` or return the value directly."),
            fix: None,
        });
    }
}
