//! `assignment-in-condition`: `<-`, `=`, `<<-`, `:=` as the direct condition of
//! an `if`/`while`. R's `=` is often a `==` typo; the other operators are
//! almost never what you mean inside a condition.

use rowan::NodeOrToken;

use crate::linter::diagnostic::{Diagnostic, Severity, ViolationData};
use crate::linter::rules::{Rule, RuleContext};
use crate::syntax::{SyntaxKind, SyntaxNode};

pub struct AssignmentInCondition;

impl Rule for AssignmentInCondition {
    fn id(&self) -> &'static str {
        "assignment-in-condition"
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn run(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for node in ctx.root.descendants() {
            match node.kind() {
                SyntaxKind::IF_EXPR | SyntaxKind::WHILE_EXPR => {
                    if let Some(assign) = direct_assignment_in_condition(&node) {
                        out.push(Diagnostic {
                            rule: "assignment-in-condition",
                            severity: Severity::Warning,
                            path: Default::default(),
                            range: assign.text_range(),
                            message: ViolationData::new(
                                "assignment-in-condition",
                                "assignment used as a condition; did you mean `==`?",
                            )
                            .with_suggestion("Replace `=` with `==` or move the assignment out."),
                            fix: None,
                        });
                    }
                }
                _ => {}
            }
        }
        out
    }
}

fn direct_assignment_in_condition(if_or_while: &SyntaxNode) -> Option<SyntaxNode> {
    // The condition lives between the LPAREN and the matching RPAREN at the
    // outermost paren depth. We find the LPAREN, then look at the immediate
    // expression child(ren) between LPAREN and the matching RPAREN.
    let elements: Vec<_> = if_or_while.children_with_tokens().collect();
    let lparen_idx = elements
        .iter()
        .position(|e| e.kind() == SyntaxKind::LPAREN)?;
    let mut depth = 0usize;
    let mut rparen_idx = None;
    for (i, el) in elements.iter().enumerate().skip(lparen_idx) {
        match el.kind() {
            SyntaxKind::LPAREN => depth += 1,
            SyntaxKind::RPAREN => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    rparen_idx = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let rparen_idx = rparen_idx?;
    for el in &elements[lparen_idx + 1..rparen_idx] {
        if let NodeOrToken::Node(node) = el
            && node.kind() == SyntaxKind::ASSIGNMENT_EXPR
        {
            return Some(node.clone());
        }
    }
    None
}
