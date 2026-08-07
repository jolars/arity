//! `roxygen2-compat`: documentation constructs mismatched with the project's
//! roxygen2 version.
//!
//! The targeted version comes from `[compat] roxygen2` in `arity.toml`, or —
//! when unset — from the package `DESCRIPTION` (`Config/roxygen2/version`,
//! then the legacy `RoxygenNote`). Without either the rule stays silent.
//!
//! Two directions, keyed on the 8.0.0 boundary:
//!
//! - **Targeting < 8.0.0** flags 8.0.0-only syntax, which an older roxygen2
//!   mishandles: the `@prop` (S7) and `@R6method` tags (warned about as
//!   unknown and dropped), `` `Rd expr` `` render-time code spans (treated as
//!   ordinary code spans), `@inheritParams` argument filters (silently
//!   misread as literal argument names — a correctness trap, not a warning),
//!   and backtick-quoted two-part names containing spaces (`` @param `arg 1`
//!   ``; older versions split on whitespace and document the wrong name).
//! - **Targeting >= 8.0.0** flags a single-line tag whose value spans lines:
//!   8.0.0 warns on those (`@aliases`, `@rdname`, `@importFrom`, … — the
//!   `tag_words`/`tag_value` family), where 7.x accepted them silently.
//!
//! No fixes: collapsing a multiline value or rewriting a filter list has no
//! single correct textual form, so the findings are report-only.

use rowan::ast::AstNode as _;

use crate::ast::RoxygenTag;
use crate::config::{CompatConfig, CompatVersion};
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct Roxygen2Compat;

const EXAMPLES: &[Example] = &[Example {
    caption: "An `@inheritParams` filter under a declared roxygen2 7.x \
              (`roxygen2 = \"7.3.2\"` under `[compat]` in `arity.toml`):",
    source: "#' Add one\n#' @inheritParams other -verbose\nadd_one <- function(x) x + 1\n",
}];

/// Tags whose value roxygen2 8.0.0 requires on a single line (`tag_words`
/// with the default single-line check, plus the `tag_value` family the NEWS
/// lists); a multiline value warns there where 7.x accepted it. Sorted for
/// binary search (capitals first, byte order).
const SINGLE_LINE_TAGS: &[&str] = &[
    "aliases",
    "concept",
    "encoding",
    "exportClass",
    "exportMethod",
    "exportPattern",
    "exportS3Method",
    "importClassesFrom",
    "importFrom",
    "importMethodsFrom",
    "include",
    "includeRmd",
    "inheritDotParams",
    "inheritParams",
    "inheritSection",
    "keywords",
    "method",
    "name",
    "order",
    "rdname",
    "template",
    "useDynLib",
];

/// The 8.0.0 boundary every check keys on.
fn v800() -> CompatVersion {
    CompatVersion::parse("8.0.0").expect("valid version")
}

impl Rule for Roxygen2Compat {
    fn id(&self) -> &'static str {
        "roxygen2-compat"
    }

    fn description(&self) -> &'static str {
        "Flag documentation constructs mismatched with the project's roxygen2 \
         version.\n\nThe targeted version comes from `[compat] roxygen2` in \
         `arity.toml`, or from the package `DESCRIPTION` \
         (`Config/roxygen2/version`, then the legacy `RoxygenNote`); without \
         either, the rule stays silent. Targeting a version below 8.0.0 flags \
         syntax only 8.0.0 understands—`@prop`, `@R6method`, `` `Rd expr` `` \
         render-time code spans, `@inheritParams` argument filters (which \
         older versions silently misread as argument names), and \
         backtick-quoted names containing spaces. Targeting 8.0.0 or later \
         flags a single-line tag (`@rdname`, `@importFrom`, …) whose value \
         spans lines, which 8.0.0 warns about."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn doc_compat(&self) -> CompatConfig {
        CompatConfig {
            r: None,
            roxygen2: Some("7.3.2".to_string()),
        }
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::ROXYGEN_TAG, SyntaxKind::ROXYGEN_MD_CODE]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        if el.kind() == SyntaxKind::ROXYGEN_MD_CODE {
            check_md_code(el, ctx, sink);
            return;
        }
        let Some(tag) = el.as_node().cloned().and_then(RoxygenTag::cast) else {
            return;
        };
        let Some(name) = tag.name() else {
            return;
        };
        let Some(floor) = ctx.roxygen2_compat_floor() else {
            return;
        };
        if floor < v800() {
            check_new_syntax(&tag, &name, &floor, sink);
        } else if SINGLE_LINE_TAGS.binary_search(&name.as_str()).is_ok()
            && tag_has_continuation_lines(&tag)
        {
            push_finding(
                sink,
                tag_head_range(&tag),
                format!(
                    "roxygen2 8.0.0 warns when `@{name}`'s value spans multiple \
                     lines (this project targets roxygen2 {floor})"
                ),
                "Join the value onto the tag line.",
            );
        }
    }
}

