use super::*;

/// The body parts of a section, grouped into roxygen2 *paragraphs* (its blank-
/// line-delimited prose blocks), excluding its `@tag` heading. roxygen2 splits
/// the section text on `\n\n`, so a block macro or markdown list that directly
/// follows a prose line — with no blank `#'` line between — belongs to the same
/// paragraph as that prose; a blank `#'` line (a *section-level* `ROXYGEN_MARKER`,
/// as opposed to the per-line markers nested inside each node) starts a new
/// paragraph. Each returned `Vec<Inline>` is one such paragraph: a prose
/// `ROXYGEN_PARAGRAPH` contributes its inline run, a block `ROXYGEN_RD_MACRO`
/// (a multi-line `\itemize`/`\describe`/…) an `Inline::Macro`, and a
/// `ROXYGEN_MD_LIST` an `Inline::MdList`, with adjacent nodes joined by a space.
pub(super) fn section_body_parts(section: &RoxygenSection) -> Vec<Vec<Inline>> {
    let mut groups: Vec<Vec<Inline>> = Vec::new();
    let mut cur: Vec<Inline> = Vec::new();
    for el in section.syntax().children_with_tokens() {
        match el.kind() {
            // An ATX heading is a structural break: it ends the current part and
            // stands alone as its own part (a single `Inline::MdHeading` marker), so
            // the description/details outline builder can split cleanly on it.
            SyntaxKind::ROXYGEN_MD_HEADING => {
                let Some(node) = el.into_node() else { continue };
                if !cur.is_empty() {
                    groups.push(std::mem::take(&mut cur));
                }
                groups.push(vec![Inline::MdHeading(node)]);
            }
            SyntaxKind::ROXYGEN_PARAGRAPH
            | SyntaxKind::ROXYGEN_RD_MACRO
            | SyntaxKind::ROXYGEN_MD_LIST
            | SyntaxKind::ROXYGEN_MD_CODE_BLOCK
            | SyntaxKind::ROXYGEN_MD_INDENTED_CODE
            | SyntaxKind::ROXYGEN_MD_HTML_BLOCK
            | SyntaxKind::ROXYGEN_MD_BLOCK_QUOTE
            | SyntaxKind::ROXYGEN_MD_TABLE => {
                let Some(node) = el.into_node() else { continue };
                let kind = node.kind();
                let inlines = match kind {
                    SyntaxKind::ROXYGEN_PARAGRAPH => RoxygenParagraph::cast(node)
                        .map(|p| paragraph_inlines(&p))
                        .unwrap_or_default(),
                    SyntaxKind::ROXYGEN_MD_LIST => vec![Inline::MdList(node)],
                    SyntaxKind::ROXYGEN_MD_CODE_BLOCK => vec![Inline::MdCodeBlock(node)],
                    SyntaxKind::ROXYGEN_MD_INDENTED_CODE => vec![Inline::MdIndentedCode(node)],
                    SyntaxKind::ROXYGEN_MD_HTML_BLOCK => vec![Inline::MdHtmlBlock(node)],
                    SyntaxKind::ROXYGEN_MD_BLOCK_QUOTE => vec![Inline::MdBlockQuote(node)],
                    SyntaxKind::ROXYGEN_MD_TABLE => vec![Inline::MdTable(node)],
                    _ => vec![Inline::Macro(node)],
                };
                // A block quote carries no separator: roxygen2 flattens it with no
                // surrounding paragraph break, so its text glues onto the preceding
                // node (`before` + `> q` → `beforeq`). Every other node joins with a
                // space (a roxygen paragraph break, collapsed by `norm_ws`).
                if !cur.is_empty() && kind != SyntaxKind::ROXYGEN_MD_BLOCK_QUOTE {
                    cur.push(Inline::Text(" ".to_string()));
                }
                cur.extend(inlines);
            }
            // A thematic break (`***`/`---`/`___`) renders empty in roxygen2 (it has
            // no thematic-break support and emits `escape_comment(xml_text)` = ""), so
            // it contributes nothing. It still separates roxygen paragraphs, so it
            // ends the current part the way a blank line does.
            SyntaxKind::ROXYGEN_MD_THEMATIC_BREAK if !cur.is_empty() => {
                groups.push(std::mem::take(&mut cur));
            }
            // A section-level `#'` marker is a blank doc-comment line: it ends the
            // current paragraph (per-line markers live *inside* the nodes above).
            SyntaxKind::ROXYGEN_MARKER if !cur.is_empty() => {
                groups.push(std::mem::take(&mut cur));
            }
            _ => {}
        }
    }
    if !cur.is_empty() {
        groups.push(cur);
    }
    groups
}

