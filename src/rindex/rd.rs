//! Render an Rd help page to the lightweight-markdown fields of [`HelpDoc`].
//!
//! A help page fetched from `help/{pkg}.rdb` decodes (via [`rds`](crate::rindex::rds))
//! to a `VECSXP` whose elements each carry an `Rd_tag` attribute — section tags
//! like `\title`, `\description`, `\usage`, `\arguments`, and, nested inside,
//! inline tags like `TEXT`, `RCODE`, `\code`, `\link`. Leaves are length-1
//! character vectors; macros are sub-lists. This module walks that tree and
//! renders the documented sections to markdown. It is pure (no I/O) and never
//! panics — a malformed page yields empty/partial sections.

use crate::rindex::rds::{Rkind, Robj};
use crate::rindex::schema::{HelpArg, HelpDoc};

/// The sections extracted from one Rd page. `title` is the page's own `\title`,
/// used only as a fallback when the `Meta/Rd.rds` title is absent.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RdSections {
    pub title: Option<String>,
    pub description: Option<String>,
    pub usage: Option<String>,
    pub arguments: Vec<HelpArg>,
}

/// Walk a fetched Rd page and extract `\title`/`\description`/`\usage`/
/// `\arguments`, rendered to markdown. A non-Rd object yields an empty result.
pub fn render_page(page: &Robj) -> RdSections {
    let mut out = RdSections::default();
    let Some(children) = page.as_list() else {
        return out;
    };
    for node in children {
        match rd_tag(node) {
            Some("\\title") => out.title = nonempty(collapse_ws(&render_children(node))),
            Some("\\description") => {
                out.description = nonempty(normalize_paragraphs(&render_children(node)));
            }
            Some("\\usage") => out.usage = render_usage(node),
            Some("\\arguments") => render_arguments(node, &mut out.arguments),
            _ => {}
        }
    }
    out
}

/// Merge a `Meta/Rd.rds` title (authoritative) with rendered page sections.
pub fn into_help_doc(title: Option<String>, sections: RdSections) -> HelpDoc {
    HelpDoc {
        title: title.or(sections.title),
        description: sections.description,
        usage: sections.usage,
        arguments: sections.arguments,
    }
}

/// The `Rd_tag` attribute of a node, if any (e.g. `"\\code"`, `"TEXT"`).
fn rd_tag(node: &Robj) -> Option<&str> {
    node.attr("Rd_tag").and_then(|t| t.as_str())
}

/// Render the inline content of a node's children to a fresh markdown string.
fn render_children(node: &Robj) -> String {
    let mut buf = String::new();
    if let Some(children) = node.as_list() {
        for c in children {
            render_inline(c, &mut buf);
        }
    } else {
        render_inline(node, &mut buf);
    }
    buf
}

/// Append the markdown rendering of a single inline node to `buf`.
fn render_inline(node: &Robj, buf: &mut String) {
    match &node.kind {
        // A text/code leaf: emit its literal content.
        Rkind::Str(v) => {
            for s in v.iter().flatten() {
                buf.push_str(s);
            }
        }
        Rkind::List(children) => match rd_tag(node) {
            // Monospace macros → backtick span (whitespace-collapsed inside).
            Some("\\code" | "\\verb" | "\\samp" | "\\kbd" | "\\env" | "\\option" | "\\command") => {
                let inner = collapse_ws(&render_node_list(children));
                if !inner.is_empty() {
                    buf.push('`');
                    buf.push_str(&inner);
                    buf.push('`');
                }
            }
            // Emphasis macros → markdown emphasis.
            Some("\\emph") => wrap(buf, "*", &render_node_list(children)),
            Some("\\strong" | "\\bold") => wrap(buf, "**", &render_node_list(children)),
            // Links and everything unrecognized: emit the inner text verbatim.
            _ => buf.push_str(&render_node_list(children)),
        },
        _ => {}
    }
}

fn render_node_list(children: &[Robj]) -> String {
    let mut buf = String::new();
    for c in children {
        render_inline(c, &mut buf);
    }
    buf
}

fn wrap(buf: &mut String, delim: &str, inner: &str) {
    let inner = collapse_ws(inner);
    if !inner.is_empty() {
        buf.push_str(delim);
        buf.push_str(&inner);
        buf.push_str(delim);
    }
}

/// The `\usage` block: verbatim R code, with surrounding blank lines trimmed.
fn render_usage(node: &Robj) -> Option<String> {
    let mut buf = String::new();
    collect_verbatim(node, &mut buf);
    nonempty(buf.trim_matches('\n').trim_end().to_string())
}

fn collect_verbatim(node: &Robj, buf: &mut String) {
    match &node.kind {
        Rkind::Str(v) => {
            for s in v.iter().flatten() {
                buf.push_str(s);
            }
        }
        Rkind::List(children) => {
            for c in children {
                collect_verbatim(c, buf);
            }
        }
        _ => {}
    }
}

/// Collect each `\item{name}{desc}` of an `\arguments` block into a [`HelpArg`].
fn render_arguments(node: &Robj, out: &mut Vec<HelpArg>) {
    let Some(children) = node.as_list() else {
        return;
    };
    for child in children {
        if rd_tag(child) != Some("\\item") {
            continue;
        }
        let Some(parts) = child.as_list() else {
            continue;
        };
        let name = parts.first().map(render_children).unwrap_or_default();
        let name = collapse_ws(&name);
        if name.is_empty() {
            continue;
        }
        let mut desc = String::new();
        for p in parts.iter().skip(1) {
            render_inline(p, &mut desc);
        }
        out.push(HelpArg {
            name,
            description: normalize_paragraphs(&desc),
        });
    }
}

