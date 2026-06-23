//! Roxygen2 doc-comment recognition and line sub-tokenization.
//!
//! A roxygen line is a comment whose text matches `^#+'` (one-or-more `#`
//! followed by a single `'`). Such lines are sub-tokenized—rather than emitted
//! as one `COMMENT` token—so their structure (marker, tags, arguments, prose)
//! lives directly in the lossless CST. The sub-tokens' texts tile the line's
//! bytes exactly, preserving the round-trip invariant.
//!
//! Block grouping (wrapping a maximal run of roxygen lines in a `ROXYGEN_BLOCK`
//! node) happens at parse time; see [`emit_roxygen_block`].

use crate::parser::events::Event;
use crate::parser::lexer::{TokKind, Token};
use crate::syntax::SyntaxKind;

/// Roxygen tags whose first content word is a *name* argument (e.g. `@param x`,
/// `@slot name`). The first whitespace-delimited word after such a tag's name
/// is emitted as `ROXYGEN_TAG_ARG` so a future formatter can hang-indent
/// continuation lines under it. Extensible.
const ARG_BEARING_TAGS: &[&str] = &[
    "param",
    "field",
    "slot",
    "inheritParams",
    "inheritSection",
    "template",
    "templateVar",
    "method",
];

fn is_arg_bearing_tag(name: &str) -> bool {
    ARG_BEARING_TAGS.contains(&name)
}

/// True iff `text` (a comment line's text, starting at `#`) is a roxygen line:
/// one-or-more `#` then a single `'`.
pub(crate) fn is_roxygen_comment(text: &str) -> bool {
    let after_hashes = text.trim_start_matches('#');
    after_hashes.len() < text.len() && after_hashes.starts_with('\'')
}

/// Sub-tokenize a roxygen line into `out`. `text` is the line's content with no
/// trailing newline or `\r`; `start` is its absolute byte offset. The pushed
/// tokens' texts concatenate to exactly `text`.
pub(crate) fn lex_roxygen_line(out: &mut Vec<Token>, text: &str, start: usize) {
    debug_assert!(is_roxygen_comment(text));
    let bytes = text.as_bytes();

    // Marker: the `#+'` run.
    let hash_count = text.len() - text.trim_start_matches('#').len();
    let marker_len = hash_count + 1; // include the `'`
    push(out, TokKind::RoxygenMarker, text, start, 0, marker_len);

    // Whitespace between the marker and the content.
    let pos = take_ws(out, text, start, marker_len);
    if pos >= text.len() {
        return;
    }

    // A tag opens with `@` immediately followed by a letter, so `@@` (escape),
    // `@ ` and `@1` are ordinary text.
    if bytes[pos] == b'@' && bytes.get(pos + 1).is_some_and(u8::is_ascii_alphabetic) {
        lex_roxygen_tag(out, text, start, pos);
    } else {
        lex_roxygen_prose(out, text, start, pos);
    }
}

fn lex_roxygen_tag(out: &mut Vec<Token>, text: &str, start: usize, mut pos: usize) {
    let bytes = text.as_bytes();

    // `@`
    push(out, TokKind::RoxygenAt, text, start, pos, 1);
    pos += 1;

    // Tag name: `[A-Za-z][A-Za-z0-9]*` (the leading letter is guaranteed by the
    // caller). ` ` and `\t` are never UTF-8 continuation bytes, and we only
    // advance over ASCII alphanumerics here, so every slice stays on a char
    // boundary.
    let name_start = pos;
    while pos < text.len() && (bytes[pos] as char).is_ascii_alphanumeric() {
        pos += 1;
    }
    let name = text[name_start..pos].to_string();
    push(
        out,
        TokKind::RoxygenTagName,
        text,
        start,
        name_start,
        pos - name_start,
    );

    pos = take_ws(out, text, start, pos);
    if pos >= text.len() {
        return;
    }

    if is_arg_bearing_tag(&name) {
        let arg_start = pos;
        while pos < text.len() && !matches!(bytes[pos], b' ' | b'\t') {
            pos += 1;
        }
        push(
            out,
            TokKind::RoxygenTagArg,
            text,
            start,
            arg_start,
            pos - arg_start,
        );
        pos = take_ws(out, text, start, pos);
    }

    lex_roxygen_prose(out, text, start, pos);
}

