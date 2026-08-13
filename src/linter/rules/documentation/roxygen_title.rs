//! `roxygen-title`: a documented function whose block has no title.
//!
//! The first untagged paragraph of a roxygen block (or an explicit `@title`)
//! becomes the topic title; without one, roxygen2 warns and the generated
//! `.Rd` is rejected by `R CMD check`. An `@export`ed function with *no*
//! documentation at all is also flagged: roxygen2 stays silent (no topic is
//! generated), but `R CMD check` then reports an undocumented export.
//!
//! Skips: `@noRd` blocks (no topic), inherited/merged topics
//! (`@rdname`/`@describeIn`/`@inherit*`/`@template`, where the title lives
//! elsewhere), import-attachment blocks (namespace tags only, nothing
//! exported), and blocks whose following statement is not a plain
//! `name <- function(...)`.

use rowan::ast::AstNode as _;

use crate::ast::RoxygenBlock;
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::roxygen::{
    asks_for_rd_on_its_own, documented_function, documents_s3_method, has_title, inherits_docs,
    wants_rd_topic,
};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct RoxygenTitle;

const EXAMPLES: &[Example] = &[Example {
    caption: "A documented, exported function with no title paragraph:",
    source: "#' @param x A number.\n#' @export\nadd_one <- function(x) x + 1\n",
}];

impl Rule for RoxygenTitle {
    fn id(&self) -> &'static str {
        "roxygen-title"
    }

    fn description(&self) -> &'static str {
        "Flag a documented function whose roxygen block has no title.\
         \n\nThe first untagged paragraph (or an explicit `@title`) becomes the \
         topic title; without one, roxygen2 warns and `R CMD check` rejects the \
         generated `.Rd`. An `@export` with no documentation at all is flagged \
         too—`R CMD check` reports it as an undocumented export. Blocks that \
         merge into or inherit another topic (`@rdname`, `@describeIn`, \
         `@inherit*`, `@template`) and `@noRd` blocks are skipped, as is a bare \
         `@export` on a function the package's NAMESPACE registers with \
         `S3method()`—that generates no topic and no undocumented export."
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
        if documented_function(&block).is_none()
            || block.has_tag("noRd")
            || inherits_docs(&block)
            || !wants_rd_topic(&block)
        {
            return;
        }
        if has_title(&block) {
            return;
        }
        // `@export` on a registered S3 method becomes `S3method(...)`, not
        // `export(...)`: no topic is generated and `R CMD check` reports no
        // undocumented export, so a bare method block owes no title. A method
        // block that asks for a topic anyway (`@param`, `@examples`, …) is
        // still flagged—roxygen2 warns "Skipping; no name and/or title" there.
        if documents_s3_method(&block, ctx) && !asks_for_rd_on_its_own(&block) {
            return;
        }
        // Point at the block's first marker, not the whole block.
        let Some(marker) = block
            .syntax()
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == SyntaxKind::ROXYGEN_MARKER)
        else {
            return;
        };
        sink.push(Diagnostic {
            rule: "roxygen-title",
            severity: Default::default(),
            path: Default::default(),
            range: marker.text_range(),
            message: ViolationData::new("roxygen-title", "documentation block has no title")
                .with_suggestion(
                    "Add a leading prose line (the first paragraph becomes the title) \
                 or an explicit `@title`.",
                ),
            fix: None,
        });
    }
}
