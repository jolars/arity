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

/// One open brace group inside a block macro's body, tracked so the body's
/// closing braces are matched to the right opener. A `Macro` frame is a *nested*
/// block macro (`\itemize{ … }` opening across lines inside its parent): we
/// emitted a `ROXYGEN_RD_MACRO` for it, so its closing `}` finalizes that node.
/// A `Plain` frame is a bare `{` in prose: literal text on both ends, tracked
/// only so its `}` is not mistaken for the enclosing macro's terminator.
enum BodyFrame {
    Macro,
    Plain,
}

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
    let k = super::rd_macro_name_end(bytes, 1);
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
    let k = super::rd_macro_name_end(bytes, 1);
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

/// Whether the prose line whose marker is at `start` opens a **markdown fenced
/// code block** (`@md` mode): its content begins with a `RoxygenMdFence` leaf.
/// The leaf is carved only under a resolved `@md` mode, so its presence is the
/// single mode signal (the builder never re-derives mode).
pub(super) fn is_md_code_block_start(tokens: &[Token], start: usize) -> bool {
    let content = line_content_start(tokens, start);
    tokens.get(content).map(|t| &t.kind) == Some(&TokKind::RoxygenMdFence)
}

/// Emit a `ROXYGEN_MD_CODE_BLOCK` node spanning the fenced code block beginning
/// at `start` (a `RoxygenMarker` whose content is a `RoxygenMdFence` opener).
/// The node owns the opener fence leaf, each verbatim code line's body tokens,
/// and the closing fence leaf; the `#'` markers, the marker→content whitespace,
/// and the inter-line newlines/indentation are threaded in as trivia at the
/// block level (losslessness), the way the block Rd macros and markdown lists
/// thread them. An unterminated block ends at the next tag opener / block end
/// (greedy and lossless, no closing fence). The trailing newline after the last
/// consumed line is left to the caller. Returns the token index just past it.
pub(super) fn emit_md_code_block(tokens: &[Token], start: usize, events: &mut Vec<Event>) -> usize {
    debug_assert_eq!(tokens[start].kind, TokKind::RoxygenMarker);
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_CODE_BLOCK));

    // Opening line: marker, marker→content whitespace, then the opener fence.
    events.push(Event::Tok(start));
    let mut i = start + 1;
    while tokens.get(i).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
        events.push(Event::Tok(i));
        i += 1;
    }
    if tokens.get(i).map(|t| &t.kind) == Some(&TokKind::RoxygenMdFence) {
        events.push(Event::Tok(i)); // opener fence
        i += 1;
    }

    loop {
        // Line boundary: fold a continuation (`\n`, indentation, `#'`) into the
        // node unless the next line is not a roxygen line or is a tag opener
        // (an unterminated block stops there).
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
        // A closing fence ends the block; any other line is verbatim code (its
        // body tokens threaded through). Both consume the whole line's content.
        let is_closer = tokens.get(i).map(|t| &t.kind) == Some(&TokKind::RoxygenMdFence);
        while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
            events.push(Event::Tok(i));
            i += 1;
        }
        if is_closer {
            break;
        }
    }

    events.push(Event::Finish); // ROXYGEN_MD_CODE_BLOCK
    i
}

/// Whether the prose line whose marker is at `start` opens a **markdown HTML
/// block** (`@md` mode): its content begins with a `RoxygenMdHtmlBlock` opener
/// leaf. The leaf is carved only under a resolved `@md` mode, so its presence is
/// the single mode signal (the builder never re-derives mode).
pub(super) fn is_md_html_block_start(tokens: &[Token], start: usize) -> bool {
    let content = line_content_start(tokens, start);
    tokens.get(content).map(|t| &t.kind) == Some(&TokKind::RoxygenMdHtmlBlock)
}

