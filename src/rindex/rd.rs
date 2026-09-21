//! Arity's help presentation over canonical Rd and the legacy public object API.
//!
//! Both inputs share layout and Markdown policy. Canonical opaque nodes make
//! the affected output field unavailable rather than silently dropping prose.

use rd_ast::{
    RdAstPath, RdDocument, RdNode, RdNodeRef, RdNodesIter, RdNodesRef, RdSystemMacro,
    RdSystemMacroItem, RdSystemMacroItems, RdSystemMacroMatch,
};

use crate::rindex::rds::{Rkind, Robj};
use crate::rindex::schema::{HelpArg, HelpDoc};

#[cfg(test)]
mod canonical_tests;

/// Rendered sections of one Rd page, before applying its metadata title.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RdSections {
    pub title: Option<String>,
    pub description: Option<String>,
    pub usage: Option<String>,
    pub arguments: Vec<HelpArg>,
}

#[derive(Debug)]
pub(crate) struct RenderIssue {
    pub field: &'static str,
    pub path: RdAstPath,
}

#[derive(Default)]
pub(crate) struct Rendered {
    pub sections: RdSections,
    pub issues: Vec<RenderIssue>,
}

/// Render a legacy object, preserving this API's permissive shape handling.
pub fn render_page(page: &Robj) -> RdSections {
    page.as_list()
        .map(|nodes| render_nodes(Nodes::Legacy(nodes.iter())).sections)
        .unwrap_or_default()
}

pub(crate) fn render_document(document: &RdDocument) -> Rendered {
    render_nodes(Nodes::canonical(document.top_level()))
}

/// Merge an authoritative metadata title with rendered page sections.
pub fn into_help_doc(title: Option<String>, sections: RdSections) -> HelpDoc {
    HelpDoc {
        title: title.or(sections.title),
        description: sections.description,
        usage: sections.usage,
        arguments: sections.arguments,
    }
}

// The borrowed adapter keeps presentation policy shared without translating a
// legacy object into a second owned syntax tree or interpreting Raw payloads.
enum Node<'a> {
    Legacy(&'a Robj),
    Canonical(RdNodeRef<'a>),
    SystemMacro(RdSystemMacroMatch<'a>),
}

enum Nodes<'a> {
    Legacy(std::slice::Iter<'a, Robj>),
    Canonical {
        nodes: RdNodesIter<'a>,
        macros: RdSystemMacroItems<'a>,
    },
}

impl<'a> Nodes<'a> {
    fn canonical(nodes: RdNodesRef<'a>) -> Self {
        Self::Canonical {
            nodes: nodes.iter(),
            macros: nodes.system_macro_items(),
        }
    }
}

impl<'a> Iterator for Nodes<'a> {
    type Item = Node<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Legacy(nodes) => nodes.next().map(Node::Legacy),
            Self::Canonical { nodes, macros } => {
                let node = nodes.next()?;
                if let Some(RdSystemMacroItem::Macro(macro_match)) = macros.next() {
                    // A validated USERMACRO marker and its expansion represent
                    // one value; neither should reach the opaque fallback.
                    for _ in 1..macro_match.consumed() {
                        nodes.next();
                    }
                    Some(Node::SystemMacro(macro_match))
                } else {
                    Some(Node::Canonical(node))
                }
            }
        }
    }
}

impl<'a> Node<'a> {
    fn tag(&self) -> Option<&str> {
        match self {
            Self::Legacy(node) => node.attr("Rd_tag").and_then(Robj::as_str),
            Self::Canonical(node) => match node.node() {
                RdNode::Tagged(tagged) => Some(tagged.tag().as_rd_tag()),
                // Reading an unrecognized Raw tag identifies which field to
                // withhold without interpreting its contents.
                RdNode::Raw(raw) => raw.tag(),
                _ => None,
            },
            Self::SystemMacro(_) => None,
        }
    }

