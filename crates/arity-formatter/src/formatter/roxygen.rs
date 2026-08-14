//! Roxygen block formatting. A `ROXYGEN_BLOCK` is emitted one `#'` line per output
//! line, reflowed to the line width, with protected markup spans (inline code, Rd
//! macros, markdown links) kept atomic.
//!
//! **Layout is chosen by a tag's [`TagClass`], never by its written form.** A tag's
//! body renders identically in roxygen2 whether written inline (`@details x`, form-1)
//! or on the next line (`@details`⏎`x`, form-2), so the formatter must produce the
//! same output for both (Tenet 1). It therefore *canonicalizes*: it gathers a
//! section's body from both places the parser can put it — inline in the
//! `ROXYGEN_TAG` node (form-1) and the sibling `ROXYGEN_PARAGRAPH`s (form-2) — and
//! re-emits it in its class's one canonical shape. The classes ([`classify`]):
//!
//! - **NameBearingProse** (`@param`/`@slot`/`@field`): the `@tag name` header stays
//!   inline and the prose hangs two spaces beneath it ([`TagUnit`]).
//! - **SectioningProse** (`@description`/`@details`/`@return`/…): free-form prose,
//!   **reflowed** — inline on the tag line when the single-paragraph body fits, else
//!   form-2 (a bare `#' @tag` line with the body flush beneath). A body containing a
//!   block/list/fence, or a second paragraph, forces form-2 ([`SectionUnit`]).
//! - **Code** (`@examples`/`@examplesIf`, and `@usage`/`@eval*`): the
//!   `@examples` body is formatted as embedded R ([`ExampleBody`]); other code tags
//!   are verbatim passthrough. Embedded R that fails to parse (e.g. wrapped in
//!   `\dontrun{}`) falls back to marker-normalized passthrough, byte-for-byte.
//! - **AtomicValue** (`@name`/`@family`/…): one line, value verbatim, overflow ok.
//! - **TokenList** (`@keywords`/`@importFrom`/…): joined onto one line
//!   ([`TokenListUnit`]). **Toggle** (`@export`/`@md`): bare `#' @tag`.
//! - **Section** (`@section Title:`): title inline, body form-2 flush.
//!
//! Blank separators, block Rd macros, markdown lists, fenced code blocks, and other
//! structured lines (table delimiter rows, headers, blockquotes) are passed through
//! marker-normalized but never reflowed — the conservative gate that keeps reflow
//! correct without a full Markdown parse. A block macro always sits flush; it never
//! hangs under a tag (the enclosing section is laid out form-2 instead).

use rowan::NodeOrToken;

use super::context::FormatContext;
use super::core::format_with_style;
use super::ir::Ir;
use super::style::FormatStyle;
use crate::ast::{AstNode, RoxygenBlock, RoxygenTag};
use crate::parser::roxygen::{is_md_standalone_html_tag, is_table_delim_row, starts_md_html_block};
use crate::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

/// One physical `#'` line, reconstructed from the logical CST for layout.
///
/// The parser models a roxygen block as *logical* content (sections, paragraphs,
/// block macros) with `#'` markers and newlines threaded in as trivia. The
/// formatter, however, emits one `#'` line per output line and reflows within the
/// line width, so it works from a physical-line view: a read-only re-segmentation
/// of the block's leaves at their marker/newline trivia. Each line carries its
/// marker, its tag (if a tag line), and the content elements after the marker
/// (inline Rd-macro nodes and the tag node kept atomic). Inter-line indentation
/// before a marker is dropped — the formatter computes its own indent.
#[derive(Clone, Default)]
struct PhysicalLine {
    marker: Option<SyntaxToken>,
    tag: Option<RoxygenTag>,
    elements: Vec<NodeOrToken<SyntaxNode, SyntaxToken>>,
    /// A multi-line block Rd macro (`\itemize{ … }`) occupying this "line": it owns
    /// its own `#'` markers and newlines internally, so it is emitted as atomic
    /// marker-preserving passthrough rather than reflowed.
    block_macro: Option<SyntaxNode>,
}

impl PhysicalLine {
    /// The `#'` marker token of this line, if present.
    fn marker(&self) -> Option<&SyntaxToken> {
        self.marker.as_ref()
    }

    /// The tag heading this line (`#' @tag ...`), if any.
    fn tag(&self) -> Option<RoxygenTag> {
        self.tag.clone()
    }

    /// A blank `#'` line (a paragraph separator): no tag and no prose content
    /// (text or a protected markup span).
    fn is_blank(&self) -> bool {
        self.tag.is_none()
            && !self
                .elements
                .iter()
                .any(|el| el.kind().is_roxygen_prose_content())
    }
}

