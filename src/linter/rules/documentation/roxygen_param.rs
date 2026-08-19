//! `roxygen-param`: `@param` documentation that does not match the function.
//!
//! Four shapes are flagged: a formal argument with no covering `@param`, a
//! `@param` naming a formal the function does not have, the same name
//! documented twice, and a `@param` missing its name or description
//! (roxygen2's "requires a name and description" warning). Coverage
//! (missing/nonexistent) is judged only when the block unambiguously
//! documents a plain `name <- function(...)` and does not inherit docs from
//! another object (`@inheritParams`, `@template`, …); duplicates are a
//! per-block fact and are always checked.
//!
//! Coverage is judged against the **topic**, not the block. `@rdname` merges
//! several functions into one `.Rd`, and roxygen2 checks `@param` against the
//! union of the merged functions' formals — so a block that owns a topic is
//! judged against every block that joins it, resolved across the whole package
//! by [`topic_members`] — roxygen2 merges topics package-wide, so a joiner in a
//! sibling `R/` file counts. A block that *joins* someone else's topic is not
//! judged at all: it is not the topic's owner.
//!
//! No fixes: adding a `@param` means inventing prose, and deleting one drops
//! prose the author wrote.

use rowan::TextRange;
use rowan::ast::AstNode as _;

use crate::ast::{RoxygenBlock, RoxygenTag};
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::roxygen::{
    ParamDoc, documented_function, documents_s3_method, has_title, inherits_params, param_doc,
    topic_members,
};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct RoxygenParam;

const EXAMPLES: &[Example] = &[Example {
    caption: "`y` is undocumented and `@param z` matches nothing:",
    source: "#' Add two numbers\n#' @param x The first number.\n#' @param z The other one.\n#' @export\nadd <- function(x, y) x + y\n",
}];

/// The argument surface of one Rd topic: every formal of the functions merged
/// into it, and every name any of their blocks documents with `@param`.
#[derive(Default)]
struct TopicParams {
    formals: Vec<String>,
    documented: Vec<String>,
}

impl TopicParams {
    fn contains_formal(&self, name: &str) -> bool {
        self.formals.iter().any(|formal| formal == name)
    }

    fn is_documented(&self, name: &str) -> bool {
        self.documented.iter().any(|doc| doc == name)
    }
}

/// The union this block's `@param`s are judged against, or `None` when the
/// topic is unknowable and coverage must be skipped entirely.
///
/// Unknowable means any of: the block inherits its argument list from another
/// object; it joins a topic someone else owns; or some member of its topic
/// documents a statement arity cannot classify (a `setMethod`, an R6 call),
/// whose formals are therefore invisible.
fn topic_params(block: &RoxygenBlock, ctx: &RuleContext<'_>) -> Option<TopicParams> {
    if inherits_params(block) {
        return None;
    }
    let members = topic_members(block, ctx)?;
    let mut params = TopicParams::default();
    for member in members {
        if member.inherits_params {
            return None;
        }
        params.formals.extend(member.formals.clone()?);
        params
            .documented
            .extend(member.documented_params.iter().cloned());
    }
    Some(params)
}

/// The range of the tag head: `@param` alone, or `@param name` when an arg is
/// present.
fn tag_head_range(tag: &RoxygenTag) -> Option<TextRange> {
    let start = tag.at()?.text_range().start();
    let end = tag
        .arg()
        .map(|arg| arg.text_range().end())
        .or_else(|| Some(tag.syntax().text_range().end()))?;
    Some(TextRange::new(start, end))
}