/// Sub-tokenize `text[pos..]` (a roxygen line's prose remainder) into an
/// alternating sequence of `RoxygenText` runs and protected-span tokens: inline
/// code `` `…` ``, Rd macros `\code{…}`/`\link[pkg]{…}`, and markdown links
/// `[text](url)`/`[func()]`. The pushed tokens' texts tile `text[pos..]` exactly.
///
/// Recognizers are conservative and line-scoped: any malformed or unterminated
/// span stays inside the surrounding prose run (so the round-trip is unaffected
/// either way, and reflow only ever treats a *complete* span as atomic).
fn lex_roxygen_prose(out: &mut Vec<Token>, text: &str, start: usize, pos: usize) {
    let bytes = text.as_bytes();
    let mut run_start = pos;
    let mut i = pos;
    while i < bytes.len() {
        let span = match bytes[i] {
            b'`' => scan_inline_code(bytes, i).map(|end| (TokKind::RoxygenCode, end)),
            b'\\' => scan_rd_macro(bytes, i).map(|end| (TokKind::RoxygenRdMacro, end)),
            b'[' => scan_md_link(bytes, i).map(|end| (TokKind::RoxygenMdLink, end)),
            _ => None,
        };
        if let Some((kind, end)) = span {
            // Flush the prose run preceding the span, then the span itself.
            push(
                out,
                TokKind::RoxygenText,
                text,
                start,
                run_start,
                i - run_start,
            );
            push(out, kind, text, start, i, end - i);
            i = end;
            run_start = i;
        } else {
            // Not a span start: advance one whole UTF-8 char. The recognized
            // starts (`` ` ``, `\`, `[`) are all ASCII, so this only skips over
            // ordinary prose bytes.
            i += utf8_len(bytes[i]);
        }
    }
    push(
        out,
        TokKind::RoxygenText,
        text,
        start,
        run_start,
        bytes.len() - run_start,
    );
}

/// Length in bytes of the UTF-8 char whose leading byte is `b`.
fn utf8_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

/// Count the run of consecutive `c` bytes starting at `i`.
fn run_len(bytes: &[u8], i: usize, c: u8) -> usize {
    let mut j = i;
    while j < bytes.len() && bytes[j] == c {
        j += 1;
    }
    j - i
}

/// A CommonMark inline-code span at `bytes[i] == b'`'`: an opening backtick run
/// of length `n`, closed by the next run of *exactly* `n` backticks on the line.
/// Returns the index past the closing run, or `None` if unterminated.
fn scan_inline_code(bytes: &[u8], i: usize) -> Option<usize> {
    let n = run_len(bytes, i, b'`');
    let mut j = i + n;
    while j < bytes.len() {
        if bytes[j] == b'`' {
            let m = run_len(bytes, j, b'`');
            if m == n {
                return Some(j + m);
            }
            j += m;
        } else {
            j += 1;
        }
    }
    None
}

/// Inline Rd macros whose `{…}` content is **verbatim** (`VERB` in
/// `tools::parse_Rd`): the body is raw text and nested `\macro` markup is *not*
/// parsed. Confirmed against `parse_Rd` (see the projector's `rd_macros` work).
/// Latexlike macros (`\code`, `\emph`, `\strong`, `\link`, …) are everything
/// else --- their content is sub-parsed, so nested macros become child nodes.
const VERBATIM_RD_MACROS: &[&str] = &["url", "verb", "samp", "env", "kbd", "option"];

/// Whether the macro named `name` (without the leading `\`) takes verbatim
/// `{…}` content. Used both when building the CST (don't recurse into a verbatim
/// body) and when projecting it (emit `VERB`, not coalesced `TEXT`).
pub(crate) fn is_verbatim_rd_macro(name: &str) -> bool {
    VERBATIM_RD_MACROS.contains(&name)
}