/// Reconstruct the physical `#'` lines of a `ROXYGEN_BLOCK` for layout: walk the
/// block's leaves in source order (descending through `ROXYGEN_SECTION` /
/// `ROXYGEN_PARAGRAPH`, keeping `ROXYGEN_TAG` and `ROXYGEN_RD_MACRO` atomic) and
/// split at each marker (line start) and newline (line end). Indentation before a
/// marker is inter-line trivia and is dropped.
fn physical_lines(block: &SyntaxNode) -> Vec<PhysicalLine> {
    let mut elements = Vec::new();
    collect_logical_elements(block, &mut elements);

    let mut lines = Vec::new();
    let mut cur = PhysicalLine::default();
    for el in elements {
        match el.kind() {
            // A block macro owns its own marker/newline trivia; it is a self-
            // contained multi-line unit, so close any pending line and emit it on
            // its own. (Checked before the marker-less drop below — its opening
            // marker is *inside* the node, so `cur.marker` is still `None` here.)
            SyntaxKind::ROXYGEN_RD_MACRO if el.as_node().is_some_and(is_block_macro) => {
                if cur.marker.is_some() || !cur.elements.is_empty() {
                    lines.push(std::mem::take(&mut cur));
                }
                lines.push(PhysicalLine {
                    block_macro: el.as_node().cloned(),
                    ..PhysicalLine::default()
                });
            }
            // A markdown list or fenced code block (`@md` mode) likewise owns its
            // `#'` markers and newlines internally; it is atomic passthrough
            // (marker-normalized, never reflowed across items/lines).
            SyntaxKind::ROXYGEN_MD_LIST
            | SyntaxKind::ROXYGEN_MD_CODE_BLOCK
            | SyntaxKind::ROXYGEN_MD_INDENTED_CODE
            | SyntaxKind::ROXYGEN_MD_HTML_BLOCK
            | SyntaxKind::ROXYGEN_MD_BLOCK_QUOTE
            | SyntaxKind::ROXYGEN_MD_TABLE
            | SyntaxKind::ROXYGEN_MD_HEADING
            | SyntaxKind::ROXYGEN_MD_THEMATIC_BREAK => {
                if cur.marker.is_some() || !cur.elements.is_empty() {
                    lines.push(std::mem::take(&mut cur));
                }
                lines.push(PhysicalLine {
                    block_macro: el.as_node().cloned(),
                    ..PhysicalLine::default()
                });
            }
            SyntaxKind::ROXYGEN_MARKER => {
                if cur.marker.is_some() {
                    lines.push(std::mem::take(&mut cur));
                }
                cur.marker = el.into_token();
            }
            SyntaxKind::NEWLINE => {
                if cur.marker.is_some() || !cur.elements.is_empty() {
                    lines.push(std::mem::take(&mut cur));
                }
            }
            // Indentation before this line's marker (inter-line trivia) is not
            // part of any line. Any *other* marker-less element is a closing
            // line's remainder after a block macro (`} tail …` — the line's
            // marker lives inside the macro node): it forms a marker-less line
            // whose `#'` the renderer supplies.
            SyntaxKind::WHITESPACE if cur.marker.is_none() && cur.elements.is_empty() => {}
            SyntaxKind::ROXYGEN_TAG => {
                cur.tag = el.as_node().cloned().and_then(RoxygenTag::cast);
                cur.elements.push(el);
            }
            _ => cur.elements.push(el),
        }
    }
    if cur.marker.is_some() || !cur.elements.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Whether `node` is a *block* Rd macro: a `ROXYGEN_RD_MACRO` spanning multiple
/// `#'` lines, which it owns as threaded `ROXYGEN_MARKER` trivia. An inline macro
/// (`\code{f}`) never contains a marker, so the presence of one is the
/// distinguisher between atomic passthrough and ordinary inline reflow.
fn is_block_macro(node: &SyntaxNode) -> bool {
    node.kind() == SyntaxKind::ROXYGEN_RD_MACRO
        && node
            .children_with_tokens()
            .any(|el| el.kind() == SyntaxKind::ROXYGEN_MARKER)
}

/// Emit a block Rd macro as atomic, marker-preserving passthrough: its own source
/// lines, each on its own output line, flush at `#'`. The node's text already
/// carries the `#'` markers (the opening one and the continuations) and the
/// in-macro indentation; the inter-line indentation *before* a continuation marker
/// is dropped (the formatter recomputes the block's own indent), matching the
/// fenced-code and air-compatible verbatim treatment of Rd lists. A block never
/// hangs under a tag: a `SectioningProse` tag carrying a block is laid out form-2,
/// so the block sits flush beneath the bare `#' @tag` line.
///
/// The **opener** line is marker-normalized rather than passed through, so the
/// indentation the author wrote between that line's `#'` and the `\name{` is
/// dropped. It is not content — the macro has not opened yet, so nothing renders
/// it — and keeping it would let the input's layout decide the output (Tenet 1),
/// splitting sibling `\item`s into two indentations purely by whether each
/// happened to wrap: a one-line `\item` is an *inline* macro reflowed flush as
/// prose, while a wrapped one lands here. Interior lines keep their indentation,
/// which is the block's own visual structure.
fn emit_block_macro(items: &mut Vec<Ir>, node: &SyntaxNode) {
    let text = node.text().to_string();
    // A *mid-prose* opener (`text \preformatted{ …`) has no marker of its own — the
    // opener line's `#'` belongs to the preceding paragraph — so its first segment
    // begins with the `\name{` itself. Give the opener its own marker-prefixed line
    // (the continuation lines already carry their markers); this is lossless and
    // idempotent (on reparse the opener is a line-start block macro that re-emits
    // identically).
    let mid_prose = node.first_token().map(|t| t.kind()) != Some(SyntaxKind::ROXYGEN_MARKER);

    for (i, seg) in text.split('\n').enumerate() {
        let line = if i == 0 {
            if mid_prose {
                format!("#' {}", seg.trim_end())
            } else {
                normalize_marker_text(seg)
            }
        } else {
            seg.trim_start().trim_end().to_string()
        };
        push_line(items, line);
    }
}

/// Emit a block Rd macro that wraps example R inside `@examples` (`\dontrun{}`,
/// `\donttest{}`, …): each line marker-normalized (marker, one space, content),
/// dropping the in-macro indentation so the example code is flush and
/// copy-pasteable rather than carrying prose-list indentation. (Formatting the R
/// *inside* the wrapper is future work; the node delimits exactly where it is.)
fn emit_block_macro_examples(items: &mut Vec<Ir>, node: &SyntaxNode) {
    for seg in node.text().to_string().split('\n') {
        push_line(items, normalize_marker_text(seg));
    }
}

/// Whether a block node's first line is **marker-less** — the block opened as a
/// tag's same-line value, so the opener `#'` belongs to the enclosing (empty)
/// tag, not the node.
fn is_from_value(node: &SyntaxNode) -> bool {
    node.first_token().map(|t| t.kind()) != Some(SyntaxKind::ROXYGEN_MARKER)
}

/// Marker-prefix a block node's **marker-less first line** (a block opening as a
/// tag's same-line value). Drops the single tag-head separator space roxygen2
/// strips; `keep_indent` keeps any further indent (it is semantic — CommonMark
/// nesting/code columns, or verbatim `\out` rendering), otherwise the content is
/// trimmed like [`normalize_marker_text`]. On reparse the line sits on its own
/// `#'` line beneath the bare tag (the next-line form), which projects
/// identically — idempotent.
fn push_value_opener_line(items: &mut Vec<Ir>, seg: &str, keep_indent: bool) {
    let content = if keep_indent {
        seg.strip_prefix([' ', '\t']).unwrap_or(seg)
    } else {
        seg.trim()
    };
    push_line(
        items,
        if content.is_empty() {
            "#'".to_string()
        } else {
            format!("#' {content}")
        },
    );
}

/// Emit a markdown list (`@md` mode) as atomic passthrough, each `#'` line
/// marker-normalized but **preserving the content's leading indentation**: in a
/// markdown list that indentation is semantic (it sets the CommonMark nesting
/// depth that the parser models as nested `ROXYGEN_MD_LIST`s), so flattening it
/// would change the rendered Rd. Only the `#'` sigil and the single conventional
/// space after it are normalized; everything the marker→content whitespace
/// carries beyond that is kept. A list whose first item began as a tag's
/// same-line value has a marker-less first line; its indent beyond the separator
/// space is kept too (the same one-based column semantics).
fn emit_md_list(items: &mut Vec<Ir>, node: &SyntaxNode) {
    let from_value = is_from_value(node);
    for (idx, seg) in node.text().to_string().split('\n').enumerate() {
        if idx == 0 && from_value {
            push_value_opener_line(items, seg, true);
        } else {
            push_line(items, normalize_list_marker_text(seg));
        }
    }
}

/// Emit a markdown fenced code block (`@md` mode) as atomic passthrough, each
/// `#'` line marker-normalized but **keeping the content's leading
/// indentation** ([`normalize_list_marker_text`]): the fence lines and verbatim
/// code lines are emitted as-is, never reflowed. That indentation is semantic
/// twice over — the opener fence's own indent sets how many columns CommonMark
/// strips from each code line, and a code line's indent both renders verbatim
/// into `\preformatted` and can be exactly what keeps a fence-lookalike line
/// *content* (a closing fence may be indented at most three columns; trimming a
/// four-space `    ```` ` line would turn it into the closer and change the
/// rendered Rd). A block whose opener fence began as a tag's same-line value
/// has a marker-less first line; it is given its own `#'` (the next-line form,
/// projecting identically).
fn emit_md_code_block(items: &mut Vec<Ir>, node: &SyntaxNode) {
    let from_value = is_from_value(node);
    for (idx, seg) in node.text().to_string().split('\n').enumerate() {
        if idx == 0 && from_value {
            push_value_opener_line(items, seg, true);
        } else {
            push_line(items, normalize_list_marker_text(seg));
        }
    }
}

/// Emit a markdown indented code block (`@md` mode) as atomic passthrough, each
/// `#'` line marker-normalized but **keeping the content's leading indentation** —
/// that >= 4-column indentation past the stripped `#'` is exactly what makes the
/// line code (trimming it, as [`normalize_marker_text`] would, turns the block back
/// into prose and changes the rendered Rd). Reuses [`normalize_list_marker_text`],
/// which drops only the single conventional space after the marker; on reparse the
/// same indentation re-forms the code block, so this is idempotent. A block that
/// began as a tag's same-line value (`@details      x`) has a marker-less first
/// line; its indent beyond the separator space is kept (it is the code column).
fn emit_md_indented_code(items: &mut Vec<Ir>, node: &SyntaxNode) {
    let from_value = is_from_value(node);
    for (idx, seg) in node.text().to_string().split('\n').enumerate() {
        if idx == 0 && from_value {
            push_value_opener_line(items, seg, true);
        } else {
            push_line(items, normalize_list_marker_text(seg));
        }
    }
}

/// Emit a markdown HTML block (`@md` mode) as atomic passthrough, each `#'` line
/// marker-normalized. The node owns its own `#'` markers and newlines; the raw
/// HTML is never reflowed across lines. Content indentation is **preserved**
/// ([`normalize_list_marker_text`], not [`normalize_marker_text`]): roxygen2
/// renders the block's lines verbatim into `\out{…}`, so a trimmed leading indent
/// (a `<pre>` line, or condition-1 verbatim content) would change the rendered Rd
/// — a fixed-point violation.
///
/// A block opening as a tag's same-line value (`#' @details <span>`) has a
/// **marker-less** first line — the opener `#'` belonged to the tag — so, like
/// [`emit_md_heading`]'s from-value setext form, that line is given its own
/// `#' `, dropping the single tag-head separator space but keeping any further
/// indent (roxygen2 renders it verbatim). On reparse the opener sits on its own
/// `#'` line beneath the bare tag (the next-line form), which projects
/// identically — idempotent.
fn emit_md_html_block(items: &mut Vec<Ir>, node: &SyntaxNode) {
    let from_value = is_from_value(node);
    for (idx, seg) in node.text().to_string().split('\n').enumerate() {
        if idx == 0 && from_value {
            push_value_opener_line(items, seg, true);
        } else {
            push_line(items, normalize_list_marker_text(seg));
        }
    }
}

/// Emit a GFM table (`@md` mode) as atomic passthrough, each `#'` line
/// marker-normalized (marker, one space, trimmed content). The node owns its own
/// `#'` markers and newlines; the table is never reflowed. Marker-normalizing the
/// header/delimiter/body rows is idempotent — on reparse the same header/delimiter
/// cell counts still form the table. A table whose header row began as a tag's
/// same-line value has a marker-less first line; it is given its own `#'` (the
/// next-line form, projecting identically).
fn emit_md_table_block(items: &mut Vec<Ir>, node: &SyntaxNode) {
    let from_value = is_from_value(node);
    for (idx, seg) in node.text().to_string().split('\n').enumerate() {
        if idx == 0 && from_value {
            push_value_opener_line(items, seg, false);
        } else {
            push_line(items, normalize_marker_text(seg));
        }
    }
}

/// Emit a markdown block quote (`@md` mode) as atomic passthrough, each `#'` line
/// marker-normalized (marker, one space, trimmed content). The node owns its own
/// `#'` markers and newlines; the quote is never reflowed. Each line keeps its
/// leading `>` (trimmed content, not touched), so on reparse the same consecutive
/// `>` lines re-form the block quote — idempotent. A quote opening as a tag's
/// same-line value has a marker-less first line; it is given its own `#'` (the
/// next-line form, projecting identically; up to three spaces before the `>` are
/// non-semantic, so the content is trimmed).
fn emit_md_block_quote(items: &mut Vec<Ir>, node: &SyntaxNode) {
    let from_value = is_from_value(node);
    for (idx, seg) in node.text().to_string().split('\n').enumerate() {
        if idx == 0 && from_value {
            push_value_opener_line(items, seg, false);
        } else {
            push_line(items, normalize_marker_text(seg));
        }
    }
}

/// Emit a markdown heading (`@md` mode) as atomic passthrough, marker-normalized.
/// An ATX heading is a single line; a setext heading is its title line(s) plus the
/// `===`/`---` underline. Marker-normalizing preserves the leading `#{1,6}` run or
/// the underline (each is trimmed content, not touched), so on reparse it still
/// forms the same heading — idempotent.
///
/// A setext heading whose title began as a tag's same-line value (`@details Title` /
/// `===`) has a **marker-less** first line — the opener `#'` belonged to the tag —
/// so, like [`emit_block_macro`]'s mid-prose opener, that line is given its own
/// `#' `. On reparse the title then sits on its own `#'` line above the underline
/// (the next-line setext form), which projects identically — idempotent.
fn emit_md_heading(items: &mut Vec<Ir>, node: &SyntaxNode) {
    let from_value = is_from_value(node);
    for (idx, seg) in node.text().to_string().split('\n').enumerate() {
        if idx == 0 && from_value {
            push_value_opener_line(items, seg, false);
        } else {
            push_line(items, normalize_marker_text(seg));
        }
    }
}

/// Emit a markdown thematic break (`@md` mode) as atomic passthrough, marker-
/// normalized. A thematic break is a single line; the node owns its own `#'` marker.
/// Marker-normalizing preserves the leading `***`/`---`/`___` run (it is trimmed
/// content, not touched), so on reparse it still forms the same break — idempotent.
/// A break opening as a tag's same-line value has a marker-less line; it is given
/// its own `#'` (the next-line form; a `---` value re-parses as a bare dash-run
/// heading no paragraph, which block dispatch promotes back to a break).
fn emit_md_thematic_break(items: &mut Vec<Ir>, node: &SyntaxNode) {
    if is_from_value(node) {
        push_value_opener_line(items, &node.text().to_string(), false);
    } else {
        for seg in node.text().to_string().split('\n') {
            push_line(items, normalize_marker_text(seg));
        }
    }
}

/// Marker-normalize a raw `#'` line string: drop surrounding whitespace (the
/// inter-line indentation), then emit the `#+'` marker, a single space, and the
/// trimmed content (or the bare marker when the content is empty).
fn normalize_marker_text(raw: &str) -> String {
    let s = raw.trim();
    let hashes = s.len() - s.trim_start_matches('#').len();
    if hashes == 0 || !s[hashes..].starts_with('\'') {
        return s.to_string();
    }
    let marker = &s[..hashes + 1];
    let content = s[hashes + 1..].trim();
    if content.is_empty() {
        marker.to_string()
    } else {
        format!("{marker} {content}")
    }
}

/// Like [`normalize_marker_text`] but **preserves the content's leading
/// indentation**: it normalizes the `#'` marker and the single conventional
/// space after it, yet keeps any further indentation the content carries. Used
/// for markdown lists, where that indentation is the semantic nesting depth (a
/// trimmed sublist would render as a flat list — a behavior change).
fn normalize_list_marker_text(raw: &str) -> String {
    let s = raw.trim();
    let hashes = s.len() - s.trim_start_matches('#').len();
    if hashes == 0 || !s[hashes..].starts_with('\'') {
        return s.to_string();
    }
    let marker = &s[..hashes + 1];
    // Drop only the one conventional space after the sigil; the rest of the
    // marker→content whitespace is the list's nesting indentation.
    let content = s[hashes + 1..]
        .strip_prefix(' ')
        .unwrap_or(&s[hashes + 1..]);
    if content.is_empty() {
        marker.to_string()
    } else {
        format!("{marker} {content}")
    }
}

/// Flatten a roxygen block into its logical leaves in source order, descending
/// through the structural `ROXYGEN_SECTION` / `ROXYGEN_PARAGRAPH` grouping but
/// keeping `ROXYGEN_TAG` and `ROXYGEN_RD_MACRO` nodes atomic (a tag header or an
/// inline/block macro is one unit of line content).
fn collect_logical_elements(
    node: &SyntaxNode,
    out: &mut Vec<NodeOrToken<SyntaxNode, SyntaxToken>>,
) {
    for el in node.children_with_tokens() {
        match el {
            NodeOrToken::Node(n)
                if matches!(
                    n.kind(),
                    SyntaxKind::ROXYGEN_SECTION | SyntaxKind::ROXYGEN_PARAGRAPH
                ) =>
            {
                collect_logical_elements(&n, out);
            }
            // A *cross-line* inline node — emphasis/strong or an inline link the
            // pass resolved across a soft line break — owns its inner `#'` markers
            // and newlines, so descend into it: its delimiter/bracket leaves and
            // text become ordinary inline elements that the marker/newline split
            // below distributes across the physical lines, where prose reflow
            // handles them (the resolved span is preserved on reparse — flanking
            // and bracket pairing are invariant under whitespace normalization). A
            // *single-line* span (no marker) stays atomic, so `*foo*` and a
            // one-line `[text](url)` each glue as one chunk.
            //
            // A cross-line raw-HTML span (`ROXYGEN_MD_HTML` node) or code span
            // (`ROXYGEN_MD_CODE` node) descends too so the physical lines
            // re-form — but unlike emphasis the paragraph carrying one bails
            // reflow entirely ([`line_has_cross_line_verbatim_span`]) instead
            // of rejoining.
            NodeOrToken::Node(n) if is_cross_line_inline(&n) => {
                collect_logical_elements(&n, out);
            }
            other => out.push(other),
        }
    }
}

/// Whether `node` is a resolved inline span (emphasis/strong, an inline link, a
/// raw-HTML span, or a code span) the inline pass carried across a soft line
/// break — it threads one or more `#'` markers as inter-line trivia. A
/// single-line span never contains a marker, so its presence distinguishes the
/// reflow-across-lines case from an atomic inline span (mirrors
/// [`is_block_macro`]).
fn is_cross_line_inline(node: &SyntaxNode) -> bool {
    matches!(
        node.kind(),
        SyntaxKind::ROXYGEN_MD_EMPH
            | SyntaxKind::ROXYGEN_MD_STRONG
            | SyntaxKind::ROXYGEN_MD_LINK
            | SyntaxKind::ROXYGEN_MD_HTML
            | SyntaxKind::ROXYGEN_MD_CODE
    ) && node
        .descendants_with_tokens()
        .any(|el| el.kind() == SyntaxKind::ROXYGEN_MARKER)
}

/// The cross-line node kinds whose paragraph bails reflow (see
/// [`line_has_cross_line_verbatim_span`]).
fn is_verbatim_span_kind(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::ROXYGEN_MD_HTML | SyntaxKind::ROXYGEN_MD_CODE
    )
}

