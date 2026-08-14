//! Shared helpers for the roxygen documentation rules (`documentation/`).
//!
//! A `ROXYGEN_BLOCK` is a *sibling* of the statement it documents, never a
//! child, so the rules need a correlation step: [`documented_function`] walks
//! to the next non-trivia sibling and classifies it, strictly conservatively —
//! anything that is not a plain `name <- function(...)` shape (S4 `setMethod`,
//! R6 classes, package-level docs, …) yields `None`, and the function-shape
//! checks skip rather than risk a wrong finding.
//!
//! [`topic_members`] answers "what merges into this block's topic?". roxygen2
//! merges `@rdname`/`@describeIn` blocks package-wide, so the answer is
//! cross-file and comes from the salsa-tracked index when the caller resolved
//! one ([`RuleContext::topics`]); [`RoxygenTopics`] is the file-local fallback
//! for the single-document paths. Both read the *same* projection
//! ([`crate::project::file_roxygen_topics`]), so the two views cannot drift.
//!
//! [`extract_examples`] recovers the R code embedded in an `@examples` /
//! `@examplesIf` section by concatenating the section's tokens (never the
//! joined tag text: under `@md` the parser tokenizes markdown inside example
//! bodies, fragmenting a code line into several leaves). Every appended piece
//! records its original byte span, so parse diagnostics on the extracted code
//! map back to exact source ranges via [`ExtractedCode::map_range`].

use rowan::ast::AstNode as _;
use rowan::{TextRange, TextSize};

use crate::ast::{RoxygenBlock, RoxygenSection};
use crate::linter::rules::RuleContext;
use crate::syntax::{SyntaxKind, SyntaxNode};

// The block-to-facts reduction lives in `src/project/roxygen.rs`: topics merge
// package-wide, so the grouping is a per-file projection the project graph
// folds, not something the linter can own. Re-exported here so the rule modules
// keep one import site.
pub(crate) use crate::project::{
    ParamDoc, TopicMember, documented_binding_name, documented_function, has_title,
    inherits_params, joins_other_topic, param_doc, topic_key,
};

/// Every tag understood by roxygen2 (8.0.0) plus the `@md`/`@noMd` toggles.
/// Version *availability* (`@prop`/`@R6method` need roxygen2 >= 8.0.0) is the
/// `roxygen2-compat` rule's concern, not unknown-ness.
/// Kept sorted for binary search; validated against roxygen2 itself by the
/// `roxygen_lint_oracle` harness. Capitalized names sort before lowercase
/// (byte order, matching `binary_search`).
const KNOWN_TAGS: &[&str] = &[
    "R6method",
    "aliases",
    "author",
    "backref",
    "concept",
    "describeIn",
    "description",
    "details",
    "docType",
    "encoding",
    "eval",
    "evalNamespace",
    "evalRd",
    "example",
    "examples",
    "examplesIf",
    "export",
    "exportClass",
    "exportMethod",
    "exportPattern",
    "exportS3Method",
    "family",
    "field",
    "format",
    "import",
    "importClassesFrom",
    "importFrom",
    "importMethodsFrom",
    "include",
    "inherit",
    "inheritDotParams",
    "inheritParams",
    "inheritSection",
    "keywords",
    "md",
    "method",
    "name",
    "noMd",
    "noRd",
    "note",
    "order",
    "param",
    "prop",
    "rawNamespace",
    "rawRd",
    "rdname",
    "references",
    "return",
    "returns",
    "section",
    "seealso",
    "slot",
    "source",
    "template",
    "templateVar",
    "title",
    "usage",
    "useDynLib",
];

/// Whether `name` (without the leading `@`) is a tag roxygen2 understands.
pub(crate) fn is_known_tag(name: &str) -> bool {
    KNOWN_TAGS.binary_search(&name).is_ok()
}

/// Tags that pull documentation in from elsewhere or merge this block into a
/// shared topic. When any is present, per-block coverage (params, title,
/// return) cannot be judged locally, so the coverage checks skip.
pub(crate) fn inherits_docs(block: &RoxygenBlock) -> bool {
    inherits_params(block) || joins_other_topic(block)
}

