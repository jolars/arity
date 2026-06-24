//! Roxygen structure building: the block-level Rd-macro and markdown machinery.
//!
//! The *third* phase, dispatched from [`super::group`]: it recognizes and emits
//! the constructs that span several `#'` lines — block Rd macros
//! (`\itemize{…}`, `\describe{…}`, `\tabular{…}{…}`) and markdown lists — as
//! direct `ROXYGEN_SECTION` children, threading the inter-line `#'`/newline/
//! indentation trivia in losslessly.

use super::group::{LineKind, classify_line, is_line_body_kind, line_content_start};
use super::{is_two_arg_rd_macro, scan_balanced, utf8_len};
use crate::parser::events::Event;
use crate::parser::lexer::{RoxygenRole, TokKind, Token};
use crate::syntax::SyntaxKind;

/// Whether the prose line whose marker is at `start` opens a **block** Rd macro
/// across following `#'` lines. Two shapes:
///
/// * `\name{ …` (Form A): a single `RoxygenText` content token beginning with
///   `\name{` whose group does not close on the line. The lexer extracts a
///   *balanced* inline `\name{…}` as a `RoxygenRdMacro` token, so a `RoxygenText`
///   starting `\name{` is necessarily an unbalanced (multi-line) opener.
/// * `\name{arg}{ …` (Form B): a *balanced* `RoxygenRdMacro` token for a
///   structural macro (`\tabular{format}`, `\item{term}`) immediately followed by
///   a `RoxygenText` that opens an unbalanced `{` body --- the macro's last
///   argument spans following lines.
pub(super) fn is_block_macro_line(tokens: &[Token], start: usize) -> bool {
    let content = line_content_start(tokens, start);
    match tokens.get(content) {
        Some(tok) if tok.kind == TokKind::RoxygenText => is_block_macro_opener(&tok.text),
        Some(tok) if tok.kind == TokKind::RoxygenRdMacro => {
            rd_macro_name(&tok.text).is_some_and(is_two_arg_rd_macro)
                && matches!(
                    tokens.get(content + 1),
                    Some(next)
                        if next.kind == TokKind::RoxygenText && opens_unbalanced_brace(&next.text)
                )
        }
        _ => false,
    }
}

/// The macro name (without the leading `\`) of a `\name…` span, or `None` when
/// `text` does not begin with `\` followed by an alphabetic run.
fn rd_macro_name(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'\\') {
        return None;
    }
    let mut k = 1;
    while k < bytes.len() && bytes[k].is_ascii_alphabetic() {
        k += 1;
    }
    (k > 1).then(|| &text[1..k])
}

/// Whether `text` is an unbalanced `{`-opener: it starts with `{` whose group
/// does not close within the line (so it spans following `#'` lines).
fn opens_unbalanced_brace(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.first() == Some(&b'{') && scan_balanced(bytes, 0, b'{', b'}').is_none()
}

/// Whether `text` begins with an unbalanced `\name{` block-macro opener.
fn is_block_macro_opener(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'\\') {
        return false;
    }
    let mut k = 1;
    while k < bytes.len() && bytes[k].is_ascii_alphabetic() {
        k += 1;
    }
    k > 1 && bytes.get(k) == Some(&b'{') && scan_balanced(bytes, k, b'{', b'}').is_none()
}

/// Whether the prose line whose marker is at `start` opens a **markdown list**
/// (`@md` mode): its content begins with a `RoxygenMdListMarker` leaf, and —
/// when it would interrupt an open paragraph (`para_open`) — the CommonMark
/// interrupt rule admits it (a bullet always, an ordered marker only if its
/// start number is 1). A marker that fails the gate stays inline prose (its
/// `RoxygenMdListMarker` leaf renders as literal text).
pub(super) fn is_md_list_start(tokens: &[Token], start: usize, para_open: bool) -> bool {
    let content = line_content_start(tokens, start);
    match tokens.get(content) {
        Some(tok) if tok.kind == TokKind::RoxygenMdListMarker => {
            !para_open || md_list_marker_can_interrupt(&tok.text)
        }
        _ => false,
    }
}

/// Whether a `RoxygenMdListMarker`'s text may *interrupt an open paragraph* per
/// CommonMark: a bullet always may, an ordered marker only when its start number
/// is 1. (At a fresh block position any marker opens a list; this gate applies
/// only mid-paragraph.)
fn md_list_marker_can_interrupt(marker: &str) -> bool {
    match marker.as_bytes().first() {
        Some(b'-' | b'*' | b'+') => true,
        _ => {
            let digits = marker.trim_end_matches(['.', ')']);
            digits.parse::<u64>().map(|n| n == 1).unwrap_or(false)
        }
    }
}

