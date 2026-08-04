//! `implicit-assignment`: an assignment (`<-`, `=`, `<<-`, `->`, `->>`) nested
//! inside a call or subscript argument, e.g. `mean(x <- 1:10)`. The assignment
//! runs as a side effect of evaluating the argument, which is easy to overlook
//! when reading the call; the binding should be made on its own line.
//!
//! Scope is deliberately narrow to stay complementary to
//! `assignment-in-condition` (which owns the `if`/`while` condition case): this
//! rule fires only when the assignment is an argument value (parent `ARG`), so
//! the two never double-report. Statement bodies (`if (a) x <- 1`), top-level
//! and block statements, and chained assignments (`x <- y <- 1`) are not
//! arguments and so are untouched. The `:=` walrus is skipped: it is the
//! data.table / rlang update operator, not a stray assignment.
//!
//! No autofix: lifting the assignment out of the call is a semantic
//! restructuring (the binding's value is also consumed in place), not a textual
//! edit the linter may perform.
//!
//! Default-off. Unlike `assignment-in-condition` (which catches a likely `==`
//! typo), implicit assignment in an argument is idiomatic in several common
//! wrappers—`system.time(res <- expr)`, `suppressWarnings(x <- expr)`,
//! `invisible(y <- expr)`—so on-by-default it is mostly readability noise
//! rather than bug-finding. It is an opt-in style preference (lintr likewise
//! ships its `implicit_assignment_linter` disabled by default).

use crate::ast::{AssignmentExpr, AstNode as _};
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct ImplicitAssignment;

const EXAMPLES: &[Example] = &[Example {
    caption: "An assignment hidden inside a call argument:",
    source: "mean(x <- 1:10)\n",
}];

impl Rule for ImplicitAssignment {
    fn id(&self) -> &'static str {
        "implicit-assignment"
    }

    fn description(&self) -> &'static str {
        "Flag an assignment (`<-`, `=`, `<<-`, `->`, `->>`) nested inside a call \
         or subscript argument, e.g. `mean(x <- 1:10)`. The binding runs as a \
         side effect of the argument and is easy to miss; assign on its own \
         line instead. The `if`/`while` condition case is covered by \
         `assignment-in-condition`, and the data.table / rlang walrus (`:=`) is \
         left alone."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    /// Opt-in: implicit assignment is idiomatic in common wrappers
    /// (`system.time`, `suppressWarnings`, `invisible`), so it is off by
    /// default and enabled explicitly via `select`.
    fn default_enabled(&self) -> bool {
        false
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::ASSIGNMENT_EXPR]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else {
            return;
        };
        // Only assignments that are an argument value (parent `ARG`) are
        // "implicit"; statement positions and conditions are handled elsewhere.
        if node.parent().map(|p| p.kind()) != Some(SyntaxKind::ARG) {
            return;
        }
        let Some(assign) = AssignmentExpr::cast(node.clone()) else {
            return;
        };
        // The walrus (`:=`) is the data.table / rlang update operator, not a
        // stray assignment; leave it alone.
        if assign.op_kind() == Some(SyntaxKind::WALRUS) {
            return;
        }

        sink.push(Diagnostic {
            rule: "implicit-assignment",
            severity: Default::default(),
            path: Default::default(),
            range: node.text_range(),
            message: ViolationData::new(
                "implicit-assignment",
                "assignment nested in a call argument; assign on its own line",
            )
            .with_suggestion("Move the assignment to its own statement."),
            fix: None,
        });
    }
}