/// Whether `line` touches a cross-line raw-HTML span (a `ROXYGEN_MD_HTML`
/// **node**) or code span (a `ROXYGEN_MD_CODE` **node**): either an element
/// sits inside one ([`collect_logical_elements`] descends the node, so a
/// covered line's elements are its children), or an atomic element — a folded
/// `ROXYGEN_TAG` whose same-line value opened the span — carries one as a
/// descendant. An HTML span's soft breaks are verbatim `\out` content (roxygen2
/// renders one VERB per source line), so joining or re-wrapping the lines it
/// touches would move a newline *inside* the rendered raw HTML — a change in
/// the rendered Rd (Tenet 1). A code span's soft break renders as a space, so a
/// rejoin *would* preserve the Rd — but re-wrapping could land a backtick run
/// at a line start where it reparses as a fence opener, so it takes the same
/// conservative bail (upgrade = backlog). The paragraph, section, or tag unit
/// carrying one is kept verbatim, marker-normalized. A *single-line* span is a
/// leaf (its parent is the paragraph), so it never trips this.
fn line_has_cross_line_verbatim_span(line: &PhysicalLine) -> bool {
    line.elements.iter().any(|el| {
        el.parent().is_some_and(|p| is_verbatim_span_kind(p.kind()))
            || el.as_node().is_some_and(|n| {
                n.descendants()
                    .any(|d| is_verbatim_span_kind(d.kind()) && is_cross_line_inline(&d))
            })
    })
}

/// Whether `line` touches a cross-line shortcut (`]`) or collapsed-reference
/// (`][]`) link node whose label is **invalid** — blank, or ending in a backslash
/// that escapes the closer after roxygen2's double-escape. Such a label never
/// defines or resolves, so its synthesized `[label]: R:…` definition *leaks* into
/// the rendered Rd with the label's soft break URL-encoded as `%0A`; joining the
/// wrap would re-encode it as `%20` — a rendered-Rd change — so the paragraph is
/// kept verbatim. A *valid* multi-line label is join-safe (it resolves through
/// whitespace-collapsing normalization and its destination encoding never
/// renders), and an inline/reference closer carries the label outside the wrapped
/// display, so neither bails.
fn line_has_leaky_cross_line_link(line: &PhysicalLine) -> bool {
    line.elements.iter().any(|el| {
        el.parent()
            .is_some_and(|p| p.kind() == SyntaxKind::ROXYGEN_MD_LINK && link_label_leaks(&p))
            || el.as_node().is_some_and(|n| {
                n.descendants().any(|d| {
                    d.kind() == SyntaxKind::ROXYGEN_MD_LINK
                        && is_cross_line_inline(&d)
                        && link_label_leaks(&d)
                })
            })
    })
}

/// Whether a `ROXYGEN_MD_LINK` node's label is the leak-invalid set of
/// [`line_has_leaky_cross_line_link`]: a shortcut/collapsed closer (the label is
/// the display) whose interior text — markers dropped — is blank or ends with a
/// backslash. Mirrors the projector's `linkref_label_is_usable` on the raw source.
fn link_label_leaks(node: &SyntaxNode) -> bool {
    let kids: Vec<_> = node.children_with_tokens().collect();
    let Some(closer) = kids.last() else {
        return false;
    };
    let closer_text = closer.to_string();
    if closer_text != "]" && closer_text != "][]" {
        return false;
    }
    let interior = kids.len().saturating_sub(1);
    let mut label = String::new();
    for el in kids.into_iter().take(interior).skip(1) {
        if el.kind() != SyntaxKind::ROXYGEN_MARKER {
            label.push_str(&el.to_string());
        }
    }
    label.ends_with('\\') || label.chars().all(|c| c.is_ascii_whitespace())
}

