//! Roxygen block grouping: wrapping a maximal run of roxygen lines in a
//! `ROXYGEN_BLOCK` and laying out its logical structure as events.
//!
//! This is the *second* phase (`Vec<Token>` → `Vec<Event>`): it decides the
//! block's section/paragraph skeleton, classifies each line, threads the `#'`
//! markers and inter-line trivia in at the open level, and dispatches the
//! block-level Rd-macro / markdown constructs to [`super::build`].

use super::build::{
    block_macro_opener_closes, emit_block_macro, emit_block_macro_inline, emit_md_block_quote,
    emit_md_block_quote_from_value, emit_md_code_block, emit_md_code_block_from_value,
    emit_md_heading, emit_md_heading_from_value, emit_md_html_block, emit_md_html_block_from_value,
    emit_md_indented_code, emit_md_indented_code_from_value, emit_md_list, emit_md_list_from_value,
    emit_md_setext_heading, emit_md_table, emit_md_table_from_value, emit_md_thematic_break,
    emit_md_thematic_break_from_value, is_block_macro_line, is_block_macro_opener,
    is_md_block_quote_start, is_md_code_block_start, is_md_heading_start, is_md_html_block_start,
    is_md_html_block_value, is_md_html_block7_line, is_md_indented_code_start,
    is_md_indented_code_value, is_md_list_start, is_md_setext_heading_start,
    is_md_setext_underline_line, is_md_setext_underline_or_dash, is_md_table_start,
    is_md_table_value, is_md_thematic_break_line,
};
use crate::parser::events::Event;
use crate::parser::lexer::{RoxygenRole, TokKind, Token};
use crate::syntax::SyntaxKind;

/// Emit a `ROXYGEN_BLOCK` for the maximal run of consecutive roxygen lines
/// beginning at `start` (which must index a `RoxygenMarker`). Returns the token
/// index just past the block.
///
/// The block owns **logical content**, not physical lines: its children are
/// `ROXYGEN_SECTION` nodes (the intro prose, then one per `@tag`), and a
/// section's prose is grouped into `ROXYGEN_PARAGRAPH`s between blank-line
/// separators. The `#'` markers, the marker→content whitespace, and the
/// inter-line newlines are threaded in as trivia leaves at the byte positions
/// they occur (the way rowan/rust-analyzer trees attach whitespace), so
/// `reconstruct(text) == text` still holds. The `Newline` (plus any leading
/// `Whitespace`) between two roxygen lines is emitted *inside* the block at the
/// currently open level; the trailing `Newline` after the final line is left for
/// the caller, so blank-line and statement separation are unaffected.
pub(crate) fn emit_roxygen_block(tokens: &[Token], start: usize, events: &mut Vec<Event>) -> usize {
    // Build the block's events into a local buffer, run the `@md` inline pass over
    // it (resolving emphasis/strong delimiter runs and multi-line raw-HTML spans
    // into nodes), then splice the result into the caller's stream. The pass is a
    // no-op without delimiter runs or (under `@md`) a `<` in prose, so non-`@md`
    // (and unaffected) blocks stay byte-identical. The block's resolved markdown
    // mode is threaded in for the HTML resolution, whose `<`-in-prose candidate
    // has no mode-carrying leaf (the same re-derivation the indented-code
    // recognition uses).
    let mut block = Vec::new();
    let end = emit_roxygen_block_events(tokens, start, &mut block);
    super::inline::resolve_emphasis(tokens, &mut block, block_md(tokens, start));
    events.append(&mut block);
    end
}