/// The Rd topics of one file, as [`TopicMember`] summaries — the fallback view
/// used when the caller resolved no project (the single-document paths).
///
/// Built once per file and memoized on [`RuleContext`], because three rules ask
/// the same question and re-walking the tree per block would be quadratic on a
/// file full of documentation. It is the *same* projection the package-wide
/// index folds ([`crate::project::file_roxygen_topics`]), so the two views
/// cannot drift.
#[derive(Debug, Default)]
pub(crate) struct RoxygenTopics {
    topics: std::collections::BTreeMap<String, Vec<TopicMember>>,
}

impl RoxygenTopics {
    pub(crate) fn build(root: &SyntaxNode) -> Self {
        Self {
            topics: crate::project::file_roxygen_topics(root),
        }
    }

    pub(crate) fn members(&self, key: &str) -> &[TopicMember] {
        self.topics.get(key).map_or(&[], Vec::as_slice)
    }
}

/// The blocks that merge into this block's topic — the block itself plus every
/// block joining it — resolved across the whole **package** when the caller
/// supplied one ([`RuleContext::topics`]), else across this file only.
///
/// roxygen2 merges `@rdname`/`@describeIn` package-wide, so a file-local answer
/// false-positives whenever the joiner that supplies a `@param`, a `@return`,
/// or a title sits in a sibling `R/` file.
///
/// `None` when the topic is not resolvable *from this block*: it joins someone
/// else's topic (so it is not the owner and is never judged) or names no topic
/// at all. A caller that gets `None` must fall back to judging the block alone,
/// or skip.
pub(crate) fn topic_members<'a>(
    block: &RoxygenBlock,
    ctx: &'a RuleContext<'_>,
) -> Option<&'a [TopicMember]> {
    if joins_other_topic(block) {
        return None;
    }
    let key = topic_key(block)?;
    Some(match ctx.topics {
        Some(package) => package.members(&key),
        None => ctx.roxygen_topics().members(&key),
    })
}

/// Tags that only drive the `NAMESPACE` (or toggle parsing modes) and never
/// generate an Rd topic on their own.
fn is_namespace_or_toggle_tag(name: &str) -> bool {
    matches!(
        name,
        "export"
            | "exportClass"
            | "exportMethod"
            | "exportPattern"
            | "exportS3Method"
            | "evalNamespace"
            | "import"
            | "importClassesFrom"
            | "importFrom"
            | "importMethodsFrom"
            | "include"
            | "md"
            | "noMd"
            | "noRd"
            | "rawNamespace"
            | "useDynLib"
    )
}

/// Whether this block asks for documentation: it carries prose, an
/// Rd-generating tag, or an `@export` (whose target `R CMD check` would then
/// report as an undocumented export). Import-attachment blocks (namespace
/// tags only, no export) do not.
pub(crate) fn wants_rd_topic(block: &RoxygenBlock) -> bool {
    block.has_tag("export") || asks_for_rd_on_its_own(block)
}

/// Whether the block asks for a topic on its own content—prose or an
/// Rd-generating tag—ignoring `@export`. This is [`wants_rd_topic`] minus the
/// clause that only holds because the target is exported, which is exactly the
/// distinction an S3 method needs: a method is never exported, but a method
/// block carrying `@param` still asks roxygen2 for a topic (and gets the
/// "Skipping; no name and/or title" warning when it has no title).
pub(crate) fn asks_for_rd_on_its_own(block: &RoxygenBlock) -> bool {
    block.intro().is_some_and(|intro| intro.has_prose())
        || block.tags().any(|tag| {
            tag.name()
                .is_some_and(|name| !is_namespace_or_toggle_tag(&name))
        })
}

/// Whether this block documents a function registered as an S3 method by the
/// package's `NAMESPACE` (`S3method(generic, class)`).
///
/// roxygen2 turns `@export` on such a name into `S3method(...)`, not
/// `export(...)`: the method is reached by dispatch, generates no Rd topic of
/// its own, and `R CMD check` never reports it as an undocumented export. So
/// the topic rules' premise—"an `@export` whose target owes documentation"—
/// does not hold, and documenting the generic while leaving each method a bare
/// `#' @export` is the standard idiom.
///
/// Answered from the parsed `NAMESPACE` rather than the name's shape: `foo.bar`
/// is only a method if a generic actually claims it, and arity never evaluates
/// R. Consequently this is `false` on the single-file paths (no project scope)
/// and for a package whose `NAMESPACE` has not been regenerated.
pub(crate) fn documents_s3_method(block: &RoxygenBlock, ctx: &RuleContext<'_>) -> bool {
    let Some(project) = ctx.project else {
        return false;
    };
    documented_binding_name(block).is_some_and(|name| project.is_s3_method(&name))
}