impl Rule for RoxygenParam {
    fn id(&self) -> &'static str {
        "roxygen-param"
    }

    fn description(&self) -> &'static str {
        "Flag `@param` documentation that does not match the documented \
         function.\
         \n\nFour shapes are reported: a formal argument with no `@param`, a \
         `@param` naming a nonexistent formal (often a rename that never \
         reached the docs), a name documented twice, and a `@param` missing \
         its name or description. Coverage is judged against the generated \
         topic, not the block: `@rdname` and `@describeIn` merge several \
         functions into one `.Rd`, so a block owning a topic is judged against \
         the union of every joiner's formals, anywhere in the package. Blocks that inherit \
         documentation from another object (`@inheritParams`, `@template`, …) \
         or that join a topic owned elsewhere are exempt from the coverage \
         checks, and a titleless S3 method (registered with `S3method()`, so it \
         generates no `.Rd`) is exempt from the missing-`@param` check; \
         duplicates are always reported."
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
        // `@noRd` suppresses the topic outright, so there is no argument list
        // that could be incomplete or name a nonexistent formal. Matches
        // `roxygen-return`, which already skips these blocks.
        if block.has_tag("noRd") {
            return;
        }
        // Import directives feed NAMESPACE but do not create an Rd topic, so
        // the following function has no argument documentation to complete.
        if is_import_only(&block) {
            return;
        }
        let function = documented_function(&block);
        let topic = topic_params(&block, ctx);
        let judge_coverage = function.is_some() && topic.is_some();
        // A registered S3 method with no title generates no `.Rd`, so it has no
        // parameter list that could be incomplete—the generic's topic documents
        // the arguments. A `@param` naming a nonexistent formal is still a
        // block-vs-function mismatch, so only the missing direction is dropped.
        let judge_missing =
            judge_coverage && !(documents_s3_method(&block, ctx) && !has_title(&block));
        let topic = topic.unwrap_or_default();
        // The block's own formals are what the missing-`@param` findings point
        // at; the topic's union is what they are judged against.
        let formals = function.map(|f| f.params()).unwrap_or_default();

        let mut documented: Vec<smol_str::SmolStr> = Vec::new();
        let mut any_unknown = false;
        for section in block.sections() {
            let Some(doc) = param_doc(&section) else {
                continue;
            };
            let tag = section.tag().expect("param sections have a tag");
            match doc {
                ParamDoc::Unknown => any_unknown = true,
                ParamDoc::Empty => {
                    let Some(range) = tag_head_range(&tag) else {
                        continue;
                    };
                    sink.push(diagnostic(
                        range,
                        "`@param` requires a name and description".to_string(),
                        "Write `@param <name> <description>`.",
                    ));
                }
                ParamDoc::Named {
                    names,
                    has_description,
                } => {
                    if !has_description {
                        let Some(range) = tag_head_range(&tag) else {
                            continue;
                        };
                        let list = names
                            .iter()
                            .map(|(n, _)| n.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        sink.push(diagnostic(
                            range,
                            format!("`@param {list}` has no description"),
                            "Describe the argument after its name.",
                        ));
                    }
                    for (name, range) in names {
                        if documented.contains(&name) {
                            sink.push(diagnostic(
                                range,
                                format!("`@param {name}` is documented more than once"),
                                "Remove or merge the duplicate entry.",
                            ));
                            continue;
                        }
                        if judge_coverage && !topic.contains_formal(&name) {
                            sink.push(diagnostic(
                                range,
                                format!(
                                    "`@param {name}` does not match a formal argument of the \
                                     documented function"
                                ),
                                "Rename it to a formal argument or remove it.",
                            ));
                        }
                        documented.push(name);
                    }
                }
            }
        }

        // An unrecoverable `@param` head means coverage can't be judged.
        if !judge_missing || any_unknown {
            return;
        }
        for formal in formals {
            if !documented.contains(&formal.name) && !topic.is_documented(&formal.name) {
                sink.push(diagnostic(
                    formal.name_token.text_range(),
                    format!(
                        "formal argument `{}` is not documented with `@param`",
                        formal.name
                    ),
                    "Add `@param` for it (or `@inheritParams` a function that documents it).",
                ));
            }
        }
    }
}

fn is_import_only(block: &RoxygenBlock) -> bool {
    let mut tags = block.tags().peekable();
    tags.peek().is_some()
        && tags.all(|tag| {
            matches!(
                tag.name().as_deref(),
                Some("import" | "importFrom" | "importClassesFrom" | "importMethodsFrom")
            )
        })
}

fn diagnostic(range: TextRange, body: String, suggestion: &str) -> Diagnostic {
    Diagnostic {
        rule: "roxygen-param",
        severity: Default::default(),
        path: Default::default(),
        range,
        message: ViolationData::new("roxygen-param", body).with_suggestion(suggestion),
        fix: None,
    }
}