/// The resolved `@md` mode of the block starting at `start`, re-derived from the
/// block's own tokens. Indented code blocks are the one construct with no
/// mode-carrying leaf (their content lexes as ordinary tokens — see
/// [`super::build::is_indent_code_line`]), so the block builder needs `@md` here to
/// decide whether a >= 5-column indent is a code block or literal Rd prose. This
/// mirrors the projector's own per-block `block_md`; it reuses the lexer's directive
/// matcher [`super::lex::roxygen_md_directive`] so what counts as `@md`/`@noMd`
/// stays a single source of truth. Default off, last directive in the block wins.
fn block_md(tokens: &[Token], start: usize) -> bool {
    let mut md = false;
    let mut i = start;
    loop {
        // Reconstruct the line text (marker + body tokens) and check for a directive.
        let mut line = String::from(&*tokens[i].text);
        let mut j = i + 1;
        while tokens.get(j).is_some_and(|t| is_line_body_kind(&t.kind)) {
            line.push_str(&tokens[j].text);
            j += 1;
        }
        if let Some(on) = super::lex::roxygen_md_directive(&line) {
            md = on;
        }
        // Continuation: `Newline`, optional continuation `Whitespace`, then a marker.
        if tokens.get(j).map(|t| &t.kind) != Some(&TokKind::Newline) {
            break;
        }
        let mut m = j + 1;
        while tokens.get(m).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
            m += 1;
        }
        if tokens.get(m).map(|t| &t.kind) == Some(&TokKind::RoxygenMarker) {
            i = m;
        } else {
            break;
        }
    }
    md
}