/// Build the IR for a `ROXYGEN_BLOCK` at the given nesting `indent`.
pub(super) fn ir_roxygen_block(node: &SyntaxNode, indent: usize, ctx: FormatContext) -> Ir {
    let style = ctx.style();
    let indent_cols = indent * style.indent_width;

    // The block's resolved markdown mode keys whether an unescaped `%` in prose is
    // a live Rd comment (non-markdown) that reflow must not move text across.
    let md = block_md(node);

    let mut items: Vec<Ir> = Vec::new();
    let mut para = Paragraph::default();
    let mut tag_unit: Option<TagUnit> = None;
    let mut section: Option<SectionUnit> = None;
    let mut tokenlist: Option<TokenListUnit> = None;
    let mut example = ExampleBody::default();
    let mut in_examples = false;
    let mut in_fence = false;
    let lw = style.line_width;

    // Flush all pending accumulators (only one is ever non-empty at a time). A
    // `SectionUnit` decides inline-vs-form-2 here from its buffered first paragraph
    // and `force_form2` flag.
    macro_rules! flush_pending {
        () => {{
            para.flush(&mut items, indent_cols, lw, md);
            flush_tag_unit(&mut tag_unit, &mut items, lw, md);
            flush_tokenlist(&mut tokenlist, &mut items);
            flush_section(&mut section, &mut items, lw, md);
            example.flush(&mut items, indent_cols, style);
        }};
    }

    for line in physical_lines(node) {
        // A block Rd macro is atomic passthrough: flush pending accumulators, then
        // emit it *flush* without reflowing. Inside an `@examples` body it wraps
        // example R (`\dontrun{}`/`\donttest{}`/…), emitted flush for copy-paste;
        // in a prose section its in-macro indentation is preserved. A block never
        // hangs under a tag — a `SectioningProse` section that carries a block is
        // forced to form-2 (bare tag line, block flush beneath), so the presence of
        // a block content-forces the section's layout before the block is emitted.
        if let Some(macro_node) = &line.block_macro {
            if let Some(s) = section.as_mut() {
                s.force_form2 = true;
            }
            flush_pending!();
            if macro_node.kind() == SyntaxKind::ROXYGEN_MD_LIST {
                // A markdown list is marker-normalized per line (the in-list
                // indentation that distinguishes nesting is not yet modeled, so
                // dropping it matches today's structured-passthrough output).
                emit_md_list(&mut items, macro_node);
            } else if macro_node.kind() == SyntaxKind::ROXYGEN_MD_CODE_BLOCK {
                // A fenced code block is atomic passthrough, each line marker-
                // normalized — byte-identical to the pre-node textual fence path.
                emit_md_code_block(&mut items, macro_node);
            } else if macro_node.kind() == SyntaxKind::ROXYGEN_MD_INDENTED_CODE {
                // An indented code block is atomic passthrough, but — unlike the
                // other blocks — its leading indentation is the semantic signal that
                // makes it code (>= 4 columns past the stripped `#'`), so it must be
                // preserved, not trimmed. Marker-normalize while keeping the content
                // indentation (as the markdown-list passthrough does for nesting).
                emit_md_indented_code(&mut items, macro_node);
            } else if macro_node.kind() == SyntaxKind::ROXYGEN_MD_HTML_BLOCK {
                // An HTML block is atomic passthrough (verbatim raw HTML), each
                // line marker-normalized — never reflowed across lines.
                emit_md_html_block(&mut items, macro_node);
            } else if macro_node.kind() == SyntaxKind::ROXYGEN_MD_BLOCK_QUOTE {
                // A block quote is atomic passthrough, each `>` line marker-
                // normalized — never reflowed. On reparse each line still opens with
                // `>`, so the same block quote re-forms (idempotent).
                emit_md_block_quote(&mut items, macro_node);
            } else if macro_node.kind() == SyntaxKind::ROXYGEN_MD_TABLE {
                // A GFM table is atomic passthrough, each line marker-normalized —
                // never reflowed (arity does not lay out roxygen table content).
                emit_md_table_block(&mut items, macro_node);
            } else if macro_node.kind() == SyntaxKind::ROXYGEN_MD_HEADING {
                // An ATX heading is a single marker-normalized line — never reflowed.
                emit_md_heading(&mut items, macro_node);
            } else if macro_node.kind() == SyntaxKind::ROXYGEN_MD_THEMATIC_BREAK {
                // A thematic break is a single marker-normalized line — never reflowed.
                // On reparse the same `***`/`---`/`___` line re-forms the break
                // (idempotent).
                emit_md_thematic_break(&mut items, macro_node);
            } else if in_examples {
                emit_block_macro_examples(&mut items, macro_node);
            } else {
                emit_block_macro(&mut items, macro_node);
            }
            continue;
        }

        // While collecting an `@examples` body, every non-tag line is embedded R
        // and belongs to the body (blank/fenced/structured lines included); a tag
        // line ends the body and falls through to the tag branch, which flushes.
        if in_examples && line.tag().is_none() {
            example.push_line(&line);
            continue;
        }

        let content = content_text(&line);
        let is_fence = is_fence_marker(&content);

        // Fenced code block: everything between fences (and the fence lines
        // themselves) is passthrough; a fence marker toggles the state. A fence
        // inside a section forces it to form-2 (the fenced block cannot be inline).
        if in_fence || is_fence {
            in_fence = !is_fence || !in_fence;
            if let Some(s) = section.as_mut() {
                s.force_form2 = true;
            }
            flush_pending!();
            emit_normalized(&mut items, &line);
            continue;
        }

        // Tag line: closes any open unit/section, then opens the accumulator its
        // class dictates. Layout is chosen by `tag_class`, never by whether the
        // body was written inline (form-1) or on the next line (form-2).
        if let Some(tag) = line.tag() {
            in_examples = tag.is_examples();
            flush_pending!();
            match tag_class(&tag) {
                // `@examples` body is captured on following iterations; other Code
                // tags (`@usage`/`@eval*`) are verbatim, header spacing normalized.
                TagClass::Code | TagClass::AtomicValue | TagClass::Toggle | TagClass::Section => {
                    emit_tag_passthrough(&mut items, &line, &tag);
                }
                TagClass::TokenList => {
                    tokenlist = Some(TokenListUnit::new(&line, &tag));
                }
                // Open unconditionally (even with no inline prose): the unit pulls
                // up the following prose under the inline `@tag name` header.
                TagClass::NameBearingProse => {
                    tag_unit = Some(TagUnit::new(&line, &tag, indent_cols));
                }
                TagClass::SectioningProse => {
                    section = Some(SectionUnit::new(&line, &tag, indent_cols));
                }
            }
            continue;
        }

        // A line continues open prose when a unit/section/paragraph is mid-flight;
        // that gates ordered-list recognition (a non-`1` marker can't interrupt it).
        let in_paragraph = tag_unit.is_some()
            || tokenlist.is_some()
            || section.as_ref().is_some_and(|s| !s.chunks.is_empty())
            || !para.lines.is_empty();

        // Blank separator: a boundary, except inside an open section, where it is
        // buffered — a *trailing* blank before the next tag must not flip the
        // section's inline/form-2 decision (Tenet 1), and a blank *followed* by
        // more prose marks a real paragraph break (handled in the prose branch).
        if line.is_blank() {
            if let Some(s) = section.as_mut() {
                s.pending_blanks.push(line.clone());
                continue;
            }
            flush_pending!();
            emit_normalized(&mut items, &line);
            continue;
        }

        // Structured line (list/blockquote/header/table delimiter row): never
        // reflowed. Inside a section it content-forces form-2, then emits flush
        // beneath the bare tag.
        if is_structured(&content, in_paragraph) {
            if let Some(s) = section.as_mut() {
                s.force_form2 = true;
            }
            flush_pending!();
            emit_normalized(&mut items, &line);
            continue;
        }

        // Plain prose. A marker change (e.g. `#'` then `##'`) starts fresh.
        let marker = marker_text(&line);

        // Continuation of an open section (same marker): absorb into the section's
        // first paragraph, unless a blank has intervened — that opens a *second*
        // paragraph, which content-forces form-2 and dissolves the rest of the
        // section body into ordinary flush prose.
        if let Some(s) = section.as_mut() {
            if s.marker == marker && s.pending_blanks.is_empty() {
                s.push_prose(&line);
                continue;
            }
            if s.marker == marker {
                s.force_form2 = true;
            }
            flush_section(&mut section, &mut items, lw, md);
        }

        // Continuation of an open token list (same marker): absorb; a boundary
        // otherwise flushes it onto its single joined line.
        if let Some(t) = tokenlist.as_mut() {
            if t.marker == marker {
                t.push_continuation(&line);
                continue;
            }
            flush_tokenlist(&mut tokenlist, &mut items);
        }

        // Continuation of an open tag unit (same marker): absorb and hang-indent.
        if let Some(unit) = tag_unit.as_mut() {
            if unit.marker == marker {
                unit.push_continuation(&line);
                continue;
            }
            flush_tag_unit(&mut tag_unit, &mut items, lw, md);
        }

        // Otherwise accumulate into the current plain-prose paragraph.
        if para.marker.as_deref().is_some_and(|m| m != marker) {
            para.flush(&mut items, indent_cols, lw, md);
        }
        if para.marker.is_none() {
            para.marker = Some(marker);
        }
        para.push_line(&line);
    }
    flush_pending!();

    Ir::concat(items)
}

/// A run of consecutive plain-prose roxygen lines awaiting reflow.
#[derive(Default)]
struct Paragraph {
    marker: Option<String>,
    /// Breakable chunks across all lines, in source order (a chunk is a maximal
    /// run with no breakable whitespace; protected spans are glued in).
    chunks: Vec<String>,
    /// The source lines, kept for the verbatim fallback.
    lines: Vec<PhysicalLine>,
}

impl Paragraph {
    fn push_line(&mut self, line: &PhysicalLine) {
        line_chunks(line, &mut self.chunks);
        self.lines.push(line.clone());
    }

    fn clear(&mut self) {
        self.marker = None;
        self.chunks.clear();
        self.lines.clear();
    }

    /// Emit the pending paragraph (if any) into `items`, then reset.
    fn flush(&mut self, items: &mut Vec<Ir>, indent_cols: usize, line_width: usize, md: bool) {
        if self.lines.is_empty() {
            return;
        }
        // Reflow only when — in non-markdown prose, which is literal Rd — no line
        // carries a live `%` comment, since reflowing would move text across the
        // comment boundary and change the rendered Rd (a Tenet-1 violation), and —
        // in markdown prose — the paragraph does not begin with a link-reference
        // definition, which reflow would join into ordinary prose (changing the
        // rendered Rd the same way); otherwise keep the original line breaks,
        // marker-normalized. A chunk that could migrate to a line start and reparse
        // as a structured construct is not a bail: `wrap_chunks` keeps such a marker
        // off any line start, preserving idempotence without abandoning reflow.
        let first_text = self.lines.first().map(content_text).unwrap_or_default();
        if self.chunks.is_empty() || prose_bails_reflow(&self.lines, &first_text, md) {
            let lines = std::mem::take(&mut self.lines);
            for line in &lines {
                emit_normalized(items, line);
            }
        } else {
            let marker = self.marker.clone().unwrap_or_else(|| "#'".to_string());
            let prefix = indent_cols + marker.chars().count() + 1;
            let budget = line_width.saturating_sub(prefix).max(1);
            for wrapped in wrap_chunks(&self.chunks, budget) {
                push_prose_line(items, &marker, &wrapped);
            }
        }
        self.clear();
    }
}

/// A tag line carrying inline prose (`@param x <prose>`) together with the
/// plain-prose lines that follow it, reflowed as one unit with the tag header on
/// the first line and a two-space hanging indent on continuation lines.
struct TagUnit {
    marker: String,
    indent_cols: usize,
    /// The normalized tag header, e.g. `@param x` (single-spaced).
    header: String,
    /// Breakable prose chunks (the tag's own prose plus absorbed continuations).
    chunks: Vec<String>,
    /// Source lines (tag line first), kept for the verbatim fallback.
    lines: Vec<PhysicalLine>,
    /// Whether the tag's own prose value (the field's block start) opens a
    /// link-reference definition, in which case reflow must not join it with the
    /// absorbed continuations (see [`text_opens_linkref_def`]).
    first_is_linkref_def: bool,
}

impl TagUnit {
    fn new(line: &PhysicalLine, tag: &RoxygenTag, indent_cols: usize) -> Self {
        let mut chunks = Vec::new();
        tag_prose_chunks(tag, &mut chunks);
        TagUnit {
            marker: marker_text(line),
            indent_cols,
            header: tag_header(tag).unwrap_or_else(|| "@".to_string()),
            chunks,
            lines: vec![line.clone()],
            first_is_linkref_def: text_opens_linkref_def(&tag_first_line_value(tag)),
        }
    }

    /// Absorb a following plain-prose line as continuation text.
    fn push_continuation(&mut self, line: &PhysicalLine) {
        line_chunks(line, &mut self.chunks);
        self.lines.push(line.clone());
    }

    /// Emit the reflowed tag unit into `items`.
    fn flush(self, items: &mut Vec<Ir>, line_width: usize, md: bool) {
        let marker_w = self.marker.chars().count();
        // In non-markdown (literal Rd) prose, a line carrying a live `%` comment
        // must not be reflowed (it would move text across the comment, changing the
        // rendered Rd); and in markdown prose, a tag value that begins with a
        // link-reference definition — or any line carrying an active `%`-swallow —
        // must not be joined with its continuations (the same render change): bail
        // to a verbatim, marker-normalized rendering. A
        // prose chunk that could migrate to a continuation-line start and reparse as
        // a list/header marker is not a bail: `wrap_chunks_hanging` keeps such a
        // marker off any line start, preserving idempotence without abandoning
        // reflow. A *whole folded line* that is already structured is different:
        // the sibling shape keeps it verbatim, so the folded shape must too
        // (`tag_folds_structured_line`).
        if self.lines.iter().any(tag_folds_structured_line)
            || self.lines.iter().any(line_has_sticky_swallow)
            || (!md && self.lines.iter().any(line_has_live_rd_comment))
            || (md
                && (self.first_is_linkref_def
                    || self.lines.iter().any(line_has_md_percent_swallow)
                    || self.lines.iter().any(line_has_cross_line_verbatim_span)))
        {
            for (i, line) in self.lines.iter().enumerate() {
                if i == 0
                    && let Some(tag) = line.tag()
                {
                    emit_tag_passthrough(items, line, &tag);
                } else {
                    emit_normalized(items, line);
                }
            }
            return;
        }

        // Line 1 starts after `marker @header `; continuations after `marker `
        // plus two extra spaces (the tidyverse hanging indent).
        let first_start = self.indent_cols + marker_w + 1 + self.header.chars().count() + 1;
        let cont_start = self.indent_cols + marker_w + 3;
        let first_budget = line_width.saturating_sub(first_start).max(1);
        let cont_budget = line_width.saturating_sub(cont_start).max(1);

        let prose = wrap_chunks_hanging(&self.chunks, first_budget, cont_budget);
        let marker = &self.marker;
        let header = &self.header;
        if prose[0].is_empty() {
            push_line(items, format!("{marker} {header}"));
        } else {
            push_line(items, format!("{marker} {header} {}", prose[0]));
        }
        for cont in &prose[1..] {
            push_line(items, format!("{marker}   {cont}"));
        }
    }
}

