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
pub(super) fn is_block_macro_opener(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'\\') {
        return false;
    }
    let k = super::rd_macro_name_end(bytes, 1);
    k > 1 && bytes.get(k) == Some(&b'{') && scan_balanced(bytes, k, b'{', b'}').is_none()
}

/// Whether the block-macro opener token at `opener` actually **closes** within
/// the block — its `{` group is balanced by a `}` on the opener line or a later
/// `#'` line, before a tag opener or the block's end. A line-start opener is
/// committed unconditionally (it can only be a block opener), but a *mid-prose*
/// `\name{` is committed to a block macro only when it closes; an unclosed one
/// stays literal prose (parse_Rd rejects an unbalanced macro outright, so this is
/// the conservative recovery — see the `roxygen_unbalanced_macro` fixture).
pub(super) fn block_macro_opener_closes(tokens: &[Token], opener: usize) -> bool {
    let mut depth = 0i32;
    let mut i = opener;
    loop {
        // Brace-count the content tokens on the current line; a balanced inline
        // span (`\code{x}`, `` `x` ``, …) is its own token and brace-neutral.
        while let Some(tok) = tokens.get(i) {
            match &tok.kind {
                TokKind::RoxygenText => {
                    if brace_scan(&tok.text, &mut depth) {
                        return true;
                    }
                    i += 1;
                }
                k if k.roxygen_role() == Some(RoxygenRole::Content) => i += 1,
                _ => break,
            }
        }
        // Line boundary: a continuation (`\n` + indentation + `#'`) keeps scanning;
        // a tag opener, a non-roxygen line, or EOF ends the block unclosed.
        if tokens.get(i).map(|t| &t.kind) != Some(&TokKind::Newline) {
            return false;
        }
        let mut m = i + 1;
        while tokens.get(m).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
            m += 1;
        }
        if tokens.get(m).map(|t| &t.kind) != Some(&TokKind::RoxygenMarker) {
            return false;
        }
        if matches!(classify_line(tokens, m), LineKind::Tag) {
            return false;
        }
        i = m + 1;
        while tokens.get(i).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
            i += 1;
        }
    }
}

/// Track the running `{`/`}` brace depth across `text` (Rd `\`-escapes skipped),
/// returning `true` the moment the depth returns to zero — i.e. the macro's group
/// closes. `*depth` carries across the body's tokens.
fn brace_scan(text: &str, depth: &mut i32) -> bool {
    let bytes = text.as_bytes();
    let mut j = 0;
    while j < bytes.len() {
        match bytes[j] {
            b'\\' => j += 2, // skip the escaped byte (`\{`, `\}`, `\\`, …)
            b'{' => {
                *depth += 1;
                j += 1;
            }
            b'}' => {
                *depth -= 1;
                j += 1;
                if *depth == 0 {
                    return true;
                }
            }
            _ => j += 1,
        }
    }
    false
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
            !para_open
                || (md_list_marker_can_interrupt(&tok.text)
                    && !md_list_item_is_empty(tokens, content))
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

/// Whether the list-item line whose `RoxygenMdListMarker` is at `marker` is
/// **empty** — only optional trailing whitespace follows it before the line ends.
/// CommonMark forbids an empty list item from interrupting a paragraph (a lone
/// `*`/`-` after prose stays paragraph text, never a spurious one-item list); this
/// gate applies only mid-paragraph, so an empty item at a fresh block position
/// still opens a list.
fn md_list_item_is_empty(tokens: &[Token], marker: usize) -> bool {
    let mut i = marker + 1;
    while tokens
        .get(i)
        .is_some_and(|t| is_line_body_kind(&t.kind) && t.text.trim().is_empty())
    {
        i += 1;
    }
    !tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind))
}

/// Whether the line whose marker is at `marker` continues a markdown list: its
/// content begins with a `RoxygenMdListMarker`. (Inside a list, any marker line
/// is another item — the interrupt rule applies only to *starting* a list.)
fn is_md_list_continuation(tokens: &[Token], marker: usize) -> bool {
    let content = line_content_start(tokens, marker);
    tokens.get(content).map(|t| &t.kind) == Some(&TokKind::RoxygenMdListMarker)
}

/// The indentation (in columns) of a list line whose `RoxygenMarker` is at
/// `marker`: the width of the `#'`→content whitespace, which — after the `#'`
/// sigil and one conventional space are stripped — is what CommonMark uses to
/// decide list nesting. (Tabs count as one column here; the corpus uses spaces.)
fn list_line_indent(tokens: &[Token], marker: usize) -> usize {
    let mut k = marker + 1;
    let mut indent = 0;
    while tokens.get(k).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
        indent += tokens[k].text.chars().count();
        k += 1;
    }
    indent
}

/// The number of leading spaces of a list item's content (the first body token
/// after its marker), clamped to CommonMark's 1..=4: a child block must be
/// indented to at least `marker_indent + marker_width + this` to nest.
fn content_leading_spaces(tokens: &[Token], content: usize) -> usize {
    tokens
        .get(content)
        .filter(|t| is_line_body_kind(&t.kind))
        .map(|t| {
            t.text
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .count()
        })
        .unwrap_or(1)
        .clamp(1, 4)
}

/// From `i` (expected at a line's trailing `Newline`), the index of the next
/// line's `RoxygenMarker` when that line continues a markdown list (its content
/// begins with a `RoxygenMdListMarker`); `None` otherwise.
fn next_list_line(tokens: &[Token], i: usize) -> Option<usize> {
    if tokens.get(i).map(|t| &t.kind) != Some(&TokKind::Newline) {
        return None;
    }
    let mut m = i + 1;
    while tokens.get(m).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
        m += 1;
    }
    (tokens.get(m).map(|t| &t.kind) == Some(&TokKind::RoxygenMarker)
        && is_md_list_continuation(tokens, m))
    .then_some(m)
}

/// Whether the prose line whose marker is at `start` opens a **GFM table**
/// (`@md` mode): its immediately-following line's content is a
/// `RoxygenMdTableDelim` leaf (a delimiter row) *and* the two lines have the same
/// number of cells (GFM recognizes a table only on a matching header/delimiter
/// cell count). The header line itself is ordinary prose — a table has no
/// header-line leaf — so this two-line look-ahead is what distinguishes a table
/// from a paragraph that merely contains pipes.
pub(super) fn is_md_table_start(tokens: &[Token], start: usize) -> bool {
    let header_end = line_content_end(tokens, start);
    let Some(delim_marker) = following_line_marker(tokens, header_end) else {
        return false;
    };
    let delim_content = line_content_start(tokens, delim_marker);
    if tokens.get(delim_content).map(|t| &t.kind) != Some(&TokKind::RoxygenMdTableDelim) {
        return false;
    }
    let header = line_raw_content(tokens, start);
    let delim = &tokens[delim_content].text;
    super::count_table_cells(&header) == super::count_table_cells(delim)
}