/// Whether the line whose marker is at `marker` continues a markdown list: its
/// content begins with a `RoxygenMdListMarker`. (Inside a list, any marker line
/// is another item — the interrupt rule applies only to *starting* a list.)
fn is_md_list_continuation(tokens: &[Token], marker: usize) -> bool {
    let content = line_content_start(tokens, marker);
    tokens.get(content).map(|t| &t.kind) == Some(&TokKind::RoxygenMdListMarker)
}

/// Emit a `ROXYGEN_MD_LIST` node spanning the consecutive markdown-list lines
/// beginning at `start` (a `RoxygenMarker` whose content opens a list item).
/// Each item is a `ROXYGEN_MD_LIST_ITEM` holding its `RoxygenMdListMarker` leaf
/// and inline content; the `#'` markers, the marker→content whitespace, and the
/// inter-line newlines/indentation are threaded in as trivia at the list level
/// (losslessness), the way the block Rd macros thread them. The trailing newline
/// after the final item is left to the caller. Returns the token index just past
/// the last consumed content.
pub(super) fn emit_md_list(tokens: &[Token], start: usize, events: &mut Vec<Event>) -> usize {
    debug_assert_eq!(tokens[start].kind, TokKind::RoxygenMarker);
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_LIST));

    let mut i = start;
    loop {
        // `i` is at a `RoxygenMarker` of a list-item line. The marker and the
        // marker→content whitespace are threaded at the list level (trivia).
        events.push(Event::Tok(i));
        i += 1;
        while tokens.get(i).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
            events.push(Event::Tok(i));
            i += 1;
        }

        // The item: its `RoxygenMdListMarker` leaf, then its inline content.
        events.push(Event::Start(SyntaxKind::ROXYGEN_MD_LIST_ITEM));
        events.push(Event::Tok(i)); // RoxygenMdListMarker
        i += 1;
        while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
            events.push(Event::Tok(i));
            i += 1;
        }
        events.push(Event::Finish); // ROXYGEN_MD_LIST_ITEM

        // Continuation: a following list-item line folds its `\n` and leading
        // indentation in as trivia, leaving its marker for the next iteration.
        if tokens.get(i).map(|t| &t.kind) != Some(&TokKind::Newline) {
            break;
        }
        let mut m = i + 1;
        while tokens.get(m).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
            m += 1;
        }
        if tokens.get(m).map(|t| &t.kind) != Some(&TokKind::RoxygenMarker)
            || !is_md_list_continuation(tokens, m)
        {
            break;
        }
        for idx in i..m {
            events.push(Event::Tok(idx));
        }
        i = m;
    }

    events.push(Event::Finish); // ROXYGEN_MD_LIST
    i
}

