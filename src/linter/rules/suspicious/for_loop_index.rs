//! `for-loop-index`: a `for` loop whose index symbol is also read in its own
//! sequence expression.
//!
//! `for (x in x)` (and its nested forms, `for (x in seq_along(x))`) evaluates
//! the sequence once, then binds the index over the top of it — so by the time
//! the loop ends the original `x` is gone, replaced by its own last element.
//! That is legal R and it "works", but the variable the reader thinks they can
//! use after the loop no longer holds what it did, and the shape is far more
//! often a typo than a deliberate choice.
//!
//! The rule is **semantic** (`sem`): the trigger is not "the name appears in the
//! sequence text" but "the sequence contains a *read* of that name". The
//! [`SemanticModel`]'s ident set is exactly that distinction, so name positions
//! that are not reads never fire — `for (x in df$x)` (a field name),
//! `for (x in list(x = 1))` (an argument name), `for (x in df[["x"]])` (a
//! string). Reads inside a function literal in the sequence
//! (`for (f in lapply(xs, function(f) f))`) are skipped too: they resolve to
//! that closure's own frame, not to the index.
//!
//! There is **no fix** — the repair is to rename the index (or the sequence),
//! and picking a name is an invention, not a mechanical edit.
//!
//! [`SemanticModel`]: crate::semantic::SemanticModel

use rowan::ast::AstNode as _;

use crate::ast::ForExpr;
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

pub struct ForLoopIndex;

const EXAMPLES: &[Example] = &[
    Example {
        caption: "The loop index overwrites the vector being iterated over:",
        source: "for (x in x) {\n  print(x)\n}\n",
    },
    Example {
        caption: "The same mistake one call deep:",
        source: "for (i in seq_along(i)) {\n  print(i)\n}\n",
    },
];

impl Rule for ForLoopIndex {
    fn id(&self) -> &'static str {
        "for-loop-index"
    }

    fn description(&self) -> &'static str {
        "Flag a `for` loop whose index symbol is also read in its own sequence \
         expression, as in `for (x in x)` or `for (x in seq_along(x))`. R \
         evaluates the sequence once and then binds the index over it, so the \
         original value is destroyed by the first iteration and is not what a \
         reader would expect after the loop.\n\nOnly a genuine *read* of the \
         name counts: a field name (`for (x in df$x)`), an argument name \
         (`for (x in list(x = 1))`), or a read belonging to a function literal \
         inside the sequence is not a re-use and is not flagged. No fix is \
         offered—the repair is to rename the index or the sequence, which \
         means inventing a name."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::FOR_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else {
            return;
        };
        let Some(for_expr) = ForExpr::cast(node.clone()) else {
            return;
        };
        let Some(clause) = matchers::for_clause(&for_expr) else {
            return;
        };
        let name = clause.index.text();

        // A read of the index name inside the sequence, other than one that
        // belongs to a nested function literal (a different frame).
        let reused = ctx.model.idents().iter().any(|ident| {
            ident.name == name
                && clause.sequence.contains_range(ident.range)
                && !in_nested_function(node, ident.range)
                // Function-position lookup has a separate namespace in R: a
                // non-function loop index does not hide `class()` or `names()`.
                && !matchers::is_callee(ctx.root, ident.range)
        });
        if !reused {
            return;
        }

        sink.push(Diagnostic {
            rule: "for-loop-index",
            severity: Default::default(),
            path: Default::default(),
            range: clause.range(),
            message: ViolationData::new(
                "for-loop-index",
                format!("loop index `{name}` is also read in the loop's sequence"),
            )
            .with_suggestion(format!(
                "Rename the loop index so iterating does not overwrite `{name}`."
            )),
            fix: None,
        });
    }
}

/// Whether the element covering `range` sits inside a function literal nested
/// within `for_node` — a read there belongs to that closure's frame, so the loop
/// index never shadows it.
fn in_nested_function(for_node: &SyntaxNode, range: rowan::TextRange) -> bool {
    for_node
        .covering_element(range)
        .ancestors()
        .take_while(|anc| anc != for_node)
        .any(|anc| anc.kind() == SyntaxKind::FUNCTION_EXPR)
}
