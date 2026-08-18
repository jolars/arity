//! Roxygen structure building: the block-level Rd-macro and markdown machinery.
//!
//! The *third* phase, dispatched from [`super::group`]: it recognizes and emits
//! the constructs that span several `#'` lines — block Rd macros
//! (`\itemize{…}`, `\describe{…}`, `\tabular{…}{…}`) and markdown lists — as
//! direct `ROXYGEN_SECTION` children, threading the inter-line `#'`/newline/
//! indentation trivia in losslessly.

use super::group::{LineKind, classify_line, is_line_body_kind, line_content_start};
use super::{
    advance_md_col, is_block_rd_macro, is_multi_arg_rd_macro, md_fence_run_closes, md_ws_gauge,
    scan_balanced, utf8_len,
};
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
///
/// Either shape must additionally name a **block-level** macro
/// ([`is_block_rd_macro`]). Spanning lines is not what makes a macro a block: an
/// inline `\code{…}`/`\href{…}{…}` the author soft-wrapped spans lines too, and
/// promoting it here would make it a section-level sibling purely because its
/// opener landed at a line start, while the same macro wrapped mid-prose stays
/// inline markup ([`super::group::emit_prose_rest`]). It falls through to the
/// prose path instead, which builds the identical inline `ROXYGEN_RD_MACRO`
/// either way (Tenet 1).
pub(super) fn is_block_macro_line(tokens: &[Token], start: usize) -> bool {
    let content = line_content_start(tokens, start);
    match tokens.get(content) {
        Some(tok) if tok.kind == TokKind::RoxygenText => {
            is_block_macro_opener(tok.text) && names_block_rd_macro(tok.text)
        }
        Some(tok) => is_form_b_block_macro(tokens, content) && names_block_rd_macro(tok.text),
        None => false,
    }
}

/// Whether `text`, a `\name…` span, names a block-level Rd macro.
fn names_block_rd_macro(text: &str) -> bool {
    rd_macro_name(text).is_some_and(is_block_rd_macro)
}

