//! `if-always-true`: an `if` whose condition is the literal constant `TRUE` or
//! `FALSE`.
//!
//! `if (TRUE) a else b` always takes `a`; `if (FALSE) a else b` always takes
//! `b`. Either way the branch is decided statically, so the `if` is dead
//! control flow—almost always a leftover from debugging or a copy-paste
//! mistake. We flag **only** the bare literals `TRUE`/`FALSE`: never a folded
//! constant expression (`if (1 == 1)`—no const-folding), and never the
//! rebindable symbols `T`/`F`, which a user can redefine.
//!
//! The fix is **unsafe**: replacing the `if` with its taken branch drops the
//! other branch (and, for a bare `if (FALSE) a`, the whole body—rewritten to
//! `NULL`), removing code the reader wrote. It is correct *by construction*—the
//! taken branch's exact source is spliced into the `if`'s exact range, so the
//! result still parses and is lossless—but it is **withheld** whenever a comment
//! sits outside the taken branch (it would be dropped) or the branch is missing.

use rowan::TextRange;
use rowan::ast::AstNode as _;

use crate::ast::IfExpr;
use crate::ast::kinds::is_trivia;
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

pub struct IfAlwaysTrue;

const EXAMPLES: &[Example] = &[Example {
    caption: "An `if` gated on a constant always takes the same branch:",
    source: "if (TRUE) {\n  f()\n} else {\n  g()\n}\n",
}];

impl Rule for IfAlwaysTrue {
    fn id(&self) -> &'static str {
        "if-always-true"
    }

    fn description(&self) -> &'static str {
        "Flag an `if` whose condition is the literal `TRUE` or `FALSE`. The \
         branch is decided statically, so the `if` is dead control flow. Only \
         the bare literals are flagged—not folded constants (`if (1 == 1)`) or \
         the rebindable symbols `T`/`F`."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::IF_EXPR]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else {
            return;
        };
        let Some(if_expr) = IfExpr::cast(node.clone()) else {
            return;
        };
        let Some(cond) = if_expr.condition_elements().as_deref().and_then(sole_expr) else {
            return;
        };
        let always_true = matchers::is_true(&cond);
        if !always_true && !matchers::is_false(&cond) {
            return;
        }
        let lit = if always_true { "TRUE" } else { "FALSE" };

        let full = node.text_range();
        let (start, end) = (usize::from(full.start()), usize::from(full.end()));

        // The branch that statically wins: `then` for `TRUE`, `else` for
        // `FALSE`. A bare `if (FALSE) a` has no `else`, so its value is
        // `NULL`—we splice that in rather than delete (deletion would break an
        // operand position like `x <- if (FALSE) a`).
        let fix = if always_true {
            branch_fix(node, if_expr.then_elements().as_deref(), start, end, lit)
        } else if if_expr.else_keyword().is_some() {
            branch_fix(node, if_expr.else_elements().as_deref(), start, end, lit)
        } else if !dropped_comment(node, None) {
            Some(Fix::unsafe_(
                start,
                end,
                "NULL",
                "Replace the always-false `if` with `NULL`",
            ))
        } else {
            None
        };

        sink.push(Diagnostic {
            rule: "if-always-true",
            severity: Default::default(),
            path: Default::default(),
            range: cond.text_range(),
            message: ViolationData::new(
                "if-always-true",
                format!("`if` condition is always `{lit}`"),
            )
            .with_suggestion(if always_true {
                "The condition always holds; the branch always runs."
            } else {
                "The condition never holds; the branch never runs."
            }),
            fix,
        });
    }
}

/// Build the replace-with-taken-branch fix, or withhold it. Withheld when the
/// branch is absent (a malformed `if`) or a comment lives outside the branch and
/// would be dropped by the splice (autofix-correctness: never lose trivia).
fn branch_fix(
    node: &SyntaxNode,
    branch: Option<&[SyntaxElement]>,
    start: usize,
    end: usize,
    lit: &str,
) -> Option<Fix> {
    let taken = branch.and_then(sole_expr)?;
    if dropped_comment(node, Some(taken.text_range())) {
        return None;
    }
    let desc = if lit == "TRUE" {
        "Remove the always-true `if`, keeping its branch"
    } else {
        "Remove the always-false `if`, keeping the `else` branch"
    };
    Some(Fix::unsafe_(
        start,
        end,
        matchers::element_text(&taken),
        desc,
    ))
}

/// The sole non-trivia, non-comment element of a slice, or `None` when it holds
/// zero or several (so a condition or branch is unambiguous).
fn sole_expr(elements: &[SyntaxElement]) -> Option<SyntaxElement> {
    let mut it = elements
        .iter()
        .filter(|e| !is_trivia(e.kind()) && e.kind() != SyntaxKind::COMMENT);
    let first = it.next()?;
    it.next().is_none().then(|| first.clone())
}

/// Whether the `if` subtree carries a comment that the splice would drop: any
/// `COMMENT` not contained in `keep` (the taken branch's range; `None` keeps
/// nothing).
fn dropped_comment(node: &SyntaxNode, keep: Option<TextRange>) -> bool {
    node.descendants_with_tokens().any(|el| {
        el.kind() == SyntaxKind::COMMENT
            && match keep {
                Some(k) => !k.contains_range(el.text_range()),
                None => true,
            }
    })
}
