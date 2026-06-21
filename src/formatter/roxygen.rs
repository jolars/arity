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
use crate::ast::{AstNode, RoxygenLine, RoxygenTag};
use crate::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

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

    for line in node.children().filter_map(RoxygenLine::cast) {
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
        // (`@examples` body lines are captured at the top of the loop.)
        if line.is_blank() || is_structured(&content) {
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
    lines: Vec<RoxygenLine>,
}

impl TagUnit {
    fn new(line: &RoxygenLine, tag: &RoxygenTag, indent_cols: usize) -> Self {
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
    fn push_continuation(&mut self, line: &RoxygenLine) {
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
    lines: Vec<RoxygenLine>,
}

impl ExampleBody {
    fn push_line(&mut self, line: &RoxygenLine) {
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

        let source = lines
            .iter()
            .map(content_text)
            .collect::<Vec<_>>()
            .join("\n");

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
        let body_style = FormatStyle {
            line_width: budget,
            indent_width: style.indent_width,
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
                for line in &lines {
                    emit_normalized(items, line);
                }
            }
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
fn line_chunks(line: &RoxygenLine, out: &mut Vec<String>) {
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

/// Emit a tag line that is not reflowed (a code/example body, a structured
/// `@section Title:` heading, a namespace directive, or a bare tag) with its
/// internal spacing normalized: marker, header (`@tag [arg]`, single-spaced),
/// then the remaining content verbatim. Falls back to plain marker
/// normalization if the tag has no name (malformed).
fn emit_tag_passthrough(items: &mut Vec<Ir>, line: &RoxygenLine, tag: &RoxygenTag) {
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
        .any(|el| el.as_token().is_some_and(|t| is_tag_prose_kind(t.kind())))
}

/// Whether `kind` is a roxygen prose leaf (plain text or a protected span).
fn is_tag_prose_kind(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::ROXYGEN_TEXT
            | SyntaxKind::ROXYGEN_CODE
            | SyntaxKind::ROXYGEN_RD_MACRO
            | SyntaxKind::ROXYGEN_MD_LINK
    )
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
        if let NodeOrToken::Token(t) = el
            && is_tag_prose_kind(t.kind())
        {
            s.push_str(t.text());
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
        .filter(|el| el.as_token().is_some_and(|t| is_tag_prose_kind(t.kind())));
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
