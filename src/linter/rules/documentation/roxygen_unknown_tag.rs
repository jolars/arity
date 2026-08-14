//! `roxygen-unknown-tag`: a roxygen tag that roxygen2 does not understand.
//!
//! roxygen2 warns on unknown tags and drops them from the generated `.Rd`,
//! so a misspelled tag (`@exprot`, `@parma`) silently loses documentation.
//! The parser recognizes *any* `@name` as a tag, so this is a lint concern,
//! not a parse error. There is no fix: the intended tag is unknowable.
//! Custom tags from extension roclets can be suppressed with `# arity-lint skip`.

use rowan::ast::AstNode as _;

use crate::ast::RoxygenTag;
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::roxygen::is_known_tag;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct RoxygenUnknownTag;

const EXAMPLES: &[Example] = &[Example {
    caption: "A misspelled `@export`:",
    source: "#' Add one\n#' @exprot\nadd_one <- function(x) x + 1\n",
}];

impl Rule for RoxygenUnknownTag {
    fn id(&self) -> &'static str {
        "roxygen-unknown-tag"
    }

    fn description(&self) -> &'static str {
        "Flag roxygen tags that roxygen2 does not understand.\
         \n\nroxygen2 warns on an unknown tag and drops it from the generated \
         `.Rd`, so a misspelled tag (`@exprot`, `@parma`) silently loses \
         documentation—or worse, an intended `@export` never reaches the \
         `NAMESPACE`. Custom tags from extension roclets can be suppressed \
         with `# arity-lint skip roxygen-unknown-tag`."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::ROXYGEN_TAG]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(tag) = el.as_node().cloned().and_then(RoxygenTag::cast) else {
            return;
        };
        let Some(name) = tag.name() else {
            return;
        };
        if is_known_tag(&name) {
            return;
        }
        // The span is `@` + name—not the tag's prose.
        let (Some(at), Some(name_token)) = (
            tag.at(),
            tag.syntax()
                .children_with_tokens()
                .filter_map(|e| e.into_token())
                .find(|t| t.kind() == SyntaxKind::ROXYGEN_TAG_NAME),
        ) else {
            return;
        };
        let range = rowan::TextRange::new(at.text_range().start(), name_token.text_range().end());
        sink.push(Diagnostic {
            rule: "roxygen-unknown-tag",
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new(
                "roxygen-unknown-tag",
                format!("`@{name}` is not a tag roxygen2 understands"),
            )
            .with_suggestion(
                "Check the spelling against the roxygen2 tag index; suppress with \
                 `# arity-lint skip roxygen-unknown-tag` for extension-roclet tags.",
            ),
            fix: None,
        });
    }
}
