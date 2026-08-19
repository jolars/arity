//! Per-file roxygen topic projection: which Rd topics a file's blocks merge
//! into, and the facts the documentation rules judge a topic by.
//!
//! roxygen2 merges every block resolving to the same topic into one `.Rd` and
//! checks the merged result, and it does that **package-wide** — an owner in
//! `R/a.R` and a `@rdname` joiner in `R/b.R` land in the same file. So the
//! question "does this topic document `x`?" is cross-file, which puts the
//! grouping here rather than in the linter, next to the other per-file
//! projections that [`crate::project::graph`] folds into a project index.
//!
//! [`TopicMember`] is the reduction: one block's contribution, boiled down to
//! the handful of booleans and name lists the three `roxygen-*` topic rules
//! ask about. It is deliberately **range-free** — like [`file_class_defs`] and
//! [`file_def_sites`], that is what lets the projection backdate across a body
//! edit so the project graph's memo survives a keystroke. A rule that needs a
//! span has the block in hand and reads it from the live tree.
//!
//! [`file_class_defs`]: crate::project::file_class_defs
//! [`file_def_sites`]: crate::project::file_def_sites

use std::collections::BTreeMap;

use rowan::ast::AstNode as _;
use rowan::{TextRange, TextSize};
use smol_str::SmolStr;

use crate::ast::{AssignmentExpr, FunctionExpr, RoxygenBlock, RoxygenSection};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

/// One roxygen block's contribution to an Rd topic.
///
/// Range-free and node-free by construction: a `TextRange` here would look
/// harmless and would silently cost the project graph a rebuild per keystroke
/// (`.claude/rules/semantic.md`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, salsa::SalsaValue)]
pub struct TopicMember {
    /// A leading prose paragraph or an explicit `@title` — roxygen2 writes no
    /// `.Rd` without one, and the first value the merged topic offers wins.
    pub has_title: bool,
    /// `@return` or its `@returns` alias: the topic's `\value` section.
    pub has_value: bool,
    /// The block pulls its argument list in from another *object*
    /// (`@inheritParams`, `@template`, …), which no amount of cross-block
    /// resolution recovers — it makes the whole topic's argument surface
    /// unknowable.
    pub inherits_params: bool,
    /// `@rdname` / `@describeIn`: this block joins a topic someone else owns.
    pub joins: bool,
    /// The documented function's formal argument names. `None` when the
    /// documented statement is not a plain `name <- function(...)` (a
    /// `setMethod`, an R6 class call, `"_PACKAGE"`), whose formals are
    /// invisible to a static reader — also making the argument surface
    /// unknowable.
    pub formals: Option<Vec<String>>,
    /// Every name this block documents with `@param`, in document order.
    pub documented_params: Vec<String>,
}

/// Group `root`'s roxygen blocks by the Rd topic they resolve to, in document
/// order. Blocks that name no topic are dropped: they merge with nothing.
///
/// The per-file half of the package-wide index — [`crate::project::graph`]
/// folds one of these per member into
/// [`RoxygenTopicIndex`](crate::project::RoxygenTopicIndex).
pub fn file_roxygen_topics(root: &SyntaxNode) -> BTreeMap<String, Vec<TopicMember>> {
    let mut topics: BTreeMap<String, Vec<TopicMember>> = BTreeMap::new();
    for block in root.descendants().filter_map(RoxygenBlock::cast) {
        if let Some(key) = topic_key(&block) {
            topics
                .entry(key.to_string())
                .or_default()
                .push(topic_member(&block));
        }
    }
    topics
}

/// Reduce one block to the facts a topic is judged by.
pub fn topic_member(block: &RoxygenBlock) -> TopicMember {
    let mut documented_params = Vec::new();
    for section in block.sections() {
        if let Some(ParamDoc::Named { names, .. }) = param_doc(&section) {
            documented_params.extend(names.into_iter().map(|(name, _)| name.to_string()));
        }
    }
    TopicMember {
        has_title: has_title(block),
        has_value: block.has_tag("return") || block.has_tag("returns"),
        inherits_params: inherits_params(block),
        joins: joins_other_topic(block),
        formals: documented_function(block)
            .map(|f| f.params().into_iter().map(|p| p.name.to_string()).collect()),
        documented_params,
    }
}

/// Tags that pull the argument list in from outside this block. Unlike a topic
/// merge ([`joins_other_topic`]), no amount of cross-block resolution recovers
/// what they add: the source is another *object*, possibly in another package.
pub fn inherits_params(block: &RoxygenBlock) -> bool {
    block.tags().any(|tag| {
        matches!(
            tag.name().as_deref(),
            Some(
                "inherit"
                    | "inheritParams"
                    | "inheritSection"
                    | "inheritDotParams"
                    | "template"
                    | "usage"
            )
        )
    })
}