/// Collapse all runs of whitespace (including newlines) to single spaces.
fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Collapse intra-paragraph whitespace while keeping blank-line paragraph
/// breaks (rendered as a markdown `\n\n`).
fn normalize_paragraphs(s: &str) -> String {
    let mut paras: Vec<String> = Vec::new();
    let mut cur = String::new();
    for line in s.split('\n') {
        if line.trim().is_empty() {
            if !cur.trim().is_empty() {
                paras.push(collapse_ws(&cur));
            }
            cur.clear();
        } else {
            cur.push(' ');
            cur.push_str(line);
        }
    }
    if !cur.trim().is_empty() {
        paras.push(collapse_ws(&cur));
    }
    paras.join("\n\n")
}

fn nonempty(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smol_str::SmolStr;

    // --- synthetic Rd-tree constructors -----------------------------------

    fn tag_attr(tag: &str) -> Vec<(SmolStr, Robj)> {
        vec![(
            SmolStr::new("Rd_tag"),
            Robj {
                kind: Rkind::Str(vec![Some(tag.to_string())]),
                attr: Vec::new(),
            },
        )]
    }
    fn leaf(tag: &str, s: &str) -> Robj {
        Robj {
            kind: Rkind::Str(vec![Some(s.to_string())]),
            attr: tag_attr(tag),
        }
    }
    fn node(tag: &str, children: Vec<Robj>) -> Robj {
        Robj {
            kind: Rkind::List(children),
            attr: tag_attr(tag),
        }
    }
    /// A brace-group inside an `\item`: a list with no own tag.
    fn group(children: Vec<Robj>) -> Robj {
        Robj {
            kind: Rkind::List(children),
            attr: Vec::new(),
        }
    }
    fn page(sections: Vec<Robj>) -> Robj {
        Robj {
            kind: Rkind::List(sections),
            attr: Vec::new(),
        }
    }

    #[test]
    fn inline_code_becomes_backticks_and_links_flatten() {
        let desc = node(
            "\\description",
            vec![
                leaf("TEXT", "Use "),
                node("\\code", vec![leaf("RCODE", "x %>% f")]),
                leaf("TEXT", " or "),
                node("\\link", vec![leaf("TEXT", "freduce")]),
                leaf("TEXT", "."),
            ],
        );
        let out = render_page(&page(vec![desc]));
        assert_eq!(
            out.description.as_deref(),
            Some("Use `x %>% f` or freduce.")
        );
    }

    #[test]
    fn description_preserves_paragraph_breaks() {
        let desc = node(
            "\\description",
            vec![
                leaf("TEXT", "\nFirst   paragraph\nwraps.\n"),
                leaf("TEXT", "\n"),
                leaf("TEXT", "Second paragraph.\n"),
            ],
        );
        let out = render_page(&page(vec![desc]));
        assert_eq!(
            out.description.as_deref(),
            Some("First paragraph wraps.\n\nSecond paragraph.")
        );
    }

    #[test]
    fn usage_is_verbatim_with_trimmed_blank_lines() {
        let usage = node(
            "\\usage",
            vec![leaf("RCODE", "\n"), leaf("RCODE", "lhs %>% rhs\n")],
        );
        let out = render_page(&page(vec![usage]));
        assert_eq!(out.usage.as_deref(), Some("lhs %>% rhs"));
    }

    #[test]
    fn arguments_collect_items_including_grouped_names() {
        let args = node(
            "\\arguments",
            vec![
                leaf("TEXT", "\n  "),
                node(
                    "\\item",
                    vec![
                        group(vec![leaf("TEXT", "x, y")]),
                        group(vec![leaf("TEXT", "Two values.")]),
                    ],
                ),
                node(
                    "\\item",
                    vec![
                        group(vec![leaf("TEXT", "lhs")]),
                        group(vec![
                            leaf("TEXT", "A value or the "),
                            node("\\code", vec![leaf("RCODE", ".")]),
                            leaf("TEXT", " placeholder."),
                        ]),
                    ],
                ),
            ],
        );
        let out = render_page(&page(vec![args]));
        assert_eq!(out.arguments.len(), 2);
        assert_eq!(out.arguments[0].name, "x, y");
        assert_eq!(out.arguments[0].description, "Two values.");
        assert_eq!(out.arguments[1].name, "lhs");
        assert_eq!(
            out.arguments[1].description,
            "A value or the `.` placeholder."
        );
    }

    #[test]
    fn unknown_macro_recurses_into_children() {
        let desc = node(
            "\\description",
            vec![node("\\insertRef", vec![leaf("TEXT", "kept text")])],
        );
        let out = render_page(&page(vec![desc]));
        assert_eq!(out.description.as_deref(), Some("kept text"));
    }

    #[test]
    fn non_list_page_is_empty_no_panic() {
        let bogus = Robj {
            kind: Rkind::Str(vec![Some("oops".into())]),
            attr: Vec::new(),
        };
        assert_eq!(render_page(&bogus), RdSections::default());
    }

    #[test]
    fn into_help_doc_prefers_meta_title() {
        let sections = RdSections {
            title: Some("Page title".into()),
            description: Some("d".into()),
            ..Default::default()
        };
        let doc = into_help_doc(Some("Meta title".into()), sections);
        assert_eq!(doc.title.as_deref(), Some("Meta title"));
        assert_eq!(doc.description.as_deref(), Some("d"));
    }

    #[test]
    fn into_help_doc_falls_back_to_page_title() {
        let sections = RdSections {
            title: Some("Page title".into()),
            ..Default::default()
        };
        let doc = into_help_doc(None, sections);
        assert_eq!(doc.title.as_deref(), Some("Page title"));
    }
}
