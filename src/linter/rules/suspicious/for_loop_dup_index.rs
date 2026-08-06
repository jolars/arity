//! `for-loop-dup-index`: a nested `for` loop that reuses an enclosing loop's
//! index variable.
//!
//! R gives loops no scope of their own: `for (i in ...)` binds `i` in the
//! enclosing frame. So an inner `for (i in ...)` nested in the body of an outer
//! `for (i in ...)` does not shadow the outer index—it *overwrites* it, and
//! when the inner loop finishes the outer one resumes with a corrupted counter.
//! Any read of `i` after the inner loop sees the inner loop's last value.
//!
//! The walk deliberately stops at a **function boundary**: a loop inside a
//! closure defined in the outer loop's body runs in its own frame, so it leaves
//! the outer index untouched (confirmed against `Rscript`). Only the outer
//! loop's *body* is considered—a `for` appearing in another loop's sequence
//! expression runs before that loop's index exists.
//!
//! There is **no fix**: the repair is to rename the inner index, and choosing a
//! name is an invention rather than a mechanical edit.

use rowan::ast::AstNode as _;

use crate::ast::ForExpr;
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct ForLoopDupIndex;

const EXAMPLES: &[Example] = &[Example {
    caption: "The inner loop overwrites the outer loop's counter:",
    source: "for (i in 1:10) {\n  for (i in 1:5) {\n    print(i)\n  }\n}\n",
}];

impl Rule for ForLoopDupIndex {
    fn id(&self) -> &'static str {
        "for-loop-dup-index"
    }

    fn description(&self) -> &'static str {
        "Flag a nested `for` loop that reuses the index variable of an \
         enclosing `for` loop. R loops introduce no scope, so the inner loop \
         overwrites the outer index rather than shadowing it: the outer loop \
         resumes with a corrupted counter and any later read of the name sees \
         the inner loop's last value.\n\nA loop nested inside a *function* \
         defined in the outer body is not flagged—it runs in its own frame and \
         leaves the outer index alone. No fix is offered, since the repair is to \
         invent a new index name."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::FOR_EXPR]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else {
            return;
        };
        let Some(inner) = ForExpr::cast(node.clone()) else {
            return;
        };
        let Some(clause) = matchers::for_clause(&inner) else {
            return;
        };
        let name = clause.index.text();

        // Walk outward for an enclosing loop with the same index, stopping at
        // the first function literal — beyond it we are in another frame.
        let mut shadowed = false;
        for ancestor in node.ancestors().skip(1) {
            if ancestor.kind() == SyntaxKind::FUNCTION_EXPR {
                break;
            }
            let Some(candidate) = ForExpr::cast(ancestor.clone()) else {
                continue;
            };
            // Only the enclosing loop's *body* re-executes around this loop; a
            // `for` sitting in its sequence clause runs before the index exists.
            let in_body = candidate
                .body_element()
                .is_some_and(|body| body.text_range().contains_range(node.text_range()));
            if in_body && matchers::for_clause(&candidate).is_some_and(|c| c.index.text() == name) {
                shadowed = true;
                break;
            }
        }
        if !shadowed {
            return;
        }

        sink.push(Diagnostic {
            rule: "for-loop-dup-index",
            severity: Default::default(),
            path: Default::default(),
            range: clause.range(),
            message: ViolationData::new(
                "for-loop-dup-index",
                format!("loop index `{name}` is already the index of an enclosing `for` loop"),
            )
            .with_suggestion(format!(
                "Rename this loop index so it does not overwrite the enclosing loop's `{name}`."
            )),
            fix: None,
        });
    }
}