/// The inline elements of a prose paragraph: its text and inline Rd-macro
/// content, with the threaded `#'` markers dropped and inter-line newlines turned
/// into a joining space (continuation lines fold into one run). An Rd macro
/// becomes an `Inline::Macro`; all other content (plain text and — in the absence
/// of resolved markdown — inline code and link spans, which are literal Rd prose)
/// becomes `Inline::Text`. Whitespace is collapsed downstream by `norm_ws`.
pub(super) fn paragraph_inlines(para: &RoxygenParagraph) -> Vec<Inline> {
    let mut out = Vec::new();
    for el in para.syntax().children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_MARKER => {} // trivia: never prose
            SyntaxKind::NEWLINE => out.push(Inline::Text(SOFT_BREAK.to_string())), // line join
            _ => push_inline(&mut out, el),
        }
    }
    out
}

/// The inline elements of a tag line: everything after the `@`, the tag name, and
/// an arg-bearing tag's argument (and the leading whitespace before the prose).
///
/// A tag with a same-line prose value folds its contiguous plain-prose
/// continuation lines into the tag node (see `emit_tag_line`), so the tag may
/// carry the threaded `#'` markers and inter-line newlines of those continuations
/// — dropped and turned into a joining soft break exactly as `paragraph_inlines`
/// does, so the folded value reads as one run (`@details *a` \n `b*` →
/// `\emph{a b}`).
pub(super) fn tag_inlines(tag: &RoxygenTag) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut seen_prose = false;
    for el in tag.syntax().children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_AT | SyntaxKind::ROXYGEN_TAG_NAME | SyntaxKind::ROXYGEN_TAG_ARG => {
                continue;
            }
            // A threaded continuation marker is trivia (never prose); an inter-line
            // newline joins the continuation into one run (norm_ws collapses the
            // soft break, which still bounds a non-markdown `%` comment).
            SyntaxKind::ROXYGEN_MARKER => {}
            SyntaxKind::NEWLINE => {
                if seen_prose {
                    out.push(Inline::Text(SOFT_BREAK.to_string()));
                }
            }
            SyntaxKind::WHITESPACE => {
                if seen_prose {
                    push_inline(&mut out, el);
                }
            }
            _ => {
                seen_prose = true;
                push_inline(&mut out, el);
            }
        }
    }
    out
}

