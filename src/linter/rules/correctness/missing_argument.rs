//! `missing-argument`: an empty, non-trailing call argument such as
//! `paste("a", , "b")` is usually accidental. The parser represents these
//! slots as empty `ARG` nodes; a trailing comma has no corresponding node, so
//! it is excluded by construction. Function formals use a different CST shape
//! and are likewise outside this rule. There is no generally safe deletion or
//! replacement, so findings carry no autofix.

use rowan::ast::AstNode as _;

use crate::ast::Arg;
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct MissingArgument;

impl Rule for MissingArgument {
    fn id(&self) -> &'static str {
        "missing-argument"
    }

    fn description(&self) -> &'static str {
        "Flag an empty, non-trailing call argument such as `f(a, , b)`. Trailing commas and intentional missing function-formal defaults are excluded."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "The second call argument is missing:",
            source: "paste(\"a\", , \"b\")\n",
        }]
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::ARG]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(arg) = el.as_node().cloned().and_then(Arg::cast) else {
            return;
        };
        if arg.name().is_some() || arg.value().is_some() {
            return;
        }
        // Missing dimensions are the standard spelling for selecting every
        // row or column, so they are intentional rather than omitted calls.
        let in_subset = arg
            .syntax()
            .parent()
            .and_then(|arg_list| arg_list.parent())
            .is_some_and(|parent| {
                matches!(
                    parent.kind(),
                    SyntaxKind::SUBSET_EXPR | SyntaxKind::SUBSET2_EXPR
                )
            });
        if in_subset {
            return;
        }
        let Some(comma) = arg
            .syntax()
            .next_sibling_or_token()
            .and_then(|el| el.into_token())
            .filter(|token| token.kind() == SyntaxKind::COMMA)
        else {
            return;
        };

        sink.push(Diagnostic {
            rule: "missing-argument",
            severity: Default::default(),
            path: Default::default(),
            range: comma.text_range(),
            message: ViolationData::new(
                "missing-argument",
                "call contains an empty argument before this comma",
            )
            .with_suggestion("Supply the intended argument explicitly."),
            fix: None,
        });
    }
}
