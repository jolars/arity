//! `redundant-ifelse`: `ifelse(c, TRUE, FALSE)` is just `c`, and
//! `ifelse(c, FALSE, TRUE)` is `!c`.

use rowan::ast::AstNode as _;

use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct RedundantIfelse;

impl Rule for RedundantIfelse {
    fn id(&self) -> &'static str {
        "redundant-ifelse"
    }

    fn description(&self) -> &'static str {
        "Flag `ifelse(c, TRUE, FALSE)` (which is just `c`) and \
         `ifelse(c, FALSE, TRUE)` (which is `!c`)."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "An `ifelse` that returns its own condition:",
            source: "flag <- ifelse(cond, TRUE, FALSE)\n",
        }]
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else {
            return;
        };
        let Some(call) = matchers::call_named(node, "ifelse") else {
            return;
        };
        // Exactly three positional args: `ifelse(test, yes, no)`.
        let all = matchers::args(&call);
        if all.len() != 3 || all.iter().any(|a| a.name.is_some()) {
            return;
        }
        let (Some(test), Some(yes), Some(no)) = (
            all[0].value.as_ref(),
            all[1].value.as_ref(),
            all[2].value.as_ref(),
        ) else {
            return;
        };
        let negate = match (
            matchers::is_true(yes) && matchers::is_false(no),
            matchers::is_false(yes) && matchers::is_true(no),
        ) {
            (true, _) => false, // (c, TRUE, FALSE) -> c
            (_, true) => true,  // (c, FALSE, TRUE) -> !c
            _ => return,
        };
        let r = call.syntax().text_range();
        let (start, end) = (usize::from(r.start()), usize::from(r.end()));
        // The negating rewrite needs an atom condition; otherwise withhold.
        let fix = match negate {
            false => Some(Fix::safe(
                start,
                end,
                matchers::element_text(test),
                "Replace `ifelse()` with the condition",
            )),
            true if matchers::is_atom(test) => Some(Fix::safe(
                start,
                end,
                format!("!{}", matchers::element_text(test)),
                "Replace `ifelse()` with the negated condition",
            )),
            true => None,
        };
        sink.push(Diagnostic {
            rule: "redundant-ifelse",
            severity: Default::default(),
            path: Default::default(),
            range: r,
            message: ViolationData::new(
                "redundant-ifelse",
                "`ifelse()` returning `TRUE`/`FALSE` is redundant",
            )
            .with_suggestion("Use the condition directly, or negate it."),
            fix,
        });
    }
}
