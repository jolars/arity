//! `vector-logic`: the vectorized `&`/`|` used where a scalar `&&`/`||` is
//! meant — directly in an `if`/`while` condition.
//!
//! An `if`/`while` condition needs a single `TRUE`/`FALSE`; R only ever looks
//! at the first element (and since R 4.2 a length > 1 condition is an error).
//! The vectorized operators `&`/`|` build a whole logical vector and don't
//! short-circuit, so `&&`/`||` are the operators actually intended there. The
//! rewrite doubles the operator token (`&` → `&&`, `|` → `||`), a tight,
//! format-clean edit, so the fix is `Safe`.
//!
//! Only operators in *conditional context* are flagged: the walk descends from
//! the condition through the logical scaffolding that preserves the scalar
//! truth value — parens, `!`, and the boolean operators `&&`/`||`/`&`/`|` —
//! but stops at a function call (`if (any(a | b))`), where a vector result is
//! exactly the point, so that `|` is left alone.

use rowan::ast::AstNode as _;

use crate::ast::{IfExpr, WhileExpr};
use crate::linter::diagnostic::{Diagnostic, Fix, Severity, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

pub struct VectorLogic;

const EXAMPLES: &[Example] = &[Example {
    caption: "Vectorized `&` in an `if` condition:",
    source: "if (a & b) {\n  go()\n}\n",
}];

impl Rule for VectorLogic {
    fn id(&self) -> &'static str {
        "vector-logic"
    }

    fn description(&self) -> &'static str {
        "Flag the vectorized `&`/`|` used directly in an `if`/`while` \
         condition, where the scalar `&&`/`||` is meant.\n\nA condition needs a \
         single `TRUE`/`FALSE`: R only looks at the first element (a length > 1 \
         condition is an error since R 4.2), and `&&`/`||` short-circuit. The \
         fix doubles the operator. Operators inside a function call \
         (`if (any(a | b))`) are left alone—a vector result is the point \
         there."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::IF_EXPR, SyntaxKind::WHILE_EXPR]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(condition) = condition_node(el) else {
            return;
        };
        let mut ops = Vec::new();
        collect_conditional_ops(&condition, &mut ops);
        for op in ops {
            let (text, scalar) = match op.kind() {
                SyntaxKind::AND => ("&", "&&"),
                SyntaxKind::OR => ("|", "||"),
                _ => continue,
            };
            let range = op.text_range();
            let fix = Fix::safe(
                usize::from(range.start()),
                usize::from(range.end()),
                scalar,
                format!("Replace `{text}` with `{scalar}`"),
            );
            sink.push(Diagnostic {
                rule: "vector-logic",
                severity: Severity::Warning,
                path: Default::default(),
                range,
                message: ViolationData::new(
                    "vector-logic",
                    format!("`{text}` in a condition; use `{scalar}`"),
                )
                .with_suggestion(format!(
                    "Use the scalar `{scalar}` in an `if`/`while` condition."
                )),
                fix: Some(fix),
            });
        }
    }
}

/// The condition expression of an `if`/`while` node, if it is a node (a bare
/// `IDENT` condition has none to descend into).
fn condition_node(el: &SyntaxElement) -> Option<SyntaxNode> {
    let node = el.as_node()?;
    let elements = if let Some(if_expr) = IfExpr::cast(node.clone()) {
        if_expr.condition_elements()?
    } else {
        WhileExpr::cast(node.clone())?.condition_elements()?
    };
    elements.into_iter().find_map(|e| e.into_node())
}

/// Collect every `&`/`|` operator token in conditional context, descending only
/// through the logical scaffolding that keeps the scalar truth value: parens,
/// `!`, and the boolean operators. A function call (or any other node) ends the
/// walk, so vector logic inside `any(...)`/`all(...)` is not flagged.
fn collect_conditional_ops(node: &SyntaxNode, out: &mut Vec<SyntaxToken>) {
    match node.kind() {
        SyntaxKind::BINARY_EXPR => {
            let Some((lhs, op, rhs)) = matchers::binary_parts(node) else {
                return;
            };
            match op.kind() {
                SyntaxKind::AND | SyntaxKind::OR => {
                    out.push(op);
                    descend(&lhs, out);
                    descend(&rhs, out);
                }
                SyntaxKind::AND2 | SyntaxKind::OR2 => {
                    descend(&lhs, out);
                    descend(&rhs, out);
                }
                _ => {}
            }
        }
        SyntaxKind::PAREN_EXPR => {
            for child in node.children() {
                collect_conditional_ops(&child, out);
            }
        }
        // Only `!` passes the logical value through scalar-wise; a `-`/`+` unary
        // does not put its operand in conditional context.
        SyntaxKind::UNARY_EXPR
            if node
                .children_with_tokens()
                .any(|e| e.kind() == SyntaxKind::BANG) =>
        {
            for child in node.children() {
                collect_conditional_ops(&child, out);
            }
        }
        _ => {}
    }
}

fn descend(el: &SyntaxElement, out: &mut Vec<SyntaxToken>) {
    if let Some(node) = el.as_node() {
        collect_conditional_ops(node, out);
    }
}