/// The index just past a line's content — at its trailing `Newline`, a
/// non-roxygen token, or EOF — starting from the `RoxygenMarker` at `marker`.
fn line_content_end(tokens: &[Token], marker: usize) -> usize {
    let mut i = line_content_start(tokens, marker);
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        i += 1;
    }
    i
}

/// From `i` (expected at a line's trailing `Newline`), the next roxygen line's
/// `RoxygenMarker`, or `None` when the block ends (non-roxygen line / EOF).
fn following_line_marker(tokens: &[Token], i: usize) -> Option<usize> {
    if tokens.get(i).map(|t| &t.kind) != Some(&TokKind::Newline) {
        return None;
    }
    let mut m = i + 1;
    while tokens.get(m).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
        m += 1;
    }
    (tokens.get(m).map(|t| &t.kind) == Some(&TokKind::RoxygenMarker)).then_some(m)
}

/// Reconstruct a roxygen line's raw content (the text after the `#'` marker and
/// its trailing whitespace) by concatenating its body tokens' text — used to
/// count a header row's cells (its inline spans are separate tokens, so the raw
/// pipe structure is only visible once they are re-joined).
fn line_raw_content(tokens: &[Token], marker: usize) -> String {
    let mut s = String::new();
    let mut i = line_content_start(tokens, marker);
    while let Some(tok) = tokens.get(i) {
        if !is_line_body_kind(&tok.kind) {
            break;
        }
        s.push_str(&tok.text);
        i += 1;
    }
    s
}

/// Whether the line whose marker is at `marker` is a **table body row**: an
/// ordinary prose line that does not itself open another block construct. A GFM
/// table greedily consumes following non-blank prose lines as rows (a pipeless
/// line is a single-cell row), breaking only at a blank line, a tag, a new block
/// (list / fenced code / HTML block / block macro), or the block's end.
fn is_table_row_line(tokens: &[Token], marker: usize) -> bool {
    matches!(classify_line(tokens, marker), LineKind::Prose)
        && !is_md_html_block_start(tokens, marker)
        // A standalone-tag line (HTML block condition 7) ends the table too: a
        // table is not a paragraph, so condition 7 opens a block after it
        // (engine-probed: a `<span>` line after a body row starts an HTML block,
        // not a single-cell row).
        && !is_md_html_block7_line(tokens, marker)
        && !is_md_code_block_start(tokens, marker)
        && !is_md_list_start(tokens, marker, false)
        && !is_block_macro_line(tokens, marker)
        && !is_md_heading_start(tokens, marker)
        && !is_md_block_quote_start(tokens, marker)
        && !is_md_thematic_break_line(tokens, marker)
}

/// Whether the prose line whose marker is at `start` opens a markdown **block
/// quote** (`@md` mode): its first content token is a `RoxygenMdBlockQuote` leaf.
/// The leaf is carved only under a resolved `@md` mode, so its presence is the
/// single mode signal (the builder never re-derives mode).
pub(super) fn is_md_block_quote_start(tokens: &[Token], start: usize) -> bool {
    let content = line_content_start(tokens, start);
    tokens.get(content).map(|t| &t.kind) == Some(&TokKind::RoxygenMdBlockQuote)
}

/// Emit a `ROXYGEN_MD_BLOCK_QUOTE` node spanning the block quote beginning at
/// `start` (a `RoxygenMarker` whose content is a `RoxygenMdBlockQuote` opener). The
/// node gathers the opener and every following **consecutive** block-quote line
/// (each a `>`-opening `#'` line); a blank line, a tag, a non-`>` prose line, or a
/// non-roxygen line ends it. The `#'` markers, marker→content whitespace, and
/// inter-line newlines/indentation are threaded in as trivia (losslessness), the
/// way the HTML block threads them. CommonMark **lazy continuation** is honored: a
/// non-`>` paragraph line immediately following a quote line (no intervening blank)
/// still belongs to the quote's open paragraph, so it is folded in too. The guard
/// is [`is_foldable_continuation`](super::group::is_foldable_continuation) — a plain
/// prose line that opens no new block; a line that starts a list, fence, heading,
/// table, block macro, thematic break, or another quote is not lazy. The trailing
/// newline after the last line is left to the caller. Returns the token index just
/// past it.
pub(super) fn emit_md_block_quote(
    tokens: &[Token],
    start: usize,
    events: &mut Vec<Event>,
) -> usize {
    debug_assert_eq!(tokens[start].kind, TokKind::RoxygenMarker);
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_BLOCK_QUOTE));

    // Opening line: marker, marker→content whitespace, then the opener content.
    events.push(Event::Tok(start));
    let mut i = start + 1;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }

    finish_md_block_quote(tokens, i, events)
}

/// Emit a `ROXYGEN_MD_BLOCK_QUOTE` node for a block quote opening as a **tag's
/// same-line value** (`#' @details > quoted`): the first line has no `#'` marker
/// of its own (that marker belongs to the enclosing tag, already emitted and
/// closed), so it starts at `ws_start` — the whitespace between the tag head and
/// the `RoxygenMdBlockQuote` leaf. The following lines gather exactly as in
/// [`emit_md_block_quote`] (consecutive `>` lines plus lazy continuations).
/// Returns the token index just past the last consumed line's content.
pub(super) fn emit_md_block_quote_from_value(
    tokens: &[Token],
    ws_start: usize,
    events: &mut Vec<Event>,
) -> usize {
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_BLOCK_QUOTE));
    let mut i = ws_start;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }
    finish_md_block_quote(tokens, i, events)
}

/// Gather a block quote's continuation lines (consecutive `>` lines and lazy
/// paragraph continuations) after its opening line, then finish the
/// `ROXYGEN_MD_BLOCK_QUOTE` node. `i` is at the opening line's trailing
/// `Newline`. Shared by the line-start ([`emit_md_block_quote`]) and tag-value
/// ([`emit_md_block_quote_from_value`]) forms.
fn finish_md_block_quote(tokens: &[Token], mut i: usize, events: &mut Vec<Event>) -> usize {
    loop {
        // Line boundary: fold a following consecutive block-quote line into the node.
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
        if !is_md_block_quote_start(tokens, m)
            && (!super::group::is_foldable_continuation(tokens, m)
                // A standalone-tag line (HTML block condition 7) is NOT a lazy
                // continuation here: the container match already failed at the
                // missing `>`, so the quote's open paragraph is never reached and
                // condition 7 opens a block (engine-probed: `> quoted` then
                // `<span>` renders the tag as an HTML block, not quote text).
                || is_md_html_block7_line(tokens, m))
        {
            break; // a blank line, tag, or new-block line ends the quote
        }
        // A `>` line or a lazy paragraph-continuation line: `\n` + indentation + `#'`
        // threaded as trivia, then the line's body.
        for idx in i..=m {
            events.push(Event::Tok(idx));
        }
        i = m + 1;
        while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
            events.push(Event::Tok(i));
            i += 1;
        }
    }

    events.push(Event::Finish); // ROXYGEN_MD_BLOCK_QUOTE
    i
}