/// The `< 8.0.0` direction: 8.0.0-only tag syntax under an older target.
fn check_new_syntax(
    tag: &RoxygenTag,
    name: &str,
    floor: &CompatVersion,
    sink: &mut Vec<Diagnostic>,
) {
    if matches!(name, "prop" | "R6method") {
        push_finding(
            sink,
            tag_head_range(tag),
            format!("`@{name}` requires roxygen2 >= 8.0.0 (this project targets {floor})"),
            "Raise `[compat] roxygen2` (or re-document with roxygen2 8.0.0, which \
             records its version in `DESCRIPTION`).",
        );
        return;
    }
    // `@inheritParams other x -z` argument filters: an older roxygen2 silently
    // reads the filters as literal argument names — a correctness trap.
    if name == "inheritParams"
        && let Some(text) = tag.text()
        && !text.text().trim().is_empty()
    {
        push_finding(
            sink,
            text.text_range(),
            format!(
                "`@inheritParams` argument filters require roxygen2 >= 8.0.0; \
                 older versions silently misread them as argument names \
                 (this project targets {floor})"
            ),
            "Drop the filters or raise `[compat] roxygen2`.",
        );
        return;
    }
    // A backtick-quoted two-part name containing whitespace: older versions
    // split on whitespace and document a wrong, partial name.
    if let Some(arg) = tag.arg() {
        let text = arg.text();
        if text.starts_with('`') && text.ends_with('`') && text.contains(char::is_whitespace) {
            push_finding(
                sink,
                arg.text_range(),
                format!(
                    "a backtick-quoted name with spaces requires roxygen2 >= 8.0.0 \
                     (this project targets {floor})"
                ),
                "Rename the argument or raise `[compat] roxygen2`.",
            );
        }
    }
}

/// An `` `Rd expr` `` render-time code span (roxygen2 8.0.0's
/// `\\Sexpr[stage=render,results=rd]` syntax): under an older target it is an
/// ordinary code span and renders literally.
fn check_md_code(el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
    let text = match el {
        SyntaxElement::Token(t) => t.text().to_string(),
        SyntaxElement::Node(n) => n.text().to_string(),
    };
    let inner = text.trim_start_matches('`');
    if !inner.starts_with("Rd ") {
        return;
    }
    let Some(floor) = ctx.roxygen2_compat_floor() else {
        return;
    };
    if floor >= v800() {
        return;
    }
    push_finding(
        sink,
        el.text_range(),
        format!(
            "an `Rd expr` render-time code span requires roxygen2 >= 8.0.0; \
             older versions render it as literal code (this project targets {floor})"
        ),
        "Write the Rd macro directly or raise `[compat] roxygen2`.",
    );
}

/// Whether a (non-prose-folding) tag's value continues past its first line:
/// its enclosing section carries paragraph content after the tag node.
fn tag_has_continuation_lines(tag: &RoxygenTag) -> bool {
    let node = tag.syntax();
    let Some(section) = node.parent() else {
        return false;
    };
    section
        .children()
        .skip_while(|child| child != node)
        .skip(1)
        .any(|child| child.kind() == SyntaxKind::ROXYGEN_PARAGRAPH)
}

/// The `@` + name span of a tag — the caret target, not the whole value.
fn tag_head_range(tag: &RoxygenTag) -> rowan::TextRange {
    let node_range = tag.syntax().text_range();
    match (tag.at(), tag.name()) {
        (Some(at), Some(name)) => rowan::TextRange::at(
            at.text_range().start(),
            (u32::from(at.text_range().len()) + name.len() as u32).into(),
        ),
        _ => node_range,
    }
}

fn push_finding(
    sink: &mut Vec<Diagnostic>,
    range: rowan::TextRange,
    message: String,
    suggestion: &str,
) {
    sink.push(Diagnostic {
        rule: "roxygen2-compat",
        severity: Default::default(),
        path: Default::default(),
        range,
        message: ViolationData::new("roxygen2-compat", message).with_suggestion(suggestion),
        fix: None,
    });
}
