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
//!
//! The `\value` section belongs to the *topic*, so a block that owns a topic
//! is satisfied by any block merging into it that carries `@return`
//! ([`topic_members`]) — anywhere in the package, since that is where roxygen2
//! merges topics.

use rowan::ast::AstNode as _;

use crate::ast::RoxygenBlock;
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::roxygen::{
    documented_function, documents_s3_method, has_title, inherits_docs, topic_members,
};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct RoxygenReturn;

const EXAMPLES: &[Example] = &[Example {
    caption: "An exported function with no `@return`:",
    source: "#' Add one\n#' @param x A number.\n#' @export\nadd_one <- function(x) x + 1\n",
}];

/// Whether the topic this block renders into describes its value: `@return`
/// (or its `@returns` alias) on the block itself or on any block merging into
/// the same topic, anywhere in the package. Falls back to the block alone when
/// the topic is not resolvable.
fn documents_value(block: &RoxygenBlock, ctx: &RuleContext<'_>) -> bool {
    match topic_members(block, ctx) {
        Some(members) => members.iter().any(|member| member.has_value),
        None => block.has_tag("return") || block.has_tag("returns"),
    }
}

impl Rule for RoxygenReturn {
    fn id(&self) -> &'static str {
        "roxygen-return"
    }

    fn description(&self) -> &'static str {
        "Flag an `@export`ed function documented without `@return`.\
         \n\nCRAN requires every exported function's documentation to describe \
         its return value (the `.Rd` `\\value` section); roxygen2 itself stays \
         silent, so the omission otherwise surfaces only at submission time. \
         `@returns` is accepted as an alias. `@noRd` blocks and blocks that \
         merge into or inherit another topic (`@rdname`, `@inherit`, …) are \
         skipped, and a block owning a topic is satisfied by a `@return` on any \
         block merging into it, anywhere in the package—the `\\value` section \
         belongs to the topic. A titleless S3 method is skipped too (registered with \
         `S3method()`, so it is not exported and generates no `.Rd`); the \
         generic's topic owns the value."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::ROXYGEN_BLOCK]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(block) = el.as_node().cloned().and_then(RoxygenBlock::cast) else {
            return;
        };
        let Some(export) = block
            .tags()
            .find(|tag| tag.name().as_deref() == Some("export"))
        else {
            return;
        };
        if block.has_tag("noRd")
            || inherits_docs(&block)
            || documented_function(&block).is_none()
            || documents_value(&block, ctx)
        {
            return;
        }
        // A registered S3 method is not exported, and without a title roxygen2
        // writes no `.Rd` at all—so there is no `\value` section for CRAN to
        // want. The generic's topic is where the value is described. A titled
        // method does generate a topic and is judged like any other function.
        if documents_s3_method(&block, ctx) && !has_title(&block) {
            return;
        }
        sink.push(Diagnostic {
            rule: "roxygen-return",
            severity: Default::default(),
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