/// Whether the prose line whose marker is at `start` is a markdown **thematic
/// break** (`@md` mode): either its first content token is a `RoxygenMdThematicBreak`
/// leaf (the `*`/`_`-based and space-separated forms, carved by the lexer), or it is
/// a `RoxygenMdSetextUnderline` leaf whose content is a run of three or more dashes.
///
/// The second case is the CommonMark precedence resolution for a contiguous `---`:
/// the lexer carves it as a setext underline (so it can promote a preceding
/// paragraph into a heading), but when it heads no paragraph it is a thematic break.
/// This predicate is only consulted at a line that reaches block dispatch on its own
/// (a promoting `---` is consumed with its paragraph before then), so a bare
/// dash-run `---` here is always a thematic break. An `===` underline is never a
/// thematic break (only `*`/`-`/`_` open one), so it stays literal prose.
pub(super) fn is_md_thematic_break_line(tokens: &[Token], start: usize) -> bool {
    let content = line_content_start(tokens, start);
    match tokens.get(content).map(|t| &t.kind) {
        Some(TokKind::RoxygenMdThematicBreak) => true,
        Some(TokKind::RoxygenMdSetextUnderline) => {
            setext_underline_is_thematic(&tokens[content].text)
        }
        _ => false,
    }
}

/// Whether a `RoxygenMdSetextUnderline` leaf's text is a **thematic break**: after
/// up to three leading spaces, a run of three or more `-` characters. A setext
/// underline is a contiguous run of one marker char, so a `=` underline (never a
/// thematic break) and a `--` (too short) are both rejected here.
fn setext_underline_is_thematic(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut j = 0;
    while j < 3 && bytes.get(j) == Some(&b' ') {
        j += 1;
    }
    let mut count = 0usize;
    while bytes.get(j) == Some(&b'-') {
        count += 1;
        j += 1;
    }
    count >= 3
}

/// Emit a single-line `ROXYGEN_MD_THEMATIC_BREAK` node for the thematic break whose
/// `RoxygenMarker` is at `start`. The node holds the `#'` marker and marker→content
/// whitespace (trivia) and the break leaf; the trailing newline is left to the
/// caller. roxygen2 renders a thematic break as empty, so the projector drops the
/// node (it contributes nothing and lets the surrounding paragraphs coalesce).
/// Returns the token index just past the line's content.
pub(super) fn emit_md_thematic_break(
    tokens: &[Token],
    start: usize,
    events: &mut Vec<Event>,
) -> usize {
    debug_assert_eq!(tokens[start].kind, TokKind::RoxygenMarker);
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_THEMATIC_BREAK));
    events.push(Event::Tok(start));
    let mut i = start + 1;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }
    events.push(Event::Finish); // ROXYGEN_MD_THEMATIC_BREAK
    i
}

/// Emit a single-line `ROXYGEN_MD_THEMATIC_BREAK` node for a thematic break
/// opening as a **tag's same-line value** (`#' @details ***`): the line has no
/// `#'` marker of its own (that marker belongs to the enclosing tag, already
/// emitted and closed), so it starts at `ws_start` — the whitespace between the
/// tag head and the `RoxygenMdThematicBreak` leaf. The value position is fresh
/// (no preceding paragraph), so a contiguous `---` value is a break here, never
/// a setext underline. Returns the token index just past the break content.
pub(super) fn emit_md_thematic_break_from_value(
    tokens: &[Token],
    ws_start: usize,
    events: &mut Vec<Event>,
) -> usize {
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_THEMATIC_BREAK));
    let mut i = ws_start;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }
    events.push(Event::Finish); // ROXYGEN_MD_THEMATIC_BREAK
    i
}

/// Emit a `ROXYGEN_MD_TABLE` node spanning the GFM table beginning at `start` (a
/// `RoxygenMarker` whose line is a table header, the following line a matching
/// delimiter row — see [`is_md_table_start`]). The node owns the header row, the
/// delimiter row, and any following body rows, with the `#'` markers,
/// marker→content whitespace, and inter-line newlines/indentation threaded in as
/// trivia (losslessness), the way the fenced code block and HTML block thread
/// them. The trailing newline after the last row is left to the caller. Returns
/// the token index just past the last consumed content.
pub(super) fn emit_md_table(tokens: &[Token], start: usize, events: &mut Vec<Event>) -> usize {
    debug_assert_eq!(tokens[start].kind, TokKind::RoxygenMarker);
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_TABLE));

    // Header line: marker, then the marker→content whitespace and content tokens.
    events.push(Event::Tok(start));
    let mut i = start + 1;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }

    finish_md_table(tokens, i, events)
}

/// Whether a prose tag's same-line value opens a **GFM table** as its header row:
/// the immediately-following line's content is a `RoxygenMdTableDelim` leaf with
/// the same cell count as the value (the from-value analog of
/// [`is_md_table_start`] — the header is generic prose, so the mode signal is the
/// delimiter leaf on the next line). The value's raw text is reconstructed from
/// its tokens (inline spans are separate tokens, as in [`line_raw_content`]).
pub(super) fn is_md_table_value(tokens: &[Token], value_start: usize) -> bool {
    let mut header = String::new();
    let mut i = value_start;
    while let Some(tok) = tokens.get(i) {
        if !is_line_body_kind(&tok.kind) {
            break;
        }
        header.push_str(&tok.text);
        i += 1;
    }
    let Some(delim_marker) = following_line_marker(tokens, i) else {
        return false;
    };
    let delim_content = line_content_start(tokens, delim_marker);
    if tokens.get(delim_content).map(|t| &t.kind) != Some(&TokKind::RoxygenMdTableDelim) {
        return false;
    }
    let delim = &tokens[delim_content].text;
    super::count_table_cells(&header) == super::count_table_cells(delim)
}

/// Emit a `ROXYGEN_MD_TABLE` node for a table whose header row is a **tag's
/// same-line value** (`#' @details | a | b |`): the first line has no `#'`
/// marker of its own (that marker belongs to the enclosing tag, already emitted
/// and closed), so it starts at `ws_start` — the whitespace between the tag head
/// and the header content. The delimiter and body rows gather exactly as in
/// [`emit_md_table`]. Returns the token index just past the last consumed row.
pub(super) fn emit_md_table_from_value(
    tokens: &[Token],
    ws_start: usize,
    events: &mut Vec<Event>,
) -> usize {
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_TABLE));
    // First (marker-less) header line: the leading whitespace and the row content.
    let mut i = ws_start;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }
    finish_md_table(tokens, i, events)
}

