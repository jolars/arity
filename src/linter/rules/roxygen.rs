//! Shared helpers for the roxygen documentation rules (`documentation/`).
//!
//! A `ROXYGEN_BLOCK` is a *sibling* of the statement it documents, never a
//! child, so the rules need a correlation step: [`documented_function`] walks
//! to the next non-trivia sibling and classifies it, strictly conservatively —
//! anything that is not a plain `name <- function(...)` shape (S4 `setMethod`,
//! R6 classes, package-level docs, …) yields `None`, and the function-shape
//! checks skip rather than risk a wrong finding.
//!
//! [`extract_examples`] recovers the R code embedded in an `@examples` /
//! `@examplesIf` section by concatenating the section's tokens (never the
//! joined tag text: under `@md` the parser tokenizes markdown inside example
//! bodies, fragmenting a code line into several leaves). Every appended piece
//! records its original byte span, so parse diagnostics on the extracted code
//! map back to exact source ranges via [`ExtractedCode::map_range`].

use rowan::ast::AstNode as _;
use rowan::{TextRange, TextSize};

use crate::ast::{AssignmentExpr, FunctionExpr, RoxygenBlock, RoxygenSection};
use crate::syntax::{SyntaxElement, SyntaxKind};

/// Every tag understood by roxygen2 (7.3.x) plus the `@md`/`@noMd` toggles.
/// Kept sorted for binary search; validated against roxygen2 itself by the
/// `roxygen_lint_oracle` harness.
const KNOWN_TAGS: &[&str] = &[
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
    block.tags().any(|tag| {
        matches!(
            tag.name().as_deref(),
            Some(
                "inherit"
                    | "inheritParams"
                    | "inheritSection"
                    | "inheritDotParams"
                    | "rdname"
                    | "describeIn"
                    | "template"
                    | "usage"
            )
        )
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
    block.has_tag("export")
        || block.intro().is_some_and(|intro| intro.has_prose())
        || block.tags().any(|tag| {
            tag.name()
                .is_some_and(|name| !is_namespace_or_toggle_tag(&name))
        })
}

/// The function this block documents, when that is unambiguous: the next
/// non-trivia sibling is `name <- function(...)` (also `=` / `<<-`, string
/// LHS) or a bare `function(...)` literal. Anything else—another roxygen
/// block, `setMethod(...)`, an R6 class call, `"_PACKAGE"`, `NULL`—returns
/// `None`.
pub(crate) fn documented_function(block: &RoxygenBlock) -> Option<FunctionExpr> {
    let mut next = block.syntax().next_sibling_or_token();
    while let Some(element) = next {
        match &element {
            SyntaxElement::Token(token) => {
                if !matches!(
                    token.kind(),
                    SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
                ) {
                    return None;
                }
            }
            SyntaxElement::Node(node) => {
                return match node.kind() {
                    SyntaxKind::FUNCTION_EXPR => FunctionExpr::cast(node.clone()),
                    SyntaxKind::ASSIGNMENT_EXPR => {
                        let assign = AssignmentExpr::cast(node.clone())?;
                        if !matches!(
                            assign.op_kind(),
                            Some(
                                SyntaxKind::ASSIGN_LEFT
                                    | SyntaxKind::ASSIGN_EQ
                                    | SyntaxKind::SUPER_ASSIGN
                            )
                        ) || assign.target_name().is_none()
                        {
                            return None;
                        }
                        match assign.value_element()? {
                            SyntaxElement::Node(value) => FunctionExpr::cast(value),
                            SyntaxElement::Token(_) => None,
                        }
                    }
                    _ => None,
                };
            }
        }
        next = element.next_sibling_or_token();
    }
    None
}

/// What a `@param` section documents.
pub(crate) enum ParamDoc {
    /// `@param` with no name and no prose at all.
    Empty,
    /// One or more names (each with the sub-range of its own text), plus
    /// whether any description prose is present.
    Named {
        names: Vec<(smol_str::SmolStr, TextRange)>,
        has_description: bool,
    },
    /// Prose exists but no name could be recovered (exotic markup head); the
    /// caller should judge nothing about this param.
    Unknown,
}

/// Classify a `@param` section. Returns `None` for non-`@param` sections.
///
/// roxygen2 folds continuation lines into the tag value and splits on the
/// first whitespace, so `@param` with the name on the next line still names a
/// param—when the arg token is absent, the first word of the section's
/// leading prose is used instead.
pub(crate) fn param_doc(section: &RoxygenSection) -> Option<ParamDoc> {
    let tag = section.tag()?;
    if tag.name().as_deref() != Some("param") {
        return None;
    }
    if tag.arg().is_some() {
        return Some(ParamDoc::Named {
            names: tag.arg_names(),
            has_description: section.has_prose(),
        });
    }
    if !section.has_prose() {
        return Some(ParamDoc::Empty);
    }
    // No arg token but prose follows: recover the name from the first word of
    // the first plain-text leaf, as roxygen2's fold-then-split would.
    let tag_end = tag.syntax().text_range().end();
    let first_text = section
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::ROXYGEN_TEXT && t.text_range().start() >= tag_end);
    let Some(token) = first_text else {
        return Some(ParamDoc::Unknown);
    };
    let text = token.text();
    let Some(word) = text.split_whitespace().next() else {
        return Some(ParamDoc::Unknown);
    };
    let word_offset = text.find(word).expect("first word is in its own text");
    let word_start = token.text_range().start() + TextSize::from(word_offset as u32);
    let names = split_comma_names(word, word_start);
    if names.is_empty() {
        return Some(ParamDoc::Unknown);
    }
    let has_description = !text[word_offset + word.len()..].trim().is_empty()
        || section.paragraphs().count() > 1
        || section
            .syntax()
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| {
                t.kind() == SyntaxKind::ROXYGEN_TEXT
                    && t.text_range().start() > token.text_range().end()
            });
    Some(ParamDoc::Named {
        names,
        has_description,
    })
}

/// Split a comma-joined name list (`a,b`) into names with their sub-ranges,
/// mirroring `RoxygenTag::arg_names`.
fn split_comma_names(text: &str, start: TextSize) -> Vec<(smol_str::SmolStr, TextRange)> {
    let mut names = Vec::new();
    let mut offset = 0usize;
    for piece in text.split(',') {
        let trimmed = piece.trim();
        if !trimmed.is_empty() {
            let lead = piece.len() - piece.trim_start().len();
            let piece_start = start + TextSize::from((offset + lead) as u32);
            names.push((
                smol_str::SmolStr::new(trimmed),
                TextRange::at(piece_start, TextSize::of(trimmed)),
            ));
        }
        offset += piece.len() + 1;
    }
    names
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
