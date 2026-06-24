//! Roxygen block formatting: marker normalization (transform 1), prose reflow
//! (transform 2), tag-prose hanging-indent reflow (transform 3), and embedded-R
//! formatting in `@examples`/`@examplesIf` bodies (transform 4).
//!
//! A `ROXYGEN_BLOCK` is emitted one `#'` line per output line. Consecutive plain
//! prose lines are grouped into a paragraph and greedily re-wrapped to the line
//! width, with protected markup spans (inline code, Rd macros, markdown links)
//! kept atomic. A tag line *with inline prose* (e.g. `@param x <prose>`) plus the
//! plain-prose lines that follow it form a single reflow unit: the tag header
//! stays on the first line and continuation lines hang-indent two extra spaces
//! under it (the tidyverse style), with internal tag spacing normalized.
//!
//! An `@examples`/`@examplesIf` body is treated as embedded R: the body lines are
//! collected, stripped of their markers, run through arity's own formatter, and
//! re-prefixed (transform 4). If the body does not parse cleanly (e.g. it wraps R
//! in Rd macros like `\dontrun{}`, which are not valid R), the whole body falls
//! back to marker-normalized passthrough, byte-for-byte. Other non-prose tag
//! content (`@usage`/`@eval`/`@evalRd` code, `@section Title:` headings, and
//! namespace directives), blank separators, fenced code blocks, and other
//! structured lines (lists, tables, headers, blockquotes) are passed through
//! marker-normalized but never reflowed — the conservative gate that keeps reflow
//! correct without a full Markdown parse.

use rowan::NodeOrToken;