/// Gather a table's delimiter and body rows after its header line, then finish
/// the `ROXYGEN_MD_TABLE` node. `i` is at the header line's trailing `Newline`.
/// Shared by the line-start ([`emit_md_table`]) and tag-value
/// ([`emit_md_table_from_value`]) forms.
fn finish_md_table(tokens: &[Token], mut i: usize, events: &mut Vec<Event>) -> usize {
    // Following lines: the delimiter row (guaranteed to be the first one by the
    // gate) and any body rows. Stop at a blank line, a tag, a new block, or EOF.
    while let Some(m) = following_line_marker(tokens, i) {
        if !is_table_row_line(tokens, m) {
            break;
        }
        for idx in i..=m {
            events.push(Event::Tok(idx)); // `\n` + indentation + `#'` (trivia)
        }
        i = m + 1;
        while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
            events.push(Event::Tok(i));
            i += 1;
        }
    }

    events.push(Event::Finish); // ROXYGEN_MD_TABLE
    i
}

/// Emit a `ROXYGEN_MD_LIST` node spanning the consecutive markdown-list lines
/// beginning at `start` (a `RoxygenMarker` whose content opens a list item),
/// modeling **nesting** by indentation (CommonMark): a following list line
/// indented to an item's content column (or deeper) opens a nested
/// `ROXYGEN_MD_LIST` inside that item, while a line back at the list's own
/// marker column is a sibling. The trailing newline after the final item is left
/// to the caller. Returns the token index just past the last consumed content.
pub(super) fn emit_md_list(tokens: &[Token], start: usize, events: &mut Vec<Event>) -> usize {
    debug_assert_eq!(tokens[start].kind, TokKind::RoxygenMarker);
    emit_md_list_level_inner(tokens, start, true, list_line_indent(tokens, start), events)
}

/// Emit a `ROXYGEN_MD_LIST` node for a list whose first item is a **tag's
/// same-line value** (`#' @details - item`): the first item line has no `#'`
/// marker of its own (that marker belongs to the enclosing tag, already emitted
/// and closed), so it starts at `ws_start` — the whitespace between the tag head
/// and the item's `RoxygenMdListMarker` leaf. That whitespace has the same
/// one-based indent semantics as a line-start item's marker→content whitespace
/// (roxygen2 strips the tag head plus one separator space, exactly as it strips
/// `#'` plus one space), so nesting and sibling decisions for the following
/// `#'` list lines work unchanged. Returns the token index just past the last
/// consumed item content.
pub(super) fn emit_md_list_from_value(
    tokens: &[Token],
    ws_start: usize,
    events: &mut Vec<Event>,
) -> usize {
    let mut indent = 0;
    let mut k = ws_start;
    while tokens.get(k).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
        indent += tokens[k].text.chars().count();
        k += 1;
    }
    emit_md_list_level_inner(tokens, ws_start, false, indent, events)
}

/// Recursion entry for nested list levels (each starts at a line's
/// `RoxygenMarker`).
fn emit_md_list_level(
    tokens: &[Token],
    start: usize,
    list_indent: usize,
    events: &mut Vec<Event>,
) -> usize {
    emit_md_list_level_inner(tokens, start, true, list_indent, events)
}

/// Emit one `ROXYGEN_MD_LIST` whose item markers sit at indentation
/// `list_indent`. Each item is a `ROXYGEN_MD_LIST_ITEM` holding its
/// `RoxygenMdListMarker` leaf, inline content, and any nested `ROXYGEN_MD_LIST`;
/// the `#'` markers, marker→content whitespace, and inter-line
/// newlines/indentation are threaded in as trivia (losslessness), the way the
/// block Rd macros thread them. Recurses for nested levels.
fn emit_md_list_level_inner(
    tokens: &[Token],
    start: usize,
    first_has_marker: bool,
    list_indent: usize,
    events: &mut Vec<Event>,
) -> usize {
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_LIST));

    let mut i = start;
    let mut has_marker = first_has_marker;
    loop {
        // `i` is at a `RoxygenMarker` of a list-item line at this level (or, for
        // a marker-less tag-value first item, directly at its leading
        // whitespace). The marker and the marker→content whitespace are threaded
        // as trivia.
        if has_marker {
            events.push(Event::Tok(i));
            i += 1;
        }
        has_marker = true;
        let mut indent = 0;
        while tokens.get(i).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
            indent += tokens[i].text.chars().count();
            events.push(Event::Tok(i));
            i += 1;
        }

        // The item: its `RoxygenMdListMarker` leaf, then its inline content. A
        // child block must reach this item's content column to nest under it.
        events.push(Event::Start(SyntaxKind::ROXYGEN_MD_LIST_ITEM));
        let marker_width = tokens[i].text.chars().count();
        events.push(Event::Tok(i)); // RoxygenMdListMarker
        i += 1;
        let content_indent = indent + marker_width + content_leading_spaces(tokens, i);
        while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
            events.push(Event::Tok(i));
            i += 1;
        }

        // Nested lists: every following list line indented to (or past) this
        // item's content column is a child list inside this item.
        while let Some(m) = next_list_line(tokens, i) {
            if list_line_indent(tokens, m) < content_indent {
                break;
            }
            for idx in i..m {
                events.push(Event::Tok(idx)); // `\n` + leading indentation (trivia)
            }
            i = emit_md_list_level(tokens, m, list_line_indent(tokens, m), events);
        }
        events.push(Event::Finish); // ROXYGEN_MD_LIST_ITEM

        // Sibling: a following list line back at this level's marker column
        // continues the list; anything shallower ends it (the caller resumes).
        let Some(m) = next_list_line(tokens, i) else {
            break;
        };
        if list_line_indent(tokens, m) != list_indent {
            break;
        }
        for idx in i..m {
            events.push(Event::Tok(idx)); // `\n` + leading indentation (trivia)
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

/// Whether the roxygen line whose marker is at `start` is an **indented-code
/// line**: its marker->content whitespace is five or more all-space columns (a
/// CommonMark indented code block needs four columns; roxygen2 strips the marker
/// and one following space first) *and* there is real content after it (a
/// whitespace-only line is blank, not code). Mode-blind — the caller gates on `md`;
/// the leading whitespace is ordinary `Whitespace` (no special leaf), so the
/// block-macro machinery's whitespace handling is unaffected.
fn is_indent_code_line(tokens: &[Token], start: usize) -> bool {
    let Some(ws) = tokens.get(start + 1) else {
        return false;
    };
    if ws.kind != TokKind::Whitespace || ws.text.len() < 5 || !ws.text.bytes().all(|b| b == b' ') {
        return false;
    }
    let content = line_content_start(tokens, start);
    tokens
        .get(content)
        .is_some_and(|t| is_line_body_kind(&t.kind))
}

/// Whether the roxygen line whose marker is at `start` opens a **markdown indented
/// code block**: the block is `@md`, the line is an indented-code line, and it does
/// not interrupt an open paragraph. A CommonMark indented code block cannot
/// interrupt a paragraph, so a >= 4-column-indented line inside a paragraph
/// (`para_open`) is a lazy continuation, not a code block — the same block-level
/// `para_open` gate the list-marker recognizer applies. `md` is threaded from the
/// block builder (there is no per-line leaf to key off, since the content lexes as
/// ordinary tokens), the way the projector re-derives it per block.
pub(super) fn is_md_indented_code_start(
    tokens: &[Token],
    start: usize,
    para_open: bool,
    md: bool,
) -> bool {
    md && !para_open && is_indent_code_line(tokens, start)
}

/// Emit a `ROXYGEN_MD_INDENTED_CODE` node spanning the indented code block
/// beginning at `start` (a `RoxygenMarker` whose content is a `RoxygenMdIndentCode`
/// leaf). The node gathers the opening line, following indented-code lines, and any
/// **interior** blank lines (a blank line joins the block only when a later line is
/// another code line — CommonMark keeps interior blanks but drops trailing ones).
/// A tag opener, a non-indented prose line, or a non-roxygen line ends the block;
/// the trailing newline (and any trailing blank lines) after the last code line are
/// left to the caller. The `#'` markers, marker->content whitespace, and inter-line
/// newlines/indentation are threaded in as trivia (losslessness), the way the fenced
/// code block threads them. Returns the token index just past the last code line.
pub(super) fn emit_md_indented_code(
    tokens: &[Token],
    start: usize,
    events: &mut Vec<Event>,
) -> usize {
    debug_assert_eq!(tokens[start].kind, TokKind::RoxygenMarker);
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_INDENTED_CODE));

    // Opening line: marker, marker->content whitespace, the code leaf.
    events.push(Event::Tok(start));
    let mut i = start + 1;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }

    finish_md_indented_code(tokens, i, events)
}