/// A contiguous piece of extracted code and where it came from.
struct Segment {
    extracted_start: u32,
    original_start: u32,
    len: u32,
}

/// R code recovered from a roxygen section, with the byte-span bookkeeping
/// needed to map ranges on the extracted string back to the original source.
#[derive(Default)]
pub(crate) struct ExtractedCode {
    pub(crate) code: String,
    segments: Vec<Segment>,
}

impl ExtractedCode {
    fn push(&mut self, text: &str, original_start: TextSize) {
        if text.is_empty() {
            return;
        }
        self.segments.push(Segment {
            extracted_start: self.code.len() as u32,
            original_start: original_start.into(),
            len: text.len() as u32,
        });
        self.code.push_str(text);
    }

    /// Whether any extracted content is non-whitespace.
    pub(crate) fn has_code(&self) -> bool {
        !self.code.trim().is_empty()
    }

    /// Map a range on the extracted string back to original source bytes.
    pub(crate) fn map_range(&self, range: TextRange) -> TextRange {
        let start = self.map_pos(range.start().into(), false);
        let end = self.map_pos(range.end().into(), true).max(start);
        TextRange::new(TextSize::from(start), TextSize::from(end))
    }

    /// Map one extracted-string position to an original byte offset. Segment
    /// boundaries are ambiguous (the end of one piece and the start of the
    /// next are the same extracted position but different source bytes):
    /// `is_end` resolves them toward the earlier segment so that a mapped
    /// range never balloons across the `#' ` prefix of the following line.
    fn map_pos(&self, pos: u32, is_end: bool) -> u32 {
        if self.segments.is_empty() {
            return 0;
        }
        let idx = self
            .segments
            .partition_point(|s| s.extracted_start < pos || (!is_end && s.extracted_start == pos));
        let seg = &self.segments[idx.saturating_sub(1)];
        let offset = pos.saturating_sub(seg.extracted_start).min(seg.len);
        seg.original_start + offset
    }
}

/// The code snippets of an `@examples` / `@examplesIf` section: the condition
/// (`@examplesIf` only) and the body, each a separately parseable snippet.
pub(crate) struct ExamplesCode {
    pub(crate) condition: Option<ExtractedCode>,
    pub(crate) body: Option<ExtractedCode>,
}

