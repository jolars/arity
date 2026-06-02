//! `duplicate-formal`: a function with two parameters of the same name.
//!
//! R warns at runtime (`Error in function (x, x) : repeated formal argument
//! 'x'`); we catch it statically.

use std::collections::HashMap;

use rowan::ast::AstNode as _;

use crate::ast::FunctionExpr;
use crate::linter::diagnostic::{Diagnostic, Severity, ViolationData};
use crate::linter::rules::{Rule, RuleContext};
use crate::syntax::SyntaxKind;

pub struct DuplicateFormal;

impl Rule for DuplicateFormal {
    fn id(&self) -> &'static str {
        "duplicate-formal"
    }

    fn default_severity(&self) -> Severity {
        Severity::Error
    }

    fn run(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for node in ctx.root.descendants() {
            if node.kind() != SyntaxKind::FUNCTION_EXPR {
                continue;
            }
            let Some(func) = FunctionExpr::cast(node) else {
                continue;
            };
            let mut seen: HashMap<String, ()> = HashMap::new();
            for param in func.params() {
                if seen.insert(param.name.to_string(), ()).is_some() {
                    out.push(Diagnostic {
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
        out
    }
}