/// Emit a multi-line block Rd macro as a `ROXYGEN_RD_MACRO` node spanning `#'`
/// lines. The node owns its opening line's marker and the inter-line markers,
/// newlines, and indentation as threaded trivia (losslessness); its body is a
/// sequence of brace-less name-only `\item`/`\cr`/… macros, nested inline macros,
/// and prose, ending at the matching `}` (or, for an unterminated macro, at the
/// next tag opener or block end — greedy and lossless, no close delimiter).
/// Returns the token index just past the last consumed content (at its trailing
/// `Newline` / non-roxygen token / EOF), leaving line separation to the caller.
pub(super) fn emit_block_macro(tokens: &[Token], start: usize, events: &mut Vec<Event>) -> usize {
    debug_assert_eq!(tokens[start].kind, TokKind::RoxygenMarker);
    events.push(Event::Start(SyntaxKind::ROXYGEN_RD_MACRO));

    // Opening line: marker and the marker→content whitespace, threaded inside.
    events.push(Event::Tok(start));
    let mut i = start + 1;
    while tokens.get(i).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
        events.push(Event::Tok(i));
        i += 1;
    }

    let mut depth = 0usize;
    let mut closed = false;

    // Opening content. Form A: a `RoxygenText` `\name{ …` --- split off the name
    // and brace, then parse trailing same-line content. Form B: a balanced
    // `RoxygenRdMacro` `\name{arg}` followed by a `RoxygenText` `{ …` body opener
    // --- emit the macro's name and leading argument group(s) as leaves, then open
    // the body brace.
    match tokens.get(i) {
        Some(tok) if tok.kind == TokKind::RoxygenText => {
            emit_block_open(events, &tok.text, &mut depth, &mut closed);
            i += 1;
        }
        Some(tok) if tok.kind == TokKind::RoxygenRdMacro => {
            emit_block_open_arg_macro(events, &tok.text);
            i += 1;
            if let Some(next) = tokens.get(i) {
                emit_block_body_open(events, &next.text, &mut depth, &mut closed);
                i += 1;
            }
        }
        _ => {}
    }

    'consume: while !closed {
        // Remaining content tokens on the current line.
        while let Some(tok) = tokens.get(i) {
            match &tok.kind {
                TokKind::RoxygenText => {
                    emit_block_content(events, &tok.text, &mut depth, &mut closed);
                    i += 1;
                }
                // A balanced inline span (`\code{x}`, `` `code` ``, `[link]`, or a
                // resolved markdown emphasis/strong/code leaf): pass the whole token
                // through; the tree builder expands a macro token. `RoxygenText` is
                // handled above, so the remaining `Content` kinds are the spans.
                k if k.roxygen_role() == Some(RoxygenRole::Content) => {
                    events.push(Event::Tok(i));
                    i += 1;
                }
                _ => break,
            }
            if closed {
                break 'consume;
            }
        }

        // Line boundary: fold a continuation (`\n`, optional indentation, `#'`)
        // into the node unless the next line is a tag opener or not a roxygen line.
        if tokens.get(i).map(|t| &t.kind) != Some(&TokKind::Newline) {
            break;
        }
        let mut m = i + 1;
        while tokens.get(m).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
            m += 1;
        }
        if tokens.get(m).map(|t| &t.kind) != Some(&TokKind::RoxygenMarker) {
            break;
        }
        if matches!(classify_line(tokens, m), LineKind::Tag) {
            break;
        }
        // `\n` + indentation + `#'` threaded as trivia, then the marker→content ws.
        for idx in i..=m {
            events.push(Event::Tok(idx));
        }
        i = m + 1;
        while tokens.get(i).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
            events.push(Event::Tok(i));
            i += 1;
        }
    }

    events.push(Event::Finish); // ROXYGEN_RD_MACRO
    i
}

/// Emit the opening `\name{` of a block macro: a `ROXYGEN_RD_MACRO_NAME`, the
/// `{` delimiter (setting brace depth to 1), then any trailing same-line content.
fn emit_block_open(events: &mut Vec<Event>, text: &str, depth: &mut usize, closed: &mut bool) {
    let bytes = text.as_bytes();
    let mut k = 1;
    while k < bytes.len() && bytes[k].is_ascii_alphabetic() {
        k += 1;
    }
    events.push(Event::Leaf(
        SyntaxKind::ROXYGEN_RD_MACRO_NAME,
        text[..k].to_string(),
    ));
    // `is_block_macro_opener` guarantees `bytes[k] == b'{'`.
    events.push(Event::Leaf(
        SyntaxKind::ROXYGEN_RD_MACRO_DELIM,
        "{".to_string(),
    ));
    *depth = 1;
    emit_block_content(events, &text[k + 1..], depth, closed);
}

/// Emit the leading `\name{arg}…` of a Form-B block macro from a *balanced*
/// `RoxygenRdMacro` token (`\tabular{rl}`): a `ROXYGEN_RD_MACRO_NAME`, an optional
/// `[opt]`, and each balanced `{…}` argument group as `{`/content/`}` leaves (the
/// content a single `ROXYGEN_TEXT` --- a format/term argument carries no nested
/// markup in practice). The leaves tile `text` exactly. The body `{` that follows
/// is opened separately by [`emit_block_body_open`].
fn emit_block_open_arg_macro(events: &mut Vec<Event>, text: &str) {
    let bytes = text.as_bytes();
    let mut k = 1;
    while k < bytes.len() && bytes[k].is_ascii_alphabetic() {
        k += 1;
    }
    events.push(Event::Leaf(
        SyntaxKind::ROXYGEN_RD_MACRO_NAME,
        text[..k].to_string(),
    ));
    let mut j = k;
    if bytes.get(j) == Some(&b'[')
        && let Some(opt_end) = scan_balanced(bytes, j, b'[', b']')
    {
        events.push(Event::Leaf(
            SyntaxKind::ROXYGEN_RD_MACRO_OPT,
            text[j..opt_end].to_string(),
        ));
        j = opt_end;
    }
    while bytes.get(j) == Some(&b'{') {
        let Some(group_end) = scan_balanced(bytes, j, b'{', b'}') else {
            break;
        };
        events.push(Event::Leaf(
            SyntaxKind::ROXYGEN_RD_MACRO_DELIM,
            "{".to_string(),
        ));
        let content = &text[j + 1..group_end - 1];
        if !content.is_empty() {
            events.push(Event::Leaf(SyntaxKind::ROXYGEN_TEXT, content.to_string()));
        }
        events.push(Event::Leaf(
            SyntaxKind::ROXYGEN_RD_MACRO_DELIM,
            "}".to_string(),
        ));
        j = group_end;
    }
    // Defensive remainder (a malformed token the gate should never admit).
    if j < text.len() {
        events.push(Event::Leaf(SyntaxKind::ROXYGEN_TEXT, text[j..].to_string()));
    }
}

