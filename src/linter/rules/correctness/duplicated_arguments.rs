//! `duplicated-arguments`: a call supplying the same argument name twice
//! (`f(a = 1, a = 2)`). The sibling of `duplicate-formal` on the call side.
//! Not always a runtime error (`c(a = 1, a = 2)` is fine — `c` takes `...`), so
//! it is a warning, not an error, and carries no autofix.
//!
//! `c()` is exempt entirely: repeated names there are legal and idiomatic (cli
//! message vectors, `c("i" = ..., "i" = ...)`), so flagging them is pure noise.

use std::collections::HashSet;

use rowan::ast::AstNode as _;

use crate::ast::CallExpr;
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct DuplicatedArguments;

impl Rule for DuplicatedArguments {
    fn id(&self) -> &'static str {
        "duplicated-arguments"
    }

    fn description(&self) -> &'static str {
        "Flag a call that supplies the same argument name more than once \
         (`f(a = 1, a = 2)`). The call-side sibling of `duplicate-formal`; \
         reported as a warning with no autofix, since it isn't always a runtime \
         error."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "The argument `a` is supplied twice:",
            source: "list(a = 1, a = 2)\n",
        }]
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(call) = el.as_node().cloned().and_then(CallExpr::cast) else {
            return;
        };
        // `c()` takes `...` and repeated names are legal and idiomatic (cli
        // message vectors: `c("i" = ..., "i" = ...)`), so duplicates there are
        // not a mistake. Everything else (including `list()`) stays flagged.
        if matchers::callee_name(&call).as_deref() == Some("c") {
            return;
        }
        let mut seen: HashSet<String> = HashSet::new();
        for arg in matchers::args(&call) {
            let (Some(name), Some(token)) = (arg.name, arg.name_token) else {
                continue;
            };
            if !seen.insert(name.to_string()) {
                sink.push(Diagnostic {
                    rule: "duplicated-arguments",
                    severity: Default::default(),
                    path: Default::default(),
                    range: token.text_range(),
                    message: ViolationData::new(
                        "duplicated-arguments",
                        format!("argument `{name}` is supplied more than once in this call"),
                    )
                    .with_suggestion("Remove or rename the duplicate argument."),
                    fix: None,
                });
            }
        }
    }
}