/// An Rd macro at `bytes[i] == b'\\'`: `\name`, an optional balanced `[…]`, then
/// a required balanced `{…}`. Returns the index past the closing `}`, or `None`
/// when there is no name or the braces are unbalanced on the line.
pub(crate) fn scan_rd_macro(bytes: &[u8], i: usize) -> Option<usize> {
    let name_start = i + 1;
    let mut j = name_start;
    while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
        j += 1;
    }
    if j == name_start {
        return None; // `\\`, `\{`, `\n`, … are not macro calls
    }
    if bytes.get(j) == Some(&b'[') {
        j = scan_balanced(bytes, j, b'[', b']')?;
    }
    if bytes.get(j) != Some(&b'{') {
        return None;
    }
    scan_balanced(bytes, j, b'{', b'}')
}

/// A markdown link at `bytes[i] == b'['`: a balanced `[…]`, then either `(…)`
/// (inline link), `[…]` (reference link), or — for a bare `[…]` — an autolink
/// whose content is a `func()`/`pkg::func()` code reference. Returns the index
/// past the link, or `None` if it is not a recognized link shape.
fn scan_md_link(bytes: &[u8], i: usize) -> Option<usize> {
    let after_text = scan_balanced(bytes, i, b'[', b']')?;
    match bytes.get(after_text) {
        Some(&b'(') => scan_balanced(bytes, after_text, b'(', b')'),
        Some(&b'[') => scan_balanced(bytes, after_text, b'[', b']'),
        _ => is_autolink_content(&bytes[i + 1..after_text - 1]).then_some(after_text),
    }
}

/// Whether `content` (the bytes inside `[…]`) is a function-autolink reference:
/// a (possibly namespaced) identifier followed by `()`, e.g. `func()` or
/// `pkg::func()`. Conservative so bracketed prose like `[1]`/`[note]` stays text.
fn is_autolink_content(content: &[u8]) -> bool {
    let Some(name) = content.strip_suffix(b"()") else {
        return false;
    };
    !name.is_empty()
        && name.iter().any(u8::is_ascii_alphanumeric)
        && name
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':'))
}

/// Scan a balanced delimited run starting at `bytes[i] == open`, tracking nesting
/// and skipping Rd backslash escapes (`\}` etc.). Returns the index past the
/// matching `close`, or `None` if it is unbalanced before end of input.
pub(crate) fn scan_balanced(bytes: &[u8], i: usize, open: u8, close: u8) -> Option<usize> {
    debug_assert_eq!(bytes[i], open);
    let mut depth = 0usize;
    let mut j = i;
    while j < bytes.len() {
        let b = bytes[j];
        if b == b'\\' {
            j += 2; // skip the escaped byte
        } else if b == open {
            depth += 1;
            j += 1;
        } else if b == close {
            depth -= 1;
            j += 1;
            if depth == 0 {
                return Some(j);
            }
        } else {
            j += 1;
        }
    }
    None
}

/// Push `text[off..off + len]` as a token of `kind` at absolute offset
/// `start + off`. A zero-length span pushes nothing (so optional whitespace and
/// empty trailing content never produce empty tokens).
fn push(out: &mut Vec<Token>, kind: TokKind, text: &str, start: usize, off: usize, len: usize) {
    if len == 0 {
        return;
    }
    out.push(Token {
        kind,
        text: text[off..off + len].to_string(),
        start: start + off,
        end: start + off + len,
    });
}

