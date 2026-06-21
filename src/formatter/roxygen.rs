//! Roxygen block formatting: marker normalization (transform 1) and prose
//! reflow (transform 2).
//!
//! A `ROXYGEN_BLOCK` is emitted one `#'` line per output line. Consecutive plain
//! prose lines are grouped into a paragraph and greedily re-wrapped to the line
//! width, with protected markup spans (inline code, Rd macros, markdown links)
//! kept atomic. Tag lines, blank separators, `@examples`/`@examplesIf` bodies,
//! fenced code blocks, and other structured lines (lists, tables, headers,
//! blockquotes) are passed through marker-normalized but never reflowed — the
//! conservative gate that keeps reflow correct without a full Markdown parse.

use rowan::NodeOrToken;

use super::context::FormatContext;
use super::ir::Ir;
use crate::ast::{AstNode, RoxygenLine};
use crate::syntax::{SyntaxKind, SyntaxNode};

/// Build the IR for a `ROXYGEN_BLOCK` at the given nesting `indent`.
pub(super) fn ir_roxygen_block(node: &SyntaxNode, indent: usize, ctx: FormatContext) -> Ir {
    let style = ctx.style();
    let indent_cols = indent * style.indent_width;

    let mut items: Vec<Ir> = Vec::new();
    let mut para = Paragraph::default();
    let mut in_examples = false;
    let mut in_fence = false;

    for line in node.children().filter_map(RoxygenLine::cast) {
        let content = content_text(&line);
        let is_fence = is_fence_marker(&content);

        // Fenced code block: everything between fences (and the fence lines
        // themselves) is passthrough; a fence marker toggles the state.
        if in_fence {
            if is_fence {
                in_fence = false;
            }
            para.flush(&mut items, indent_cols, style.line_width);
            emit_normalized(&mut items, &line);
            continue;
        }
        if is_fence {
            in_fence = true;
            para.flush(&mut items, indent_cols, style.line_width);
            emit_normalized(&mut items, &line);
            continue;
        }

        // Tag line: a paragraph boundary; (re)arm the `@examples` passthrough.
        if let Some(tag) = line.tag() {
            in_examples = tag.is_examples();
            para.flush(&mut items, indent_cols, style.line_width);
            emit_normalized(&mut items, &line);
            continue;
        }

        // Blank separator, an `@examples` body line, or a structured line:
        // passthrough, and a paragraph boundary.
        if line.is_blank() || in_examples || is_structured(&content) {
            para.flush(&mut items, indent_cols, style.line_width);
            emit_normalized(&mut items, &line);
            continue;
        }

        // Plain prose: accumulate into the current paragraph. A marker change
        // (e.g. `#'` then `##'`) starts a fresh paragraph.
        let marker = marker_text(&line);
        if para.marker.as_deref().is_some_and(|m| m != marker) {
            para.flush(&mut items, indent_cols, style.line_width);
        }
        if para.marker.is_none() {
            para.marker = Some(marker);
        }
        para.push_line(&line);
    }
    para.flush(&mut items, indent_cols, style.line_width);

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
    lines: Vec<RoxygenLine>,
}

impl Paragraph {
    fn push_line(&mut self, line: &RoxygenLine) {
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
fn line_chunks(line: &RoxygenLine, out: &mut Vec<String>) {
    let mut cur = String::new();
    for el in content_elements(line) {
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
/// marker→content whitespace (which the formatter drops).
fn content_elements(
    line: &RoxygenLine,
) -> impl Iterator<Item = NodeOrToken<SyntaxNode, crate::syntax::SyntaxToken>> + '_ {
    let mut seen_content = false;
    line.syntax()
        .children_with_tokens()
        .filter(move |el| match el.kind() {
            SyntaxKind::ROXYGEN_MARKER => false,
            SyntaxKind::WHITESPACE if !seen_content => false,
            _ => {
                seen_content = true;
                true
            }
        })
}

/// The trimmed text content of a line (everything after the marker), used for
/// structured-line classification.
fn content_text(line: &RoxygenLine) -> String {
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
fn marker_text(line: &RoxygenLine) -> String {
    line.marker()
        .map(|t| t.text().to_string())
        .unwrap_or_else(|| "#'".to_string())
}

/// Emit a line marker-normalized (transform 1): marker, a single space, the
/// content verbatim, trailing whitespace trimmed; a blank line is just the
/// marker. Boundary lines (tags, blanks, structured, fenced, examples) take
/// this path.
fn emit_normalized(items: &mut Vec<Ir>, line: &RoxygenLine) {
    push_line(items, normalize_roxygen_line(line.syntax()));
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
fn is_structured(content: &str) -> bool {
    content.starts_with("- ")
        || content.starts_with("* ")
        || content.starts_with("+ ")
        || content.starts_with("> ")
        || content.starts_with('#')
        || is_fence_marker(content)
        || content.contains('|')
        || is_ordered_list_marker(content)
}

/// Whether `content` begins with an ordered-list marker: digits then `.`/`)`
/// then a space (e.g. `1. ` or `12) `).
fn is_ordered_list_marker(content: &str) -> bool {
    let digits = content.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0
        && matches!(content.as_bytes().get(digits), Some(b'.' | b')'))
        && content.as_bytes().get(digits + 1) == Some(&b' ')
}

/// Whether a chunk placed at the start of a wrapped line could reparse as a
/// structured construct, which would make reflow non-idempotent. Conservative:
/// such a paragraph is kept verbatim rather than risk a migrating marker.
fn is_unsafe_line_start(chunk: &str) -> bool {
    matches!(chunk, "-" | "*" | "+" | ">")
        || chunk.starts_with('#')
        || chunk.starts_with("```")
        || chunk.starts_with("~~~")
        || is_bare_ordered_marker(chunk)
}

/// Whether `chunk` is a bare ordered-list marker (`1.`, `12)`): digits then a
/// single `.`/`)`.
fn is_bare_ordered_marker(chunk: &str) -> bool {
    let digits = chunk.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && digits + 1 == chunk.len() && matches!(chunk.as_bytes()[digits], b'.' | b')')
}

/// Normalize one `#'` line: the marker verbatim, then a single space before the
/// content (a tag node or prose tokens), with trailing whitespace trimmed. A
/// blank line (marker only, or marker followed by whitespace) yields just the
/// marker.
///
/// Only the whitespace directly between the marker and the content is touched;
/// tag-internal spacing lives inside the `ROXYGEN_TAG` node and is preserved
/// verbatim (its normalization is a later transform).
fn normalize_roxygen_line(line: &SyntaxNode) -> String {
    let mut marker = String::new();
    let mut content = String::new();
    for el in line.children_with_tokens() {
        match el {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::ROXYGEN_MARKER => {
                marker = t.text().to_string();
            }
            // The lone whitespace token sitting directly under the line, between
            // marker and content; drop it before any content has accumulated.
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::WHITESPACE && content.is_empty() => {}
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