/// Emit the pending tag unit (if any) into `items`, then clear it.
fn flush_tag_unit(unit: &mut Option<TagUnit>, items: &mut Vec<Ir>, line_width: usize, md: bool) {
    if let Some(unit) = unit.take() {
        unit.flush(items, line_width, md);
    }
}

/// Whether a run of prose lines must not be reflowed, so it is kept verbatim
/// (marker-normalized). In non-markdown (literal Rd) prose any line carrying a
/// live `%` comment forbids reflow (it would move text across the comment,
/// changing the rendered Rd); in markdown prose a leading link-reference
/// definition — or any line with an active `%`-swallow — does (joining it moves
/// text across the swallow or turns a definition back into prose). `first_text`
/// is the paragraph's first prose line — for a tag-headed body, the tag *value*
/// rather than the whole `@tag …` line, so the linkref check reads the right text.
fn prose_bails_reflow(lines: &[PhysicalLine], first_text: &str, md: bool) -> bool {
    // A brace-less sticky code/verbatim swallow forbids reflow in either mode: it
    // makes each physical line a verbatim `RCODE`/`VERB` atom, so joining/splitting
    // changes the atom count (Tenet 1).
    lines.iter().any(line_has_sticky_swallow)
        || if md {
            text_opens_linkref_def(first_text)
                || lines.iter().any(line_has_md_percent_swallow)
                || lines.iter().any(line_has_cross_line_verbatim_span)
                || lines.iter().any(line_has_leaky_cross_line_link)
        } else {
            lines.iter().any(line_has_live_rd_comment)
        }
}

/// A `SectioningProse` tag (`@details`, `@return`, …) together with its body,
/// laid out by class rather than by written form: the body is emitted inline on
/// the tag line when it is a single paragraph that fits the line width, otherwise
/// form-2 (a bare `#' @tag` line with the body flush beneath). Because the tag's
/// inline prose (form-1) and a following sibling paragraph (form-2) both feed the
/// same first paragraph here, the two written forms converge on identical output.
///
/// Only the *first* paragraph is buffered: once a block, a structured line, or a
/// second paragraph appears, `force_form2` is set and the header + first paragraph
/// are emitted form-2, after which the rest of the section body flows through the
/// ordinary flush-prose path. Trailing blanks are buffered in `pending_blanks` so
/// a blank before the next tag cannot flip the inline/form-2 decision (Tenet 1).
struct SectionUnit {
    marker: String,
    indent_cols: usize,
    /// The normalized tag header, e.g. `@return` or `@format`.
    header: String,
    /// Breakable chunks of the body's first paragraph.
    chunks: Vec<String>,
    /// The first paragraph's source lines (tag line first), for the reflow bail.
    lines: Vec<PhysicalLine>,
    /// The first paragraph's first prose line's text (tag value for form-1, the
    /// first following prose line for form-2), for the markdown linkref-def bail.
    first_text: Option<String>,
    /// Buffered blank separators after the first paragraph (trailing blanks are
    /// dropped; a blank followed by prose forces form-2, handled by the caller).
    pending_blanks: Vec<PhysicalLine>,
    /// Set when a block / structured line / second paragraph makes the body
    /// unrepresentable inline, forcing the bare-tag-line (form-2) layout.
    force_form2: bool,
}

impl SectionUnit {
    fn new(line: &PhysicalLine, tag: &RoxygenTag, indent_cols: usize) -> Self {
        let mut chunks = Vec::new();
        tag_prose_chunks(tag, &mut chunks);
        // Form-1: the tag carries the body's first prose inline; record its value.
        let first_text = (!chunks.is_empty()).then(|| tag_first_line_value(tag));
        SectionUnit {
            marker: marker_text(line),
            indent_cols,
            header: tag_header(tag).unwrap_or_else(|| "@".to_string()),
            chunks,
            lines: vec![line.clone()],
            first_text,
            pending_blanks: Vec::new(),
            force_form2: false,
        }
    }

    /// Absorb a following plain-prose line into the body's first paragraph.
    fn push_prose(&mut self, line: &PhysicalLine) {
        // Form-2: the first following prose line supplies the paragraph's first text.
        if self.first_text.is_none() {
            self.first_text = Some(content_text(line));
        }
        line_chunks(line, &mut self.chunks);
        self.lines.push(line.clone());
    }

    /// Emit the section (header + first paragraph) into `items`, then the buffered
    /// separators. Subsequent body content (if `force_form2` was set) is emitted by
    /// the caller through the ordinary flush-prose path.
    fn flush(self, items: &mut Vec<Ir>, line_width: usize, md: bool) {
        let marker = &self.marker;
        let header = &self.header;
        let first_text = self.first_text.as_deref().unwrap_or("");
        if !self.chunks.is_empty()
            && (prose_bails_reflow(&self.lines, first_text, md)
                || self.lines.iter().any(tag_folds_structured_line))
        {
            // Cannot reflow safely: passthrough the source lines marker-normalized
            // (form preserved), the tag line keeping its inline value.
            for (i, line) in self.lines.iter().enumerate() {
                if i == 0
                    && let Some(tag) = line.tag()
                {
                    emit_tag_passthrough(items, line, &tag);
                } else {
                    emit_normalized(items, line);
                }
            }
        } else if self.chunks.is_empty() {
            push_line(items, format!("{marker} {header}"));
        } else {
            let one = self.chunks.join(" ");
            let inline_w = self.indent_cols
                + marker.chars().count()
                + 1
                + header.chars().count()
                + 1
                + one.chars().count();
            if !self.force_form2 && inline_w <= line_width {
                push_line(items, format!("{marker} {header} {one}"));
            } else {
                push_line(items, format!("{marker} {header}"));
                let prefix = self.indent_cols + marker.chars().count() + 1;
                let budget = line_width.saturating_sub(prefix).max(1);
                for wrapped in wrap_chunks(&self.chunks, budget) {
                    push_prose_line(items, marker, &wrapped);
                }
            }
        }
        for blank in &self.pending_blanks {
            emit_normalized(items, blank);
        }
    }
}

/// Emit the pending section (if any) into `items`, then clear it.
fn flush_section(
    section: &mut Option<SectionUnit>,
    items: &mut Vec<Ir>,
    line_width: usize,
    md: bool,
) {
    if let Some(section) = section.take() {
        section.flush(items, line_width, md);
    }
}

/// A `TokenList` tag (`@keywords`, `@importFrom`, …) together with any following
/// continuation lines, all joined onto the tag's single line. These are
/// directives, not prose, so the joined line is never wrapped (overflow tolerated).
struct TokenListUnit {
    marker: String,
    header: String,
    chunks: Vec<String>,
}

impl TokenListUnit {
    fn new(line: &PhysicalLine, tag: &RoxygenTag) -> Self {
        let mut chunks = Vec::new();
        tag_prose_chunks(tag, &mut chunks);
        TokenListUnit {
            marker: marker_text(line),
            header: tag_header(tag).unwrap_or_else(|| "@".to_string()),
            chunks,
        }
    }

    /// Absorb a following plain-prose line's tokens onto the joined line.
    fn push_continuation(&mut self, line: &PhysicalLine) {
        line_chunks(line, &mut self.chunks);
    }

    /// Emit the single joined `#' @tag <tokens>` line (bare `#' @tag` when empty).
    fn flush(self, items: &mut Vec<Ir>) {
        let marker = &self.marker;
        let header = &self.header;
        if self.chunks.is_empty() {
            push_line(items, format!("{marker} {header}"));
        } else {
            push_line(
                items,
                format!("{marker} {header} {}", self.chunks.join(" ")),
            );
        }
    }
}

/// Emit the pending token list (if any) into `items`, then clear it.
fn flush_tokenlist(tokenlist: &mut Option<TokenListUnit>, items: &mut Vec<Ir>) {
    if let Some(tokenlist) = tokenlist.take() {
        tokenlist.flush(items);
    }
}

/// A run of `@examples`/`@examplesIf` body lines awaiting embedded-R formatting
/// (transform 4). The lines are kept so they can be re-emitted verbatim
/// (marker-normalized) if the collected source fails to parse as R.
#[derive(Default)]
struct ExampleBody {
    marker: Option<String>,
    lines: Vec<PhysicalLine>,
}

impl ExampleBody {
    fn push_line(&mut self, line: &PhysicalLine) {
        if self.marker.is_none() {
            self.marker = Some(marker_text(line));
        }
        self.lines.push(line.clone());
    }

    /// Format the collected body as embedded R and emit it re-prefixed, clearing
    /// the buffer. The body is formatted with a line-width budget reduced by the
    /// marker prefix and indentation so the `#'`-prefixed lines respect the line
    /// width (Tenet 1). On a parse error — or a blank-only body — the original
    /// lines are passed through marker-normalized instead.
    fn flush(&mut self, items: &mut Vec<Ir>, indent_cols: usize, style: FormatStyle) {
        if self.lines.is_empty() {
            return;
        }
        let lines = std::mem::take(&mut self.lines);
        let marker = self.marker.take().unwrap_or_else(|| "#'".to_string());

        // Trailing blank lines are separators before the next tag (or block end),
        // not code: the embedded-R formatter would strip them, so peel them off
        // and re-emit them marker-normalized after the formatted body.
        let body_end = lines
            .iter()
            .rposition(|l| !l.is_blank())
            .map_or(0, |i| i + 1);
        let (body, trailing) = lines.split_at(body_end);

        let source = body.iter().map(content_text).collect::<Vec<_>>().join("\n");

        // A blank-only body has nothing to format; keep it as-is.
        if source.trim().is_empty() {
            for line in &lines {
                emit_normalized(items, line);
            }
            return;
        }

        let budget = style
            .line_width
            .saturating_sub(indent_cols + marker.len() + 1)
            .max(1);
        // Only the width budget differs; `.lines()` below strips the embedded
        // output's newlines, so its line ending is immaterial (the outer pass
        // applies the configured one).
        let body_style = FormatStyle {
            line_width: budget,
            ..style
        };

        match format_with_style(&source, body_style) {
            Ok(formatted) => {
                for code in formatted.lines() {
                    if code.is_empty() {
                        push_line(items, marker.clone());
                    } else {
                        push_line(items, format!("{marker} {code}"));
                    }
                }
            }
            Err(_) => {
                for line in body {
                    emit_normalized(items, line);
                }
            }
        }
        for line in trailing {
            emit_normalized(items, line);
        }
    }
}