/// Whether this block merges into a topic named by someone else (`@rdname`,
/// `@describeIn`). Such a block is a joiner, not the topic's owner.
pub fn joins_other_topic(block: &RoxygenBlock) -> bool {
    block.has_tag("rdname") || block.has_tag("describeIn")
}

/// Whether the block supplies a topic title: a leading prose paragraph (whose
/// first sentence becomes the title) or an explicit `@title`. roxygen2 writes
/// no `.Rd` without one, so this gates every rule whose subject is a section
/// of the generated topic.
pub fn has_title(block: &RoxygenBlock) -> bool {
    block.intro().is_some_and(|intro| intro.has_prose()) || block.has_tag("title")
}

/// The Rd topic this block resolves to, in roxygen2's precedence: an explicit
/// `@name`, else `@rdname`, else `@describeIn`'s destination (its first word,
/// the rest being the description), else the name the documented statement
/// binds — an object's default topic. `None` when the block names no topic and
/// binds nothing.
///
/// `@name`/`@rdname`/`@describeIn` are not arg-bearing tags (`ARG_BEARING_TAGS`
/// in the roxygen sub-lexer), so the value comes from the tag's text — read via
/// [`RoxygenTag::value_text`](crate::ast::RoxygenTag::value_text), because
/// under `@md` a name containing `_` or `*` lexes as several leaves around an
/// unresolved markdown delimiter.
///
/// The test-only Rd projector carries a narrower twin of this
/// (`crate::roxygen::project_rd`'s `topic_name`): it stops at `@name`/`@rdname`
/// because merging *sections* never needs an object's default topic, while
/// judging an owner block against its topic does. Keep the two in step.
pub fn topic_key(block: &RoxygenBlock) -> Option<SmolStr> {
    let mut rdname: Option<SmolStr> = None;
    let mut describe_in: Option<SmolStr> = None;
    for tag in block.tags() {
        match tag.name().as_deref() {
            Some("name") => {
                if let Some(value) = tag.value_text() {
                    return Some(SmolStr::new(value));
                }
            }
            Some("rdname") if rdname.is_none() => rdname = tag.value_text().map(SmolStr::new),
            Some("describeIn") if describe_in.is_none() => {
                describe_in = tag
                    .value_text()
                    .and_then(|value| value.split_whitespace().next().map(SmolStr::new));
            }
            _ => {}
        }
    }
    rdname
        .or(describe_in)
        .or_else(|| documented_binding_name(block))
}

/// The statement this block documents: the next sibling node, skipping only
/// trivia. A non-trivia *token* before it (so the block does not lead a
/// statement) ends the walk.
fn documented_statement(block: &RoxygenBlock) -> Option<SyntaxNode> {
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
            SyntaxElement::Node(node) => return Some(node.clone()),
        }
        next = element.next_sibling_or_token();
    }
    None
}

/// The assignment this block documents, when it is a plain binding: `name <-`
/// (also `=` / `<<-`, string LHS). Excludes replacement-style targets
/// (`dim(x) <-`), which have no `target_name`.
fn documented_assignment(block: &RoxygenBlock) -> Option<AssignmentExpr> {
    let node = documented_statement(block)?;
    if node.kind() != SyntaxKind::ASSIGNMENT_EXPR {
        return None;
    }
    let assign = AssignmentExpr::cast(node)?;
    if !matches!(
        assign.op_kind(),
        Some(SyntaxKind::ASSIGN_LEFT | SyntaxKind::ASSIGN_EQ | SyntaxKind::SUPER_ASSIGN)
    ) || assign.target_name().is_none()
    {
        return None;
    }
    Some(assign)
}

/// The name this block's documented statement binds, when it is a plain
/// binding. `None` for a bare `function(...)` literal and every non-assignment
/// shape.
pub fn documented_binding_name(block: &RoxygenBlock) -> Option<SmolStr> {
    documented_assignment(block)?.target_name()
}

/// The function this block documents, when that is unambiguous: the next
/// non-trivia sibling is `name <- function(...)` (also `=` / `<<-`, string
/// LHS) or a bare `function(...)` literal. Anything else—another roxygen
/// block, `setMethod(...)`, an R6 class call, `"_PACKAGE"`, `NULL`—returns
/// `None`.
pub fn documented_function(block: &RoxygenBlock) -> Option<FunctionExpr> {
    let node = documented_statement(block)?;
    match node.kind() {
        SyntaxKind::FUNCTION_EXPR => FunctionExpr::cast(node),
        SyntaxKind::ASSIGNMENT_EXPR => match documented_assignment(block)?.value_element()? {
            SyntaxElement::Node(value) => FunctionExpr::cast(value),
            SyntaxElement::Token(_) => None,
        },
        _ => None,
    }
}