/// Whether a prose tag's same-line value opens a markdown **indented code
/// block**: the block is `@md` (threaded from the block builder — indented code
/// has no mode-carrying leaf), and the whitespace run between the tag head and
/// the value is five or more all-space columns (roxygen2 strips only the single
/// separator space after the tag head, so four further columns reach CommonMark's
/// indented-code threshold — the from-value analog of [`is_indent_code_line`]).
/// The value position is always a fresh block position (the tag's markdown
/// document starts there), so no paragraph gate applies.
pub(super) fn is_md_indented_code_value(tokens: &[Token], value_start: usize, md: bool) -> bool {
    if !md || value_start == 0 {
        return false;
    }
    let ws = &tokens[value_start - 1];
    ws.kind == TokKind::Whitespace && ws.text.len() >= 5 && ws.text.bytes().all(|b| b == b' ')
}

/// Emit a `ROXYGEN_MD_INDENTED_CODE` node for an indented code block opening as a
/// **tag's same-line value** (`#' @details      x`): the first line has no `#'`
/// marker of its own (that marker belongs to the enclosing tag, already emitted
/// and closed), so it starts at `ws_start` — the >= 5-column whitespace between
/// the tag head and the value. The following lines gather exactly as in
/// [`emit_md_indented_code`]. Returns the token index just past the last code
/// line.
pub(super) fn emit_md_indented_code_from_value(
    tokens: &[Token],
    ws_start: usize,
    events: &mut Vec<Event>,
) -> usize {
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_INDENTED_CODE));
    // First (marker-less) line: the leading whitespace and the code content.
    let mut i = ws_start;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }
    finish_md_indented_code(tokens, i, events)
}

/// Gather an indented code block's continuation lines (following code lines and
/// interior blanks) after its opening line, then finish the
/// `ROXYGEN_MD_INDENTED_CODE` node. `i` is at the opening line's trailing
/// `Newline`. Shared by the line-start ([`emit_md_indented_code`]) and tag-value
/// ([`emit_md_indented_code_from_value`]) forms.
fn finish_md_indented_code(tokens: &[Token], mut i: usize, events: &mut Vec<Event>) -> usize {
    loop {
        // `i` is at the trailing `Newline` of the last emitted code line. Scan
        // forward across zero or more blank lines to the next code line; a blank run
        // only joins the block when a code line follows it.
        if tokens.get(i).map(|t| &t.kind) != Some(&TokKind::Newline) {
            break;
        }
        let mut probe = i; // at a `Newline`
        let code_end = loop {
            let mut m = probe + 1;
            while tokens.get(m).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
                m += 1;
            }
            if tokens.get(m).map(|t| &t.kind) != Some(&TokKind::RoxygenMarker) {
                break None; // a non-roxygen line ends the block
            }
            if is_indent_code_line(tokens, m) {
                let mut e = m + 1;
                while tokens.get(e).is_some_and(|t| is_line_body_kind(&t.kind)) {
                    e += 1;
                }
                break Some(e);
            }
            if matches!(classify_line(tokens, m), LineKind::Blank) {
                // A blank line: tentatively part of the block; keep scanning past it.
                let mut e = m + 1;
                while tokens.get(e).is_some_and(|t| is_line_body_kind(&t.kind)) {
                    e += 1;
                }
                if tokens.get(e).map(|t| &t.kind) != Some(&TokKind::Newline) {
                    break None; // a trailing blank line at EOF is not in the block
                }
                probe = e;
            } else {
                break None; // a tag or non-indented prose line ends the block
            }
        };
        match code_end {
            // Thread the intervening trivia (newlines, continuation indentation,
            // interior blank-line markers) and the code line's tokens into the node.
            Some(end) => {
                for idx in i..end {
                    events.push(Event::Tok(idx));
                }
                i = end;
            }
            None => break,
        }
    }

    events.push(Event::Finish); // ROXYGEN_MD_INDENTED_CODE
    i
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

    finish_md_code_block(tokens, i, events)
}