/// Whether the token at `i` begins a **Form B** block macro: a *balanced*
/// `RoxygenRdMacro` token for a multi-argument macro (`\tabular{format}`,
/// `\item{term}`, `\ifelse{html}{yes}`) immediately followed by a `RoxygenText`
/// opening an unbalanced `{` — the macro's next argument spans following `#'`
/// lines (the token carries the ones that fit on the opener line). Shared by the
/// line-start gate ([`is_block_macro_line`]) and the mid-body one
/// ([`emit_block_macro_from_opener`]), where the same shape appears nested
/// inside an enclosing block macro's body (`\item{a}{def …}` in a `\describe`),
/// and the mid-prose one ([`super::group::emit_prose_rest`]).
pub(super) fn is_form_b_block_macro(tokens: &[Token], i: usize) -> bool {
    tokens.get(i).is_some_and(|tok| {
        tok.kind == TokKind::RoxygenRdMacro
            && rd_macro_name(tok.text).is_some_and(is_multi_arg_rd_macro)
    }) && matches!(
        tokens.get(i + 1),
        Some(next) if next.kind == TokKind::RoxygenText && opens_unbalanced_brace(next.text)
    )
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
                    if brace_scan(tok.text, &mut depth) {
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
/// start number is 1, and never at four or more columns of indentation: such
/// a line is would-be indented code, which cannot interrupt a paragraph, so
/// the marker is lazy paragraph text). A marker that fails the gate stays
/// inline prose (its `RoxygenMdListMarker` leaf renders as literal text).
pub(super) fn is_md_list_start(tokens: &[Token], start: usize, para_open: bool) -> bool {
    let content = line_content_start(tokens, start);
    match tokens.get(content) {
        Some(tok) if tok.kind == TokKind::RoxygenMdListMarker => {
            !para_open
                || (md_list_marker_can_interrupt(tok.text)
                    && !md_list_item_is_empty(tokens, content)
                    && !is_indent_code_line(tokens, start))
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

/// Whether a list item's content at `i` opens a **same-line nested list**: an
/// all-whitespace prose run (the marker→marker separator the lexer carved in
/// `carve_md_list_markers`) followed by a `RoxygenMdListMarker` leaf.
fn is_same_line_sublist(tokens: &[Token], i: usize) -> bool {
    is_same_line_child(tokens, i, &TokKind::RoxygenMdListMarker)
}

/// Whether a list item's content at `i` opens a **same-line block quote**
/// (`- > quoted`, cm-294/295): the separator run followed by a
/// `RoxygenMdBlockQuote` leaf the lexer carved in `carve_md_list_markers`.
fn is_same_line_quote(tokens: &[Token], i: usize) -> bool {
    is_same_line_child(tokens, i, &TokKind::RoxygenMdBlockQuote)
}

/// Whether a list item's content at `i` opens a **same-line ATX heading**
/// (`- # Foo`, cm-302): the separator run followed by a `RoxygenMdHeading`
/// leaf the lexer carved in `carve_md_list_markers`.
fn is_same_line_heading(tokens: &[Token], i: usize) -> bool {
    is_same_line_child(tokens, i, &TokKind::RoxygenMdHeading)
}

/// Whether a list item's content at `i` opens a **same-line fenced code block**
/// (`- ```` ``` ````, cm-320/326): the separator run followed by a
/// `RoxygenMdFence` leaf the lexer carved in `carve_md_list_markers`.
fn is_same_line_fence(tokens: &[Token], i: usize) -> bool {
    is_same_line_child(tokens, i, &TokKind::RoxygenMdFence)
}

/// A thematic break opening at a list item's content start on the marker line
/// (`- * * *`, cm-061), carved by the lexer as a `RoxygenMdThematicBreak` leaf
/// past the separator run.
fn is_same_line_break(tokens: &[Token], i: usize) -> bool {
    is_same_line_child(tokens, i, &TokKind::RoxygenMdThematicBreak)
}

/// Whether the item content at `i` opens with a **same-line HTML block**: a
/// `RoxygenMdHtmlBlock` opener leaf the lexer carved in `carve_md_list_markers`.
fn is_same_line_html_block(tokens: &[Token], i: usize) -> bool {
    is_same_line_child(tokens, i, &TokKind::RoxygenMdHtmlBlock)
}

/// Whether the token at `i` is a marker→child all-whitespace separator run
/// followed by a leaf of `kind` — the shape `carve_md_list_markers` produces for
/// a child block starting on the item's marker line.
fn is_same_line_child(tokens: &[Token], i: usize, kind: &TokKind) -> bool {
    tokens.get(i).is_some_and(|t| {
        is_line_body_kind(&t.kind)
            && !t.text.is_empty()
            && t.text.chars().all(|c| c == ' ' || c == '\t')
    }) && tokens.get(i + 1).map(|t| &t.kind) == Some(kind)
}

/// The indentation (in columns) of a list line whose `RoxygenMarker` is at
/// `marker`: the one-based gauge of the `#'`→content whitespace — after the
/// `#'` sigil and the one whitespace character roxygen2 strips, the rest
/// expands with 4-column tab stops ([`md_ws_gauge`]) into what CommonMark uses
/// to decide list nesting and indented-code thresholds (cm-001/002/008/009).
fn list_line_indent(tokens: &[Token], marker: usize) -> usize {
    let mut k = marker + 1;
    let mut texts = Vec::new();
    while tokens.get(k).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
        texts.push(tokens[k].text);
        k += 1;
    }
    md_ws_gauge(texts)
}

/// The number of leading whitespace **columns** of a list item's content (the
/// first body token after its marker), clamped to CommonMark's 1..=4: a child
/// block must be indented to at least `marker_indent + marker_width + this` to
/// nest. `start_col` is the value column just past the marker (the marker
/// line's gauge minus one, plus the marker width) — the anchor tab stops are
/// measured from ([`advance_md_col`]; a separator tab spans to the next
/// 4-column stop, cm-007).
///
/// Two CommonMark start conditions snap this to **one** instead: an item whose
/// first line has no content after the marker (its content, if any, starts on
/// the next line — cm-280/281), and an item whose content sits five or more
/// columns past the marker (the content then *starts with indented code*, and
/// only one column belongs to the item separator — cm-275/276).
fn content_leading_spaces(tokens: &[Token], content: usize, start_col: usize) -> usize {
    let Some(first) = tokens.get(content).filter(|t| is_line_body_kind(&t.kind)) else {
        return 1;
    };
    let mut col = start_col;
    for c in first.text.chars().take_while(|c| *c == ' ' || *c == '\t') {
        col = advance_md_col(col, c);
    }
    let leading = col - start_col;
    let mut k = content;
    let mut has_content = false;
    while let Some(t) = tokens.get(k).filter(|t| is_line_body_kind(&t.kind)) {
        if !t.text.trim().is_empty() {
            has_content = true;
            break;
        }
        k += 1;
    }
    if !has_content || leading >= 5 {
        return 1;
    }
    leading.clamp(1, 4)
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

/// From `i` (expected at a line's trailing `Newline`), the index of the next
/// list line's `RoxygenMarker` when it is separated from the current position
/// only by **blank** roxygen lines: CommonMark does not end a list at a blank
/// line when another list line follows — the blank only makes the list *loose*,
/// a distinction roxygen2's Rd rendering ignores (a loose and a tight list
/// render the same `\itemize`). Returns `None` when no blank intervenes (the
/// immediate case is [`next_list_line`]'s — keeping the two disjoint means
/// blanks are consumed only when a list line actually follows them) or when
/// the first non-blank line is not a list line (the blanks then end the list
/// and stay with the enclosing section).
fn next_list_line_across_blanks(tokens: &[Token], i: usize) -> Option<usize> {
    let mut j = i;
    let mut crossed = false;
    loop {
        let m = following_line_marker(tokens, j)?;
        if matches!(classify_line(tokens, m), LineKind::Blank) {
            crossed = true;
            j = line_content_end(tokens, m);
            continue;
        }
        return (crossed && is_md_list_continuation(tokens, m)).then_some(m);
    }
}

/// From `i` (expected at a line's trailing `Newline`), the next roxygen line's
/// `RoxygenMarker` when it is a **paragraph-continuation** of a list item
/// reached only across one or more **blank** roxygen lines. A blank line closes
/// an item's open paragraph, but the item continues: a subsequent line indented
/// to (or past) the item's content column opens a *new* paragraph inside the
/// same item (a loose list item), which Rd rendering flattens into the item text
/// (`- a` / blank / `  more` → item text `a more`, engine-probed). Requires at
/// least one intervening blank (the no-blank lazy case is the direct fold loop),
/// a following non-blank line that opens no block
/// ([`is_md_item_lazy_continuation`] — a list marker nests/siblings instead, a
/// block opener is out of scope), and `None` otherwise. The indent test is the
/// caller's (a below-content-column line ends the item).
fn next_prose_line_across_blanks(tokens: &[Token], i: usize) -> Option<usize> {
    let mut j = i;
    let mut crossed = false;
    loop {
        let m = following_line_marker(tokens, j)?;
        if matches!(classify_line(tokens, m), LineKind::Blank) {
            crossed = true;
            j = line_content_end(tokens, m);
            continue;
        }
        return (crossed && is_md_item_lazy_continuation(tokens, m)).then_some(m);
    }
}

/// From `i` (expected at a line's trailing `Newline`), the next **non-blank**
/// roxygen line's `RoxygenMarker`, crossing any number of intervening **blank**
/// lines; `None` at a non-roxygen line / EOF. Unlike
/// [`next_prose_line_across_blanks`] this places no requirement on the line's
/// kind and does not require a blank to intervene — a caller inspects the
/// returned line itself (e.g. a child block start at the item's content column,
/// which folds into the item with or without a separating blank).
fn next_content_line(tokens: &[Token], i: usize) -> Option<usize> {
    let mut j = i;
    loop {
        let m = following_line_marker(tokens, j)?;
        if matches!(classify_line(tokens, m), LineKind::Blank) {
            j = line_content_end(tokens, m);
            continue;
        }
        return Some(m);
    }
}

/// From `i` (expected at a line's trailing `Newline`), the next **non-blank**
/// roxygen line's `RoxygenMarker` reached across one or more **blank** lines —
/// like [`next_content_line`] but *requiring* at least one intervening blank.
/// A blank line closes a list item's open paragraph, which an indented code
/// block folded into the item needs: a CommonMark indented code block cannot
/// interrupt a paragraph, so a no-blank over-indented line is a lazy
/// continuation instead. `None` when no blank intervenes or the block ends
/// first. The indent test is the caller's.
fn next_content_line_across_blanks(tokens: &[Token], i: usize) -> Option<usize> {
    let mut j = i;
    let mut crossed = false;
    loop {
        let m = following_line_marker(tokens, j)?;
        if matches!(classify_line(tokens, m), LineKind::Blank) {
            crossed = true;
            j = line_content_end(tokens, m);
            continue;
        }
        return crossed.then_some(m);
    }
}

/// The list-*type* discriminant of a `RoxygenMdListMarker`'s text: the bullet
/// character itself (`-`/`*`/`+`), or the ordered delimiter (`.`/`)`).
/// CommonMark items belong to the same list only when this matches — changing
/// the bullet char or the ordered delimiter starts a new list, while the start
/// number is irrelevant (`1.` … `5.` is one list, engine-probed).
fn md_list_marker_type(marker: &str) -> u8 {
    match marker.as_bytes().first() {
        Some(c @ (b'-' | b'*' | b'+')) => *c,
        _ => *marker.as_bytes().last().unwrap_or(&b'.'),
    }
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
        s.push_str(tok.text);
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

/// Whether the line whose marker is at `marker` is a **lazy paragraph
/// continuation of a list item**: ordinary prose that opens no block construct,
/// so CommonMark folds it into the item's open paragraph — even unindented
/// ("lazy") or indented past the content column. Mirrors
/// [`is_foldable_continuation`](super::group::is_foldable_continuation) with
/// three item-specific differences, all engine-probed:
///   * **any** list-marker line is not lazy — inside a list it is the next
///     item, even an empty `-` (which mid-paragraph could not *start* a list)
///     becomes an empty sibling item;
///   * a **setext underline folds** (`- a` then `===` renders `a ===`): the
///     underline cannot apply across the container boundary, and a bare `===`
///     opens no other block. A `---` stays excluded — in this position it is a
///     thematic break, which interrupts;
///   * a **table header folds** (`- a` then `| x | y |` + delimiter renders as
///     item text): a GFM table cannot interrupt a paragraph.
///
/// A standalone-tag line (HTML block condition 7) is not lazy either: the
/// container match already failed at the item's content column, so condition 7
/// opens a block after a list item (the same positional gate as the table and
/// block-quote gathers).
fn is_md_item_lazy_continuation(tokens: &[Token], marker: usize) -> bool {
    matches!(classify_line(tokens, marker), LineKind::Prose)
        && !is_md_list_continuation(tokens, marker)
        && !is_md_html_block_start(tokens, marker)
        && !is_md_html_block7_line(tokens, marker)
        && !is_md_code_block_start(tokens, marker)
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
    let mut state = QuoteInnerState::default();
    quote_state_update_line(tokens, start, &mut state);
    let mut i = start + 1;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }

    finish_md_block_quote(tokens, i, state, events)
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
    let mut state = QuoteInnerState::default();
    let mut i = ws_start;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        if tokens[i].kind == TokKind::RoxygenMdBlockQuote {
            quote_state_update(&mut state, quote_inner_content(tokens[i].text));
        }
        events.push(Event::Tok(i));
        i += 1;
    }
    finish_md_block_quote(tokens, i, state, events)
}

/// Block-level state of a quote's **innermost** content, tracked line-by-line so
/// lazy continuation can be gated on an *open paragraph* (CommonMark: laziness
/// only continues a paragraph). Each folded `>` line's inner content — every
/// quote level stripped ([`quote_inner_content`]) — updates it: plain prose opens
/// a paragraph; a blank line, an indented-code line (when no paragraph is open),
/// a fence opener (until its closer), an ATX heading, a thematic break, or a
/// promoting setext underline closes it. This is a per-line approximation, not a
/// block tree: a fence opened in a *nested* quote is tracked as if it were the
/// innermost block (a mixed-depth quote around a fence can misclassify — deferred
/// to the block→inline pass), and HTML blocks are not modeled.
#[derive(Default)]
struct QuoteInnerState {
    /// The innermost content's paragraph is open (a lazy line may continue it).
    para_open: bool,
    /// An open fenced code block: fence character and opening run length.
    fence: Option<(u8, usize)>,
}

/// Update the quote-inner block state with one `>` line's inner content.
fn quote_state_update(state: &mut QuoteInnerState, inner: &str) {
    if let Some((ch, run)) = state.fence {
        state.para_open = false;
        // A closing fence: <= 3 columns of indentation, a run of the opening
        // character at least as long as the opener, and nothing else.
        let t = inner.trim_start_matches(' ');
        if inner.len() - t.len() <= 3 {
            let r = t.bytes().take_while(|&b| b == ch).count();
            if r >= run && t[r..].trim().is_empty() {
                state.fence = None;
            }
        }
        return;
    }
    let t = inner.trim_start_matches(' ');
    if t.is_empty() {
        state.para_open = false; // blank line: the paragraph ends
        return;
    }
    let indent = inner.len() - t.len();
    if indent >= 4 {
        // >= 4 columns: paragraph continuation when open (indented code cannot
        // interrupt a paragraph), indented code when closed — either way the
        // paragraph-open state is unchanged.
        return;
    }
    if let Some(fence) = quote_inner_fence_opener(t) {
        state.fence = Some(fence);
        state.para_open = false;
        return;
    }
    if quote_inner_is_atx_heading(t) || quote_inner_is_thematic_break(t) {
        state.para_open = false;
        return;
    }
    if state.para_open && quote_inner_is_setext_underline(t) {
        state.para_open = false; // the underline promotes the paragraph
        return;
    }
    if !state.para_open && matches!(t.as_bytes()[0], b'-' | b'*' | b'+') && t[1..].trim().is_empty()
    {
        return; // an empty list item opens no paragraph
    }
    // Plain prose — or a list item with content, whose own paragraph opens.
    state.para_open = true;
}

/// Update the quote-inner state from the quote line whose `RoxygenMarker` is at
/// `start` (its content is the whole-line `RoxygenMdBlockQuote` leaf).
fn quote_state_update_line(tokens: &[Token], start: usize, state: &mut QuoteInnerState) {
    let content = line_content_start(tokens, start);
    if let Some(tok) = tokens.get(content)
        && tok.kind == TokKind::RoxygenMdBlockQuote
    {
        quote_state_update(state, quote_inner_content(tok.text));
    }
}

/// Strip every leading block-quote marker level (up to three spaces, `>`, one
/// optional space, repeatedly) from a quote line's content, yielding the
/// innermost content the state machine classifies.
pub(super) fn quote_inner_content(mut s: &str) -> &str {
    loop {
        let b = s.as_bytes();
        let mut j = 0;
        while j < 3 && b.get(j) == Some(&b' ') {
            j += 1;
        }
        if b.get(j) != Some(&b'>') {
            return s;
        }
        j += 1;
        if b.get(j) == Some(&b' ') {
            j += 1;
        }
        s = &s[j..];
    }
}

/// A CommonMark fence opener in a quote's inner content (already indent-trimmed):
/// a run of three or more backticks or tildes; a backtick fence's info string may
/// not contain a backtick. Returns the fence character and run length.
fn quote_inner_fence_opener(t: &str) -> Option<(u8, usize)> {
    let ch = match t.as_bytes().first() {
        Some(&c @ (b'`' | b'~')) => c,
        _ => return None,
    };
    let run = t.bytes().take_while(|&b| b == ch).count();
    if run < 3 || (ch == b'`' && t[run..].contains('`')) {
        return None;
    }
    Some((ch, run))
}

/// An ATX heading in a quote's inner content: one to six `#`, then a space or
/// the end of the line.
fn quote_inner_is_atx_heading(t: &str) -> bool {
    let hashes = t.bytes().take_while(|&b| b == b'#').count();
    (1..=6).contains(&hashes) && t.as_bytes().get(hashes).is_none_or(|&b| b == b' ')
}

/// A thematic break in a quote's inner content: three or more of one of
/// `*`/`-`/`_`, interleaved with spaces only.
fn quote_inner_is_thematic_break(t: &str) -> bool {
    let ch = match t.as_bytes().first() {
        Some(&c @ (b'*' | b'-' | b'_')) => c,
        _ => return false,
    };
    let mut count = 0;
    for b in t.bytes() {
        match b {
            _ if b == ch => count += 1,
            b' ' | b'\t' => {}
            _ => return false,
        }
    }
    count >= 3
}

/// A setext underline in a quote's inner content: a run of `=` or `-` with only
/// trailing whitespace. (A `---` run is also a thematic break — the caller checks
/// that first; while a paragraph is open the promoting reading closes it either
/// way.)
fn quote_inner_is_setext_underline(t: &str) -> bool {
    let ch = match t.as_bytes().first() {
        Some(&c @ (b'=' | b'-')) => c,
        _ => return false,
    };
    let run = t.bytes().take_while(|&b| b == ch).count();
    t[run..].trim().is_empty()
}

/// Gather a block quote's continuation lines (consecutive `>` lines and lazy
/// paragraph continuations) after its opening line, then finish the
/// `ROXYGEN_MD_BLOCK_QUOTE` node. `i` is at the opening line's trailing
/// `Newline`; `state` reflects the opening line ([`QuoteInnerState`]). Shared by
/// the line-start ([`emit_md_block_quote`]) and tag-value
/// ([`emit_md_block_quote_from_value`]) forms.
fn finish_md_block_quote(
    tokens: &[Token],
    mut i: usize,
    mut state: QuoteInnerState,
    events: &mut Vec<Event>,
) -> usize {
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
        if is_md_block_quote_start(tokens, m) {
            quote_state_update_line(tokens, m, &mut state);
        } else {
            // An unmarked line folds only as a lazy continuation of the quote's
            // open paragraph (CommonMark laziness continues paragraphs, nothing
            // else): after a blank `>` line, an indented-code line, or a fence
            // opener inside the quote, the paragraph is closed and the quote ends.
            if !state.para_open {
                break;
            }
            // A setext underline (`===`/`==`, or a `--` dash run too short to be a
            // thematic break) cannot be a lazy continuation *underline* in a block
            // quote (CommonMark), so it never promotes the quote's paragraph into a
            // heading; instead it folds in as ordinary paragraph-continuation text
            // (engine-probed: `> foo` then `===` renders `foo===`). It is excluded
            // from `is_foldable_continuation` — correct for a tag's prose value,
            // where an underline *does* promote — so fold it explicitly here. A `---`
            // (or longer) dash run is a thematic break, which interrupts a paragraph
            // and ends the quote, so it is not folded.
            let is_lazy_setext =
                is_md_setext_underline_line(tokens, m) && !is_md_thematic_break_line(tokens, m);
            if !is_lazy_setext
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
            // A lazy line is paragraph text: the paragraph stays open.
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
///
/// The line's content must sit within CommonMark's three-space indent allowance:
/// at column five or beyond (the one-based [`list_line_indent`] gauge, tab-stop
/// expanded) the line is indented-code territory, so it is no break — after a
/// paragraph it lazily folds as ordinary prose (`Foo` then `    ***` is one
/// paragraph, cm-049; at a fresh position the indented-code arm claims it first,
/// cm-048). The lexer carves the leaf without seeing the marker→content
/// whitespace, so the column gate lives here at block level.
pub(super) fn is_md_thematic_break_line(tokens: &[Token], start: usize) -> bool {
    if list_line_indent(tokens, start) >= 5 {
        return false;
    }
    let content = line_content_start(tokens, start);
    match tokens.get(content).map(|t| &t.kind) {
        Some(TokKind::RoxygenMdThematicBreak) => true,
        Some(TokKind::RoxygenMdSetextUnderline) => {
            setext_underline_is_thematic(tokens[content].text)
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
        header.push_str(tok.text);
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
/// `ROXYGEN_MD_LIST` inside that item, while a shallower marker line is a
/// sibling of the same list (CommonMark ties an item to a list by its marker
/// falling short of the previous item's content column, not by matching the
/// list's own marker column — `- a` / ` - b` / `  - c` is one flat list). The
/// trailing newline after the final item is left to the caller. Returns the
/// token index just past the last consumed content. The container floor is `1`:
/// a section's content column in [`list_line_indent`]'s one-based gauge (the
/// conventional `#' ` separator space counts as one column there).
pub(super) fn emit_md_list(tokens: &[Token], start: usize, events: &mut Vec<Event>) -> usize {
    debug_assert_eq!(tokens[start].kind, TokKind::RoxygenMarker);
    emit_md_list_level_inner(tokens, start, ListItemStart::Line, 1, events)
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
    emit_md_list_level_inner(tokens, ws_start, ListItemStart::TagValue, 1, events)
}

/// Recursion entry for nested list levels (each starts at a line's
/// `RoxygenMarker`). `container_indent` is the parent item's content column —
/// the child list's container floor.
fn emit_md_list_level(
    tokens: &[Token],
    start: usize,
    container_indent: usize,
    events: &mut Vec<Event>,
) -> usize {
    emit_md_list_level_inner(tokens, start, ListItemStart::Line, container_indent, events)
}

/// How a list's **first item** starts, for [`emit_md_list_level_inner`]. Every
/// later item is a line-start item (`Line`); the variants differ only in what
/// precedes the first `RoxygenMdListMarker`.
enum ListItemStart {
    /// A line-start item: `start` is the line's `RoxygenMarker` (`#'`), followed
    /// by `Whitespace` indentation, then the list marker.
    Line,
    /// A tag's same-line value (`#' @details - item`): `start` is the whitespace
    /// between the tag head and the list marker (the enclosing tag owns the
    /// line's `#'`).
    TagValue,
    /// A **same-line nested list** (`- - foo`): `start` is the nested
    /// `RoxygenMdListMarker` itself, mid-line. There is no `#'` and no
    /// `Whitespace` indentation to consume — the marker sits exactly at the
    /// enclosing item's content column, so its indent *is* `container_indent`.
    MidLine,
}

/// Emit one `ROXYGEN_MD_LIST` inside the container whose content column is
/// `container_indent` (`1` for a section-level list — [`list_line_indent`]'s
/// one-based gauge — or the parent item's content column for a nested one).
/// Each item is a `ROXYGEN_MD_LIST_ITEM` holding its
/// `RoxygenMdListMarker` leaf, inline content, and any nested `ROXYGEN_MD_LIST`;
/// the `#'` markers, marker→content whitespace, and inter-line
/// newlines/indentation are threaded in as trivia (losslessness), the way the
/// block Rd macros thread them. Recurses for nested levels.
fn emit_md_list_level_inner(
    tokens: &[Token],
    start: usize,
    first: ListItemStart,
    container_indent: usize,
    events: &mut Vec<Event>,
) -> usize {
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_LIST));

    let mut i = start;
    let mut first = Some(first);
    loop {
        // `i` is at a `RoxygenMarker` of a list-item line at this level (or,
        // for a marker-less first item, at its leading whitespace or directly
        // at its `RoxygenMdListMarker` — see `ListItemStart`). The marker and
        // the marker→content whitespace are threaded as trivia.
        let this = first.take();
        let indent = if matches!(this, Some(ListItemStart::MidLine)) {
            container_indent
        } else {
            if !matches!(this, Some(ListItemStart::TagValue)) {
                events.push(Event::Tok(i));
                i += 1;
            }
            let mut ws_texts = Vec::new();
            while tokens.get(i).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
                ws_texts.push(tokens[i].text);
                events.push(Event::Tok(i));
                i += 1;
            }
            md_ws_gauge(ws_texts)
        };

        // The item: its `RoxygenMdListMarker` leaf, then its inline content. A
        // child block must reach this item's content column to nest under it.
        events.push(Event::Start(SyntaxKind::ROXYGEN_MD_LIST_ITEM));
        let marker_width = tokens[i].text.chars().count();
        let item_marker = i;
        events.push(Event::Tok(i)); // RoxygenMdListMarker
        i += 1;
        // The value column just past the marker: the gauge is value column + 1,
        // and a gauge-0 line (no `#'`→content whitespace at all) still puts its
        // marker at value column 0.
        let marker_end_col = indent.saturating_sub(1) + marker_width;
        let content_indent =
            indent + marker_width + content_leading_spaces(tokens, i, marker_end_col);
        let content_start = i;
        // A same-line nested list (`- - foo`): the lexer carved the item's
        // content-opening list marker, leaving the marker→marker separating
        // whitespace as its own all-whitespace prose run. The rest of the line
        // (and any continuation lines it claims) is a child list whose
        // container floor is this item's content column — exactly the nested
        // marker's own column in the line gauge.
        let mut item_has_content;
        if is_same_line_sublist(tokens, i) {
            events.push(Event::Tok(i)); // separating whitespace (prose run)
            i = emit_md_list_level_inner(
                tokens,
                i + 1,
                ListItemStart::MidLine,
                content_indent,
                events,
            );
            item_has_content = true;
        } else if is_same_line_quote(tokens, i) {
            // A block quote opening at the item's content start on the marker
            // line (`- > quoted`, cm-294/295): the lexer carved the rest of the
            // line as a `RoxygenMdBlockQuote` leaf past the separator run. The
            // quote node starts at the leaf — a marker-less first line, the
            // from-value shape — and gathers its continuation lines (`>` lines
            // and lazy paragraph text) exactly like a from-value quote.
            events.push(Event::Tok(i)); // separating whitespace (prose run)
            i = emit_md_block_quote_from_value(tokens, i + 1, events);
            item_has_content = true;
        } else if is_same_line_heading(tokens, i) {
            // An ATX heading at the item's content start on the marker line
            // (`- # Foo`, cm-302): the lexer carved the rest of the line as a
            // `RoxygenMdHeading` leaf past the separator run. A heading is one
            // line, so the node holds just the leaf; the projector hoists a
            // level-1 heading to a top-level `\section` (dropping the sliced
            // `\itemize`) and nests a deeper one as an in-item `\subsection`.
            events.push(Event::Tok(i)); // separating whitespace (prose run)
            i = emit_md_heading_from_value(tokens, i + 1, events);
            item_has_content = true;
        } else if is_same_line_fence(tokens, i) {
            // A fenced code block opening at the item's content start on the
            // marker line (`- ```` ``` ````, cm-320/326): the lexer carved the
            // rest of the line as a `RoxygenMdFence` leaf past the separator
            // run. The block node starts at the leaf — a marker-less first
            // line, the from-value shape — and gathers its code lines to the
            // closing fence, whose indent window is keyed to this item's
            // content column (not the section level).
            events.push(Event::Tok(i)); // separating whitespace (prose run)
            i = emit_md_code_block_from_value(tokens, i + 1, content_indent, events);
            item_has_content = true;
        } else if is_same_line_html_block(tokens, i) {
            // An HTML block opening at the item's content start on the marker
            // line (`- <div>`, cm-177): the lexer carved the rest of the line
            // as a `RoxygenMdHtmlBlock` opener leaf past the separator run.
            // The block node starts at the leaf — a marker-less first line,
            // the from-value shape — and gathers continuation lines per its
            // start condition, but only while they reach this item's content
            // column (an under-indented line ends the item, and an HTML block
            // has no lazy continuation).
            events.push(Event::Tok(i)); // separating whitespace (prose run)
            i = emit_md_html_block_from_value(tokens, i + 1, content_indent, events);
            item_has_content = true;
        } else if is_same_line_break(tokens, i) {
            // A thematic break at the item's content start on the marker line
            // (`- * * *`, cm-061): the lexer carved the rest of the line as a
            // `RoxygenMdThematicBreak` leaf past the separator run. The break
            // node holds just the leaf; roxygen2 renders a thematic break
            // empty, so the projector drops it and the item stays bare
            // (`\item` with no text).
            events.push(Event::Tok(i)); // separating whitespace (prose run)
            i = emit_md_thematic_break_from_value(tokens, i + 1, events);
            item_has_content = true;
        } else if item_first_line_opens_indented_code(tokens, i, marker_end_col) {
            // The item's content sits five or more columns past the marker, so
            // it *starts with indented code* (cm-275/276): `content_indent`
            // snapped to marker + 1 (`content_leading_spaces`), and the line's
            // remainder — one separator column, then the code's own indent —
            // is an indented code block *inside* the item.
            i = emit_md_indented_code_mid_line(tokens, i, content_indent + 4, events);
            item_has_content = true;
        } else if let Some(underline) = item_setext_underline_ahead(tokens, i, content_indent) {
            i = emit_md_item_setext_heading(tokens, i, underline, events);
            item_has_content = true;
        } else {
            while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
                events.push(Event::Tok(i));
                i += 1;
            }
            item_has_content = tokens[content_start..i]
                .iter()
                .any(|t| !t.text.trim().is_empty());
        }

        // The item body: paragraph continuations and nested lists, in source
        // order. Each iteration folds one continuation into the item, so a
        // lazy line, a blank-separated paragraph, and a nested list interleave
        // as they appear. An **empty** item has no open paragraph to continue
        // (an item starting with a blank needs indented content,
        // engine-probed), so no prose continuation folds into it.
        loop {
            if let Some(m) = following_line_marker(tokens, i)
                && is_block_macro_line(tokens, m)
                && (item_has_content || list_line_indent(tokens, m) >= content_indent)
            {
                for idx in i..m {
                    events.push(Event::Tok(idx)); // `\n` (trivia)
                }
                i = emit_item_block_macro(tokens, m, events);
                continue;
            }
            if item_has_content
                && let Some(m) = next_content_line_across_blanks(tokens, i)
                && is_block_macro_line(tokens, m)
                && (content_indent..content_indent + 4).contains(&list_line_indent(tokens, m))
            {
                for idx in i..m {
                    events.push(Event::Tok(idx)); // `\n` + blank lines (trivia)
                }
                i = emit_item_block_macro(tokens, m, events);
                continue;
            }

            if let Some(m) = next_content_line(tokens, i)
                && list_line_indent(tokens, m) >= content_indent
                && is_md_table_start(tokens, m)
            {
                for idx in i..m {
                    events.push(Event::Tok(idx)); // `\n` + blank lines (trivia)
                }
                i = emit_md_table(tokens, m, events);
                continue;
            }

            if item_has_content
                && let Some(m) = next_content_line(tokens, i)
                && (content_indent..content_indent + 4).contains(&list_line_indent(tokens, m))
                && is_md_block_quote_start(tokens, m)
            {
                for idx in i..m {
                    events.push(Event::Tok(idx)); // `\n` + blank lines (trivia)
                }
                i = emit_md_block_quote(tokens, m, events);
                continue;
            }

            if item_has_content
                && let Some(m) = next_content_line(tokens, i)
                && (content_indent..content_indent + 4).contains(&list_line_indent(tokens, m))
                && is_md_heading_start(tokens, m)
            {
                for idx in i..m {
                    events.push(Event::Tok(idx)); // `\n` + blank lines (trivia)
                }
                i = emit_md_heading(tokens, m, events);
                continue;
            }

            // An **empty** item's first content: CommonMark lets an item begin
            // with its marker alone, the content starting on the *immediately*
            // following line at (or past) the content column (`-` then `  foo`,
            // cm-280/281). "A list item can begin with at most one blank line",
            // and the marker line is that one blank — an actual blank line in
            // between keeps the content out of the item (cm-282), hence
            // `following_line_marker` (no blank crossing). Indented code first:
            // a next line four or more columns past the content column is an
            // indented code block inside the item (cm-280's `baz`) — checked
            // before the prose arm, which would claim the over-indented line.
            if !item_has_content
                && let Some(m) = following_line_marker(tokens, i)
                && is_indent_code_line_min(tokens, m, content_indent + 4)
            {
                for idx in i..m {
                    events.push(Event::Tok(idx)); // `\n` + indentation (trivia)
                }
                i = emit_md_indented_code_min(tokens, m, content_indent + 4, events);
                item_has_content = true;
                continue;
            }
            if !item_has_content
                && let Some(m) = following_line_marker(tokens, i)
                && list_line_indent(tokens, m) >= content_indent
                && is_md_item_lazy_continuation(tokens, m)
            {
                for idx in i..=m {
                    events.push(Event::Tok(idx)); // `\n` + indentation + `#'` (trivia)
                }
                i = m + 1;
                while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
                    events.push(Event::Tok(i));
                    i += 1;
                }
                item_has_content = true;
                continue;
            }

            // A no-blank lazy continuation: a following plain-prose line that
            // opens no block folds into the item's open paragraph (CommonMark
            // paragraph continuation text) — even unindented or over-indented.
            if item_has_content
                && let Some(m) = following_line_marker(tokens, i)
                && is_md_item_lazy_continuation(tokens, m)
            {
                for idx in i..=m {
                    events.push(Event::Tok(idx)); // `\n` + indentation + `#'` (trivia)
                }
                i = m + 1;
                while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
                    events.push(Event::Tok(i));
                    i += 1;
                }
                continue;
            }

            if item_has_content
                && let Some(m) = following_line_marker(tokens, i)
                && is_md_list_continuation(tokens, m)
                && (container_indent + 4..content_indent).contains(&list_line_indent(tokens, m))
            {
                for idx in i..=m {
                    events.push(Event::Tok(idx)); // `\n` + indentation + `#'` (trivia)
                }
                i = m + 1;
                while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
                    events.push(Event::Tok(i));
                    i += 1;
                }
                continue;
            }

            if item_has_content
                && let Some(m) = next_content_line_across_blanks(tokens, i)
                && is_indent_code_line_min(tokens, m, content_indent + 4)
            {
                for idx in i..m {
                    events.push(Event::Tok(idx)); // `\n` + blank lines + `#'` (trivia)
                }
                i = emit_md_indented_code_min(tokens, m, content_indent + 4, events);
                continue;
            }

            if item_has_content
                && let Some(m) = next_prose_line_across_blanks(tokens, i)
                && list_line_indent(tokens, m) >= content_indent
            {
                for idx in i..=m {
                    events.push(Event::Tok(idx)); // `\n` + blank lines + indentation + `#'`
                }
                i = m + 1;
                while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
                    events.push(Event::Tok(i));
                    i += 1;
                }
                continue;
            }

            if let Some(m) = next_content_line(tokens, i)
                && list_line_indent(tokens, m) >= content_indent
                && is_md_code_block_start(tokens, m)
            {
                for idx in i..m {
                    events.push(Event::Tok(idx)); // `\n` + blank lines (trivia)
                }
                i = emit_md_code_block(tokens, m, content_indent, events);
                continue;
            }

            // A nested list: a following list line indented to (or past) the
            // item's content column is a child list inside this item — even
            // across blank lines (a blank ends the item's paragraph but not the
            // item; a list line at the content column still nests).
            let m = match next_list_line(tokens, i) {
                Some(m) => m,
                None => match next_list_line_across_blanks(tokens, i) {
                    Some(m) if list_line_indent(tokens, m) >= content_indent => m,
                    _ => break,
                },
            };
            if list_line_indent(tokens, m) < content_indent {
                break;
            }
            for idx in i..m {
                events.push(Event::Tok(idx)); // `\n` + blank lines + indentation (trivia)
            }
            i = emit_md_list_level(tokens, m, content_indent, events);
        }
        events.push(Event::Finish); // ROXYGEN_MD_LIST_ITEM

        let sibling_window = container_indent..content_indent.min(container_indent + 4);
        let m = if let Some(m) = next_list_line(tokens, i) {
            if !sibling_window.contains(&list_line_indent(tokens, m)) {
                break;
            }
            m
        } else {
            let Some(m) = next_list_line_across_blanks(tokens, i) else {
                break;
            };
            if !sibling_window.contains(&list_line_indent(tokens, m)) {
                break;
            }
            m
        };
        // A change of list *type* — a different bullet char or ordered
        // delimiter — starts a new list rather than continuing this one
        // (CommonMark; engine-probed: `-` … `*` and `1.` … `2)` split), whether
        // or not a blank line intervenes.
        let sibling_marker = &tokens[line_content_start(tokens, m)].text;
        if md_list_marker_type(sibling_marker) != md_list_marker_type(tokens[item_marker].text) {
            break;
        }
        for idx in i..m {
            events.push(Event::Tok(idx)); // `\n` + blank lines + indentation (trivia)
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
/// line**: its marker->content whitespace gauges five or more columns (a
/// CommonMark indented code block needs four columns; roxygen2 strips the marker
/// and one following whitespace character first, and a tab expands to the next
/// 4-column stop — cm-001/002/008) *and* there is real content after it (a
/// whitespace-only line is blank, not code). Mode-blind — the caller gates on `md`;
/// the leading whitespace is ordinary `Whitespace` (no special leaf), so the
/// block-macro machinery's whitespace handling is unaffected.
pub(super) fn is_indent_code_line(tokens: &[Token], start: usize) -> bool {
    is_indent_code_line_min(tokens, start, 5)
}

/// Like [`is_indent_code_line`] but with a caller-supplied minimum indentation
/// (in gauge columns after the `#'` marker). A **top-level** indented code
/// line needs five columns (roxygen2 strips the marker and one space, leaving
/// CommonMark's four); an indented code block **folded into a list item** needs
/// the item's content column plus four (the item container consumes the content
/// column before CommonMark's four apply), so the caller passes
/// `content_indent + 4`.
fn is_indent_code_line_min(tokens: &[Token], start: usize, min_ws: usize) -> bool {
    if tokens.get(start + 1).map(|t| &t.kind) != Some(&TokKind::Whitespace) {
        return false;
    }
    if list_line_indent(tokens, start) < min_ws {
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
    emit_md_indented_code_min(tokens, start, 5, events)
}

/// Whether a list item's first-line content **starts with indented code**
/// (CommonMark's start condition, cm-275/276): the remainder after the marker
/// leads with five or more space/tab columns *and* carries real content. The
/// item's content indent then snaps to marker + 1 ([`content_leading_spaces`]),
/// and the remainder — less the one separator column — is an indented code
/// block inside the item. `content` is the token just past the
/// `RoxygenMdListMarker`; `start_col` is the value column just past the marker
/// (the tab-stop anchor: `-\t\tfoo` leads with seven columns, cm-007).
fn item_first_line_opens_indented_code(tokens: &[Token], content: usize, start_col: usize) -> bool {
    let Some(first) = tokens.get(content).filter(|t| is_line_body_kind(&t.kind)) else {
        return false;
    };
    let mut col = start_col;
    for c in first.text.chars().take_while(|c| *c == ' ' || *c == '\t') {
        col = advance_md_col(col, c);
    }
    if col - start_col < 5 {
        return false;
    }
    let mut k = content;
    while let Some(t) = tokens.get(k).filter(|t| is_line_body_kind(&t.kind)) {
        if !t.text.trim().is_empty() {
            return true;
        }
        k += 1;
    }
    false
}

/// Emit a `ROXYGEN_MD_INDENTED_CODE` node for an indented code block opening
/// **mid-line as a list item's first content** (`1.     code`): `start` is the
/// token just past the item's `RoxygenMdListMarker` — the prose run whose
/// leading whitespace holds the one separator column plus the code's own
/// indent. The following lines gather exactly as in [`emit_md_indented_code`],
/// gauged against `min_ws` (the item's content column plus four). Returns the
/// token index just past the last code line.
fn emit_md_indented_code_mid_line(
    tokens: &[Token],
    start: usize,
    min_ws: usize,
    events: &mut Vec<Event>,
) -> usize {
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_INDENTED_CODE));
    // First (marker-less, mid-line) line: the rest of the item's opening line.
    let mut i = start;
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }
    finish_md_indented_code(tokens, i, min_ws, events)
}

/// Like [`emit_md_indented_code`] but with a caller-supplied minimum indentation
/// (see [`is_indent_code_line_min`]): a block folded into a list item passes
/// `content_indent + 4` so both its opening line and its continuation lines are
/// gauged against the item's content column, not the top-level threshold.
pub(super) fn emit_md_indented_code_min(
    tokens: &[Token],
    start: usize,
    min_ws: usize,
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

    finish_md_indented_code(tokens, i, min_ws, events)
}

/// Whether a prose tag's same-line value opens a markdown **indented code
/// block**: the block is `@md` (threaded from the block builder — indented code
/// has no mode-carrying leaf), and the whitespace run between the tag head and
/// the value gauges five or more columns (roxygen2 strips only the single
/// separator character after the tag head, so four further columns — tab stops
/// included — reach CommonMark's indented-code threshold, the from-value analog
/// of [`is_indent_code_line`]). The value position is always a fresh block
/// position (the tag's markdown document starts there), so no paragraph gate
/// applies.
pub(super) fn is_md_indented_code_value(tokens: &[Token], value_start: usize, md: bool) -> bool {
    if !md || value_start == 0 {
        return false;
    }
    let ws = &tokens[value_start - 1];
    ws.kind == TokKind::Whitespace && md_ws_gauge([ws.text]) >= 5
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
    finish_md_indented_code(tokens, i, 5, events)
}

/// Gather an indented code block's continuation lines (following code lines and
/// interior blanks) after its opening line, then finish the
/// `ROXYGEN_MD_INDENTED_CODE` node. `i` is at the opening line's trailing
/// `Newline`; `min_ws` is the continuation-line indentation threshold (see
/// [`is_indent_code_line_min`]). Shared by the line-start
/// ([`emit_md_indented_code`]), tag-value ([`emit_md_indented_code_from_value`]),
/// and folded-into-a-list-item forms.
fn finish_md_indented_code(
    tokens: &[Token],
    mut i: usize,
    min_ws: usize,
    events: &mut Vec<Event>,
) -> usize {
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
            if is_indent_code_line_min(tokens, m, min_ws) {
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
/// consumed line is left to the caller. `base_indent` is the enclosing
/// container's content column in marker→content whitespace width (`1` at
/// section level — the single conventional space; a list item's
/// `content_indent` when the block is folded into an item): a closing fence may
/// be indented at most three columns past it. Returns the token index just past
/// the last consumed line's content.
pub(super) fn emit_md_code_block(
    tokens: &[Token],
    start: usize,
    base_indent: usize,
    events: &mut Vec<Event>,
) -> usize {
    debug_assert_eq!(tokens[start].kind, TokKind::RoxygenMarker);
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_CODE_BLOCK));

    // Opening line: marker, marker→content whitespace, then the opener fence.
    events.push(Event::Tok(start));
    let mut i = start + 1;
    while tokens.get(i).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
        events.push(Event::Tok(i));
        i += 1;
    }
    let mut opener = "";
    if tokens.get(i).map(|t| &t.kind) == Some(&TokKind::RoxygenMdFence) {
        events.push(Event::Tok(i)); // opener fence
        opener = tokens[i].text;
        i += 1;
    }

    finish_md_code_block(tokens, i, opener, base_indent, events)
}

/// Emit a `ROXYGEN_MD_CODE_BLOCK` node for a fenced code block opening as a
/// **tag's same-line value** (`#' @details ```r`) or on a **list item's marker
/// line** (`- ```` ``` ````, cm-320/326): the first line has no `#'` marker of
/// its own (that marker belongs to the enclosing tag or item line), so it
/// starts at `ws_start` — the whitespace between the head and the opener fence
/// leaf. `base_indent` is the enclosing container's content column, keying the
/// closer's indent window: the single conventional space for a tag value, the
/// item's content column for a mid-line item fence. The following lines gather
/// exactly as in [`emit_md_code_block`]. Returns the token index just past the
/// last consumed line's content.
pub(super) fn emit_md_code_block_from_value(
    tokens: &[Token],
    ws_start: usize,
    base_indent: usize,
    events: &mut Vec<Event>,
) -> usize {
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_CODE_BLOCK));
    // First (marker-less) line: the leading whitespace and the opener fence.
    let mut i = ws_start;
    let mut opener = "";
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        if tokens[i].kind == TokKind::RoxygenMdFence && opener.is_empty() {
            opener = tokens[i].text;
        }
        events.push(Event::Tok(i));
        i += 1;
    }
    finish_md_code_block(tokens, i, opener, base_indent, events)
}