/// Emit a `ROXYGEN_MD_HTML_BLOCK` node spanning the markdown HTML block beginning
/// at `start` (a `RoxygenMarker` whose content is a `RoxygenMdHtmlBlock` opener).
/// Per CommonMark start condition 6, the block runs to the next blank line; every
/// line until then — the opener and any following prose — is verbatim block
/// content (its body tokens threaded through). A tag opener or a non-roxygen line
/// also ends it (greedy and lossless). The `#'` markers, the marker→content
/// whitespace, and the inter-line newlines/indentation are threaded in as trivia
/// at the block level, the way the fenced code block threads them. The trailing
/// newline after the last consumed line is left to the caller. Returns the token
/// index just past it.
pub(super) fn emit_md_html_block(tokens: &[Token], start: usize, events: &mut Vec<Event>) -> usize {
    debug_assert_eq!(tokens[start].kind, TokKind::RoxygenMarker);
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_HTML_BLOCK));

    // Opening line: marker, marker→content whitespace, then the opener content.
    events.push(Event::Tok(start));
    let mut i = start + 1;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }

    loop {
        // Line boundary: fold a continuation into the block. The block runs until a
        // blank line (CommonMark condition 6); a tag opener or a non-roxygen line
        // also ends it.
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
        if !matches!(classify_line(tokens, m), LineKind::Prose) {
            break; // a blank line or tag ends the HTML block
        }
        // `\n` + indentation + `#'` threaded as trivia, then the line's body.
        for idx in i..=m {
            events.push(Event::Tok(idx));
        }
        i = m + 1;
        while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
            events.push(Event::Tok(i));
            i += 1;
        }
    }

    events.push(Event::Finish); // ROXYGEN_MD_HTML_BLOCK
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

    // The body's open brace groups (the parent macro's own body is the empty-stack
    // baseline, so a `}` at an empty stack terminates it).
    let mut frames: Vec<BodyFrame> = Vec::new();
    let mut closed = false;

    // Opening content. Form A: a `RoxygenText` `\name{ …` --- split off the name
    // and brace, then parse trailing same-line content. Form B: a balanced
    // `RoxygenRdMacro` `\name{arg}` followed by a `RoxygenText` `{ …` body opener
    // --- emit the macro's name and leading argument group(s) as leaves, then open
    // the body brace.
    match tokens.get(i) {
        Some(tok) if tok.kind == TokKind::RoxygenText => {
            emit_block_open(events, &tok.text, &mut frames, &mut closed);
            i += 1;
        }
        Some(tok) if tok.kind == TokKind::RoxygenRdMacro => {
            emit_block_open_arg_macro(events, &tok.text);
            i += 1;
            if let Some(next) = tokens.get(i) {
                emit_block_body_open(events, &next.text, &mut frames, &mut closed);
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
                    emit_block_content(events, &tok.text, &mut frames, &mut closed);
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

    // An unterminated macro (ended by a tag opener / block end, no closing `}`)
    // may leave nested macros open: close each so the event stream stays balanced.
    for frame in frames.into_iter().rev() {
        if matches!(frame, BodyFrame::Macro) {
            events.push(Event::Finish); // nested ROXYGEN_RD_MACRO
        }
    }

    events.push(Event::Finish); // ROXYGEN_RD_MACRO
    i
}

/// Emit the opening `\name{` of a block macro: a `ROXYGEN_RD_MACRO_NAME`, the
/// `{` delimiter, then any trailing same-line content. The parent body is the
/// empty-frame baseline ([`emit_block_content`]).
fn emit_block_open(
    events: &mut Vec<Event>,
    text: &str,
    frames: &mut Vec<BodyFrame>,
    closed: &mut bool,
) {
    let bytes = text.as_bytes();
    let k = super::rd_macro_name_end(bytes, 1);
    events.push(Event::Leaf(
        SyntaxKind::ROXYGEN_RD_MACRO_NAME,
        text[..k].to_string(),
    ));
    // `is_block_macro_opener` guarantees `bytes[k] == b'{'`.
    events.push(Event::Leaf(
        SyntaxKind::ROXYGEN_RD_MACRO_DELIM,
        "{".to_string(),
    ));
    emit_block_content(events, &text[k + 1..], frames, closed);
}

/// Emit the leading `\name{arg}…` of a Form-B block macro from a *balanced*
/// `RoxygenRdMacro` token (`\tabular{rl}`): a `ROXYGEN_RD_MACRO_NAME`, an optional
/// `[opt]`, and each balanced `{…}` argument group as `{`/content/`}` leaves (the
/// content a single `ROXYGEN_TEXT` --- a format/term argument carries no nested
/// markup in practice). The leaves tile `text` exactly. The body `{` that follows
/// is opened separately by [`emit_block_body_open`].
fn emit_block_open_arg_macro(events: &mut Vec<Event>, text: &str) {
    let bytes = text.as_bytes();
    let k = super::rd_macro_name_end(bytes, 1);
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
/// the `{` delimiter, then parse any trailing same-line body content. The gate
/// guarantees `text` begins with `{`; the body is the empty-frame baseline.
fn emit_block_body_open(
    events: &mut Vec<Event>,
    text: &str,
    frames: &mut Vec<BodyFrame>,
    closed: &mut bool,
) {
    debug_assert_eq!(text.as_bytes().first(), Some(&b'{'));
    events.push(Event::Leaf(
        SyntaxKind::ROXYGEN_RD_MACRO_DELIM,
        "{".to_string(),
    ));
    emit_block_content(events, &text[1..], frames, closed);
}

/// Parse one `RoxygenText` token's worth of block-macro body, emitting leaves:
/// brace-less name-only macros (`\item`, `\cr`, …, a `\name` not followed by
/// `{`), *nested* block macros (`\name{ … }` opening across lines, modeled as a
/// child `ROXYGEN_RD_MACRO`), the closing `}` delimiter that terminates the
/// enclosing macro (setting `closed`), and prose runs as `ROXYGEN_TEXT`. The open
/// brace `frames` are tracked across calls, so a group can open and close on
/// different `#'` lines.
fn emit_block_content(
    events: &mut Vec<Event>,
    text: &str,
    frames: &mut Vec<BodyFrame>,
    closed: &mut bool,
) {
    let bytes = text.as_bytes();
    let mut run_start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                let name_start = i + 1;
                let k = super::rd_macro_name_end(bytes, name_start);
                if k == name_start {
                    // An escape (`\\`, `\{`, `\}`, `\%`): two literal bytes that
                    // never open a brace group.
                    i = (i + 2).min(bytes.len());
                } else if bytes.get(k) == Some(&b'{') {
                    // An unbalanced nested `\name{` opener: a nested block macro
                    // whose body spans following `#'` lines. Open a child
                    // `ROXYGEN_RD_MACRO`; its matching `}` (its `Macro` frame)
                    // finalizes it.
                    push_text(events, &text[run_start..i]);
                    events.push(Event::Start(SyntaxKind::ROXYGEN_RD_MACRO));
                    events.push(Event::Leaf(
                        SyntaxKind::ROXYGEN_RD_MACRO_NAME,
                        text[i..k].to_string(),
                    ));
                    events.push(Event::Leaf(
                        SyntaxKind::ROXYGEN_RD_MACRO_DELIM,
                        "{".to_string(),
                    ));
                    frames.push(BodyFrame::Macro);
                    i = k + 1;
                    run_start = i;
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
            // A bare `{` in prose: literal text on both ends, tracked only so its
            // matching `}` is not mistaken for the enclosing macro's terminator.
            b'{' => {
                frames.push(BodyFrame::Plain);
                i += 1;
            }
            b'}' => match frames.pop() {
                // No open group: this `}` terminates the enclosing block macro.
                None => {
                    push_text(events, &text[run_start..i]);
                    events.push(Event::Leaf(
                        SyntaxKind::ROXYGEN_RD_MACRO_DELIM,
                        "}".to_string(),
                    ));
                    *closed = true;
                    run_start = i + 1;
                    push_text(events, &text[run_start..]);
                    return;
                }
                // Closes a nested block macro: finalize its `ROXYGEN_RD_MACRO`.
                Some(BodyFrame::Macro) => {
                    push_text(events, &text[run_start..i]);
                    events.push(Event::Leaf(
                        SyntaxKind::ROXYGEN_RD_MACRO_DELIM,
                        "}".to_string(),
                    ));
                    events.push(Event::Finish); // nested ROXYGEN_RD_MACRO
                    i += 1;
                    run_start = i;
                }
                // Closes a bare prose group: the `}` stays literal text.
                Some(BodyFrame::Plain) => i += 1,
            },
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