/// Consume a run of spaces/tabs starting at `pos`, pushing a `Whitespace` token
/// if non-empty, and return the new position.
fn take_ws(out: &mut Vec<Token>, text: &str, start: usize, pos: usize) -> usize {
    let bytes = text.as_bytes();
    let mut end = pos;
    while end < text.len() && matches!(bytes[end], b' ' | b'\t') {
        end += 1;
    }
    push(out, TokKind::Whitespace, text, start, pos, end - pos);
    end
}

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
                if is_block_macro_line(tokens, i) {
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
                    i = emit_line_tokens(tokens, i, events);
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
enum LineKind {
    /// `@name …` — opens a new section.
    Tag,
    /// No prose content (marker only, or marker + whitespace) — a paragraph
    /// separator.
    Blank,
    /// Carries prose (text / inline code / Rd macro / markdown link).
    Prose,
}

/// Classify the roxygen line whose `RoxygenMarker` is at `start`.
fn classify_line(tokens: &[Token], start: usize) -> LineKind {
    let content = line_content_start(tokens, start);
    if tokens.get(content).map(|t| &t.kind) == Some(&TokKind::RoxygenAt) {
        return LineKind::Tag;
    }
    let mut i = content;
    while let Some(tok) = tokens.get(i) {
        match tok.kind {
            TokKind::RoxygenText
            | TokKind::RoxygenCode
            | TokKind::RoxygenRdMacro
            | TokKind::RoxygenMdLink => return LineKind::Prose,
            TokKind::Whitespace => i += 1,
            _ => break,
        }
    }
    LineKind::Blank
}

/// Index of the first token after the marker at `marker` and the single
/// marker→content whitespace run.
fn line_content_start(tokens: &[Token], marker: usize) -> usize {
    let mut i = marker + 1;
    while tokens.get(i).map(|t| &t.kind) == Some(&TokKind::Whitespace) {
        i += 1;
    }
    i
}

/// Whether `kind` is a roxygen line-body token (everything that can follow the
/// marker on a line).
fn is_line_body_kind(kind: &TokKind) -> bool {
    matches!(
        kind,
        TokKind::RoxygenAt
            | TokKind::RoxygenTagName
            | TokKind::RoxygenTagArg
            | TokKind::RoxygenText
            | TokKind::RoxygenCode
            | TokKind::RoxygenRdMacro
            | TokKind::RoxygenMdLink
            | TokKind::Whitespace
    )
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

/// Whether the prose line whose marker is at `start` opens a **block** Rd macro
/// --- a `\name{ … }` whose group does not close on the line, so it spans
/// following `#'` lines. The lexer extracts a *balanced* inline `\name{…}` as a
/// `RoxygenRdMacro` token; a `RoxygenText` content token beginning with `\name{`
/// is therefore necessarily an unbalanced (multi-line) opener.
fn is_block_macro_line(tokens: &[Token], start: usize) -> bool {
    let content = line_content_start(tokens, start);
    matches!(
        tokens.get(content),
        Some(tok) if tok.kind == TokKind::RoxygenText && is_block_macro_opener(&tok.text)
    )
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

/// Emit a multi-line block Rd macro as a `ROXYGEN_RD_MACRO` node spanning `#'`
/// lines. The node owns its opening line's marker and the inter-line markers,
/// newlines, and indentation as threaded trivia (losslessness); its body is a
/// sequence of brace-less name-only `\item`/`\cr`/… macros, nested inline macros,
/// and prose, ending at the matching `}` (or, for an unterminated macro, at the
/// next tag opener or block end — greedy and lossless, no close delimiter).
/// Returns the token index just past the last consumed content (at its trailing
/// `Newline` / non-roxygen token / EOF), leaving line separation to the caller.
fn emit_block_macro(tokens: &[Token], start: usize, events: &mut Vec<Event>) -> usize {
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

    // Opening `\name{` content token: split off the name and brace, then parse any
    // trailing same-line content.
    if let Some(tok) = tokens.get(i) {
        emit_block_open(events, &tok.text, &mut depth, &mut closed);
        i += 1;
    }

    'consume: while !closed {
        // Remaining content tokens on the current line.
        while let Some(tok) = tokens.get(i) {
            match tok.kind {
                TokKind::RoxygenText => {
                    emit_block_content(events, &tok.text, &mut depth, &mut closed);
                    i += 1;
                }
                // A balanced inline span (`\code{x}`, `` `code` ``, `[link]`): pass
                // the whole token through; the tree builder expands a macro token.
                TokKind::RoxygenCode | TokKind::RoxygenRdMacro | TokKind::RoxygenMdLink => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::lexer::lex;

    fn kinds(input: &str) -> Vec<TokKind> {
        lex(input).into_iter().map(|t| t.kind).collect()
    }

    /// Every lexing must be lossless: token texts concatenate to the input.
    fn assert_lossless(input: &str) {
        let joined: String = lex(input).into_iter().map(|t| t.text).collect();
        assert_eq!(joined, input, "lexing was not lossless for {input:?}");
    }

    #[test]
    fn recognizes_roxygen_prefix() {
        assert!(is_roxygen_comment("#'"));
        assert!(is_roxygen_comment("#' x"));
        assert!(is_roxygen_comment("#'x"));
        assert!(is_roxygen_comment("##' x"));
        assert!(!is_roxygen_comment("# 'x"));
        assert!(!is_roxygen_comment("# x"));
        assert!(!is_roxygen_comment("#!/usr/bin/env Rscript"));
        assert!(!is_roxygen_comment("###"));
        assert!(!is_roxygen_comment(""));
    }

    #[test]
    fn plain_comment_stays_one_token() {
        assert_eq!(kinds("# x\n"), vec![TokKind::Comment, TokKind::Newline]);
        assert_eq!(kinds("# 'x\n"), vec![TokKind::Comment, TokKind::Newline]);
    }

    #[test]
    fn simple_roxygen_line() {
        assert_eq!(
            kinds("#' Title\n"),
            vec![
                TokKind::RoxygenMarker,
                TokKind::Whitespace,
                TokKind::RoxygenText,
                TokKind::Newline,
            ]
        );
        assert_lossless("#' Title\n");
    }

    #[test]
    fn no_space_after_marker() {
        assert_eq!(
            kinds("#'x\n"),
            vec![
                TokKind::RoxygenMarker,
                TokKind::RoxygenText,
                TokKind::Newline
            ]
        );
        assert_lossless("#'x\n");
    }

    #[test]
    fn blank_roxygen_line() {
        assert_eq!(
            kinds("#'\n"),
            vec![TokKind::RoxygenMarker, TokKind::Newline]
        );
        assert_lossless("#'\n");
    }

    #[test]
    fn multi_hash_marker() {
        let toks = lex("##' x\n");
        assert_eq!(toks[0].kind, TokKind::RoxygenMarker);
        assert_eq!(toks[0].text, "##'");
        assert_lossless("##' x\n");
    }

    #[test]
    fn arg_bearing_tag() {
        assert_eq!(
            kinds("#' @param x A number.\n"),
            vec![
                TokKind::RoxygenMarker,
                TokKind::Whitespace,
                TokKind::RoxygenAt,
                TokKind::RoxygenTagName,
                TokKind::Whitespace,
                TokKind::RoxygenTagArg,
                TokKind::Whitespace,
                TokKind::RoxygenText,
                TokKind::Newline,
            ]
        );
        assert_lossless("#' @param x A number.\n");
    }

    #[test]
    fn non_arg_tag_has_no_arg_token() {
        assert_eq!(
            kinds("#' @return value\n"),
            vec![
                TokKind::RoxygenMarker,
                TokKind::Whitespace,
                TokKind::RoxygenAt,
                TokKind::RoxygenTagName,
                TokKind::Whitespace,
                TokKind::RoxygenText,
                TokKind::Newline,
            ]
        );
    }

    #[test]
    fn bare_tag_no_content() {
        assert_eq!(
            kinds("#' @examples\n"),
            vec![
                TokKind::RoxygenMarker,
                TokKind::Whitespace,
                TokKind::RoxygenAt,
                TokKind::RoxygenTagName,
                TokKind::Newline,
            ]
        );
    }

    #[test]
    fn at_escape_and_midline_at_are_text() {
        // `@@` escape and a mid-line `@` are plain text, not a tag.
        assert_eq!(
            kinds("#' @@esc\n"),
            vec![
                TokKind::RoxygenMarker,
                TokKind::Whitespace,
                TokKind::RoxygenText,
                TokKind::Newline,
            ]
        );
        assert_eq!(
            kinds("#' a @ b\n"),
            vec![
                TokKind::RoxygenMarker,
                TokKind::Whitespace,
                TokKind::RoxygenText,
                TokKind::Newline,
            ]
        );
    }

    #[test]
    fn crlf_keeps_newline_token_clean() {
        // The trailing `\r` is left to the main loop, so it joins `\n` as one
        // CRLF Newline token and never lands inside roxygen content.
        let toks = lex("#' Title\r\n");
        assert_eq!(
            toks.iter().map(|t| t.kind.clone()).collect::<Vec<_>>(),
            vec![
                TokKind::RoxygenMarker,
                TokKind::Whitespace,
                TokKind::RoxygenText,
                TokKind::Newline,
            ]
        );
        assert_eq!(toks.last().unwrap().text, "\r\n");
        assert_eq!(toks[2].text, "Title");
        assert_lossless("#' Title\r\n");
    }

    #[test]
    fn roxygen_at_eof_without_newline() {
        assert_eq!(
            kinds("#' Title"),
            vec![
                TokKind::RoxygenMarker,
                TokKind::Whitespace,
                TokKind::RoxygenText
            ]
        );
        assert_lossless("#' Title");
    }

    /// Texts of the protected-span (and surrounding text) tokens on the line.
    fn prose_texts(input: &str) -> Vec<(TokKind, String)> {
        lex(input)
            .into_iter()
            .filter(|t| {
                matches!(
                    t.kind,
                    TokKind::RoxygenText
                        | TokKind::RoxygenCode
                        | TokKind::RoxygenRdMacro
                        | TokKind::RoxygenMdLink
                )
            })
            .map(|t| (t.kind, t.text))
            .collect()
    }

    #[test]
    fn inline_code_span() {
        assert_eq!(
            prose_texts("#' Use `x + y` now\n"),
            vec![
                (TokKind::RoxygenText, "Use ".into()),
                (TokKind::RoxygenCode, "`x + y`".into()),
                (TokKind::RoxygenText, " now".into()),
            ]
        );
        assert_lossless("#' Use `x + y` now\n");
    }

    #[test]
    fn inline_code_multi_backtick_fence() {
        // A double-backtick span may contain a single backtick.
        assert_eq!(
            prose_texts("#' ``a `b` c`` end\n"),
            vec![
                (TokKind::RoxygenCode, "``a `b` c``".into()),
                (TokKind::RoxygenText, " end".into()),
            ]
        );
        assert_lossless("#' ``a `b` c`` end\n");
    }

    #[test]
    fn rd_macro_span() {
        assert_eq!(
            prose_texts("#' See \\code{f} here\n"),
            vec![
                (TokKind::RoxygenText, "See ".into()),
                (TokKind::RoxygenRdMacro, "\\code{f}".into()),
                (TokKind::RoxygenText, " here".into()),
            ]
        );
        assert_lossless("#' See \\code{f} here\n");
    }

    #[test]
    fn rd_macro_with_pkg_option() {
        assert_eq!(
            prose_texts("#' \\link[pkg]{f}\n"),
            vec![(TokKind::RoxygenRdMacro, "\\link[pkg]{f}".into())]
        );
        assert_lossless("#' \\link[pkg]{f}\n");
    }

    #[test]
    fn rd_macro_nested_braces() {
        assert_eq!(
            prose_texts("#' \\code{f(g())} x\n"),
            vec![
                (TokKind::RoxygenRdMacro, "\\code{f(g())}".into()),
                (TokKind::RoxygenText, " x".into()),
            ]
        );
        assert_lossless("#' \\code{f(g())} x\n");
    }

    #[test]
    fn md_inline_link() {
        assert_eq!(
            prose_texts("#' see [the docs](https://x.y) now\n"),
            vec![
                (TokKind::RoxygenText, "see ".into()),
                (TokKind::RoxygenMdLink, "[the docs](https://x.y)".into()),
                (TokKind::RoxygenText, " now".into()),
            ]
        );
        assert_lossless("#' see [the docs](https://x.y) now\n");
    }

    #[test]
    fn md_function_autolink() {
        assert_eq!(
            prose_texts("#' Call [func()] and [pkg::g()].\n"),
            vec![
                (TokKind::RoxygenText, "Call ".into()),
                (TokKind::RoxygenMdLink, "[func()]".into()),
                (TokKind::RoxygenText, " and ".into()),
                (TokKind::RoxygenMdLink, "[pkg::g()]".into()),
                (TokKind::RoxygenText, ".".into()),
            ]
        );
        assert_lossless("#' Call [func()] and [pkg::g()].\n");
    }

    #[test]
    fn md_reference_link() {
        assert_eq!(
            prose_texts("#' a [text][ref] b\n"),
            vec![
                (TokKind::RoxygenText, "a ".into()),
                (TokKind::RoxygenMdLink, "[text][ref]".into()),
                (TokKind::RoxygenText, " b".into()),
            ]
        );
        assert_lossless("#' a [text][ref] b\n");
    }

    #[test]
    fn bracketed_prose_is_not_a_link() {
        // Citations / plain brackets are not autolinks; stay one prose run.
        assert_eq!(
            prose_texts("#' see [1] and [a note]\n"),
            vec![(TokKind::RoxygenText, "see [1] and [a note]".into())]
        );
        assert_lossless("#' see [1] and [a note]\n");
    }

    #[test]
    fn unterminated_code_stays_prose() {
        assert_eq!(
            prose_texts("#' a ` b c\n"),
            vec![(TokKind::RoxygenText, "a ` b c".into())]
        );
        assert_lossless("#' a ` b c\n");
    }

    #[test]
    fn unbalanced_macro_stays_prose() {
        assert_eq!(
            prose_texts("#' \\code{ oops\n"),
            vec![(TokKind::RoxygenText, "\\code{ oops".into())]
        );
        assert_lossless("#' \\code{ oops\n");
    }

    #[test]
    fn backslash_without_name_stays_prose() {
        // `\\` escape and `\{` are not macro calls.
        assert_eq!(
            prose_texts("#' a \\\\ b \\{ c\n"),
            vec![(TokKind::RoxygenText, "a \\\\ b \\{ c".into())]
        );
        assert_lossless("#' a \\\\ b \\{ c\n");
    }

    #[test]
    fn spans_inside_tag_prose() {
        // Protected spans are recognized after a tag arg too.
        assert_eq!(
            prose_texts("#' @param x A \\code{value} to use\n"),
            vec![
                (TokKind::RoxygenText, "A ".into()),
                (TokKind::RoxygenRdMacro, "\\code{value}".into()),
                (TokKind::RoxygenText, " to use".into()),
            ]
        );
        assert_lossless("#' @param x A \\code{value} to use\n");
    }

    #[test]
    fn mixed_inline_markup_is_lossless() {
        assert_lossless("#' Use `x`, \\link[base]{sum}, and [g()] per [d](u).\n");
    }

    #[test]
    fn utf8_prose_around_spans_is_lossless() {
        assert_lossless("#' café `x` naïve \\code{f} résumé\n");
    }

    /// Dependency-free fuzz: every concatenation of these fragments (which are
    /// rich in markup delimiters, including malformed ones) must round-trip. The
    /// recognizers are the riskiest new code, so this exhaustively walks short
    /// combinations rather than relying on a proptest dependency.
    #[test]
    fn prose_recognizers_round_trip_exhaustively() {
        // Fragments mixing well-formed and malformed markup, brackets, escapes,
        // backticks, and multibyte prose.
        let frags = [
            "a ",
            "`x`",
            "`",
            "``",
            "\\code{f}",
            "\\code{",
            "\\",
            "\\\\",
            "[g()]",
            "[d](u)",
            "[",
            "]",
            "[1]",
            "{",
            "}",
            "café ",
            " ",
            "::",
            "()",
        ];
        for &a in &frags {
            for &b in &frags {
                for &c in &frags {
                    let input = format!("#' {a}{b}{c}\n");
                    let joined: String = lex(&input).into_iter().map(|t| t.text).collect();
                    assert_eq!(joined, input, "not lossless for {input:?}");
                }
            }
        }
    }
}
