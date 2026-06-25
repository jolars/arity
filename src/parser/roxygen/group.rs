//! Roxygen block grouping: wrapping a maximal run of roxygen lines in a
//! `ROXYGEN_BLOCK` and laying out its logical structure as events.
//!
//! This is the *second* phase (`Vec<Token>` → `Vec<Event>`): it decides the
//! block's section/paragraph skeleton, classifies each line, threads the `#'`
//! markers and inter-line trivia in at the open level, and dispatches the
//! block-level Rd-macro / markdown constructs to [`super::build`].

use super::build::{
    block_macro_opener_closes, emit_block_macro, emit_block_macro_inline, emit_md_code_block,
    emit_md_html_block, emit_md_list, is_block_macro_line, is_block_macro_opener,
    is_md_code_block_start, is_md_html_block_start, is_md_list_start,
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
    // it (resolving emphasis/strong delimiter runs into nodes), then splice the
    // result into the caller's stream. The pass is a no-op without delimiter runs,
    // so non-`@md` (and delimiter-free) blocks stay byte-identical.
    let mut block = Vec::new();
    let end = emit_roxygen_block_events(tokens, start, &mut block);
    super::inline::resolve_emphasis(tokens, &mut block);
    events.append(&mut block);
    end
}

fn emit_roxygen_block_events(tokens: &[Token], start: usize, events: &mut Vec<Event>) -> usize {
    debug_assert_eq!(tokens[start].kind, TokKind::RoxygenMarker);
    events.push(Event::Start(SyntaxKind::ROXYGEN_BLOCK));

    let mut i = start;
    let mut section_open = false;
    let mut para_open = false;

    loop {
        // `i` is at a `RoxygenMarker` (a logical line start).
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
                i = emit_tag_line(tokens, i, events);
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
                } else if is_md_code_block_start(tokens, i) {
                    // A markdown fenced code block (`@md` mode) is a direct
                    // section child, like a block macro: close any open paragraph
                    // and emit the code block as a sibling.
                    if para_open {
                        events.push(Event::Finish); // ROXYGEN_PARAGRAPH
                        para_open = false;
                    }
                    i = emit_md_code_block(tokens, i, events);
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
                } else {
                    if !para_open {
                        events.push(Event::Start(SyntaxKind::ROXYGEN_PARAGRAPH));
                        para_open = true;
                    }
                    i = emit_prose_line(tokens, i, events);
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
fn emit_tag_line(tokens: &[Token], start: usize, events: &mut Vec<Event>) -> usize {
    events.push(Event::Tok(start)); // RoxygenMarker
    let mut i = start + 1;
    while tokens.get(i).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
        events.push(Event::Tok(i)); // marker→content whitespace
        i += 1;
    }
    events.push(Event::Start(SyntaxKind::ROXYGEN_TAG));
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }
    events.push(Event::Finish); // ROXYGEN_TAG
    i
}