/// Greedy first-fit wrap where the first line has its own (typically smaller)
/// budget — the room left beside the tag header — and every continuation line
/// uses `cont_budget`. The returned vector's first element is the line-1 prose
/// (empty when nothing fits beside the header); the rest are continuation lines.
fn wrap_chunks_hanging(chunks: &[String], first_budget: usize, cont_budget: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    let mut budget = first_budget;
    for chunk in chunks {
        let w = chunk.chars().count();
        if cur.is_empty() {
            // The first prose chunk does not fit beside the header: leave line 1
            // header-only and start it on a continuation line — unless the chunk
            // is a complete standalone tag, which alone on the line directly
            // after a bare `@tag` header would reparse as an HTML block
            // (CommonMark condition 7); keep it beside the header, accepting
            // overflow.
            if lines.is_empty()
                && budget == first_budget
                && w > first_budget
                && first_budget < cont_budget
                && !is_md_standalone_html_tag(chunk)
            {
                lines.push(String::new());
                budget = cont_budget;
            }
            cur.push_str(chunk);
            cur_w = w;
        } else if cur_w + 1 + w <= budget || is_unsafe_line_start(chunk) {
            // Fits, or must not break here: breaking would drop an unsafe marker
            // onto a continuation-line start, where it could reparse as a block
            // construct (breaking idempotence). Keep it inline, accepting overflow.
            cur.push(' ');
            cur.push_str(chunk);
            cur_w += 1 + w;
        } else {
            lines.push(std::mem::take(&mut cur));
            budget = cont_budget;
            cur.push_str(chunk);
            cur_w = w;
        }
    }
    lines.push(cur);
    lines
}