fn emit_roxygen_block_events(tokens: &[Token], start: usize, events: &mut Vec<Event>) -> usize {
    let md = block_md(tokens, start);
    debug_assert_eq!(tokens[start].kind, TokKind::RoxygenMarker);
    events.push(Event::Start(SyntaxKind::ROXYGEN_BLOCK));

    let mut i = start;
    let mut section_open = false;
    let mut para_open = false;

    loop {
        // `i` is at a `RoxygenMarker` (a logical line start).
        //
        // An indented code block (a >= 5-column `@md` indent with no open paragraph)
        // is a section-level construct that pre-empts the line's Tag/Prose
        // classification: at column 5 even an `@param` is code text, not a tag. The
        // interrupt rule (a code block cannot interrupt a paragraph) is the
        // `!para_open` gate inside `is_md_indented_code_start`, so a >= 5-column line
        // inside an open paragraph falls through and folds in as a lazy continuation.
        if is_md_indented_code_start(tokens, i, para_open, md) {
            if !section_open {
                events.push(Event::Start(SyntaxKind::ROXYGEN_SECTION));
                section_open = true;
            }
            i = emit_md_indented_code(tokens, i, events);
        } else {
            match classify_line(tokens, i) {
                LineKind::Tag => {
                    if para_open {
                        events.push(Event::Finish); // ROXYGEN_PARAGRAPH
                        para_open = false;
                    }
                    if section_open {
                        events.push(Event::Finish); // previous ROXYGEN_SECTION
                    }
                    events.push(Event::Start(SyntaxKind::ROXYGEN_SECTION));
                    section_open = true;
                    i = emit_tag_line(tokens, i, md, events);
                }
                LineKind::Blank => {
                    if para_open {
                        events.push(Event::Finish); // ROXYGEN_PARAGRAPH
                        para_open = false;
                    }
                    if !section_open {
                        events.push(Event::Start(SyntaxKind::ROXYGEN_SECTION));
                        section_open = true;
                    }
                    i = emit_line_tokens(tokens, i, events); // marker (+ trailing ws)
                }
                LineKind::Prose => {
                    if !section_open {
                        events.push(Event::Start(SyntaxKind::ROXYGEN_SECTION));
                        section_open = true;
                    }
                    if is_md_html_block_start(tokens, i) {
                        // A markdown HTML block (`@md` mode) is a direct section child,
                        // like a block macro: close any open paragraph and emit the
                        // HTML block as a sibling.
                        if para_open {
                            events.push(Event::Finish); // ROXYGEN_PARAGRAPH
                            para_open = false;
                        }
                        i = emit_md_html_block(tokens, i, events);
                    } else if !para_open && is_md_html_block7_line(tokens, i) {
                        // A complete standalone tag (HTML block start condition 7,
                        // `@md` mode) opens a blank-line-terminated HTML block — but
                        // only at a fresh position: condition 7 cannot interrupt a
                        // paragraph, so mid-paragraph the line falls through and
                        // folds in as ordinary prose (a lazy continuation).
                        i = emit_md_html_block(tokens, i, events);
                    } else if is_md_block_quote_start(tokens, i) {
                        // A markdown block quote (`> quoted`, `@md` mode) is a direct
                        // section child, like a block macro: it interrupts an open
                        // paragraph (CommonMark block quotes interrupt paragraphs).
                        if para_open {
                            events.push(Event::Finish); // ROXYGEN_PARAGRAPH
                            para_open = false;
                        }
                        i = emit_md_block_quote(tokens, i, events);
                    } else if is_md_code_block_start(tokens, i) {
                        // A markdown fenced code block (`@md` mode) is a direct
                        // section child, like a block macro: close any open paragraph
                        // and emit the code block as a sibling.
                        if para_open {
                            events.push(Event::Finish); // ROXYGEN_PARAGRAPH
                            para_open = false;
                        }
                        // Section level: the container content column is the
                        // single conventional space after the `#'` marker.
                        i = emit_md_code_block(tokens, i, 1, events);
                    } else if is_md_list_start(tokens, i, para_open) {
                        // A markdown list (`@md` mode) is a direct section child, like
                        // a block macro: close any open paragraph and build the list.
                        if para_open {
                            events.push(Event::Finish); // ROXYGEN_PARAGRAPH
                            para_open = false;
                        }
                        i = emit_md_list(tokens, i, events);
                    } else if is_block_macro_line(tokens, i) {
                        // A block Rd macro (`\itemize{ … }` across lines) is a direct
                        // section child, not paragraph prose: close any open paragraph
                        // and emit the macro as a sibling.
                        if para_open {
                            events.push(Event::Finish); // ROXYGEN_PARAGRAPH
                            para_open = false;
                        }
                        i = emit_block_macro(tokens, i, events);
                    } else if is_md_table_start(tokens, i) {
                        // A GFM table (header + delimiter row) is a direct section
                        // child, like a block macro; it interrupts an open paragraph.
                        if para_open {
                            events.push(Event::Finish); // ROXYGEN_PARAGRAPH
                            para_open = false;
                        }
                        i = emit_md_table(tokens, i, events);
                    } else if is_md_heading_start(tokens, i) {
                        // An ATX heading is a direct section child, like a block macro;
                        // it interrupts an open paragraph. The projector hoists it into
                        // an Rd `\section`/`\subsection`.
                        if para_open {
                            events.push(Event::Finish); // ROXYGEN_PARAGRAPH
                            para_open = false;
                        }
                        i = emit_md_heading(tokens, i, events);
                    } else if !para_open && is_md_setext_heading_start(tokens, i) {
                        // A setext heading: this prose line's paragraph is terminated by a
                        // `===`/`---` underline. Emit the whole paragraph + underline as a
                        // `ROXYGEN_MD_HEADING` (a direct section child, like ATX). Detected
                        // only at a fresh paragraph, so it captures the paragraph's full
                        // extent — the underline promotes every contiguous prose line above
                        // it, matching CommonMark.
                        i = emit_md_setext_heading(tokens, i, events);
                    } else if is_md_thematic_break_line(tokens, i) {
                        // A thematic break (`***`/`---`/`___`) is a direct section child,
                        // like a block quote: it interrupts an open paragraph (CommonMark).
                        // A promoting `---` is consumed as a setext heading above, so any
                        // dash-run reaching here heads nothing. roxygen2 renders a thematic
                        // break as empty; the projector drops the node so the surrounding
                        // paragraphs coalesce.
                        if para_open {
                            events.push(Event::Finish); // ROXYGEN_PARAGRAPH
                            para_open = false;
                        }
                        i = emit_md_thematic_break(tokens, i, events);
                    } else {
                        if !para_open {
                            events.push(Event::Start(SyntaxKind::ROXYGEN_PARAGRAPH));
                            para_open = true;
                        }
                        i = emit_prose_line(tokens, i, events);
                    }
                }
            }
        }

        // `i` is at the line's trailing `Newline` (or a non-roxygen token / EOF).
        // A continuation — one `Newline`, optional leading `Whitespace`, then
        // another `RoxygenMarker` — folds that separator into the block at the
        // currently open level (so a newline between two prose lines lands inside
        // the open paragraph). Otherwise the trailing `Newline` is the caller's.
        if tokens.get(i).map(|t| &t.kind) == Some(&TokKind::Newline) {
            let mut m = i + 1;
            while tokens.get(m).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
                m += 1;
            }
            if tokens.get(m).map(|t| &t.kind) == Some(&TokKind::RoxygenMarker) {
                for idx in i..m {
                    events.push(Event::Tok(idx));
                }
                i = m;
                continue;
            }
        }
        break;
    }

    if para_open {
        events.push(Event::Finish); // ROXYGEN_PARAGRAPH
    }
    if section_open {
        events.push(Event::Finish); // ROXYGEN_SECTION
    }
    events.push(Event::Finish); // ROXYGEN_BLOCK
    i
}