/// What a `@param` section documents.
pub enum ParamDoc {
    /// `@param` with no name and no prose at all.
    Empty,
    /// One or more names (each with the sub-range of its own text), plus
    /// whether any description prose is present.
    Named {
        names: Vec<(SmolStr, TextRange)>,
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
///
/// The ranges are for the linter, which reports on them; [`topic_member`]
/// drops them, which is what keeps the projection range-free.
pub fn param_doc(section: &RoxygenSection) -> Option<ParamDoc> {
    let tag = section.tag()?;
    if tag.name().as_deref() != Some("param") {
        return None;
    }
    if tag.arg().is_some() {
        return Some(ParamDoc::Named {
            names: tag
                .arg_names()
                .into_iter()
                .map(|(name, range)| {
                    let name = if name == "\\ldots" {
                        SmolStr::new("...")
                    } else {
                        name
                    };
                    (name, range)
                })
                .collect(),
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
fn split_comma_names(text: &str, start: TextSize) -> Vec<(SmolStr, TextRange)> {
    let mut names = Vec::new();
    let mut offset = 0usize;
    for piece in text.split(',') {
        let trimmed = piece.trim();
        if !trimmed.is_empty() {
            let lead = piece.len() - piece.trim_start().len();
            let piece_start = start + TextSize::from((offset + lead) as u32);
            names.push((
                SmolStr::new(trimmed),
                TextRange::at(piece_start, TextSize::of(trimmed)),
            ));
        }
        offset += piece.len() + 1;
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn topics(src: &str) -> BTreeMap<String, Vec<TopicMember>> {
        file_roxygen_topics(&parse(src).cst)
    }

    fn only(src: &str, key: &str) -> TopicMember {
        topics(src)
            .remove(key)
            .and_then(|mut m| (m.len() == 1).then(|| m.remove(0)))
            .unwrap_or_else(|| panic!("exactly one member under `{key}` in {src:?}"))
    }

    #[test]
    fn groups_an_owner_with_its_joiners() {
        let src = "#' Owner\n#' @param x X.\nf <- function() NULL\n\n\
                   #' @rdname f\ng <- function(x) x\n\n\
                   #' Unrelated\nh <- function() 1\n";
        let topics = topics(src);
        assert_eq!(topics["f"].len(), 2);
        assert_eq!(topics["h"].len(), 1);
        assert!(!topics.contains_key("nope"));
        // The owner leads; the joiner follows, in document order.
        assert!(!topics["f"][0].joins);
        assert!(topics["f"][1].joins);
    }

    #[test]
    fn records_the_facts_a_topic_is_judged_by() {
        let member = only(
            "#' Add\n#' @param x X.\n#' @return A number.\nf <- function(x, y) x\n",
            "f",
        );
        assert!(member.has_title);
        assert!(member.has_value);
        assert!(!member.inherits_params);
        assert!(!member.joins);
        assert_eq!(member.formals, Some(vec!["x".to_string(), "y".to_string()]));
        assert_eq!(member.documented_params, ["x"]);
    }

    #[test]
    fn returns_is_an_alias_for_return() {
        assert!(only("#' Add\n#' @returns A number.\nf <- function() 1\n", "f").has_value);
    }

    #[test]
    fn an_unclassifiable_statement_has_unknowable_formals() {
        // `setMethod`'s formals are invisible to a static reader, so the topic's
        // argument surface is unknowable rather than empty.
        let member = only(
            "#' Show\n#' @name show_c\nsetMethod(\"show\", \"C\", function(object) 1)\n",
            "show_c",
        );
        assert_eq!(member.formals, None);
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
            let keys: Vec<String> = topics(src).into_keys().collect();
            match expect {
                Some(key) => assert_eq!(keys, [key.to_string()], "case: {src:?}"),
                None => assert!(keys.is_empty(), "case: {src:?} -> {keys:?}"),
            }
        }
    }

    #[test]
    fn a_markdown_topic_name_is_not_truncated() {
        // Under `@md` an underscore is carved as an unresolved markdown
        // delimiter, so `@rdname missing_arg` is three leaves. Reading only the
        // first would resolve the topic to `missing` and lose the merge.
        let src = "#' @md\n#' Missing argument\nmissing_arg <- function() NULL\n\n\
                   #' @md\n#' @rdname missing_arg\nis_missing <- function(x) TRUE\n";
        assert_eq!(topics(src)["missing_arg"].len(), 2);
    }

    #[test]
    fn is_range_free_across_a_body_edit() {
        // The firewall property, stated locally: shifting a body leaves the
        // projection byte-identical, which is what lets salsa backdate it.
        let before = "#' Add\n#' @param x X.\nf <- function(x) {\n  x\n}\n";
        let after = "#' Add\n#' @param x X.\nf <- function(x) {\n  x + 0\n}\n";
        assert_eq!(topics(before), topics(after));
    }
}