    fn children(&self) -> Option<Nodes<'a>> {
        match self {
            Self::Legacy(node) => node.as_list().map(|nodes| Nodes::Legacy(nodes.iter())),
            Self::Canonical(node) => match node.node() {
                RdNode::Tagged(_) | RdNode::Group(_) => Some(Nodes::canonical(node.children())),
                _ => None,
            },
            Self::SystemMacro(macro_match) => match macro_match.semantic() {
                RdSystemMacro::I { .. } => Some(Nodes::canonical(
                    macro_match.source_nodes().get(0)?.children(),
                )),
                _ => None,
            },
        }
    }

    fn append_text(&self, out: &mut String) -> bool {
        match self {
            Self::Legacy(Robj {
                kind: Rkind::Str(values),
                ..
            }) => {
                for value in values.iter().flatten() {
                    out.push_str(value);
                }
                true
            }
            Self::Canonical(node) => match node.node() {
                // Preserve the legacy renderer's treatment of comment leaves;
                // changing presentation is separate from reader migration.
                RdNode::Text(s) | RdNode::RCode(s) | RdNode::Verb(s) | RdNode::Comment(s) => {
                    out.push_str(s);
                    true
                }
                _ => false,
            },
            Self::SystemMacro(macro_match) => {
                let text = match macro_match.semantic() {
                    RdSystemMacro::Doi { id } => id,
                    RdSystemMacro::CranPkg { package } => package,
                    RdSystemMacro::Sspace => " ",
                    _ => return false,
                };
                out.push_str(text);
                true
            }
            _ => false,
        }
    }

    fn opaque_path(&self) -> Option<RdAstPath> {
        match self {
            Self::Legacy(_) => return None,
            Self::Canonical(node) => match node.node() {
                RdNode::Text(_) | RdNode::RCode(_) | RdNode::Verb(_) | RdNode::Comment(_) => {
                    return None;
                }
                RdNode::Tagged(_) | RdNode::Group(_) => {}
                _ => return Some(node.path().clone()),
            },
            Self::SystemMacro(macro_match) => match macro_match.semantic() {
                RdSystemMacro::Doi { .. }
                | RdSystemMacro::CranPkg { .. }
                | RdSystemMacro::Sspace => return None,
                RdSystemMacro::I { .. } => {}
                _ => return Some(macro_match.anchor_path().clone()),
            },
        }
        self.children()?.find_map(|child| child.opaque_path())
    }
}

fn render_nodes(nodes: Nodes<'_>) -> Rendered {
    let mut rendered = Rendered::default();
    let out = &mut rendered.sections;
    let mut arguments_unavailable = false;
    for node in nodes {
        let field = match node.tag() {
            Some("\\title") => "title",
            Some("\\description") => "description",
            Some("\\usage") => "usage",
            Some("\\arguments") => "arguments",
            _ => continue,
        };
        if let Some(path) = node.opaque_path() {
            rendered.issues.push(RenderIssue { field, path });
            match field {
                "title" => out.title = None,
                "description" => out.description = None,
                "usage" => out.usage = None,
                _ => {
                    out.arguments.clear();
                    arguments_unavailable = true;
                }
            }
            continue;
        }
        match field {
            "title" => out.title = nonempty(collapse_ws(&render_children(&node))),
            "description" => {
                out.description = nonempty(normalize_paragraphs(&render_children(&node)))
            }
            "usage" => {
                let mut text = String::new();
                collect_verbatim(&node, &mut text);
                out.usage = nonempty(text.trim_matches('\n').trim_end().to_owned());
            }
            _ if !arguments_unavailable => render_arguments(&node, &mut out.arguments),
            _ => {}
        }
    }
    rendered
}

fn render_children(node: &Node<'_>) -> String {
    let mut out = String::new();
    if let Some(children) = node.children() {
        for child in children {
            render_inline(&child, &mut out);
        }
    } else {
        render_inline(node, &mut out);
    }
    out
}

fn render_inline(node: &Node<'_>, out: &mut String) {
    if node.append_text(out) {
        return;
    }
    if node.children().is_none() {
        return;
    }
    let delimiter = match node.tag() {
        Some("\\code" | "\\verb" | "\\samp" | "\\kbd" | "\\env" | "\\option" | "\\command") => {
            Some("`")
        }
        Some("\\emph") => Some("*"),
        Some("\\strong" | "\\bold") => Some("**"),
        _ => None,
    };
    let text = render_children(node);
    if let Some(delimiter) = delimiter {
        let text = collapse_ws(&text);
        if !text.is_empty() {
            out.push_str(delimiter);
            out.push_str(&text);
            out.push_str(delimiter);
        }
    } else {
        out.push_str(&text);
    }
}

fn collect_verbatim(node: &Node<'_>, out: &mut String) {
    if node.append_text(out) {
        return;
    }
    if let Some(children) = node.children() {
        for child in children {
            collect_verbatim(&child, out);
        }
    }
}

fn render_arguments(node: &Node<'_>, out: &mut Vec<HelpArg>) {
    let Some(children) = node.children() else {
        return;
    };
    for child in children {
        if child.tag() != Some("\\item") {
            continue;
        }
        let Some(mut parts) = child.children() else {
            continue;
        };
        let name = collapse_ws(
            &parts
                .next()
                .map(|part| render_children(&part))
                .unwrap_or_default(),
        );
        if name.is_empty() {
            continue;
        }
        let mut description = String::new();
        for part in parts {
            render_inline(&part, &mut description);
        }
        out.push(HelpArg {
            name,
            description: normalize_paragraphs(&description),
        });
    }
}

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