/// The logical kind of a roxygen line, decided from the first content token
/// after the `#'` marker and its trailing whitespace.
pub(super) enum LineKind {
    /// `@name …` — opens a new section.
    Tag,
    /// No prose content (marker only, or marker + whitespace) — a paragraph
    /// separator.
    Blank,
    /// Carries prose (text / inline code / Rd macro / markdown link).
    Prose,
}

/// Classify the roxygen line whose `RoxygenMarker` is at `start`.
pub(super) fn classify_line(tokens: &[Token], start: usize) -> LineKind {
    let content = line_content_start(tokens, start);
    let mut i = content;
    while let Some(tok) = tokens.get(i) {
        match tok.kind.roxygen_role() {
            Some(RoxygenRole::At) => return LineKind::Tag,
            Some(RoxygenRole::Content) => return LineKind::Prose,
            _ if tok.kind == TokKind::Whitespace => i += 1,
            _ => break,
        }
    }
    LineKind::Blank
}

/// Index of the first token after the marker at `marker` and the single
/// marker→content whitespace run.
pub(super) fn line_content_start(tokens: &[Token], marker: usize) -> usize {
    let mut i = marker + 1;
    while tokens.get(i).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
        i += 1;
    }
    i
}

/// Whether `kind` is a roxygen line-body token (everything that can follow the
/// marker on a line).
pub(super) fn is_line_body_kind(kind: &TokKind) -> bool {
    matches!(kind, TokKind::Whitespace)
        || matches!(kind.roxygen_role(), Some(role) if role != RoxygenRole::Marker)
}

/// Emit a line's tokens — marker then body — verbatim as `Tok` events. Returns
/// the index just past the line content (at the trailing `Newline` / non-roxygen
/// token / EOF). Used for prose and blank lines, whose tokens sit directly under
/// the open paragraph/section.
fn emit_line_tokens(tokens: &[Token], start: usize, events: &mut Vec<Event>) -> usize {
    events.push(Event::Tok(start)); // RoxygenMarker
    let mut i = start + 1;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }
    i
}

/// Emit a prose line's tokens into the open paragraph, promoting a **mid-prose**
/// block-macro opener (`\name{ …` that closes on a later `#'` line) to an inline
/// `ROXYGEN_RD_MACRO` sibling of the surrounding prose. A line-start opener is
/// handled earlier as a section-level block macro ([`is_block_macro_line`]); an
/// opener that never closes stays literal prose (the lexer split it into its own
/// `RoxygenText`, which is emitted unchanged here).
fn emit_prose_line(tokens: &[Token], start: usize, events: &mut Vec<Event>) -> usize {
    events.push(Event::Tok(start)); // RoxygenMarker
    let mut i = start + 1;
    while let Some(tok) = tokens.get(i) {
        if !is_line_body_kind(&tok.kind) {
            break;
        }
        if tok.kind == TokKind::RoxygenText
            && is_block_macro_opener(&tok.text)
            && block_macro_opener_closes(tokens, i)
        {
            // Consumes the opener and its body across following lines, then resumes
            // emitting any trailing prose on the macro's closing line.
            i = emit_block_macro_inline(tokens, i, events);
            continue;
        }
        events.push(Event::Tok(i));
        i += 1;
    }
    i
}