/// Emit a `ROXYGEN_MD_CODE_BLOCK` node for a fenced code block opening as a
/// **tag's same-line value** (`#' @details ```r`): the first line has no `#'`
/// marker of its own (that marker belongs to the enclosing tag, already emitted
/// and closed), so it starts at `ws_start` — the whitespace between the tag head
/// and the opener fence leaf. The following lines gather exactly as in
/// [`emit_md_code_block`]. Returns the token index just past the last consumed
/// line's content.
pub(super) fn emit_md_code_block_from_value(
    tokens: &[Token],
    ws_start: usize,
    events: &mut Vec<Event>,
) -> usize {
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_CODE_BLOCK));
    // First (marker-less) line: the leading whitespace and the opener fence.
    let mut i = ws_start;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }
    finish_md_code_block(tokens, i, events)
}

/// Gather a fenced code block's lines (after its opening line) up to and
/// including the closing fence, then finish the `ROXYGEN_MD_CODE_BLOCK` node.
/// `i` is at the opening line's trailing `Newline`. Shared by the line-start
/// ([`emit_md_code_block`]) and tag-value ([`emit_md_code_block_from_value`])
/// forms.
fn finish_md_code_block(tokens: &[Token], mut i: usize, events: &mut Vec<Event>) -> usize {
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

/// Whether the roxygen line whose marker is at `start` is a CommonMark **HTML
/// block start condition 7** line: a complete standalone tag — its content is
/// exactly one inline `RoxygenMdHtml` *tag* leaf (open, closing, or self-closing
/// form) followed by nothing but whitespace. The leaf is carved only under a
/// resolved `@md` mode, so its presence is the single mode signal.
///
/// Conditions 1–6 claim their openers in the lexer at line-content start (a
/// `RoxygenMdHtmlBlock` leaf), so a content-start inline-HTML *tag* leaf is
/// exactly the condition-7 candidate set — the engine applies no tag-name
/// exclusion to the closing/self-closing forms (`</pre>` and `<pre/>` both
/// open). A content-start `RoxygenMdHtml` leaf is always a tag (the block
/// scanner claims the comment/PI/CDATA/declaration forms first), but the tag
/// shape is checked explicitly to keep the invariant local.
///
/// Condition 7 **cannot interrupt a paragraph** — callers gate on the open
/// paragraph. In the engine the gate is positional (cmark blocks condition 7
/// only when the deepest *matched* container is an open paragraph), so a
/// standalone-tag line directly continuing a paragraph folds as a lazy
/// continuation, while the same line after a block-quote or list line — where
/// the container match already failed — opens a block.
pub(super) fn is_md_html_block7_line(tokens: &[Token], start: usize) -> bool {
    is_md_html_block7_at(tokens, line_content_start(tokens, start))
}

/// [`is_md_html_block7_line`] with the line-content position already resolved:
/// whether the tokens at `content` form a complete standalone tag with nothing
/// but whitespace to the end of the line. Shared with the tag-value form
/// ([`is_md_html_block_value`]), where the "line" starts at the tag's value.
fn is_md_html_block7_at(tokens: &[Token], content: usize) -> bool {
    let Some(tok) = tokens.get(content) else {
        return false;
    };
    if tok.kind != TokKind::RoxygenMdHtml {
        return false;
    }
    let bytes = tok.text.as_bytes();
    if bytes.first() != Some(&b'<')
        || !bytes
            .get(1)
            .is_some_and(|&b| b == b'/' || b.is_ascii_alphabetic())
    {
        return false;
    }
    tokens[content + 1..]
        .iter()
        .take_while(|t| is_line_body_kind(&t.kind))
        .all(|t| t.kind == TokKind::Whitespace)
}

/// Whether a tag's same-line value at `value_start` (its first Content token)
/// opens a **markdown HTML block**: a `RoxygenMdHtmlBlock` opener leaf
/// (conditions 1–6, carved by the lexer at the value position of a prose tag
/// under `@md`), or a **condition-7** standalone complete tag — a single inline
/// `RoxygenMdHtml` tag leaf with nothing but whitespace to the end of the line.
/// Condition 7 cannot interrupt a paragraph, but a tag's value starts its own
/// markdown document, so the value position is always fresh and the block opens
/// (engine-probed). The condition-7 arm re-applies the lexer's indent gate:
/// roxygen2 strips only the single separator space after the tag head, so a
/// value >= 4 columns past it is an indented code block, not an HTML block
/// (from-value indented code is backlog).
pub(super) fn is_md_html_block_value(tokens: &[Token], value_start: usize) -> bool {
    match tokens.get(value_start).map(|t| &t.kind) {
        Some(TokKind::RoxygenMdHtmlBlock) => true,
        Some(TokKind::RoxygenMdHtml) => {
            let indent_ok = value_start == 0
                || tokens[value_start - 1].kind != TokKind::Whitespace
                || tokens[value_start - 1].text.len() <= 4;
            indent_ok && is_md_html_block7_at(tokens, value_start)
        }
        _ => false,
    }
}

/// Whether an HTML-block opener's line content begins (case-insensitively) with a
/// CommonMark **condition 1** verbatim tag (`<pre`/`<script`/`<style`/`<textarea`)
/// followed by a boundary (whitespace, `>`, or the end of the line — **not** `/`:
/// a self-closing `<pre/>` is condition 7, blank-terminated, engine-probed).
/// Mirrors the lexer's [`super::lex::scan_md_html_block`] verbatim branch; the
/// opener leaf starts at the tag (leading marker→content whitespace is stripped by
/// [`line_raw_content`]).
fn is_html_verbatim_opener(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    super::lex::HTML_VERBATIM_TAGS.iter().any(|tag| {
        lower
            .strip_prefix('<')
            .and_then(|rest| rest.strip_prefix(tag))
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t', '>']))
    })
}

/// The condition-1 verbatim close tags (`</pre>` etc., case-insensitive — per
/// CommonMark the closer need not match the opening tag).
const HTML_VERBATIM_CLOSERS: &[&str] = &["</pre>", "</script>", "</style>", "</textarea>"];

/// The line-containing-closer terminator strings for a **terminator-based** HTML
/// block (CommonMark start conditions 1–5), re-derived from the opener text, or
/// `None` for a **blank-line-terminated** block (condition 6). The closers are
/// matched case-insensitively via [`html_line_contains_closer`].
fn html_block_closers(opener: &str) -> Option<&'static [&'static str]> {
    let lower = opener.to_ascii_lowercase();
    if lower.starts_with("<!--") {
        Some(&["-->"]) // condition 2 (comment)
    } else if lower.starts_with("<![cdata[") {
        Some(&["]]>"]) // condition 5 (CDATA)
    } else if lower.starts_with("<?") {
        Some(&["?>"]) // condition 3 (processing instruction)
    } else if lower
        .strip_prefix("<!")
        .is_some_and(|rest| rest.as_bytes().first().is_some_and(u8::is_ascii_alphabetic))
    {
        Some(&[">"]) // condition 4 (declaration)
    } else if is_html_verbatim_opener(opener) {
        Some(HTML_VERBATIM_CLOSERS) // condition 1 (verbatim tag)
    } else {
        None // condition 6 (blank-line terminated)
    }
}

