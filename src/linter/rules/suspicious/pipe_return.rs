//! `pipe-return`: base `return` used as a `%>%` stage.
//!
//! Magrittr evaluates each pipe stage in its own context, so `return` on the
//! right-hand side returns from that stage rather than from the surrounding
//! function. Both the call and bare-name spellings are accepted by magrittr.
//!
//! This is namespace-confirmed (`ns`): the RHS must resolve to base R's
//! `return`, and a locally redefined `%>%` is left alone. The rule reports the
//! misleading stage without a fix because the intended control flow cannot be
//! inferred from the expression.

use rowan::ast::AstNode as _;

use crate::ast::CallExpr;
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::matchers::{self, PipeOperator};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct PipeReturn;

const EXAMPLES: &[Example] = &[Example {
    caption: "A `return()` stage does not exit the surrounding function:",
    source: "f <- function(x) {\n  x %>% sum() %>% return()\n  FALSE\n}\n",
}];

impl Rule for PipeReturn {
    fn id(&self) -> &'static str {
        "pipe-return"
    }

    fn description(&self) -> &'static str {
        "Flag base `return` used directly on the right-hand side of the magrittr \
         pipe `%>%`, whether written as `return()` or a bare name. It returns \
         from the pipe stage rather than from the surrounding function, so the \
         apparent early return is misleading. Wrap the whole pipeline in \
         `return()`, or assign its result and return that value. No fix is \
         offered because the intended control flow cannot be inferred. A \
         locally redefined `%>%` is left alone."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::BINARY_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(chain) = el.as_node().and_then(matchers::pipe_chain) else {
            return;
        };

        for stage in chain
            .stages
            .into_iter()
            .filter(|stage| stage.operator == PipeOperator::Magrittr)
        {
            if ctx.is_locally_shadowed(stage.operator_token.text_range())
                || !is_base_return(&stage.rhs, ctx)
            {
                continue;
            }
            sink.push(Diagnostic {
                rule: "pipe-return",
                severity: Default::default(),
                path: Default::default(),
                range: stage.rhs.text_range(),
                message: ViolationData::new(
                    "pipe-return",
                    "`return` after `%>%` does not exit the surrounding function",
                )
                .with_suggestion(
                    "Wrap the pipeline in `return()`, or assign and return its result.",
                ),
                fix: None,
            });
        }
    }
}

fn is_base_return(rhs: &SyntaxElement, ctx: &RuleContext<'_>) -> bool {
    match rhs {
        SyntaxElement::Node(node) => CallExpr::cast(node.clone())
            .filter(|call| matchers::callee_name(call).as_deref() == Some("return"))
            .is_some_and(|call| ctx.resolves_to_base(&call)),
        SyntaxElement::Token(token) => {
            token.kind() == SyntaxKind::IDENT
                && token.text() == "return"
                && ctx.read_resolves_to_base(token)
        }
    }
}
