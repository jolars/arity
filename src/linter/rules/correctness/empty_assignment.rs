//! `empty-assignment`: an assignment whose value is an empty block, `x <- {}`.
//!
//! An empty block `{}` evaluates to `NULL`, so `x <- {}` is a roundabout,
//! usually accidental, way of writing `x <- NULL`—typically a leftover from
//! deleting the block's contents. We flag the assignment forms (`<-`, `=`,
//! `<<-`, and the right-assign `->`/`->>`) whose *value* side is a block with no
//! statements. A comment-only block still counts as empty (it contains no
//! statement), mirroring lintr's `empty_assignment_linter`.
//!
//! Only the assignment's direct value is inspected, so an empty function body
//! (`f <- function() {}`) or an empty `if` branch (`x <- if (a) {} else b`) is
//! left alone—the empty block there is the callee/branch, not the assigned
//! value. No fix ships: rewriting to `NULL` is a semantic judgment (the author
//! may have meant to fill the block in), so we report and let the reader decide.

use rowan::ast::AstNode as _;

use crate::ast::{AssignmentExpr, BlockExpr};
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct EmptyAssignment;

const EXAMPLES: &[Example] = &[Example {
    caption: "Assigning an empty block is the same as assigning `NULL`:",
    source: "x <- {}\n",
}];

impl Rule for EmptyAssignment {
    fn id(&self) -> &'static str {
        "empty-assignment"
    }

    fn description(&self) -> &'static str {
        "Flag an assignment whose value is an empty block (`x <- {}`). An empty \
         block evaluates to `NULL`, so this is a roundabout `x <- NULL`—usually \
         a leftover from deleting the block's body. An empty function body or \
         `if` branch is not flagged."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::ASSIGNMENT_EXPR]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else {
            return;
        };
        let Some(assign) = AssignmentExpr::cast(node.clone()) else {
            return;
        };
        // The value side (right for `<-`/`=`/`<<-`, left for `->`/`->>`) must be
        // a block node with no statements.
        let Some(value) = assign.value_element().and_then(|e| e.into_node()) else {
            return;
        };
        let Some(block) = BlockExpr::cast(value) else {
            return;
        };
        if block.statements().next().is_some() {
            return;
        }
        let range = block.syntax().text_range();
        sink.push(Diagnostic {
            rule: "empty-assignment",
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new(
                "empty-assignment",
                "assigning an empty block `{}` is the same as assigning `NULL`",
            )
            .with_suggestion("Assign `NULL` or a meaningful value instead."),
            fix: None,
        });
    }
}