/// Open a Form-B block macro's body brace from a `RoxygenText` `{ …` token: emit
/// the `{` delimiter (setting brace depth to 1), then parse any trailing same-line
/// body content. The gate guarantees `text` begins with `{`.
fn emit_block_body_open(events: &mut Vec<Event>, text: &str, depth: &mut usize, closed: &mut bool) {
    debug_assert_eq!(text.as_bytes().first(), Some(&b'{'));
    events.push(Event::Leaf(
        SyntaxKind::ROXYGEN_RD_MACRO_DELIM,
        "{".to_string(),
    ));
    *depth = 1;
    emit_block_content(events, &text[1..], depth, closed);
}

/// Parse one `RoxygenText` token's worth of block-macro body, emitting leaves:
/// brace-less name-only macros (`\item`, `\cr`, …, a `\name` not followed by
/// `{`), the closing `}` delimiter when it returns brace depth to zero (setting
/// `closed`), and prose runs as `ROXYGEN_TEXT`. Tracks `depth` across calls so a
/// group can open and close on different `#'` lines.
fn emit_block_content(events: &mut Vec<Event>, text: &str, depth: &mut usize, closed: &mut bool) {
    let bytes = text.as_bytes();
    let mut run_start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                let name_start = i + 1;
                let mut k = name_start;
                while k < bytes.len() && bytes[k].is_ascii_alphabetic() {
                    k += 1;
                }
                if k == name_start {
                    // An escape (`\\`, `\{`, `\}`, `\%`): two literal bytes that
                    // never affect brace depth.
                    i = (i + 2).min(bytes.len());
                } else if bytes.get(k) == Some(&b'{') {
                    // An unbalanced nested `\name{` opener (nested block macro,
                    // out of scope): leave it as text; the `{` is depth-counted.
                    i = k;
                } else {
                    // A brace-less name-only macro.
                    push_text(events, &text[run_start..i]);
                    events.push(Event::Start(SyntaxKind::ROXYGEN_RD_MACRO));
                    events.push(Event::Leaf(
                        SyntaxKind::ROXYGEN_RD_MACRO_NAME,
                        text[i..k].to_string(),
                    ));
                    events.push(Event::Finish);
                    i = k;
                    // The whitespace separating it from its sibling text is its own
                    // leaf (kept out of the text run).
                    let ws = i;
                    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
                        i += 1;
                    }
                    if i > ws {
                        events.push(Event::Leaf(SyntaxKind::WHITESPACE, text[ws..i].to_string()));
                    }
                    run_start = i;
                }
            }
            b'{' => {
                *depth += 1;
                i += 1;
            }
            b'}' if *depth <= 1 => {
                push_text(events, &text[run_start..i]);
                events.push(Event::Leaf(
                    SyntaxKind::ROXYGEN_RD_MACRO_DELIM,
                    "}".to_string(),
                ));
                *depth = 0;
                *closed = true;
                run_start = i + 1;
                push_text(events, &text[run_start..]);
                return;
            }
            b'}' => {
                *depth -= 1;
                i += 1;
            }
            b => i += utf8_len(b),
        }
    }
    push_text(events, &text[run_start..]);
}

/// Push a non-empty `ROXYGEN_TEXT` leaf for a prose run.
fn push_text(events: &mut Vec<Event>, text: &str) {
    if !text.is_empty() {
        events.push(Event::Leaf(SyntaxKind::ROXYGEN_TEXT, text.to_string()));
    }
}