/// Extract the embedded R code of an examples section. Returns `None` for
/// non-examples sections. Mirrors roxygen2's comment stripping: the roxygen
/// marker and exactly one following space are dropped, so deeper indentation
/// survives into the extracted code.
pub(crate) fn extract_examples(section: &RoxygenSection) -> Option<ExamplesCode> {
    let tag = section.tag()?;
    if !tag.is_examples() {
        return None;
    }
    let is_conditional = tag.name().as_deref() == Some("examplesIf");

    // The tag head—`@examples` and the single space after it—is skipped;
    // everything before it on that line (marker, leading whitespace) sits
    // earlier in the source and is skipped by the same offset check.
    let name_token = tag
        .name()
        .and_then(|_| tag.at())
        .and_then(|at| at.next_sibling_or_token())?;
    let mut head_end = name_token.text_range().end();
    if let Some(next) = name_token.next_sibling_or_token()
        && next.kind() == SyntaxKind::WHITESPACE
    {
        head_end = next.text_range().end();
    }
    let tag_end = tag.syntax().text_range().end();

    let mut condition = ExtractedCode::default();
    let mut body = ExtractedCode::default();
    let mut after_marker = false;
    for element in section.syntax().descendants_with_tokens() {
        let Some(token) = element.as_token() else {
            continue;
        };
        let range = token.text_range();
        if range.end() <= head_end {
            continue;
        }
        // Same-line trailing tokens inside the tag node are the condition for
        // `@examplesIf`, and simply leading code for `@examples`.
        let dest = if is_conditional && range.end() <= tag_end {
            &mut condition
        } else {
            &mut body
        };
        match token.kind() {
            SyntaxKind::ROXYGEN_MARKER => {
                after_marker = true;
                continue;
            }
            SyntaxKind::WHITESPACE if after_marker => {
                // Drop exactly one space of the `#' ` prefix, as roxygen2 does.
                dest.push(&token.text()[1..], range.start() + TextSize::from(1));
            }
            SyntaxKind::NEWLINE => dest.push("\n", range.start()),
            _ => dest.push(token.text(), range.start()),
        }
        after_marker = false;
    }

    Some(ExamplesCode {
        condition: condition.has_code().then_some(condition),
        body: body.has_code().then_some(body),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn first_block(src: &str) -> RoxygenBlock {
        parse(src)
            .cst
            .descendants()
            .find_map(RoxygenBlock::cast)
            .expect("a roxygen block")
    }

    #[test]
    fn known_tags_is_sorted_and_hits() {
        assert!(KNOWN_TAGS.is_sorted());
        for tag in ["export", "param", "returns", "examplesIf", "noRd"] {
            assert!(is_known_tag(tag), "{tag} should be known");
        }
        assert!(!is_known_tag("exprot"));
        assert!(!is_known_tag(""));
    }

    #[test]
    fn documented_function_classifies_shapes() {
        let cases: &[(&str, bool)] = &[
            ("#' T\nf <- function(x) x\n", true),
            ("#' T\nf = function(x) x\n", true),
            ("#' T\nf <<- function(x) x\n", true),
            ("#' T\n\"f\" <- function(x) x\n", true),
            ("#' T\nfunction(x) x\n", true),
            // Comments between the block and the function are trivia.
            ("#' T\n# helper\nf <- function(x) x\n", true),
            (
                "#' T\nsetMethod(\"show\", \"C\", function(object) 1)\n",
                false,
            ),
            ("#' T\n\"_PACKAGE\"\n", false),
            ("#' T\nNULL\n", false),
            ("#' T\nf <- 1\n", false),
            ("#' T\ndim(x) <- function() 1\n", false),
            // `->` binds inside the function body (`function(x) (x -> f)`),
            // so this is a bare function literal.
            ("#' T\nfunction(x) x -> f\n", true),
            // A following roxygen block (after a blank separator line) ends
            // the walk: the function belongs to the *second* block.
            ("#' T\n\n#' U\nf <- function(x) x\n", false),
            ("#' T\n", false),
        ];
        for (src, expect) in cases {
            let block = first_block(src);
            assert_eq!(
                documented_function(&block).is_some(),
                *expect,
                "case: {src:?}"
            );
        }
    }

    #[test]
    fn topic_key_follows_roxygen2_precedence() {
        let cases: &[(&str, Option<&str>)] = &[
            // No topic tag: the object's default topic is its binding name.
            ("#' T\nf <- function(x) x\n", Some("f")),
            ("#' T\n\"f\" <- function(x) x\n", Some("f")),
            // `@name` wins over `@rdname`.
            ("#' @name a\n#' @rdname b\nf <- function() 1\n", Some("a")),
            ("#' @rdname b\nf <- function() 1\n", Some("b")),
            // `@describeIn`'s first word is the destination; the rest is prose.
            (
                "#' @describeIn b Some variant.\nf <- function() 1\n",
                Some("b"),
            ),
            // An empty tag value falls through to the next candidate.
            ("#' @name\nf <- function() 1\n", Some("f")),
            // Nothing names a topic and nothing is bound.
            (
                "#' T\nsetMethod(\"show\", \"C\", function(object) 1)\n",
                None,
            ),
        ];
        for (src, expect) in cases {
            let block = first_block(src);
            assert_eq!(topic_key(&block).as_deref(), *expect, "case: {src:?}");
        }
    }

    #[test]
    fn joins_other_topic_is_rdname_and_describe_in() {
        for src in [
            "#' @rdname b\nf <- function() 1\n",
            "#' @describeIn b Variant.\nf <- function() 1\n",
        ] {
            assert!(joins_other_topic(&first_block(src)), "case: {src:?}");
        }
        for src in [
            "#' @name b\nf <- function() 1\n",
            "#' T\nf <- function() 1\n",
        ] {
            assert!(!joins_other_topic(&first_block(src)), "case: {src:?}");
        }
    }

    #[test]
    fn topics_group_owner_with_its_joiners() {
        let src = "#' Owner\n#' @param x X.\nf <- function() NULL\n\n\
                   #' @rdname f\ng <- function(x) x\n\n\
                   #' Unrelated\nh <- function() 1\n";
        let topics = RoxygenTopics::build(&parse(src).cst);
        assert_eq!(topics.members("f").len(), 2);
        assert_eq!(topics.members("h").len(), 1);
        assert!(topics.members("nope").is_empty());
    }

    #[test]
    fn second_block_documents_its_own_function() {
        // In `#' A\n#' @export\nf <- ...` the single block has two sections;
        // in `#' A\nNULL\n#' B\ng <- function() 1` the second block associates
        // with `g`.
        let src = "#' A\nNULL\n\n#' B\ng <- function() 1\n";
        let blocks: Vec<_> = parse(src)
            .cst
            .descendants()
            .filter_map(RoxygenBlock::cast)
            .collect();
        assert_eq!(blocks.len(), 2);
        assert!(documented_function(&blocks[0]).is_none());
        assert!(documented_function(&blocks[1]).is_some());
    }

    fn examples_code(src: &str) -> ExamplesCode {
        let block = first_block(src);
        block
            .sections()
            .find_map(|s| extract_examples(&s))
            .expect("an examples section")
    }

    #[test]
    fn extract_examples_plain_body() {
        let src = "#' @examples\n#' g(1)\n#' g(2)\nf <- function() 1\n";
        let ex = examples_code(src);
        assert!(ex.condition.is_none());
        let body = ex.body.expect("body");
        assert_eq!(body.code, "\ng(1)\ng(2)");
        // `g(1)` maps back to its exact source span.
        let pos = body.code.find("g(1)").unwrap() as u32;
        let mapped = body.map_range(TextRange::new(TextSize::from(pos), TextSize::from(pos + 4)));
        assert_eq!(&src[mapped], "g(1)");
    }

    #[test]
    fn extract_examples_same_line_code_and_indent() {
        let src = "#' @examples g(1)\n#'   g(2)\nf <- function() 1\n";
        let ex = examples_code(src);
        let body = ex.body.expect("body");
        // One space of the `#' ` prefix is stripped; deeper indent survives.
        assert_eq!(body.code, "g(1)\n  g(2)");
    }

    #[test]
    fn extract_examples_if_splits_condition_and_body() {
        let src = "#' @examplesIf interactive()\n#' g(1)\nf <- function() 1\n";
        let ex = examples_code(src);
        let condition = ex.condition.expect("condition");
        assert_eq!(condition.code, "interactive()");
        let body = ex.body.expect("body");
        assert_eq!(body.code.trim(), "g(1)");
        let mapped = condition.map_range(TextRange::new(
            TextSize::from(0),
            TextSize::of(&condition.code),
        ));
        assert_eq!(&src[mapped], "interactive()");
    }

    #[test]
    fn extract_examples_md_fragmented_line_is_reassembled() {
        // Under `@md` the parser tokenizes markdown inside example bodies,
        // splitting `x <- *emph* + 1` across several leaves; token-level
        // concatenation must reassemble the exact line.
        let src = "#' @md\n#' @examples\n#' x <- *emph* + 1\nf <- function() 1\n";
        let ex = examples_code(src);
        let body = ex.body.expect("body");
        assert_eq!(body.code, "\nx <- *emph* + 1");
        let pos = body.code.find("*emph*").unwrap() as u32;
        let mapped = body.map_range(TextRange::new(TextSize::from(pos), TextSize::from(pos + 6)));
        assert_eq!(&src[mapped], "*emph*");
    }

    #[test]
    fn extract_examples_blank_comment_lines_keep_line_structure() {
        let src = "#' @examples\n#' g(1)\n#'\n#' g(2)\nf <- function() 1\n";
        let body = examples_code(src).body.expect("body");
        assert_eq!(body.code, "\ng(1)\n\ng(2)");
    }

    #[test]
    fn extract_examples_empty_section_has_no_code() {
        let src = "#' @examples\n#'\nf <- function() 1\n";
        let ex = examples_code(src);
        assert!(ex.condition.is_none());
        assert!(ex.body.is_none());
    }

    #[test]
    fn map_range_endpoints_stay_on_their_lines() {
        // A multi-line range must not swallow the `#' ` prefix of the next
        // line: the end position sits at a segment boundary and resolves to
        // the earlier segment.
        let src = "#' @examples\n#' g(1\n#' h(2)\nf <- function() 1\n";
        let body = examples_code(src).body.expect("body");
        assert_eq!(body.code, "\ng(1\nh(2)");
        let start = body.code.find("g(1").unwrap() as u32;
        let end = body.code.find("h(2)").unwrap() as u32 + 4;
        let mapped = body.map_range(TextRange::new(TextSize::from(start), TextSize::from(end)));
        assert_eq!(&src[mapped], "g(1\n#' h(2)");
    }
}