/// Emit a tag line: the marker and the marker→content whitespace sit directly
/// under the section, then a `ROXYGEN_TAG` node wraps the `@name [arg] <prose>`
/// content. Returns the index past the line content.
///
/// A tag with a *same-line prose value* folds its contiguous plain-prose
/// continuation lines into the `ROXYGEN_TAG` node, so the whole field value is
/// one logical run: roxygen2 treats a tag's value as spanning every line until
/// the next `@tag` or a blank line, so an `@md` emphasis/link span opened in the
/// value must resolve across the soft line break (the emphasis pass is bounded by
/// the tag node, so opener and closer have to live in the *same* node). A block
/// macro, markdown list, fenced/HTML block, blank line, or new tag ends the value
/// and stays a section-level sibling (its own paragraph/block).
fn emit_tag_line(tokens: &[Token], start: usize, md: bool, events: &mut Vec<Event>) -> usize {
    events.push(Event::Tok(start)); // RoxygenMarker
    let mut i = start + 1;
    while tokens.get(i).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
        events.push(Event::Tok(i)); // marker→content whitespace
        i += 1;
    }
    events.push(Event::Start(SyntaxKind::ROXYGEN_TAG));

    // Pre-scan this line's body: the tag name (for the prose-folding / setext
    // decisions) and the first Content (value) token — everything before it is the
    // tag *head* (`@`, name, an arg-bearing tag's `ROXYGEN_TAG_ARG`, and interior
    // whitespace).
    let mut tag_name: Option<&str> = None;
    let mut value_start: Option<usize> = None;
    let mut j = i;
    while tokens.get(j).is_some_and(|t| is_line_body_kind(&t.kind)) {
        let tok = &tokens[j];
        if tok.kind == TokKind::RoxygenTagName {
            tag_name = Some(&tok.text);
        }
        if tok.kind.roxygen_role() == Some(RoxygenRole::Content) {
            value_start = Some(j);
            break;
        }
        j += 1;
    }
    let has_value = value_start.is_some();
    let folds = has_value && tag_name.is_some_and(super::tag_folds_prose_continuation);

    // A prose tag's same-line value is the start of the tag's **markdown
    // document**, so a value that begins a CommonMark block opens that block
    // exactly as at line start. The value can't stay inside the `ROXYGEN_TAG`
    // (the block owns its lines), so the head closes the tag empty
    // ([`close_tag_at_value`]) and the value becomes a sibling block node with a
    // marker-less first line — the shape the next-line form already produces, so
    // the projector and formatter need no new section shape. Dispatch order
    // mirrors the block loop's precedence, with two forced choices: an indented
    // code value (>= 5 whitespace columns after the tag head — roxygen2 strips
    // only the single separator space) pre-empts everything (the lexer gated all
    // value carves on that indent, so its content lexes as ordinary tokens), and
    // every block check runs before the setext pre-scan (a fence/HTML block
    // swallows a following `===` underline as a verbatim line; a list item is
    // never setext-headed — engine-probed).
    if folds && is_md_indented_code_value(tokens, value_start.unwrap(), md) {
        let head_end = close_tag_at_value(tokens, i, value_start.unwrap(), events);
        return emit_md_indented_code_from_value(tokens, head_end, events);
    }
    // An HTML block from the value: a conditions-1–6 opener (a
    // `RoxygenMdHtmlBlock` leaf, carved by the lexer at the value of a prose
    // tag) or a condition-7 standalone tag (the value position is always fresh,
    // so the can't-interrupt rule never blocks it).
    if folds && is_md_html_block_value(tokens, value_start.unwrap()) {
        let head_end = close_tag_at_value(tokens, i, value_start.unwrap(), events);
        return emit_md_html_block_from_value(tokens, head_end, events);
    }
    // A fenced code block from the value (`@details ```r`): the opener fence
    // leaf, then the code lines to the closing fence.
    if folds && tokens[value_start.unwrap()].kind == TokKind::RoxygenMdFence {
        let head_end = close_tag_at_value(tokens, i, value_start.unwrap(), events);
        return emit_md_code_block_from_value(tokens, head_end, events);
    }
    // A markdown list from the value (`@details - item`): the value position is
    // a fresh block position, so any marker opens a list (the mid-paragraph
    // interrupt rule never applies); following list lines join as items.
    if folds && tokens[value_start.unwrap()].kind == TokKind::RoxygenMdListMarker {
        let head_end = close_tag_at_value(tokens, i, value_start.unwrap(), events);
        return emit_md_list_from_value(tokens, head_end, events);
    }
    // An ATX heading from the value (`@details # Title`). Checked before the
    // table: a heading line is never a GFM table header (a table's header must
    // be a paragraph line, engine-probed), even when a delimiter row follows.
    if folds && tokens[value_start.unwrap()].kind == TokKind::RoxygenMdHeading {
        let head_end = close_tag_at_value(tokens, i, value_start.unwrap(), events);
        return emit_md_heading_from_value(tokens, head_end, events);
    }
    // A GFM table whose header row is the value (`@details | a | b |`, a
    // matching delimiter row on the next line).
    if folds && is_md_table_value(tokens, value_start.unwrap()) {
        let head_end = close_tag_at_value(tokens, i, value_start.unwrap(), events);
        return emit_md_table_from_value(tokens, head_end, events);
    }
    // A thematic break from the value (`@details ***`/`---`): roxygen2 renders
    // it empty (the projector drops the node), so only following prose survives
    // as the tag's content. The value position is fresh, so a `---` value is a
    // break, never a setext underline (the lexer carved it as the break leaf).
    if folds && tokens[value_start.unwrap()].kind == TokKind::RoxygenMdThematicBreak {
        let head_end = close_tag_at_value(tokens, i, value_start.unwrap(), events);
        return emit_md_thematic_break_from_value(tokens, head_end, events);
    }
    // A block quote from the value (`@details > quoted`): following `>` lines
    // and lazy continuations join the quote, which roxygen2 flattens to plain
    // text (no block-quote support).
    if folds && tokens[value_start.unwrap()].kind == TokKind::RoxygenMdBlockQuote {
        let head_end = close_tag_at_value(tokens, i, value_start.unwrap(), events);
        return emit_md_block_quote_from_value(tokens, head_end, events);
    }

    // A same-line value whose (folded) paragraph is terminated by a `===`/`---`
    // underline is a **setext heading**, exactly as when the value starts on the
    // next `#'` line. The value can't stay inside the `ROXYGEN_TAG` (the underline
    // must be adjacent to its heading text), so the head closes the tag empty and
    // the value + underline become a sibling `ROXYGEN_MD_HEADING` — the shape the
    // next-line form already produces, so the projector hoists it (description /
    // details) or inlines the title (other prose tags) with no extra arm.
    if folds && is_md_setext_heading_start(tokens, start) {
        let head_end = close_tag_at_value(tokens, i, value_start.unwrap(), events);
        return emit_md_setext_heading_from_value(tokens, head_end, events);
    }

    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }
    // Only a *prose* tag's field spans its continuation lines; a code/examples,
    // verbatim-value, token-list, toggle, `@section`, or verbatim-Rd tag keeps its
    // own line structure (the formatter reformats or passes those through per line).
    if has_value && tag_name.is_some_and(super::tag_folds_prose_continuation) {
        while tokens.get(i).map(|t| &t.kind) == Some(&TokKind::Newline) {
            let mut m = i + 1;
            while tokens.get(m).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
                m += 1;
            }
            if tokens.get(m).map(|t| &t.kind) != Some(&TokKind::RoxygenMarker)
                || !is_foldable_continuation(tokens, m)
            {
                break;
            }
            // Fold the inter-line trivia (newline + continuation indentation) and
            // the continuation prose line into the still-open tag node.
            for idx in i..m {
                events.push(Event::Tok(idx));
            }
            i = emit_prose_line(tokens, m, events);
        }
    }
    events.push(Event::Finish); // ROXYGEN_TAG
    i
}