/// Whether a terminator-based HTML block line **contains** one of `closers`
/// (case-insensitive). The block ends on the first such line, inclusive.
fn html_line_contains_closer(content: &str, closers: &[&str]) -> bool {
    let lower = content.to_ascii_lowercase();
    closers.iter().any(|c| lower.contains(c))
}

/// Emit a `ROXYGEN_MD_HTML_BLOCK` node spanning the markdown HTML block beginning
/// at `start` (a `RoxygenMarker` whose content is a `RoxygenMdHtmlBlock` opener,
/// or a standalone inline-tag line for condition 7 — [`is_md_html_block7_line`]).
/// The `#'` markers, the marker→content whitespace, and the inter-line newlines/
/// indentation are threaded in as trivia at the block level, the way the fenced
/// code block threads them. The trailing newline after the last consumed line is
/// left to the caller. Returns the token index just past it.
///
/// The block's **terminator** depends on the opener's CommonMark HTML-block start
/// condition, re-derived here from the opener text (the leaf already implies `@md`;
/// re-deriving the *condition* is not re-deriving the mode):
///
/// * **Conditions 1–5** ([`html_block_closers`] returns the closer set): the block
///   runs until a line **containing** one of its closer strings (`</pre>` etc. /
///   `-->` / `?>` / `>` / `]]>`, case-insensitive, inclusive) — through blank
///   lines. A new tag (section boundary) or a non-roxygen line/EOF also ends it. If
///   the opener line already contains the closer, the block is that single line.
/// * **Conditions 6 and 7** (block-level tag / standalone complete tag —
///   [`html_block_closers`] returns `None` for both): the block runs to the next
///   **blank line**; a tag opener or a non-roxygen line also ends it.
pub(super) fn emit_md_html_block(tokens: &[Token], start: usize, events: &mut Vec<Event>) -> usize {
    debug_assert_eq!(tokens[start].kind, TokKind::RoxygenMarker);
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_HTML_BLOCK));

    // Opening line: marker, marker→content whitespace, then the opener content.
    let opener = line_raw_content(tokens, start);
    events.push(Event::Tok(start));
    let mut i = start + 1;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }

    finish_md_html_block(tokens, i, &opener, events)
}

/// Emit a `ROXYGEN_MD_HTML_BLOCK` node for an HTML block opening as a **tag's
/// same-line value** ([`is_md_html_block_value`]): the first line has no `#'`
/// marker of its own (that marker belongs to the enclosing tag, already emitted
/// and closed), so it starts at `ws_start` — the whitespace between the tag head
/// and the value. roxygen2 strips only the single separator space after the tag
/// head, so any further indent is part of the block's first rendered line and
/// stays inside the node. The following lines gather per the opener's start
/// condition exactly as in [`emit_md_html_block`]. Returns the token index just
/// past the last consumed line's content.
pub(super) fn emit_md_html_block_from_value(
    tokens: &[Token],
    ws_start: usize,
    events: &mut Vec<Event>,
) -> usize {
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_HTML_BLOCK));
    // First (marker-less) line: the leading whitespace and the value content. The
    // opener text for the terminator decision starts at the first non-whitespace
    // token (the closer-set prefixes match from `<`).
    let mut opener = String::new();
    let mut i = ws_start;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        if !(opener.is_empty() && tokens[i].kind == TokKind::Whitespace) {
            opener.push_str(&tokens[i].text);
        }
        events.push(Event::Tok(i));
        i += 1;
    }
    finish_md_html_block(tokens, i, &opener, events)
}

/// Gather an HTML block's continuation lines (after its opening line, whose
/// content is `opener`) per the opener's start condition, then finish the
/// `ROXYGEN_MD_HTML_BLOCK` node. `i` is at the opening line's trailing
/// `Newline`. Shared by the line-start ([`emit_md_html_block`]) and tag-value
/// ([`emit_md_html_block_from_value`]) forms.
fn finish_md_html_block(
    tokens: &[Token],
    mut i: usize,
    opener: &str,
    events: &mut Vec<Event>,
) -> usize {
    if let Some(closers) = html_block_closers(opener) {
        // Conditions 1–5: run until a line containing a closer, inclusive (through
        // blank lines). Skip the loop when the opener line already closes.
        if !html_line_contains_closer(opener, closers) {
            loop {
                if tokens.get(i).map(|t| &t.kind) != Some(&TokKind::Newline) {
                    break;
                }
                let mut m = i + 1;
                while tokens.get(m).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
                    m += 1;
                }
                if tokens.get(m).map(|t| &t.kind) != Some(&TokKind::RoxygenMarker) {
                    break; // non-roxygen line / EOF
                }
                if matches!(classify_line(tokens, m), LineKind::Tag) {
                    break; // a new tag (section boundary) ends the block
                }
                // Thread `\n` + indentation + `#'`, then the line's body.
                let line = line_raw_content(tokens, m);
                for idx in i..=m {
                    events.push(Event::Tok(idx));
                }
                i = m + 1;
                while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
                    events.push(Event::Tok(i));
                    i += 1;
                }
                if html_line_contains_closer(&line, closers) {
                    break;
                }
            }
        }
        events.push(Event::Finish); // ROXYGEN_MD_HTML_BLOCK
        return i;
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

/// Whether the prose line whose marker is at `start` is an ATX **heading**
/// (`@md` mode): its content begins with a `RoxygenMdHeading` leaf. The leaf is
/// carved only under a resolved `@md` mode, so its presence is the single mode
/// signal (the builder never re-derives mode).
pub(super) fn is_md_heading_start(tokens: &[Token], start: usize) -> bool {
    let content = line_content_start(tokens, start);
    tokens.get(content).map(|t| &t.kind) == Some(&TokKind::RoxygenMdHeading)
}

/// Emit a single-line `ROXYGEN_MD_HEADING` node for the ATX heading whose marker
/// is at `start`: the `#'` marker, the marker→content whitespace, and the verbatim
/// heading leaf, threaded in as its children. A heading is exactly one line
/// (unlike the HTML block / table, which gather following lines), so the trailing
/// newline is left to the caller. Returns the token index just past the heading
/// content.
pub(super) fn emit_md_heading(tokens: &[Token], start: usize, events: &mut Vec<Event>) -> usize {
    debug_assert_eq!(tokens[start].kind, TokKind::RoxygenMarker);
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_HEADING));
    events.push(Event::Tok(start));
    let mut i = start + 1;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }
    events.push(Event::Finish); // ROXYGEN_MD_HEADING
    i
}

