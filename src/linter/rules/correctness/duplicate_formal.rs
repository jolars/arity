//! `duplicate-formal`: a function with two parameters of the same name.
//!
//! R warns at runtime (`Error in function (x, x) : repeated formal argument
//! 'x'`); we catch it statically.

use std::collections::HashMap;

use rowan::ast::AstNode as _;

use crate::ast::FunctionExpr;
use crate::linter::diagnostic::{Diagnostic, Severity, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct DuplicateFormal;

impl Rule for DuplicateFormal {
    fn id(&self) -> &'static str {
        "duplicate-formal"
    }

    fn description(&self) -> &'static str {
        "Flag a function defined with two parameters of the same name. R raises \
         a runtime error (`repeated formal argument`); this catches it \
         statically."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "Two parameters named `x`:",
            source: "f <- function(x, x) x\n",
        }]
    }

    fn default_severity(&self) -> Severity {
        Severity::Error
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::FUNCTION_EXPR]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(func) = el.as_node().cloned().and_then(FunctionExpr::cast) else {
            return;
        };
        let mut seen: HashMap<String, ()> = HashMap::new();
        for param in func.params() {
            if seen.insert(param.name.to_string(), ()).is_some() {
                sink.push(Diagnostic {
                    rule: "duplicate-formal",
                    severity: Severity::Error,
                    path: Default::default(),
                    range: param.name_token.text_range(),
                    message: ViolationData::new(
                        "duplicate-formal",
                        format!(
                            "parameter `{}` is declared more than once in this function",
                            param.name
                        ),
                    )
                    .with_suggestion("Rename one of the parameters."),
                    fix: None,
                });
            }
        }
    }
}