/// Append `el` to an inline run: a macro node as `Inline::Macro`, anything else
/// as `Inline::Text` of its source text.
pub(super) fn push_inline(
    out: &mut Vec<Inline>,
    el: NodeOrToken<SyntaxNode, crate::syntax::SyntaxToken>,
) {
    match el {
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_RD_MACRO => {
            out.push(Inline::Macro(n));
        }
        // A nested `ROXYGEN_MD_LIST` (a sublist inside a list item) projects as
        // its own `\itemize`/`\enumerate`, the way a top-level list does.
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_MD_LIST => {
            out.push(Inline::MdList(n));
        }
        // A `ROXYGEN_MD_CODE_BLOCK` folded into a list item (a fenced code block
        // at the item's content column) projects to roxygen2's three-atom
        // `\if{html}…\preformatted…\if{html}` sequence, the same as a
        // section-level fenced block.
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_MD_CODE_BLOCK => {
            out.push(Inline::MdCodeBlock(n));
        }
        // A `ROXYGEN_MD_INDENTED_CODE` folded into a list item (a block indented
        // four columns past the item's content column) projects to the same
        // three-atom sequence; [`serialize_md_indented_code`] strips the item's
        // content column on top of CommonMark's four (the item container consumes
        // the content indentation first).
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_MD_INDENTED_CODE => {
            out.push(Inline::MdIndentedCode(n));
        }
        // A `ROXYGEN_MD_BLOCK_QUOTE` folded into a list item (a quote at the
        // item's content column) flattens to plain text glued onto the item's
        // prose, the same as a section-level quote (roxygen2 has no block-quote
        // support and renders the concatenated descendant text).
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_MD_BLOCK_QUOTE => {
            out.push(Inline::MdBlockQuote(n));
        }
        // A `ROXYGEN_MD_TABLE` folded into a list item (a GFM table at the item's
        // content column) projects to roxygen2's `\tabular`, the same as a
        // section-level table; [`serialize_md_table`] strips the item's content
        // indentation per line via `strip_marker`.
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_MD_TABLE => {
            out.push(Inline::MdTable(n));
        }
        // A `ROXYGEN_MD_HTML_BLOCK` folded into a list item (an HTML block
        // opening at the item's content start, `- <div>`, cm-177) projects to
        // roxygen2's `\if{html}{\out{…}}`, the same as a section-level block.
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_MD_HTML_BLOCK => {
            out.push(Inline::MdHtmlBlock(n));
        }
        // A `ROXYGEN_MD_THEMATIC_BREAK` folded into a list item (a same-line
        // `- * * *`, cm-061): roxygen2 has no thematic-break support and renders
        // it empty, so it contributes nothing — the item stays bare, the same
        // way `section_body_parts` drops a section-level break.
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_MD_THEMATIC_BREAK => {}
        // A `ROXYGEN_MD_HEADING` folded into a list item (a same-line `- # Foo`,
        // a content-column ATX line, or a promoted setext paragraph, cm-302): a
        // structural marker like its section-level counterpart. A level >= 2
        // heading renders as an in-item `\subsection` over the item's following
        // content ([`serialize_md_list`]); a level-1 heading hoists to a
        // top-level `\section`, poisoning the sliced `\itemize` pieces
        // ([`emit_section_with_headings`]'s in-list split).
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_MD_HEADING => {
            out.push(Inline::MdHeading(n));
        }
        // A resolved emphasis/strong *node* (the inline pass's output): recurse
        // into its inner inline run, skipping the opener/closer delimiter leaves
        // (and any inter-line trivia), so nesting projects as structure.
        NodeOrToken::Node(n)
            if matches!(
                n.kind(),
                SyntaxKind::ROXYGEN_MD_EMPH | SyntaxKind::ROXYGEN_MD_STRONG
            ) =>
        {
            let strong = n.kind() == SyntaxKind::ROXYGEN_MD_STRONG;
            // The span's *own* delimiters are the first child (opener) and the last
            // child (closer); they are skipped. Any *interior* `ROXYGEN_MD_DELIM` is
            // an unmatched delimiter and stays literal text (`_foo_bar_baz_` →
            // `\emph` over `foo_bar_baz`), reached via push_inline's text fallback.
            let kids: Vec<_> = n.children_with_tokens().collect();
            let interior = kids.len().saturating_sub(1);
            let mut children = Vec::new();
            for child in kids.into_iter().take(interior).skip(1) {
                match child.kind() {
                    SyntaxKind::ROXYGEN_MARKER => {} // threaded trivia: never prose
                    SyntaxKind::NEWLINE => children.push(Inline::Text(SOFT_BREAK.to_string())),
                    _ => push_inline(&mut children, child),
                }
            }
            out.push(Inline::MdEmphasis { strong, children });
        }
        // A resolved inline-link *node* (`ROXYGEN_MD_LINK`): the inline pass's
        // output for a bracket-paired link. The first child is the `[` opener leaf
        // and the last child the closer leaf; the display in between recurses (so
        // emphasis/code spans inside the link surface as structure). The closer text
        // distinguishes the three paired forms: `](url)` is an inline link carrying
        // its destination (`\href`/`\url`); `][ref]` is a *reference* link whose
        // `[ref]` topic option is dropped, projecting to `\link{display}`; a bare `]`
        // is a *shortcut* link whose display is the destination. A *collapsed*
        // reference (`][]`) strips to an **empty** `dest`, which downstream reads as
        // "label = display" ([`link_ref_label`]) and renders shortcut-style.
        // An **autolink** `ROXYGEN_MD_LINK` node: the inline pass's whole-run
        // rescan carved a `<scheme:…>`/`<addr>` autolink across already-carved
        // tokens (its span consumed a `](url)` closer's `]` left-to-right,
        // cm-528). No bracket structure — the node text is the autolink source
        // (always single-line: an autolink body admits no whitespace or control
        // chars), so it projects exactly like the opaque autolink leaf.
        NodeOrToken::Node(n)
            if n.kind() == SyntaxKind::ROXYGEN_MD_LINK
                && n.text().char_at(0.into()) == Some('<') =>
        {
            out.push(Inline::MdLink(n.text().to_string()));
        }
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_MD_LINK => {
            let kids: Vec<_> = n.children_with_tokens().collect();
            // A *cross-line* inline destination (cm-512): the closer is the
            // unique `](` delimiter leaf and the consumed destination tokens
            // follow it inside the node. Rebuild the composite closer from
            // their cmark-visible text — content verbatim, a soft break as
            // `\n`, the `#'` markers and continuation indentation stripped
            // (cmark strips a paragraph continuation line's leading
            // whitespace, so none of it reaches the destination gaps).
            let cross_dest = kids
                .iter()
                .position(|c| c.kind() == SyntaxKind::ROXYGEN_MD_DELIM && c.to_string() == "](");
            let (interior, closer) = match cross_dest {
                Some(idx) => {
                    let mut close = String::from("](");
                    for c in &kids[idx + 1..] {
                        match c.kind() {
                            SyntaxKind::ROXYGEN_MARKER | SyntaxKind::WHITESPACE => {}
                            SyntaxKind::NEWLINE => close.push('\n'),
                            _ => close.push_str(&c.to_string()),
                        }
                    }
                    (idx, close)
                }
                None => (
                    kids.len().saturating_sub(1),
                    kids.last().map(|c| c.to_string()).unwrap_or_default(),
                ),
            };
            let mut display = Vec::new();
            let mut after_marker = false;
            for child in kids.into_iter().take(interior).skip(1) {
                match child.kind() {
                    SyntaxKind::ROXYGEN_MARKER => after_marker = true, // threaded trivia
                    SyntaxKind::NEWLINE => {
                        after_marker = false;
                        display.push(Inline::Text(SOFT_BREAK.to_string()));
                    }
                    // A continuation line's marker→content whitespace: the first
                    // space is the marker separator (roxygen2 strips `#' ?`), the
                    // rest is label content — observable in the URL-encoded
                    // synthesized destination (`[\n ]` → `R:%0A%20`, cm-554).
                    SyntaxKind::WHITESPACE if after_marker => {
                        after_marker = false;
                        let text = child.to_string();
                        let content = text.strip_prefix(' ').unwrap_or(&text);
                        if !content.is_empty() {
                            display.push(Inline::Text(content.to_string()));
                        }
                    }
                    _ => {
                        after_marker = false;
                        push_inline(&mut display, child);
                    }
                }
            }
            if closer == "]" {
                // A bare `]` closer: a cross-line *shortcut* link `[text]`.
                out.push(Inline::MdShortcutLink { display });
            } else if let Some(dest) = closer.strip_prefix("][").and_then(|s| s.strip_suffix(']')) {
                out.push(Inline::MdRefLink {
                    dest: dest.to_string(),
                    display,
                });
            } else {
                out.push(Inline::MdInlineLink {
                    url: inline_link_dest(&closer),
                    display,
                });
            }
        }
        // A **multi-line** raw inline-HTML span — a `ROXYGEN_MD_HTML` *node* the
        // inline pass resolved across soft breaks. Its rendered text is the
        // span's cmark-visible content ([`md_html_node_text`]): the `#'` markers
        // and continuation indentation strip, the soft breaks stay as real
        // newlines (the span text is verbatim `\out` content, one VERB per line).
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_MD_HTML => {
            out.push(Inline::MdHtml(md_html_node_text(&n)));
        }
        // A **multi-line** code span — a `ROXYGEN_MD_CODE` *node* the inline
        // pass resolved across soft breaks. Same cmark-visible walk, then the
        // ordinary code-span strip: [`strip_code_span`] converts the interior
        // line endings to spaces (CommonMark: a code span's line endings are
        // treated like spaces) before its single-space trim.
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_MD_CODE => {
            out.push(Inline::MdCode(strip_code_span(&md_html_node_text(&n))));
        }
        NodeOrToken::Node(n) => out.push(Inline::Text(n.text().to_string())),
        // Markdown inline leaves (emitted only under `@md`): carve off their
        // delimiters and carry the inner content; the kind chooses the Rd macro.
        NodeOrToken::Token(t) if t.kind() == SyntaxKind::ROXYGEN_MD_CODE => {
            out.push(Inline::MdCode(strip_code_span(t.text())));
        }
        // A markdown link leaf: the inline `[text](url)` form projects to `\href`;
        // the reference (`[text][ref]`) and shortcut (`[dest]`) forms resolve to a
        // `\link` (optionally `\code`-wrapped) per roxygen2's `parse_link` (see
        // [`resolve_md_link`]). A leaf that resolves to nothing (an unrecognized
        // shape) falls through to literal prose.
        NodeOrToken::Token(t)
            if t.kind() == SyntaxKind::ROXYGEN_MD_LINK && resolve_md_link(t.text()).is_some() =>
        {
            out.push(Inline::MdLink(t.text().to_string()));
        }
        // A markdown image leaf `![alt](url "title")` → `\figure` (see
        // [`resolve_md_image`]). A leaf that resolves to nothing falls through to
        // literal prose — except the *collapsed* form `![alt][]`, which resolves
        // to nothing on its own yet must still materialize so a user
        // `[alt]: url` definition can rewrite it ([`apply_user_linkrefs`]);
        // undefined, it serializes back into the prose run.
        NodeOrToken::Token(t)
            if t.kind() == SyntaxKind::ROXYGEN_MD_IMAGE
                && (resolve_md_image(t.text()).is_some() || image_is_collapsed(t.text())) =>
        {
            out.push(Inline::MdImage(t.text().to_string()));
        }
        // A raw inline-HTML leaf `<tag>` → `\if{html}{\out{<tag>}}` (see
        // [`html_inline_atom`]).
        NodeOrToken::Token(t) if t.kind() == SyntaxKind::ROXYGEN_MD_HTML => {
            out.push(Inline::MdHtml(t.text().to_string()));
        }
        // A `ROXYGEN_MD_LIST_MARKER` that reached an inline run (rather than a
        // `ROXYGEN_MD_LIST`) is a marker that did not form a list — the CommonMark
        // interrupt rule kept it inline. roxygen2 renders it as literal text.
        NodeOrToken::Token(t) => out.push(Inline::Text(t.text().to_string())),
    }
}

/// The cmark-visible text of a multi-line raw inline-HTML span
/// (`ROXYGEN_MD_HTML` node): content children verbatim (a covered Rd macro or
/// carved leaf contributes its raw source — roxygen2 restores a placeholder to
/// its original text in the rendered output), a soft break as a real `\n`, and
/// the threaded `#'` markers and marker→content indentation dropped (roxygen2
/// strips `#' `, and cmark strips a paragraph continuation line's remaining
/// leading whitespace — engine-probed: `#'    y -->` renders `y -->`).
fn md_html_node_text(node: &SyntaxNode) -> String {
    let mut s = String::new();
    for el in node.children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_MARKER | SyntaxKind::WHITESPACE => {}
            SyntaxKind::NEWLINE => s.push('\n'),
            _ => match el {
                NodeOrToken::Token(t) => s.push_str(t.text()),
                NodeOrToken::Node(n) => s.push_str(&n.text().to_string()),
            },
        }
    }
    s
}