/// Emit a single-line `ROXYGEN_MD_HEADING` node for an ATX heading opening as a
/// **tag's same-line value** (`#' @details # Title`): the line has no `#'`
/// marker of its own (that marker belongs to the enclosing tag, already emitted
/// and closed), so it starts at `ws_start` — the whitespace between the tag head
/// and the `RoxygenMdHeading` leaf. Returns the token index just past the
/// heading content.
pub(super) fn emit_md_heading_from_value(
    tokens: &[Token],
    ws_start: usize,
    events: &mut Vec<Event>,
) -> usize {
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_HEADING));
    let mut i = ws_start;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }
    events.push(Event::Finish); // ROXYGEN_MD_HEADING
    i
}

/// Whether the roxygen line whose marker is at `start` is a **setext heading
/// underline** line — its first content token is a `RoxygenMdSetextUnderline` leaf
/// (carved only under a resolved `@md` mode, so the builder never re-derives mode).
pub(super) fn is_md_setext_underline_line(tokens: &[Token], start: usize) -> bool {
    let content = line_content_start(tokens, start);
    tokens.get(content).map(|t| &t.kind) == Some(&TokKind::RoxygenMdSetextUnderline)
}

/// Whether the line whose marker is at `start` is a **lone dash bullet** (`-`/`- `)
/// with no item content — a `RoxygenMdListMarker` leaf whose text is a single `-`
/// followed only by trailing whitespace. CommonMark resolves such a line, when it
/// *follows a paragraph*, as a level-2 setext underline (an empty list item cannot
/// interrupt a paragraph), so the dash bullet the lexer carved as a list marker
/// serves here as an underline. Restricted to `-` (a `*`/`+` empty bullet is never
/// a setext underline); the projector reads the level 2 from the leaf text (`-`).
fn is_md_setext_dash_underline(tokens: &[Token], start: usize) -> bool {
    let content = line_content_start(tokens, start);
    tokens.get(content).is_some_and(|t| {
        t.kind == TokKind::RoxygenMdListMarker
            && t.text == "-"
            && md_list_item_is_empty(tokens, content)
    })
}

/// Whether the line whose marker is at `start` can serve as a **setext H2/H1
/// underline**: a genuine `===`/`---` underline leaf, or a lone dash bullet
/// ([`is_md_setext_dash_underline`]). Used only by the setext-heading look-back and
/// emit, both reached solely from a paragraph open — at a fresh block position the
/// same dash bullet still opens an empty list (the block loop's list check runs
/// first), so this never mis-fires on a list.
pub(super) fn is_md_setext_underline_or_dash(tokens: &[Token], start: usize) -> bool {
    is_md_setext_underline_line(tokens, start) || is_md_setext_dash_underline(tokens, start)
}

/// Whether the prose line whose marker is at `start` opens a **setext heading**:
/// its paragraph — the maximal run of foldable prose continuation lines — is
/// terminated *immediately* by a setext underline line. A setext underline heads
/// nothing on its own, so the current line must carry prose (not be the underline).
/// The whole preceding paragraph becomes the heading text; this block-level
/// look-back is what distinguishes a setext H2 (`para` then `---`) from a thematic
/// break (`---` after a blank). Only called at a paragraph open, so the run scanned
/// here is exactly the paragraph the grouper would otherwise build.
pub(super) fn is_md_setext_heading_start(tokens: &[Token], start: usize) -> bool {
    if is_md_setext_underline_or_dash(tokens, start) {
        return false;
    }
    let mut line = start;
    loop {
        let Some(next) = super::group::next_roxygen_line_marker(tokens, line) else {
            return false;
        };
        if is_md_setext_underline_or_dash(tokens, next) {
            return true;
        }
        if super::group::is_foldable_continuation(tokens, next) {
            line = next;
            continue;
        }
        return false;
    }
}

/// Emit a `ROXYGEN_MD_HEADING` node for a **setext heading**: the preceding prose
/// paragraph (one or more `#'` lines) plus its `===`/`---` underline line. The
/// `#'` markers, marker->content whitespace, and inter-line newlines are threaded
/// in as trivia leaves; the trailing newline after the underline is left to the
/// caller. Returns the token index just past the underline line's content.
///
/// The projector reads the level from the underline (`=` -> 1, `-` -> 2) and the
/// title from the prose lines before it; the formatter emits each line
/// marker-normalized (`emit_md_heading`), so the pair reparses to the same heading
/// (idempotent). Reuses the ATX heading's node kind — both are `ROXYGEN_MD_HEADING`.
pub(super) fn emit_md_setext_heading(
    tokens: &[Token],
    start: usize,
    events: &mut Vec<Event>,
) -> usize {
    debug_assert_eq!(tokens[start].kind, TokKind::RoxygenMarker);
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_HEADING));
    let mut marker = start;
    loop {
        events.push(Event::Tok(marker)); // RoxygenMarker
        let mut i = marker + 1;
        while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
            events.push(Event::Tok(i));
            i += 1;
        }
        if is_md_setext_underline_or_dash(tokens, marker) {
            events.push(Event::Finish); // ROXYGEN_MD_HEADING
            return i;
        }
        // Not the underline yet: thread the inter-line trivia (newline + continuation
        // indentation) and advance. `is_md_setext_heading_start` guaranteed the run
        // terminates in an underline, so a next line always exists.
        let next = super::group::next_roxygen_line_marker(tokens, marker)
            .expect("setext heading run terminates in an underline");
        for idx in i..next {
            events.push(Event::Tok(idx));
        }
        marker = next;
    }
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
    emit_block_macro_from_opener(tokens, i, events)
}

/// Emit a block Rd macro whose `\name{` opener appears **mid-prose** (not as the
/// line's first content). The enclosing `ROXYGEN_PARAGRAPH` stays open, so the
/// macro nests inside it as an inline sibling of the preceding prose (the way the
/// projector folds an abutting block macro into the same section). `opener` must
/// index the `\name{…` opener token; unlike [`emit_block_macro`] there is no
/// leading marker to thread (it belongs to the prose that precedes the opener).
pub(super) fn emit_block_macro_inline(
    tokens: &[Token],
    opener: usize,
    events: &mut Vec<Event>,
) -> usize {
    events.push(Event::Start(SyntaxKind::ROXYGEN_RD_MACRO));
    emit_block_macro_from_opener(tokens, opener, events)
}

/// Emit the body of a `ROXYGEN_RD_MACRO` (already `Start`ed) from its opener token
/// at `i`, consuming following `#'` lines until the group closes (or a tag / block
/// end terminates it), and `Finish` the node. Shared by the line-start
/// [`emit_block_macro`] and the mid-prose [`emit_block_macro_inline`].
fn emit_block_macro_from_opener(tokens: &[Token], mut i: usize, events: &mut Vec<Event>) -> usize {
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
