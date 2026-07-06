//! `roxygen-return`: an `@export`ed function documented without `@return`.
//!
//! CRAN's incoming checks require every exported function's `.Rd` to carry a
//! `\value` section; roxygen2 itself never warns here, so the gap surfaces
//! only at submission time. The check fires on blocks that both `@export`
//! and document a plain `name <- function(...)`.
//!
//! Skips: `@noRd` blocks, and inherited/merged topics
//! (`@rdname`/`@describeIn`/`@inherit*`/`@template`—`@inherit other return`
//! is precisely how a shared value section is pulled in).

use rowan::ast::AstNode as _;

use crate::ast::RoxygenBlock;
use crate::linter::diagnostic::{Diagnostic, Severity, ViolationData};
use crate::linter::rules::roxygen::{documented_function, inherits_docs};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct RoxygenReturn;

const EXAMPLES: &[Example] = &[Example {
    caption: "An exported function with no `@return`:",
    source: "#' Add one\n#' @param x A number.\n#' @export\nadd_one <- function(x) x + 1\n",
}];

impl Rule for RoxygenReturn {
    fn id(&self) -> &'static str {
        "roxygen-return"
    }

    fn description(&self) -> &'static str {
        "Flag an `@export`ed function documented without `@return`.\
         \n\nCRAN requires every exported function's documentation to describe \
         its return value (the `.Rd` `\\value` section); roxygen2 itself stays \
         silent, so the omission otherwise surfaces only at submission time. \
         `@returns` is accepted as an alias. `@noRd` blocks and merged or \
         inherited topics (`@rdname`, `@inherit`, …) are skipped."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::ROXYGEN_BLOCK]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(block) = el.as_node().cloned().and_then(RoxygenBlock::cast) else {
            return;
        };
        let Some(export) = block
            .tags()
            .find(|tag| tag.name().as_deref() == Some("export"))
        else {
            return;
        };
        if block.has_tag("return")
            || block.has_tag("returns")
            || block.has_tag("noRd")
            || inherits_docs(&block)
            || documented_function(&block).is_none()
        {
            return;
        }
        sink.push(Diagnostic {
            rule: "roxygen-return",
            severity: Severity::Warning,
            path: Default::default(),
            range: export.syntax().text_range(),
            message: ViolationData::new(
                "roxygen-return",
                "exported function is documented without `@return`",
            )
            .with_suggestion("Add `@return` (or `@returns`) describing the value."),
            fix: None,
        });
    }
}
