//! A standalone base `return` reads the primitive without calling it.
//!
//! Only evaluated statement and control-flow body positions inside functions
//! are checked. Semantic reads exclude quoted code and data masks, while
//! namespace confirmation excludes shadowing. The fix is unsafe because calling
//! `return()` changes control flow.

use rowan::ast::AstNode as _;

use crate::ast::{BlockExpr, ForExpr, FunctionExpr, IfExpr, RepeatExpr, WhileExpr};
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct BareReturn;

const EXAMPLES: &[Example] = &[Example {
    caption: "A bare `return` does not exit the function:",
    source: "f <- function() {\n  return\n  42\n}\nf()\n",
}];

impl Rule for BareReturn {
    fn id(&self) -> &'static str {
        "bare-return"
    }

    fn description(&self) -> &'static str {
        "Flag a standalone base `return` in a function body or control-flow branch. \
         The bare symbol evaluates to the function object without calling it, so \
         execution continues. Use `return()` to exit the function. The fix is \
         unsafe because it changes control flow. Shadowed names, quoted code, \
         and value uses such as `ret <- return` or `lapply(xs, return)` are left alone."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::IDENT]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(token) = el.as_token().filter(|t| t.text() == "return") else {
            return;
        };
        // Quoted code and data-mask names cannot be assumed to execute a base read.
        if !ctx
            .model
            .idents()
            .iter()
            .any(|i| i.range == token.text_range() && !i.data_masked)
            || !ctx.read_resolves_to_base(token)
        {
            return;
        }
        let Some(parent) = token.parent() else {
            return;
        };
        if !parent.ancestors().any(|n| FunctionExpr::can_cast(n.kind())) {
            return;
        }
        let is_body = match parent.kind() {
            SyntaxKind::BLOCK_EXPR => BlockExpr::cast(parent)
                .unwrap()
                .statements()
                .any(|s| s == *el),
            SyntaxKind::FUNCTION_EXPR => {
                FunctionExpr::cast(parent).unwrap().body().as_ref() == Some(el)
            }
            SyntaxKind::IF_EXPR => {
                let branch = IfExpr::cast(parent).unwrap();
                branch
                    .then_elements()
                    .into_iter()
                    .flatten()
                    .chain(branch.else_elements().into_iter().flatten())
                    .any(|s| s == *el)
            }
            SyntaxKind::FOR_EXPR => {
                ForExpr::cast(parent).unwrap().body_element().as_ref() == Some(el)
            }
            SyntaxKind::WHILE_EXPR => {
                WhileExpr::cast(parent).unwrap().body_element().as_ref() == Some(el)
            }
            SyntaxKind::REPEAT_EXPR => {
                RepeatExpr::cast(parent).unwrap().body().as_ref() == Some(el)
            }
            _ => false,
        };
        if !is_body {
            return;
        }
        let range = token.text_range();
        sink.push(Diagnostic {
            rule: self.id(),
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new(self.id(), "bare `return` does not exit the function")
                .with_suggestion("Use `return()` to exit the function."),
            fix: Some(Fix::unsafe_(
                usize::from(range.start()),
                usize::from(range.end()),
                "return()",
                "Call `return()`",
            )),
        });
    }
}