/// Greedy first-fit wrap of `chunks` into lines no wider than `budget` (in
/// chars). A chunk wider than `budget` gets its own line, un-broken. Returns at
/// least one line when `chunks` is non-empty.
fn wrap_chunks(chunks: &[String], budget: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    for chunk in chunks {
        let w = chunk.chars().count();
        if cur.is_empty() {
            cur.push_str(chunk);
            cur_w = w;
        } else if cur_w + 1 + w <= budget
            || is_unsafe_line_start(chunk)
            // Never end the paragraph's FIRST line as a complete standalone tag:
            // at a fresh position that line reparses as an HTML block (CommonMark
            // condition 7). Later lines are safe — condition 7 cannot interrupt
            // the then-open paragraph.
            || (lines.is_empty() && is_md_standalone_html_tag(&cur))
        {
            // Fits, or must not break here: breaking would drop an unsafe marker
            // onto a line start, where it could reparse as a block construct
            // (breaking idempotence). Keep it inline, accepting overflow.
            cur.push(' ');
            cur.push_str(chunk);
            cur_w += 1 + w;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur.push_str(chunk);
            cur_w = w;
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Split a roxygen line's content into breakable chunks, appending to `out`.
/// Prose whitespace (inside `ROXYGEN_TEXT`) is a break opportunity; protected
/// spans are glued to whatever abuts them (so `[g()].` stays one chunk). The
/// line boundary itself ends a chunk.
fn line_chunks(line: &PhysicalLine, out: &mut Vec<String>) {
    chunk_elements(content_elements(line), out);
}

/// Split a sequence of content elements into breakable chunks, appending to
/// `out`. `ROXYGEN_TEXT` whitespace is a break opportunity; every other token or
/// node (protected spans included) is glued to whatever abuts it.
fn chunk_elements<I>(elements: I, out: &mut Vec<String>)
where
    I: Iterator<Item = NodeOrToken<SyntaxNode, SyntaxToken>>,
{
    let mut cur = String::new();
    chunk_into(elements, &mut cur, out);
    if !cur.is_empty() {
        out.push(cur);
    }
}

/// Chunk `elements` into `out`, threading the in-progress chunk `cur` so a
/// descent into a cross-line span preserves gluing across the recursion boundary.
fn chunk_into<I>(elements: I, cur: &mut String, out: &mut Vec<String>)
where
    I: Iterator<Item = NodeOrToken<SyntaxNode, SyntaxToken>>,
{
    for el in elements {
        match el {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::ROXYGEN_TEXT => {
                for ch in t.text().chars() {
                    if ch.is_whitespace() {
                        if !cur.is_empty() {
                            out.push(std::mem::take(cur));
                        }
                    } else {
                        cur.push(ch);
                    }
                }
            }
            // A folded continuation threads its inter-line trivia into the tag: a
            // newline (soft break) and a marker→content whitespace are break
            // opportunities, the `#'` marker is dropped. (For every other caller
            // these never appear, so this is a no-op there.)
            NodeOrToken::Token(t)
                if matches!(t.kind(), SyntaxKind::NEWLINE | SyntaxKind::WHITESPACE) =>
            {
                if !cur.is_empty() {
                    out.push(std::mem::take(cur));
                }
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::ROXYGEN_MARKER => {}
            // A cross-line emphasis/strong/link span owns its inner `#'` markers and
            // newlines: descend so its text chunks across the soft break the same
            // way a paragraph's does (a single-line span has no marker, so it is
            // glued whole below).
            NodeOrToken::Node(n) if is_cross_line_inline(&n) => {
                chunk_into(n.children_with_tokens(), cur, out);
            }
            // Protected span (or any other content token/node): glue it in.
            NodeOrToken::Token(t) => cur.push_str(t.text()),
            NodeOrToken::Node(n) => cur.push_str(&n.text().to_string()),
        }
    }
}

/// The content elements of a line: everything after the marker and the single
/// marker→content whitespace (which the formatter drops). The marker itself is
/// already held apart in `PhysicalLine::marker`, so only the leading whitespace
/// needs skipping.
fn content_elements(
    line: &PhysicalLine,
) -> impl Iterator<Item = NodeOrToken<SyntaxNode, crate::syntax::SyntaxToken>> + '_ {
    let mut seen_content = false;
    line.elements
        .iter()
        .filter(move |el| match el.kind() {
            SyntaxKind::WHITESPACE if !seen_content => false,
            _ => {
                seen_content = true;
                true
            }
        })
        .cloned()
}

/// The trimmed text content of a line (everything after the marker), used for
/// structured-line classification.
fn content_text(line: &PhysicalLine) -> String {
    let mut s = String::new();
    for el in content_elements(line) {
        match el {
            NodeOrToken::Token(t) => s.push_str(t.text()),
            NodeOrToken::Node(n) => s.push_str(&n.text().to_string()),
        }
    }
    s.trim().to_string()
}

/// The `#'` marker text of a line (defaulting to `#'` if somehow absent).
fn marker_text(line: &PhysicalLine) -> String {
    line.marker()
        .map(|t| t.text().to_string())
        .unwrap_or_else(|| "#'".to_string())
}

/// Emit a line marker-normalized (transform 1): marker, a single space, the
/// content verbatim, trailing whitespace trimmed; a blank line is just the
/// marker. Boundary lines (tags, blanks, structured, fenced, examples) take
/// this path.
fn emit_normalized(items: &mut Vec<Ir>, line: &PhysicalLine) {
    push_line(items, normalize_roxygen_line(line));
}

/// Emit a tag line that is not reflowed (a code/example body, a structured
/// `@section Title:` heading, a namespace directive, or a bare tag) with its
/// internal spacing normalized: marker, header (`@tag [arg]`, single-spaced),
/// then the remaining content verbatim. Falls back to plain marker
/// normalization if the tag has no name (malformed).
fn emit_tag_passthrough(items: &mut Vec<Ir>, line: &PhysicalLine, tag: &RoxygenTag) {
    let Some(header) = tag_header(tag) else {
        emit_normalized(items, line);
        return;
    };
    let marker = marker_text(line);
    // A folded multi-line tag (a same-line value with plain-prose continuations)
    // is not reflowed on the bail path: keep its physical-line structure so a
    // `%`-swallow / linkref-def / live-`%` line is never joined with its
    // continuation, and a folded structured line stays on its own line. Each
    // source line is marker-normalized **keeping its content indentation** (a
    // reflowed tag unit's hanging indent, or a list continuation's nesting
    // indent, must survive the passthrough or the reflowed and bailed shapes
    // would never agree on a fixed point); the folded continuation segments
    // already carry their `#'` markers, and the first segment carries the
    // `@name [arg] <value>` header verbatim.
    if is_multiline_tag(tag.syntax()) {
        for (i, seg) in tag.syntax().text().to_string().split('\n').enumerate() {
            if i == 0 {
                push_line(items, format!("{marker} {}", seg.trim_end()));
            } else {
                push_line(items, normalize_list_marker_text(seg));
            }
        }
        return;
    }
    let rest = tag_rest_verbatim(tag);
    if rest.is_empty() {
        push_line(items, format!("{marker} {header}"));
    } else {
        push_line(items, format!("{marker} {header} {rest}"));
    }
}

/// The layout class of a roxygen tag: the *single* axis that decides how the tag
/// and its body are laid out. Layout is chosen by class, never by the input's
/// written form (Tenet 1: `@details x` and `@details`⏎`x` render identically in
/// roxygen2, so they must format identically). Classes are derived from roxygen2's
/// own tag-parser model; the comments on [`classify`] record that provenance so
/// moving a tag between classes is a one-line, auditable edit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TagClass {
    /// `tag_name_description` — an inline name label followed by prose, e.g.
    /// `@param x <prose>`. Canonical: header inline, continuations hang two spaces.
    NameBearingProse,
    /// `tag_markdown` — free-form section prose with no inline label. Canonical:
    /// body inline on the tag line when it fits, else form-2 (bare tag line, body
    /// flush at `#'`).
    SectioningProse,
    /// `tag_code`/`tag_examples` — embedded R or verbatim code, never prose-reflowed.
    Code,
    /// `tag_value` — a single value that occupies the rest of the line (spaces
    /// significant), one line, overflow tolerated.
    AtomicValue,
    /// `tag_words` and namespace directives — a token list joined onto one line.
    TokenList,
    /// `tag_toggle` — a bare directive with no body (`@export`, `@noRd`, `@md`).
    Toggle,
    /// `@section`: a verbatim `Title:` kept inline, body always form-2 flush.
    Section,
}

/// Classify a tag by name into its [`TagClass`]. The `match` arms are grouped by
/// roxygen2 parser (`tag_name_description`, `tag_markdown`, `tag_code`, `tag_value`,
/// `tag_words`, `tag_toggle`) so a reclassification is a one-line move. Unknown or
/// custom tags default to [`TagClass::SectioningProse`] (matching the prior "an
/// unrecognized prose tag reflows" behavior); this is the sole guessed assignment.
fn classify(name: &str) -> TagClass {
    use TagClass::*;
    match name {
        // tag_name_description (+ `@prop`, roxygen2 8.0.0's S7 two-part tag)
        "param" | "slot" | "field" | "prop" => NameBearingProse,
        // tag_markdown (section-producing prose)
        "description" | "details" | "return" | "value" | "format" | "note" | "references"
        | "source" | "seealso" | "author" | "title" => SectioningProse,
        "section" => Section,
        // tag_examples / tag_code
        "examples" | "examplesIf" | "usage" | "eval" | "evalRd" | "evalNamespace" => Code,
        // tag_value (single verbatim value; interior spaces significant).
        // `R6method` (roxygen2 8.0.0) carries a single `Class$method` target.
        "name" | "rdname" | "docType" | "encoding" | "family" | "concept" | "inheritParams"
        | "backref" | "exportClass" | "exportMethod" | "exportPattern" | "R6method" => AtomicValue,
        // tag_words / namespace directives (join to one line)
        "keywords" | "aliases" | "import" | "importFrom" | "importClassesFrom"
        | "importMethodsFrom" | "exportS3Method" | "useDynLib" | "rawNamespace" => TokenList,
        // tag_toggle
        "export" | "noRd" | "md" | "noMd" => Toggle,
        _ => SectioningProse,
    }
}

/// The tag's layout class, defaulting a nameless (malformed) tag to
/// [`TagClass::SectioningProse`] so it degrades to the safe passthrough fallback.
fn tag_class(tag: &RoxygenTag) -> TagClass {
    tag.name()
        .as_deref()
        .map_or(TagClass::SectioningProse, classify)
}

/// Whether `kind` is a roxygen prose element (plain text or a protected span).
/// `ROXYGEN_RD_MACRO` is a *node* (its content is sub-parsed) while the others
/// are leaf tokens; `el.kind()` reports the same kind for either, so callers
/// match on the element's kind rather than requiring a token.
fn is_tag_prose_kind(kind: SyntaxKind) -> bool {
    kind.is_roxygen_prose_content()
}

/// The normalized tag header: `@name` plus, for an arg-bearing tag, ` arg`
/// (single-spaced). `None` when the tag has no name.
fn tag_header(tag: &RoxygenTag) -> Option<String> {
    let name = tag.name()?;
    let mut header = String::from("@");
    header.push_str(&name);
    if let Some(arg) = tag.arg() {
        header.push(' ');
        header.push_str(arg.text());
    }
    Some(header)
}

/// The tag's *first physical line's* prose value (everything after the header, up
/// to the first folded soft break), trimmed. For a single-line tag this is the
/// whole value; for a folded multi-line tag it is only the same-line value — which
/// is what the reflow-bail check reads (a link-reference definition or `%`-swallow
/// keys on the field's first line, not the joined continuation).
fn tag_first_line_value(tag: &RoxygenTag) -> String {
    let mut s = String::new();
    for el in tag.syntax().children_with_tokens() {
        match el.kind() {
            SyntaxKind::NEWLINE => break,
            k if is_tag_prose_kind(k) => match el {
                NodeOrToken::Token(t) => s.push_str(t.text()),
                NodeOrToken::Node(n) if is_cross_line_inline(&n) => {
                    // A cross-line span extends past the first newline; take only
                    // its first physical line's text, then stop.
                    let text = n.text().to_string();
                    s.push_str(text.split('\n').next().unwrap_or(&text));
                    break;
                }
                NodeOrToken::Node(n) => s.push_str(&n.text().to_string()),
            },
            _ => {}
        }
    }
    s.trim().to_string()
}

/// The tag's prose content (everything after the header) concatenated verbatim
/// and trimmed — used for non-reflowed passthrough tags.
fn tag_rest_verbatim(tag: &RoxygenTag) -> String {
    let mut s = String::new();
    for el in tag.syntax().children_with_tokens() {
        if is_tag_prose_kind(el.kind()) {
            match el {
                NodeOrToken::Token(t) => s.push_str(t.text()),
                NodeOrToken::Node(n) => s.push_str(&n.text().to_string()),
            }
        }
    }
    s.trim().to_string()
}

/// Append the tag's prose content as breakable chunks (the same text/protected-
/// span treatment as plain prose), descending past the `@`, name, and arg. A tag
/// with a same-line value folds its plain-prose continuation lines in (see
/// `emit_tag_line`), so the threaded inter-line newlines are kept as break
/// opportunities (`chunk_elements` treats a newline as a break and drops the
/// continuation markers), letting the whole field value reflow as one run.
fn tag_prose_chunks(tag: &RoxygenTag, out: &mut Vec<String>) {
    let prose = tag
        .syntax()
        .children_with_tokens()
        .filter(|el| is_tag_prose_kind(el.kind()) || el.kind() == SyntaxKind::NEWLINE);
    chunk_elements(prose, out);
}

/// Whether `node` is a `ROXYGEN_TAG` that folded plain-prose continuation lines
/// into its value (see `emit_tag_line`) — it threads one or more `#'` markers, so
/// it spans multiple physical lines. A single-line tag never contains a marker.
fn is_multiline_tag(node: &SyntaxNode) -> bool {
    node.kind() == SyntaxKind::ROXYGEN_TAG
        && node
            .descendants_with_tokens()
            .any(|el| el.kind() == SyntaxKind::ROXYGEN_MARKER)
}

/// Whether `line` carries a folded multi-line tag with a **structured**
/// continuation segment. A prose tag's same-line value folds its contiguous
/// plain-prose continuations into the `ROXYGEN_TAG` node, and outside `@md`
/// mode a list item (or other structured-looking line) lexes as plain prose,
/// so it can hide inside the fold. A sibling structured line is kept verbatim
/// by the block loop's [`is_structured`] guard; a folded one must take the
/// same conservative bail — reflowing it would join the marker into prose that
/// the sibling (next-line) shape preserves, so the two shapes would disagree
/// and formatting would not reach a fixed point.
fn tag_folds_structured_line(line: &PhysicalLine) -> bool {
    let Some(tag) = line.tag() else {
        return false;
    };
    if !is_multiline_tag(tag.syntax()) {
        return false;
    }
    // Continuation segments each start with their own `#+'` marker (the fold
    // threads it into the tag node); strip it to classify the content, which
    // continues the tag's open prose (`in_paragraph`).
    tag.syntax().text().to_string().lines().skip(1).any(|seg| {
        let s = seg.trim();
        let hashes = s.len() - s.trim_start_matches('#').len();
        let content = match s[hashes..].strip_prefix('\'') {
            Some(rest) if hashes > 0 => rest.trim(),
            _ => s,
        };
        is_structured(content, true)
    })
}

/// Append `line` as an IR text node, preceded by a hard line break unless it is
/// the first emitted line.
fn push_line(items: &mut Vec<Ir>, line: String) {
    if !items.is_empty() {
        items.push(Ir::hard_line());
    }
    items.push(Ir::text(line));
}

/// Whether `content` (a line's trimmed content) opens a fenced code block.
fn is_fence_marker(content: &str) -> bool {
    content.starts_with("```") || content.starts_with("~~~")
}

/// Whether `content` (a line's trimmed content) is a structured line that must
/// not be reflowed: a list item, blockquote, ATX header, table delimiter row,
/// or fence. `in_paragraph` is whether this line continues open prose (a
/// paragraph or a tag unit), which gates ordered-list recognition (see
/// `starts_ordered_list_item`).
///
/// A line that merely *contains* a `|` (an R pipe `|>`, an `||`, prose like
/// `this | that`) is ordinary prose: GFM recognizes a table only on a
/// header/delimiter row pair, and the parser owns that recognition (a matched
/// table arrives as a `ROXYGEN_MD_TABLE` block macro, handled before this
/// check; the parser also folds pipe-bearing continuations into a tag's value,
/// where they reflow). Only a *delimiter row* — the linchpin of table
/// formation — is kept structured, so reflow never joins one into prose where
/// a new header/delimiter pairing could form on reparse.
fn is_structured(content: &str, in_paragraph: bool) -> bool {
    content.starts_with("- ")
        || content.starts_with("* ")
        || content.starts_with("+ ")
        || content.starts_with("> ")
        || content.starts_with('#')
        || is_fence_marker(content)
        || is_table_delim_row(content)
        || starts_ordered_list_item(content, in_paragraph)
}

/// Whether `content` opens an ordered-list item that markdown would honor in
/// this position. The marker must be followed by a space; and — per CommonMark —
/// an ordered list whose start number is not 1 *cannot interrupt a paragraph*,
/// so a non-`1` marker only opens a list when it is not continuing open prose
/// (`in_paragraph` is false). This keeps a year like `2008.` mid-sentence from
/// being mistaken for a list (the common false positive).
fn starts_ordered_list_item(content: &str, in_paragraph: bool) -> bool {
    match ordered_marker(content) {
        Some((n, len)) if content.as_bytes().get(len) == Some(&b' ') => !in_paragraph || n == 1,
        _ => false,
    }
}

/// If `s` begins with an ordered-list marker — a run of ASCII digits (CommonMark
/// caps it at nine) followed by `.` or `)` — return the start number and the
/// marker's byte length (digits + delimiter). `None` otherwise.
fn ordered_marker(s: &str) -> Option<(u64, usize)> {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 9 {
        return None;
    }
    match s.as_bytes().get(digits) {
        Some(b'.' | b')') => Some((s[..digits].parse().ok()?, digits + 1)),
        _ => None,
    }
}

/// Whether `line`'s content carries a live Rd comment: an unescaped `%`, which in
/// literal (non-markdown) Rd prose begins a comment running to end of line
/// (parse_Rd). Reflowing such a line — joining it with following text, or moving
/// words past the `%` — changes which text the comment swallows and so alters the
/// rendered Rd, so a non-markdown paragraph or tag unit containing one is kept
/// verbatim. The escape rule mirrors the projector's `strip_rd_line_comment`
/// (`\%` survives, `\\%` does not).
fn line_has_live_rd_comment(line: &PhysicalLine) -> bool {
    let mut escaped = false;
    for c in content_text(line).chars() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '%' {
            return true;
        }
    }
    false
}

/// Whether `line`'s content carries an active `@md` `%`-swallow: a `%` whose
/// preceding maximal backslash run has **odd** length. Under `@md`, roxygen2's
/// markdown→Rd pass escapes a rendered `%` to `\%`; when the markdown already
/// places a literal backslash before the `%`, that escaping backslash collides
/// with the literal one and leaves a bare `%` that comments to end of the physical
/// line (parity-keyed on the backslash-run length). Reflowing such a line — moving
/// the continuation past the `%` — changes what the comment swallows and so alters
/// the rendered Rd, so the markdown paragraph or tag unit containing one is kept
/// verbatim, mirroring the non-markdown [`line_has_live_rd_comment`] gate. Mirrors
/// the projector's `md_percent_swallow_line`.
fn line_has_md_percent_swallow(line: &PhysicalLine) -> bool {
    let mut backslashes = 0usize;
    for c in content_text(line).chars() {
        if c == '\\' {
            backslashes += 1;
        } else {
            if c == '%' && backslashes % 2 == 1 {
                return true;
            }
            backslashes = 0;
        }
    }
    false
}

/// Whether `line`'s content carries a brace-less sticky code/verbatim Rd-macro
/// trigger (`\code z`/`\verb z`/…). Out of an argument context such a macro drops
/// parse_Rd into R-code/verbatim mode, so every *physical source line* from there
/// to section end becomes its own `RCODE`/`VERB` atom (the `arity` crate
/// projector's `roxygen::project_rd::split_sticky_braceless_swallow`). Reflowing —
/// joining a soft-wrapped continuation, or splitting an overlong line — changes the
/// atom count and thus the rendered Rd (Tenet 1), so a paragraph or tag unit
/// containing one is kept verbatim, both markdown modes (the swallow is
/// mode-independent). The scan mirrors the projector's `find_sticky_trigger`: an
/// odd-length backslash run opening a brace-less sticky name. Conservative — it
/// fires even where the projector then withholds the swallow (an impure tail),
/// which only forgoes a reflow and never changes the rendered Rd.
fn line_has_sticky_swallow(line: &PhysicalLine) -> bool {
    let text = content_text(line);
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            i += 1;
            continue;
        }
        let run_start = i;
        while i < bytes.len() && bytes[i] == b'\\' {
            i += 1;
        }
        if (i - run_start) % 2 == 1 {
            let name_end = crate::parser::roxygen::rd_macro_name_end(bytes, i);
            if name_end > i
                && bytes.get(name_end) != Some(&b'{')
                && crate::parser::roxygen::sticky_braceless_code_mode(&text[i..name_end]).is_some()
            {
                return true;
            }
        }
    }
    false
}