/// Close an open `ROXYGEN_TAG` **empty at its value**: emit the tag-head tokens
/// from `head_start` up to (but not including) the whitespace run before
/// `value_start`, then finish the tag node. That whitespace starts the sibling
/// block's first, marker-less line — roxygen2 strips only the single separator
/// space after the tag head, so any further indent belongs to the block and
/// renders. Returns the whitespace run's start index (the sibling emitter's
/// entry point).
fn close_tag_at_value(
    tokens: &[Token],
    head_start: usize,
    value_start: usize,
    events: &mut Vec<Event>,
) -> usize {
    let mut head_end = value_start;
    while head_end > head_start && tokens[head_end - 1].kind == TokKind::Whitespace {
        head_end -= 1;
    }
    for idx in head_start..head_end {
        events.push(Event::Tok(idx));
    }
    events.push(Event::Finish); // ROXYGEN_TAG
    head_end
}

/// Emit a `ROXYGEN_MD_HEADING` node for a **setext heading** whose title begins as a
/// tag's same-line value: the first line has no `#'` marker of its own (that marker
/// belongs to the enclosing tag, already emitted and closed), so it starts at
/// `value_start` — the whitespace/content between the tag head and the value. The
/// remaining lines are ordinary `#'` continuation lines threaded in as trivia up to
/// and including the `===`/`---` underline. Returns the token index just past the
/// underline line's content (its trailing `Newline` is the grouper's, so the body
/// prose below becomes a sibling paragraph). Mirrors [`super::build::emit_md_setext_heading`],
/// which handles the case where the whole heading starts at a marker.
fn emit_md_setext_heading_from_value(
    tokens: &[Token],
    value_start: usize,
    events: &mut Vec<Event>,
) -> usize {
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_HEADING));
    // First (marker-less) line: the same-line value and its leading whitespace.
    let mut i = value_start;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }
    // Each subsequent `#'` line, threaded in until the underline. `is_md_setext_-
    // heading_start` guaranteed a reachable underline, so a next line always exists.
    loop {
        // `i` is at the trailing `Newline` of the just-emitted line.
        let mut m = i + 1;
        while tokens.get(m).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
            m += 1;
        }
        debug_assert_eq!(
            tokens.get(m).map(|t| &t.kind),
            Some(&TokKind::RoxygenMarker),
            "setext heading run terminates in an underline"
        );
        for idx in i..=m {
            events.push(Event::Tok(idx)); // newline, continuation indent, marker
        }
        let mut k = m + 1;
        while tokens.get(k).is_some_and(|t| is_line_body_kind(&t.kind)) {
            events.push(Event::Tok(k));
            k += 1;
        }
        if is_md_setext_underline_or_dash(tokens, m) {
            events.push(Event::Finish); // ROXYGEN_MD_HEADING
            return k;
        }
        i = k;
    }
}