/// Gather a fenced code block's lines (after its opening line) up to and
/// including the closing fence, then finish the `ROXYGEN_MD_CODE_BLOCK` node.
/// `i` is at the opening line's trailing `Newline`. A fence line is a **closer**
/// only when it matches the `opener` fence — same fence character, a run at
/// least as long, no info string ([`md_fence_run_closes`]) — and is indented at
/// most three columns past `base_indent` (the container's content column, in
/// marker→content whitespace width); any other fence line is verbatim content
/// (CommonMark 4.5). Shared by the line-start ([`emit_md_code_block`]) and
/// tag-value ([`emit_md_code_block_from_value`]) forms.
fn finish_md_code_block(
    tokens: &[Token],
    mut i: usize,
    opener: &str,
    base_indent: usize,
    events: &mut Vec<Event>,
) -> usize {
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
        let mut ws_width = 0;
        while tokens.get(i).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
            ws_width += tokens[i].text.chars().count();
            events.push(Event::Tok(i));
            i += 1;
        }
        // A matching closing fence ends the block; any other line — including a
        // fence that is too short, the wrong character, info-string-bearing, or
        // over-indented — is verbatim code (its body tokens threaded through).
        // Both consume the whole line's content.
        let is_closer = tokens.get(i).is_some_and(|t| {
            t.kind == TokKind::RoxygenMdFence
                && ws_width <= base_indent + 3
                && md_fence_run_closes(opener, t.text)
        });
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

    finish_md_html_block(tokens, i, &opener, 0, events)
}