/// Whether `text` (a paragraph's or tag value's first line) *opens* a CommonMark
/// link-reference definition — a leaf block cmark consumes, rendering nothing
/// while giving every referencing link a destination. A definition may span
/// lines (the label, destination, and title can each sit on their own line), and
/// joining its lines during reflow can both *destroy* one (following prose lands
/// after the destination as junk, cm-210's tail) and *create* one (a next-line
/// `b>` completes an unclosed `<a` angle destination), either way changing the
/// rendered Rd (Tenet 1) — so a markdown paragraph whose first line opens a
/// possible definition is kept verbatim. The check is deliberately a superset of
/// "is a complete definition": any `[label]:` head bails, whatever follows, and
/// so does a cross-line label opener (a `[` whose label does not close on this
/// line, cm-210). Over-bailing only forgoes reflow — verbatim output is always
/// render-preserving. A definition is recognized only at a block start, which is
/// why the caller checks the paragraph's *first* line; a later definition-shaped
/// line is a paragraph continuation (it cannot interrupt), and joining it is
/// render-preserving. The projector's `parse_linkref_def_tail` holds the full
/// grammar.
fn text_opens_linkref_def(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('[') else {
        return false;
    };
    match rest.find(']') {
        // Cross-line label opener: the label does not close on this line, so a
        // multi-line definition may begin here. Mirrors the lexer's bracket-free
        // restriction (a second `[` is not a definition label).
        None => !rest.contains('['),
        // A `[label]:` head (label bracket-free and non-blank, colon immediately
        // after the `]`): a definition or a possible multi-line one — bail either
        // way, whatever the destination/title tail holds.
        Some(close) => {
            !rest[..close].trim().is_empty()
                && !rest[..close].contains('[')
                && rest[close + 1..].starts_with(':')
        }
    }
}

/// The block's resolved markdown mode (a standalone `@md`/`@noMd` directive line,
/// last one wins, default off), mirroring the lexer's `resolve_roxygen_block` and
/// the projector's `block_md`. Plain prose leaves carry no mode in their kind, so
/// the formatter re-derives it here to decide whether an unescaped `%` is a live
/// Rd comment (non-markdown) or escaped literal text (markdown).
fn block_md(node: &SyntaxNode) -> bool {
    let Some(block) = RoxygenBlock::cast(node.clone()) else {
        return false;
    };
    let mut md = false;
    for section in block.sections() {
        if let Some(tag) = section.tag()
            && tag.arg().is_none()
            && tag.text().is_none()
        {
            match tag.name().as_deref() {
                Some("md") => md = true,
                Some("noMd") => md = false,
                _ => {}
            }
        }
    }
    md
}

/// Whether `text`, emitted straight after a marker and a single space, would
/// reparse as a roxygen **tag**: an `@` followed by a letter.
///
/// roxygen2 consumes at most one whitespace character after the `#'` marker
/// before demanding the `@`, so prose that merely begins that way (`#'  @param x`,
/// written with a wide separator, is description text) survives only while the
/// separator stays wide. Collapsing it to one space would promote the prose to a
/// real tag and change the rendered Rd — a behavior-preservation bug, not a
/// layout choice. Mirrors the parser's `MAX_TAG_SEPARATOR_WS` rule; `@@` (the
/// escape) is not a tag and needs no widening.
fn reparses_as_tag(text: &str) -> bool {
    text.strip_prefix('@')
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_alphabetic()))
}

/// Push one prose line: `marker`, the separator, then `text`. The separator is
/// the conventional single space, widened to two exactly when `text` would
/// otherwise reparse as a tag (see [`reparses_as_tag`]).
fn push_prose_line(items: &mut Vec<Ir>, marker: &str, text: &str) {
    let sep = if reparses_as_tag(text) { "  " } else { " " };
    push_line(items, format!("{marker}{sep}{text}"));
}

/// Whether a chunk placed at the start of a wrapped line could reparse as a
/// structured construct, which would make reflow non-idempotent. Conservative:
/// such a paragraph is kept verbatim rather than risk a migrating marker.
fn is_unsafe_line_start(chunk: &str) -> bool {
    matches!(chunk, "-" | "*" | "+" | ">")
        || chunk.starts_with('#')
        || chunk.starts_with("```")
        || chunk.starts_with("~~~")
        || is_unsafe_ordered_marker(chunk)
        // An HTML block opener (conditions 1–6) interrupts a paragraph, so an
        // inline HTML span (or prose that merely looks like an opener, e.g. an
        // unterminated `<!--`) migrating to a line start would reparse as a
        // block and change the rendered Rd.
        || starts_md_html_block(chunk)
}

/// Whether `chunk` is a bare ordered-list marker that would interrupt a
/// paragraph if it migrated to a continuation-line start. A migrated chunk always
/// lands mid-paragraph, where (per CommonMark) only a `1.`/`1)` marker opens a
/// list; a higher start number is inert there and safe to move. Mirrors the
/// `n == 1` gate in `starts_ordered_list_item` so the guard and the reparse
/// classifier agree.
fn is_unsafe_ordered_marker(chunk: &str) -> bool {
    matches!(ordered_marker(chunk), Some((1, len)) if len == chunk.len())
}

/// Normalize one `#'` line: the marker verbatim, then a single space before the
/// content (a tag node or prose tokens), with trailing whitespace trimmed. A
/// blank line (marker only, or marker followed by whitespace) yields just the
/// marker.
///
/// Only the whitespace directly between the marker and the content is touched;
/// tag-internal spacing lives inside the `ROXYGEN_TAG` node and is preserved
/// verbatim (its normalization is a later transform).
fn normalize_roxygen_line(line: &PhysicalLine) -> String {
    let marker = marker_text(line);
    // A line inside a leaky cross-line link keeps everything after the marker
    // byte-verbatim: the marker→content whitespace past the separator space and
    // any trailing whitespace before the soft break are *label content*, and the
    // label's exact bytes URL-encode into the leaked `R:` destination (see
    // [`line_has_leaky_cross_line_link`]).
    if line_has_leaky_cross_line_link(line) {
        let mut content = String::new();
        for el in &line.elements {
            match el {
                NodeOrToken::Token(t) => content.push_str(t.text()),
                NodeOrToken::Node(n) => content.push_str(&n.text().to_string()),
            }
        }
        return format!("{marker}{content}");
    }
    let mut content = String::new();
    for el in content_elements(line) {
        match el {
            NodeOrToken::Token(t) => content.push_str(t.text()),
            NodeOrToken::Node(n) => content.push_str(&n.text().to_string()),
        }
    }
    let content = content.trim_end();
    if content.is_empty() {
        return marker;
    }
    // Prose that *looks* like a tag keeps a widened separator so it stays prose;
    // a line whose content really is a `ROXYGEN_TAG` gets the conventional single
    // space (see [`reparses_as_tag`]).
    if line.tag().is_none() && reparses_as_tag(content) {
        return format!("{marker}  {content}");
    }
    format!("{marker} {content}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text_opens_linkref_def` recognizes every line shape that can open a
    /// (possibly multi-line) link-reference definition, so the reflow bail keeps
    /// such a paragraph verbatim — joining could destroy or create a definition,
    /// changing the rendered Rd.
    #[test]
    fn linkref_def_detection() {
        // Complete same-line definitions: bare, angle-bracketed, and titled.
        assert!(text_opens_linkref_def("[foo]: https://example.com"));
        assert!(text_opens_linkref_def("[foo]: <https://example.com>"));
        assert!(text_opens_linkref_def(
            "[foo]: https://example.com \"a title\""
        ));
        assert!(text_opens_linkref_def(
            "[foo]: https://example.com 'a title'"
        ));
        assert!(text_opens_linkref_def(
            "[foo]: https://example.com (a title)"
        ));

        // Multi-line openers: a bare `[label]:` head (destination on the next
        // line), an unclosed angle destination (a next-line `b>` would complete
        // it), a messy tail (junk now, but the join outcome is unknowable
        // line-locally — conservative bail), and a cross-line label opener.
        assert!(text_opens_linkref_def("[foo]:"));
        assert!(text_opens_linkref_def("[foo]: <https://example"));
        assert!(text_opens_linkref_def(
            "[foo]: https://example.com trailing junk"
        ));
        assert!(text_opens_linkref_def("["));
        assert!(text_opens_linkref_def("[foo"));

        // Cannot open a definition: no colon after the label, a blank or
        // bracket-bearing label, or no bracket at all (a join is
        // render-preserving, so the bail must not fire).
        assert!(!text_opens_linkref_def("[foo] https://example.com"));
        assert!(!text_opens_linkref_def("[]: https://example.com"));
        assert!(!text_opens_linkref_def("just some prose"));
        assert!(!text_opens_linkref_def("[foo] and [bar] in prose."));
        assert!(!text_opens_linkref_def("[a[b"));
        assert!(!text_opens_linkref_def("[a[b]: /url"));
    }
}