use super::context::FormatContext;
use super::core::format_with_style;
use super::ir::Ir;
use super::style::FormatStyle;
use crate::ast::{AstNode, RoxygenTag};
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
            | SyntaxKind::ROXYGEN_MD_HTML_BLOCK => {
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
                if cur.marker.is_some() {
                    lines.push(std::mem::take(&mut cur));
                }
            }
            // Trivia before this line's marker (continuation indentation) is not
            // part of any line.
            _ if cur.marker.is_none() => {}
            SyntaxKind::ROXYGEN_TAG => {
                cur.tag = el.as_node().cloned().and_then(RoxygenTag::cast);
                cur.elements.push(el);
            }
            _ => cur.elements.push(el),
        }
    }
    if cur.marker.is_some() {
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
/// lines, each on its own output line. The node's text already carries the `#'`
/// markers (the opening one and the continuations) and the in-macro indentation;
/// only the inter-line indentation *before* a continuation marker is dropped (the
/// formatter recomputes the block's own indent), matching the fenced-code and
/// air-compatible verbatim treatment of Rd lists.
fn emit_block_macro(items: &mut Vec<Ir>, node: &SyntaxNode) {
    let text = node.text().to_string();
    for (i, seg) in text.split('\n').enumerate() {
        let line = if i == 0 { seg } else { seg.trim_start() };
        push_line(items, line.trim_end().to_string());
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

/// Emit a markdown list (`@md` mode) as atomic passthrough, each `#'` line
/// marker-normalized (marker, one space, trimmed content). The node owns its own
/// `#'` markers and newlines; only the inter-line indentation is dropped. (A
/// canonical re-indent that models nesting is future work — see `emit_block_macro`,
/// which preserves in-macro indentation for Rd lists.)
fn emit_md_list(items: &mut Vec<Ir>, node: &SyntaxNode) {
    for seg in node.text().to_string().split('\n') {
        push_line(items, normalize_marker_text(seg));
    }
}

/// Emit a markdown fenced code block (`@md` mode) as atomic passthrough, each
/// `#'` line marker-normalized (marker, one space, trimmed content). Mirrors the
/// pre-node textual fence path (`is_fence`/`emit_normalized`): the fence lines
/// and verbatim code lines are emitted as-is, never reflowed. (Code indentation
/// beyond the marker is dropped, matching that prior behavior; a canonical
/// re-indent is future work, as for the Rd-list and markdown-list passthroughs.)
fn emit_md_code_block(items: &mut Vec<Ir>, node: &SyntaxNode) {
    for seg in node.text().to_string().split('\n') {
        push_line(items, normalize_marker_text(seg));
    }
}

/// Emit a markdown HTML block (`@md` mode) as atomic passthrough, each `#'` line
/// marker-normalized (marker, one space, trimmed content). The node owns its own
/// `#'` markers and newlines; the raw HTML is never reflowed across lines.
fn emit_md_html_block(items: &mut Vec<Ir>, node: &SyntaxNode) {
    for seg in node.text().to_string().split('\n') {
        push_line(items, normalize_marker_text(seg));
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
            other => out.push(other),
        }
    }
}

/// Build the IR for a `ROXYGEN_BLOCK` at the given nesting `indent`.
pub(super) fn ir_roxygen_block(node: &SyntaxNode, indent: usize, ctx: FormatContext) -> Ir {
    let style = ctx.style();
    let indent_cols = indent * style.indent_width;

    let mut items: Vec<Ir> = Vec::new();
    let mut para = Paragraph::default();
    let mut tag_unit: Option<TagUnit> = None;
    let mut example = ExampleBody::default();
    let mut in_examples = false;
    let mut in_fence = false;
    let lw = style.line_width;

    // Flush all pending accumulators (only one is ever non-empty at a time).
    macro_rules! flush_pending {
        () => {{
            para.flush(&mut items, indent_cols, lw);
            flush_tag_unit(&mut tag_unit, &mut items, lw);
            example.flush(&mut items, indent_cols, style);
        }};
    }

    for line in physical_lines(node) {
        // A block Rd macro is atomic passthrough: flush pending accumulators, then
        // emit it without reflowing. In a prose section its in-macro indentation is
        // preserved (a `\itemize` list); inside an `@examples` body it wraps
        // example R (`\dontrun{}`/`\donttest{}`/…), which is meant to be
        // copy-pasted, so it is emitted *flush* (marker-normalized, no extra
        // indent). `in_examples` stays set — more example lines may follow.
        if let Some(macro_node) = &line.block_macro {
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
            } else if macro_node.kind() == SyntaxKind::ROXYGEN_MD_HTML_BLOCK {
                // An HTML block is atomic passthrough (verbatim raw HTML), each
                // line marker-normalized — never reflowed across lines.
                emit_md_html_block(&mut items, macro_node);
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
        // themselves) is passthrough; a fence marker toggles the state.
        if in_fence {
            if is_fence {
                in_fence = false;
            }
            flush_pending!();
            emit_normalized(&mut items, &line);
            continue;
        }
        if is_fence {
            in_fence = true;
            flush_pending!();
            emit_normalized(&mut items, &line);
            continue;
        }

        // Tag line: a paragraph/tag-unit boundary; (re)arm the `@examples`
        // passthrough.
        if let Some(tag) = line.tag() {
            in_examples = tag.is_examples();
            flush_pending!();
            if in_examples || is_non_prose_tag(&tag) || !tag_has_prose(&tag) {
                // Code/example body, structured (`@section Title:`) or namespace
                // directive, or a bare tag: passthrough, internal spacing
                // normalized.
                emit_tag_passthrough(&mut items, &line, &tag);
            } else {
                // `@tag [arg] <prose>`: open a reflow unit that absorbs the
                // following continuation prose lines.
                tag_unit = Some(TagUnit::new(&line, &tag, indent_cols));
            }
            continue;
        }

        // Blank separator or a structured line: passthrough, and a boundary.
        // (`@examples` body lines are captured at the top of the loop.) A line
        // continues open prose when a tag unit or paragraph is mid-flight; that
        // gates ordered-list recognition (a non-`1` marker can't interrupt it).
        let in_paragraph = tag_unit.is_some() || !para.lines.is_empty();
        if line.is_blank() || is_structured(&content, in_paragraph) {
            flush_pending!();
            emit_normalized(&mut items, &line);
            continue;
        }

        // Plain prose. A marker change (e.g. `#'` then `##'`) starts fresh.
        let marker = marker_text(&line);

        // Continuation of an open tag unit (same marker): absorb and hang-indent.
        if let Some(unit) = tag_unit.as_mut() {
            if unit.marker == marker {
                unit.push_continuation(&line);
                continue;
            }
            flush_tag_unit(&mut tag_unit, &mut items, lw);
        }

        // Otherwise accumulate into the current plain-prose paragraph.
        if para.marker.as_deref().is_some_and(|m| m != marker) {
            para.flush(&mut items, indent_cols, lw);
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
    fn flush(&mut self, items: &mut Vec<Ir>, indent_cols: usize, line_width: usize) {
        if self.lines.is_empty() {
            return;
        }
        // Reflow only when no chunk could migrate to a line start and reparse as
        // a structured construct (which would break idempotence); otherwise keep
        // the original line breaks, marker-normalized.
        if self.chunks.is_empty() || self.chunks.iter().any(|c| is_unsafe_line_start(c)) {
            let lines = std::mem::take(&mut self.lines);
            for line in &lines {
                emit_normalized(items, line);
            }
        } else {
            let marker = self.marker.clone().unwrap_or_else(|| "#'".to_string());
            let prefix = indent_cols + marker.chars().count() + 1;
            let budget = line_width.saturating_sub(prefix).max(1);
            for wrapped in wrap_chunks(&self.chunks, budget) {
                push_line(items, format!("{marker} {wrapped}"));
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
        }
    }

    /// Absorb a following plain-prose line as continuation text.
    fn push_continuation(&mut self, line: &PhysicalLine) {
        line_chunks(line, &mut self.chunks);
        self.lines.push(line.clone());
    }

    /// Emit the reflowed tag unit into `items`.
    fn flush(self, items: &mut Vec<Ir>, line_width: usize) {
        let marker_w = self.marker.chars().count();
        // A prose chunk that could migrate to a continuation-line start and
        // reparse as a list/header marker would break idempotence: bail to a
        // verbatim, marker-normalized rendering of the source lines instead.
        if self.chunks.iter().any(|c| is_unsafe_line_start(c)) {
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
fn flush_tag_unit(unit: &mut Option<TagUnit>, items: &mut Vec<Ir>, line_width: usize) {
    if let Some(unit) = unit.take() {
        unit.flush(items, line_width);
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
            // header-only and start it on a continuation line.
            if lines.is_empty()
                && budget == first_budget
                && w > first_budget
                && first_budget < cont_budget
            {
                lines.push(String::new());
                budget = cont_budget;
            }
            cur.push_str(chunk);
            cur_w = w;
        } else if cur_w + 1 + w <= budget {
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
        } else if cur_w + 1 + w <= budget {
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
    for el in elements {
        match el {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::ROXYGEN_TEXT => {
                for ch in t.text().chars() {
                    if ch.is_whitespace() {
                        if !cur.is_empty() {
                            out.push(std::mem::take(&mut cur));
                        }
                    } else {
                        cur.push(ch);
                    }
                }
            }
            // Protected span (or any other content token/node): glue it in.
            NodeOrToken::Token(t) => cur.push_str(t.text()),
            NodeOrToken::Node(n) => cur.push_str(&n.text().to_string()),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
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
    let rest = tag_rest_verbatim(tag);
    if rest.is_empty() {
        push_line(items, format!("{marker} {header}"));
    } else {
        push_line(items, format!("{marker} {header} {rest}"));
    }
}

/// Roxygen tags whose inline content is *not* hanging-indent prose, so it must
/// not be reflowed: embedded R (`usage`/`eval`/`evalRd`; `examples` is handled
/// separately), the `@section Title:` heading shape, and namespace/identifier
/// directives whose content is symbols rather than prose. Conservative and
/// extensible — reflowing an omitted identifier tag stays correct (it parses and
/// is idempotent), just not ideal.
const NON_PROSE_TAGS: &[&str] = &[
    "usage",
    "eval",
    "evalRd",
    "evalNamespace",
    "section",
    "export",
    "exportClass",
    "exportMethod",
    "exportS3Method",
    "exportPattern",
    "import",
    "importFrom",
    "importClassesFrom",
    "importMethodsFrom",
    "rawNamespace",
    "useDynLib",
    "rdname",
    "name",
    "aliases",
    "keywords",
    "family",
    "concept",
    "docType",
    "encoding",
    "backref",
];

/// Whether `tag`'s inline content should be passed through rather than reflowed.
fn is_non_prose_tag(tag: &RoxygenTag) -> bool {
    tag.name()
        .as_deref()
        .is_some_and(|n| NON_PROSE_TAGS.contains(&n))
}

/// Whether the tag carries inline prose on its own line (a `ROXYGEN_TEXT` run or
/// a protected span after the header), as opposed to a bare tag like `@export`.
fn tag_has_prose(tag: &RoxygenTag) -> bool {
    tag.syntax()
        .children_with_tokens()
        .any(|el| is_tag_prose_kind(el.kind()))
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
/// span treatment as plain prose), descending past the `@`, name, and arg.
fn tag_prose_chunks(tag: &RoxygenTag, out: &mut Vec<String>) {
    let prose = tag
        .syntax()
        .children_with_tokens()
        .filter(|el| is_tag_prose_kind(el.kind()));
    chunk_elements(prose, out);
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
/// not be reflowed: a list item, blockquote, ATX header, table row, or fence.
/// `in_paragraph` is whether this line continues open prose (a paragraph or a
/// tag unit), which gates ordered-list recognition (see
/// `starts_ordered_list_item`).
fn is_structured(content: &str, in_paragraph: bool) -> bool {
    content.starts_with("- ")
        || content.starts_with("* ")
        || content.starts_with("+ ")
        || content.starts_with("> ")
        || content.starts_with('#')
        || is_fence_marker(content)
        || content.contains('|')
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

/// Whether a chunk placed at the start of a wrapped line could reparse as a
/// structured construct, which would make reflow non-idempotent. Conservative:
/// such a paragraph is kept verbatim rather than risk a migrating marker.
fn is_unsafe_line_start(chunk: &str) -> bool {
    matches!(chunk, "-" | "*" | "+" | ">")
        || chunk.starts_with('#')
        || chunk.starts_with("```")
        || chunk.starts_with("~~~")
        || is_unsafe_ordered_marker(chunk)
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
    let mut content = String::new();
    for el in content_elements(line) {
        match el {
            NodeOrToken::Token(t) => content.push_str(t.text()),
            NodeOrToken::Node(n) => content.push_str(&n.text().to_string()),
        }
    }
    let content = content.trim_end();
    if content.is_empty() {
        marker
    } else {
        format!("{marker} {content}")
    }
}