/// Emit a `ROXYGEN_MD_HTML_BLOCK` node for an HTML block opening as a **tag's
/// same-line value** ([`is_md_html_block_value`]): the first line has no `#'`
/// marker of its own (that marker belongs to the enclosing tag, already emitted
/// and closed), so it starts at `ws_start` — the whitespace between the tag head
/// and the value. roxygen2 strips only the single separator space after the tag
/// head, so any further indent is part of the block's first rendered line and
/// stays inside the node. The following lines gather per the opener's start
/// condition exactly as in [`emit_md_html_block`], additionally gated by
/// `container_indent` — the enclosing container's content column (`0` for a tag
/// value, disabling the gate; the item's content column for a mid-line item
/// block, cm-177): a prose line indented below it exits the container, so it
/// never folds. Returns the token index just past the last consumed line's
/// content.
pub(super) fn emit_md_html_block_from_value(
    tokens: &[Token],
    ws_start: usize,
    container_indent: usize,
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
            opener.push_str(tokens[i].text);
        }
        events.push(Event::Tok(i));
        i += 1;
    }
    finish_md_html_block(tokens, i, &opener, container_indent, events)
}

/// Gather an HTML block's continuation lines (after its opening line, whose
/// content is `opener`) per the opener's start condition, then finish the
/// `ROXYGEN_MD_HTML_BLOCK` node. `i` is at the opening line's trailing
/// `Newline`. A non-zero `container_indent` (the enclosing item's content
/// column) additionally ends the block at a prose line indented below it — the
/// line exits the container, and an HTML block has no lazy continuation.
/// Shared by the line-start ([`emit_md_html_block`]) and tag-value/mid-line
/// ([`emit_md_html_block_from_value`]) forms.
fn finish_md_html_block(
    tokens: &[Token],
    mut i: usize,
    opener: &str,
    container_indent: usize,
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
                if container_indent > 0
                    && matches!(classify_line(tokens, m), LineKind::Prose)
                    && list_line_indent(tokens, m) < container_indent
                {
                    break; // an under-indented prose line exits the container
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
        if container_indent > 0 && list_line_indent(tokens, m) < container_indent {
            break; // an under-indented prose line exits the container
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

/// Whether the line whose marker is at `start` is a setext underline **within
/// CommonMark's three-space indent allowance**: at column five or beyond (the
/// one-based [`list_line_indent`] gauge, tab-stop expanded — the same gate as
/// [`is_md_thematic_break_line`]) the line is indented-code territory, so it
/// never promotes a heading; after a paragraph it lazily folds as ordinary
/// prose instead (`Foo` then `    ---` is one paragraph, cm-087). The raw leaf
/// test ([`is_md_setext_underline_line`]) stays separate for the callers with
/// their own column window (the in-item promotion) or fold intent (the
/// block-quote lazy arm, where an over-indented underline folds either way).
pub(super) fn is_md_promoting_setext_underline(tokens: &[Token], start: usize) -> bool {
    list_line_indent(tokens, start) < 5 && is_md_setext_underline_line(tokens, start)
}

/// Whether the line whose marker is at `start` can serve as a **setext H2/H1
/// underline**: a genuine `===`/`---` underline leaf, or a lone dash bullet
/// ([`is_md_setext_dash_underline`]), each within the three-space indent
/// allowance ([`is_md_promoting_setext_underline`]'s column gate). Used only by
/// the setext-heading look-back and emit, both reached solely from a paragraph
/// open — at a fresh block position the same dash bullet still opens an empty
/// list (the block loop's list check runs first), so this never mis-fires on a
/// list.
pub(super) fn is_md_setext_underline_or_dash(tokens: &[Token], start: usize) -> bool {
    if list_line_indent(tokens, start) >= 5 {
        return false;
    }
    is_md_setext_underline_line(tokens, start) || is_md_setext_dash_underline(tokens, start)
}

/// Whether the prose line whose marker is at `start` opens a **setext heading**:
/// its paragraph — the maximal run of foldable prose continuation lines — is
/// terminated *immediately* by a setext underline line. A setext underline heads
/// nothing on its own, so the current line must carry prose (not be the underline).
/// The whole preceding paragraph becomes the heading text; this block-level
/// look-back is what distinguishes a setext H2 (`para` then `---`) from a thematic
/// break (`---` after a blank). Only called at a paragraph open, so the run scanned
/// here is exactly the paragraph the grouper would otherwise build. A thematic-break
/// line never opens a paragraph (block structure wins over paragraph text in
/// CommonMark), so `***` followed by `---` is two breaks, not a `***`-titled heading
/// (cm-043).
pub(super) fn is_md_setext_heading_start(tokens: &[Token], start: usize) -> bool {
    if is_md_setext_underline_or_dash(tokens, start) || is_md_thematic_break_line(tokens, start) {
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

/// Whether a list item's marker-line prose at `i` (the item's first content,
/// past the `RoxygenMdListMarker`) is promoted by a following **setext
/// underline at the item's content column** — returning that underline line's
/// `RoxygenMarker` index. The paragraph run may span further prose lines, each
/// folding at (or past) the content column; the underline itself must sit in
/// the `[content_indent, content_indent + 4)` window (below it is outside the
/// item — a lazy `===` fold or a list-ending `---` thematic break, both the
/// existing arms' business; at or past `content_indent + 4` it is indented-code
/// territory). Only a genuine `===`/`---` underline leaf promotes here; a lone
/// dash bullet at the content column still nests an empty sublist (unlike the
/// section-level dash-underline rule — backlog if roxygen2 disagrees). A blank
/// line, a non-prose line, or a below-column continuation ends the look-ahead
/// without promoting (conservative: those shapes keep their current arms).
fn item_setext_underline_ahead(tokens: &[Token], i: usize, content_indent: usize) -> Option<usize> {
    // The marker line must carry real paragraph content for an underline to
    // promote (an empty item has no open paragraph).
    let mut j = i;
    let mut has_content = false;
    while tokens.get(j).is_some_and(|t| is_line_body_kind(&t.kind)) {
        has_content |= !tokens[j].text.trim().is_empty();
        j += 1;
    }
    if !has_content {
        return None;
    }
    loop {
        let m = following_line_marker(tokens, j)?;
        let indent = list_line_indent(tokens, m);
        let in_window = (content_indent..content_indent + 4).contains(&indent);
        if in_window && is_md_setext_underline_line(tokens, m) {
            return Some(m);
        }
        if indent >= content_indent && is_md_item_lazy_continuation(tokens, m) {
            j = line_content_end(tokens, m);
            continue;
        }
        return None;
    }
}

/// Emit a `ROXYGEN_MD_HEADING` node for an item's **setext heading**: the item's
/// marker-line prose (starting at `ws_start`, past the list marker — a
/// marker-less first line, the from-value shape), any continuation prose lines,
/// and the underline line whose `RoxygenMarker` is `underline`
/// ([`item_setext_underline_ahead`] found it). Inter-line `#'` markers,
/// indentation, and newlines thread in as trivia, as in
/// [`emit_md_setext_heading`]. Returns the index just past the underline line's
/// content.
fn emit_md_item_setext_heading(
    tokens: &[Token],
    ws_start: usize,
    underline: usize,
    events: &mut Vec<Event>,
) -> usize {
    events.push(Event::Start(SyntaxKind::ROXYGEN_MD_HEADING));
    let mut i = ws_start;
    loop {
        let was_underline_line = i > underline;
        while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
            events.push(Event::Tok(i));
            i += 1;
        }
        if was_underline_line {
            events.push(Event::Finish); // ROXYGEN_MD_HEADING
            return i;
        }
        let next = following_line_marker(tokens, i)
            .expect("item setext heading run terminates in an underline");
        for idx in i..=next {
            events.push(Event::Tok(idx)); // `\n` + indentation + `#'` (trivia)
        }
        i = next + 1;
    }
}

/// Emit a block Rd macro folded into a list item's content, then place its
/// closing line's post-close remainder (and any further tokens on that line) as
/// item prose right after the macro node — the item folds paragraph content as
/// bare tokens, so no paragraph wrapper is opened.
fn emit_item_block_macro(tokens: &[Token], start: usize, events: &mut Vec<Event>) -> usize {
    let (mut i, tail) = emit_block_macro(tokens, start, events);
    if !tail.is_empty() {
        events.push(Event::Leaf(SyntaxKind::ROXYGEN_TEXT, tail));
    }
    while tokens.get(i).is_some_and(|t| is_line_body_kind(&t.kind)) {
        events.push(Event::Tok(i));
        i += 1;
    }
    i
}

/// Emit a multi-line block Rd macro as a `ROXYGEN_RD_MACRO` node spanning `#'`
/// lines. The node owns its opening line's marker and the inter-line markers,
/// newlines, and indentation as threaded trivia (losslessness); its body is a
/// sequence of brace-less name-only `\item`/`\cr`/… macros, nested inline macros,
/// and prose, ending at the matching `}` (or, for an unterminated macro, at the
/// next tag opener or block end — greedy and lossless, no close delimiter). A
/// multi-argument macro ([`super::rd_macro_arity`]) whose closing `}` is
/// immediately followed by `{` consumes that group into the node too, up to its
/// arity (parse_Rd's adjacent-argument rule; a group may itself span lines).
/// Returns the token index just past the last consumed content (at its trailing
/// `Newline` / non-roxygen token / EOF) plus the closing line's post-close
/// remainder — prose *outside* the macro, which the caller places in the
/// enclosing context (parse_Rd resumes plain text right after the `}`).
pub(super) fn emit_block_macro(
    tokens: &[Token],
    start: usize,
    events: &mut Vec<Event>,
) -> (usize, String) {
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
/// macro nests inside it as an inline sibling of the surrounding prose (the way the
/// projector folds an abutting block macro into the same section). `opener` must
/// index the `\name{…` opener token; unlike [`emit_block_macro`] there is no
/// leading marker to thread (it belongs to the prose that precedes the opener).
/// Returns the next token index plus the post-close remainder (see
/// [`emit_block_macro`]), which the caller emits into the open paragraph.
pub(super) fn emit_block_macro_inline(
    tokens: &[Token],
    opener: usize,
    events: &mut Vec<Event>,
) -> (usize, String) {
    events.push(Event::Start(SyntaxKind::ROXYGEN_RD_MACRO));
    emit_block_macro_from_opener(tokens, opener, events)
}

/// Emit the body of a `ROXYGEN_RD_MACRO` (already `Start`ed) from its opener token
/// at `i`, consuming following `#'` lines until the group closes (or a tag / block
/// end terminates it), and `Finish` the node. Shared by the line-start
/// [`emit_block_macro`] and the mid-prose [`emit_block_macro_inline`]. Returns the
/// next token index and the closing line's remainder *after* the macro's last
/// consumed `}` — that text is outside the macro, so the caller places it.
fn emit_block_macro_from_opener(
    tokens: &[Token],
    mut i: usize,
    events: &mut Vec<Event>,
) -> (usize, String) {
    // The body's open brace groups (the parent macro's own body is the empty-stack
    // baseline, so a `}` at an empty stack terminates it).
    let mut frames: Vec<BodyFrame> = Vec::new();
    let mut closed = false;
    // Whether a markdown paragraph is open coming into the next line. cmark sees
    // the field text flat — the `\name{` opener line is ordinary paragraph text —
    // so the body starts inside an open paragraph, and a blank `#'` line closes it.
    let mut para_open = true;
    // Argument `{` groups opened for this macro so far (Form A: the body is group
    // 1; Form B: the leading balanced groups plus the body), and the macro's total
    // argument count. A close short of that count may consume a further *adjacent*
    // `{…}` (which may itself span lines).
    let mut groups = 1usize;
    let mut arity = 1usize;
    let mut tail = String::new();

    // Opening content. Form A: a `RoxygenText` `\name{ …` --- split off the name
    // and brace, then parse trailing same-line content. Form B: a balanced
    // `RoxygenRdMacro` `\name{arg}` followed by a `RoxygenText` `{ …` body opener
    // --- emit the macro's name and leading argument group(s) as leaves, then open
    // the body brace.
    match tokens.get(i) {
        Some(tok) if tok.kind == TokKind::RoxygenText => {
            arity = rd_macro_name(tok.text).map_or(1, super::rd_macro_arity);
            // The opener token is unbalanced to end-of-line (the block-macro
            // gates), so this cannot close the macro; the remainder is empty.
            emit_block_open(events, tok.text, &mut frames, &mut closed);
            i += 1;
        }
        Some(tok) if tok.kind == TokKind::RoxygenRdMacro => {
            arity = rd_macro_name(tok.text).map_or(1, super::rd_macro_arity);
            groups = emit_block_open_arg_macro(events, tok.text) + 1;
            i += 1;
            if let Some(next) = tokens.get(i) {
                emit_block_body_open(events, next.text, &mut frames, &mut closed);
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
                    let mut rest = emit_block_content(events, tok.text, &mut frames, &mut closed);
                    i += 1;
                    // A multi-argument macro's further adjacent groups: parse_Rd
                    // consumes a `{` touching the closing `}` (`\deqn{…}{ascii}`,
                    // `\ifelse{fmt}{…}{…}`) as the next argument — same-line
                    // balanced or spanning following lines. A spaced/next-line
                    // `{…}` (and any group past the arity) stays outside as
                    // literal prose.
                    while closed && groups < arity && rest.starts_with('{') {
                        events.push(Event::Leaf(
                            SyntaxKind::ROXYGEN_RD_MACRO_DELIM,
                            "{".to_string(),
                        ));
                        groups += 1;
                        closed = false;
                        rest = emit_block_content(events, &rest[1..], &mut frames, &mut closed);
                    }
                    if closed {
                        tail = rest;
                    }
                }
                // A *nested* Form-B block macro: a two-argument macro whose last
                // argument opens here and closes on a following `#'` line (an
                // `\item{term}{def …}` inside a `\describe` body). Recurse, then
                // feed its closing line's remainder back through this body's own
                // frames — that text is outside the child but inside us, so it may
                // itself close this macro.
                TokKind::RoxygenRdMacro if is_form_b_block_macro(tokens, i) => {
                    let (next, remainder) = emit_block_macro_inline(tokens, i, events);
                    i = next;
                    let rest = emit_block_content(events, &remainder, &mut frames, &mut closed);
                    if closed {
                        tail = rest;
                    }
                }
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
        // A markdown **block** construct inside the body (`@md`). roxygen2 runs
        // cmark over the whole field text, which is flat: the `\describe{` and
        // `\item{term}{…` lines are ordinary paragraph text, so a list or fence at
        // this column opens a real block rather than continuing the macro's prose.
        // Emit it as a child node — it owns its own `#'` markers, so only the
        // newline and indentation are threaded here.
        let depth = frames.len() as i32;
        if is_md_block_in_body(tokens, m, para_open, depth) {
            for idx in i..m {
                events.push(Event::Tok(idx));
            }
            i = emit_md_block_in_body(tokens, m, para_open, depth, events);
            para_open = false;
            continue;
        }
        // `\n` + indentation + `#'` threaded as trivia, then the marker→content ws.
        for idx in i..=m {
            events.push(Event::Tok(idx));
        }
        para_open = !matches!(classify_line(tokens, m), LineKind::Blank);
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
    (i, tail)
}

/// Whether the `#'` line whose marker is at `start` opens a markdown **block**
/// construct *inside* a block Rd macro's body.
///
/// roxygen2 runs cmark over the whole field text before any Rd parsing, and that
/// text is flat — a block macro is not a CommonMark container, so its `\describe{`
/// / `\item{term}{…` lines are ordinary paragraph text. A list or fenced code
/// block on a following line is therefore a genuine block at the *section*'s
/// column convention (one space after the marker), exactly as it would be outside
/// the macro; the only difference is where the node lands in the CST.
///
/// Only the constructs whose Rd rendering nests cleanly inside a macro argument
/// are recognized: markdown lists (`\itemize`/`\enumerate`) and fenced code
/// blocks (roxygen2's `\if{html}…\preformatted…\if{html}` triple). Headings hoist
/// to a top-level `\section`, and tables/quotes/thematic breaks have no
/// argument-level rendering, so they stay body prose. Both gates key on
/// `@md`-only leaf kinds, so this is inert with markdown off.
///
/// `depth` is the count of brace groups the body already has open. A block whose
/// *own opening line* closes the enclosing macro is withheld (the macro's closing
/// delimiter cannot live inside the block's node), leaving that line body prose.
fn is_md_block_in_body(tokens: &[Token], start: usize, para_open: bool, depth: i32) -> bool {
    (is_md_code_block_start(tokens, start) || is_md_list_start(tokens, start, para_open))
        && md_block_body_horizon(tokens, start, depth).is_some()
}

/// Emit the markdown block construct at `start` (see [`is_md_block_in_body`]) as a
/// child of the enclosing `ROXYGEN_RD_MACRO`. The node owns its own `#'` markers
/// and inter-line trivia; returns the token index at its last line's trailing
/// `Newline`, the same convention as the section-level emitters.
///
/// The container content column is the section-level `1` (the conventional single
/// space after `#'`) because the enclosing macro adds no CommonMark container
/// depth. The block is bounded at [`md_block_body_horizon`] by *truncating the
/// token slice*: the emitters stop at its end exactly as they would at EOF, and
/// every `Event::Tok` index they emit stays valid because only the tail is cut.
fn emit_md_block_in_body(
    tokens: &[Token],
    start: usize,
    para_open: bool,
    depth: i32,
    events: &mut Vec<Event>,
) -> usize {
    let horizon = md_block_body_horizon(tokens, start, depth)
        .expect("gated by is_md_block_in_body, which withholds a self-closing opener");
    let bounded = &tokens[..horizon];
    if is_md_code_block_start(bounded, start) {
        emit_md_code_block(bounded, start, 1, events)
    } else {
        debug_assert!(is_md_list_start(bounded, start, para_open));
        emit_md_list(bounded, start, events)
    }
}

/// The token index bounding a markdown block that starts at the `#'` line whose
/// marker is at `start`, inside a block Rd macro's body whose enclosing group is
/// `depth` open frames deep. The result is the trailing `Newline` of the last line
/// *before* the first line that closes the enclosing macro (or `tokens.len()` when
/// no line does), so truncating there keeps that closing line out of the block.
///
/// **This is a deliberate, scoped deviation from CommonMark.** A line holding only
/// the macro's `}` is, to cmark, a *lazy continuation* of the block's last
/// paragraph — cmark swallows it, and roxygen2's braces then balance out on the
/// *rendered* Rd (the `\itemize{…}` wrapper roxygen2 emits supplies the closer the
/// swallowed `}` consumed). Modeling that faithfully means doing the brace
/// arithmetic on rendered text, because the closer that terminates the macro is
/// **synthetic** — it has no byte in the source, so a lossless token-tiling CST
/// cannot carry it. Bounding here instead keeps the macro's extent source-derived.
///
/// The two readings agree whenever the swallowed lines are nothing but the macro's
/// own closers (the overwhelmingly common shape); they diverge when real content
/// follows on a lazily-continued line, which stays recorded backlog.
///
/// `None` means the block's *own opening line* closes the enclosing macro, so no
/// bound can keep that closer outside the block: the caller withholds the md path.
fn md_block_body_horizon(tokens: &[Token], start: usize, mut depth: i32) -> Option<usize> {
    let mut i = line_content_start(tokens, start);
    // Set at each line boundary, so it always names the trailing `Newline` of the
    // last line scanned in full; `None` until the opening line has cleared.
    let mut horizon = None;
    loop {
        while let Some(tok) = tokens.get(i) {
            match &tok.kind {
                TokKind::RoxygenText => {
                    if brace_scan_closes_body(tok.text, &mut depth) {
                        return horizon;
                    }
                    i += 1;
                }
                k if k.roxygen_role() == Some(RoxygenRole::Content) => i += 1,
                _ => break,
            }
        }
        // Line boundary: a continuation keeps scanning; anything else ends the
        // enclosing macro anyway, so the block needs no bound.
        if tokens.get(i).map(|t| &t.kind) != Some(&TokKind::Newline) {
            return Some(tokens.len());
        }
        horizon = Some(i);
        let mut m = i + 1;
        while tokens.get(m).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
            m += 1;
        }
        if tokens.get(m).map(|t| &t.kind) != Some(&TokKind::RoxygenMarker)
            || matches!(classify_line(tokens, m), LineKind::Tag)
        {
            return Some(tokens.len());
        }
        i = line_content_start(tokens, m);
    }
}

/// Track the running brace `depth` across `text` (Rd `\`-escapes skipped),
/// returning `true` the moment a `}` appears with no group of the body's own
/// open — that `}` closes the *enclosing* block macro. Unlike [`brace_scan`], the
/// baseline is the body (depth `0`), not the macro's own group.
fn brace_scan_closes_body(text: &str, depth: &mut i32) -> bool {
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
                if *depth == 0 {
                    return true;
                }
                *depth -= 1;
                j += 1;
            }
            _ => j += 1,
        }
    }
    false
}

/// Emit the opening `\name{` of a block macro: a `ROXYGEN_RD_MACRO_NAME`, the
/// `{` delimiter, then any trailing same-line content. The parent body is the
/// empty-frame baseline ([`emit_block_content`]). The opener token is unbalanced
/// to end-of-line (the block-macro gates), so the body never closes here and the
/// remainder is always empty.
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
/// `[opt]`, and each balanced `{…}` argument group as `{`/content/`}`. The
/// content goes through the tree builder's own expansion
/// ([`crate::parser::tree_builder::build_rd_content`], reached through its
/// `Event` sink), so a nested `\code{x}` in an `\item` term is modeled exactly
/// as it would be in a single-line call, and verbatim stays per *argument*
/// ([`super::is_verbatim_rd_arg`]): `\href`'s URL is a `ROXYGEN_RD_MACRO_VERB`
/// while `\tabular`'s format is `ROXYGEN_TEXT`. The leaves tile `text` exactly.
/// The body `{` that follows is opened separately by [`emit_block_body_open`].
/// Returns the number of argument groups emitted (the caller counts the body
/// group on top).
fn emit_block_open_arg_macro(events: &mut Vec<Event>, text: &str) -> usize {
    let bytes = text.as_bytes();
    let k = super::rd_macro_name_end(bytes, 1);
    events.push(Event::Leaf(
        SyntaxKind::ROXYGEN_RD_MACRO_NAME,
        text[..k].to_string(),
    ));
    let name = &text[1..k];
    let mut j = k;
    let mut emitted = 0usize;
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
        if super::is_verbatim_rd_arg(name, emitted) {
            if !content.is_empty() {
                events.push(Event::Leaf(
                    SyntaxKind::ROXYGEN_RD_MACRO_VERB,
                    content.to_string(),
                ));
            }
        } else {
            crate::parser::tree_builder::build_rd_content(events, content);
        }
        events.push(Event::Leaf(
            SyntaxKind::ROXYGEN_RD_MACRO_DELIM,
            "}".to_string(),
        ));
        j = group_end;
        emitted += 1;
    }
    // Defensive remainder (a malformed token the gate should never admit).
    if j < text.len() {
        events.push(Event::Leaf(SyntaxKind::ROXYGEN_TEXT, text[j..].to_string()));
    }
    emitted
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
/// different `#'` lines. Returns the text *after* the terminating `}` (empty
/// unless this call closed the macro): that remainder is outside the macro, so
/// the caller decides its placement (an adjacent second argument group, or prose
/// in the enclosing context) rather than it landing inside the node.
fn emit_block_content(
    events: &mut Vec<Event>,
    text: &str,
    frames: &mut Vec<BodyFrame>,
    closed: &mut bool,
) -> String {
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
                    return text[i + 1..].to_string();
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
    String::new()
}

/// Push a non-empty `ROXYGEN_TEXT` leaf for a prose run.
fn push_text(events: &mut Vec<Event>, text: &str) {
    if !text.is_empty() {
        events.push(Event::Leaf(SyntaxKind::ROXYGEN_TEXT, text.to_string()));
    }
}