/// Whether the roxygen line whose `RoxygenMarker` is at `marker` is a plain-prose
/// continuation that folds into a preceding tag's value: ordinary prose that is
/// not itself a section-level block (a block macro, markdown list, fenced code, or
/// HTML block, each of which opens its own sibling node). The markdown-list check
/// uses the mid-paragraph interrupt rule (`in_paragraph = true`), since a marker
/// after a prose value can only start a list if it would interrupt a paragraph.
pub(super) fn is_foldable_continuation(tokens: &[Token], marker: usize) -> bool {
    matches!(classify_line(tokens, marker), LineKind::Prose)
        && !is_md_html_block_start(tokens, marker)
        && !is_md_code_block_start(tokens, marker)
        && !is_md_list_start(tokens, marker, true)
        && !is_block_macro_line(tokens, marker)
        && !is_md_table_start(tokens, marker)
        && !is_md_heading_start(tokens, marker)
        && !is_md_setext_underline_line(tokens, marker)
        && !is_md_block_quote_start(tokens, marker)
        && !is_md_thematic_break_line(tokens, marker)
}

/// From a line whose `RoxygenMarker` is at `marker`, the marker index of the *next*
/// roxygen line — one `Newline`, optional continuation `Whitespace`, then a
/// `RoxygenMarker` — or `None` when this line is the block's last. Used by the
/// setext-heading look-back to walk a paragraph's continuation lines.
pub(super) fn next_roxygen_line_marker(tokens: &[Token], marker: usize) -> Option<usize> {
    let mut i = marker + 1;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        i += 1;
    }
    if tokens.get(i).map(|t| &t.kind) != Some(&TokKind::Newline) {
        return None;
    }
    let mut m = i + 1;
    while tokens.get(m).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
        m += 1;
    }
    (tokens.get(m).map(|t| &t.kind) == Some(&TokKind::RoxygenMarker)).then_some(m)
}
