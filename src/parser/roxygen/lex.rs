//! Roxygen line sub-lexing: turning a `#'` comment line into roxygen tokens.
//!
//! A roxygen line is a comment whose text matches `^#+'` (one-or-more `#`
//! followed by a single `'`). Such lines are sub-tokenized—rather than emitted
//! as one `COMMENT` token—so their structure (marker, tags, arguments, prose)
//! lives directly in the lossless CST. The sub-tokens' texts tile the line's
//! bytes exactly, preserving the round-trip invariant.
//!
//! This module owns the *first* phase only (text → `Vec<Token>`): block-mode
//! resolution, the per-line sub-tokenizer, and the inline-span recognizers.
//! Block grouping (`Vec<Token>` → `Vec<Event>`) lives in [`super::group`] and
//! [`super::build`].

use super::{is_two_arg_rd_macro, scan_balanced, utf8_len};
use crate::parser::lexer::{TokKind, Token};

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

/// Resolve the markdown mode of the roxygen block whose first line begins at
/// `input[start]` (the `#` of a roxygen comment), and report the byte offset of
/// that block's final line's terminating newline (or `input.len()` at EOF).
///
/// The mode is **off by default** (Rd-first); an `@md` directive line in the
/// block turns it on and an `@noMd` line turns it off (the last one in the block
/// wins, matching roxygen2's block-level toggle). The loose-file global default
/// is intentionally *not* honored yet — only an explicit per-block `@md` enables
/// markdown — so no existing block changes meaning.
///
/// A block is a maximal run of roxygen-comment lines; a continuation line may
/// carry leading indentation before its `#'` (mirroring the parser's block
/// grouping). The returned end offset lets the caller cache one resolution per
/// block: every line of the block starts before it, and the next block's first
/// line starts at or after it.
pub(crate) fn resolve_roxygen_block(input: &str, start: usize) -> (bool, usize) {
    let bytes = input.as_bytes();
    let mut md = false;
    let mut pos = start;
    loop {
        let line_end = line_run_end(bytes, pos);
        let content_end = if input[pos..line_end].ends_with('\r') {
            line_end - 1
        } else {
            line_end
        };
        if let Some(on) = roxygen_md_directive(&input[pos..content_end]) {
            md = on;
        }
        // A continuation line: skip the `\n`, then optional indentation, and check
        // for another roxygen marker. Anything else ends the block at `line_end`.
        if line_end >= bytes.len() {
            return (md, line_end);
        }
        let mut next = line_end + 1;
        while next < bytes.len() && matches!(bytes[next], b' ' | b'\t') {
            next += 1;
        }
        if next < bytes.len()
            && bytes[next] == b'#'
            && is_roxygen_comment(&input[next..line_run_end(bytes, next)])
        {
            pos = next;
        } else {
            return (md, line_end);
        }
    }
}

/// The end (exclusive) of the line starting at `i`: the next `\n`, or EOF.
fn line_run_end(bytes: &[u8], i: usize) -> usize {
    let mut j = i;
    while j < bytes.len() && bytes[j] != b'\n' {
        j += 1;
    }
    j
}

/// Whether `line` (a roxygen line's text, starting at `#`, no trailing newline)
/// is an `@md` / `@noMd` mode directive: `Some(true)` for `@md`, `Some(false)`
/// for `@noMd`, `None` otherwise. The tag must stand alone after the marker
/// (roxygen2 errors on a directive line carrying other content).
fn roxygen_md_directive(line: &str) -> Option<bool> {
    let after_hashes = line.trim_start_matches('#');
    let body = after_hashes.strip_prefix('\'')?.trim();
    match body {
        "@md" => Some(true),
        "@noMd" => Some(false),
        _ => None,
    }
}

/// Sub-tokenize a roxygen line into `out`. `text` is the line's content with no
/// trailing newline or `\r`; `start` is its absolute byte offset; `md` is the
/// block's resolved markdown mode (see [`resolve_roxygen_block`]), which keys
/// the inline grammar (markdown emphasis/strong/code is recognized only when
/// `md` is on). The pushed tokens' texts concatenate to exactly `text`.
pub(crate) fn lex_roxygen_line(out: &mut Vec<Token>, text: &str, start: usize, md: bool) {
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
        lex_roxygen_tag(out, text, start, pos, md);
    } else {
        // A prose line's content begins a fresh markdown block, so a leading list
        // marker is recognized here (`line_start`); a tag line's content is not a
        // block start, so its prose never opens a list.
        lex_roxygen_prose(out, text, start, pos, md, true);
    }
}

fn lex_roxygen_tag(out: &mut Vec<Token>, text: &str, start: usize, mut pos: usize, md: bool) {
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

    lex_roxygen_prose(out, text, start, pos, md, false);
}

/// Sub-tokenize `text[pos..]` (a roxygen line's prose remainder) into an
/// alternating sequence of `RoxygenText` runs and protected-span tokens: inline
/// code `` `…` ``, Rd macros `\code{…}`/`\link[pkg]{…}`, and markdown links
/// `[text](url)`/`[func()]`. The pushed tokens' texts tile `text[pos..]` exactly.
///
/// Recognizers are conservative and line-scoped: any malformed or unterminated
/// span stays inside the surrounding prose run (so the round-trip is unaffected
/// either way, and reflow only ever treats a *complete* span as atomic).
fn lex_roxygen_prose(
    out: &mut Vec<Token>,
    text: &str,
    start: usize,
    pos: usize,
    md: bool,
    line_start: bool,
) {
    let bytes = text.as_bytes();
    let mut run_start = pos;
    let mut i = pos;
    // Under `@md`, a prose line whose content begins with a list marker carves it
    // off as a `RoxygenMdListMarker` leaf (the trailing space stays in the prose
    // run). Whether the marker actually forms a list is a block-level decision
    // (the CommonMark interrupt rule), made later in `emit_roxygen_block`.
    if md
        && line_start
        && let Some(marker_end) = scan_md_list_marker(bytes, pos)
    {
        push(
            out,
            TokKind::RoxygenMdListMarker,
            text,
            start,
            pos,
            marker_end - pos,
        );
        run_start = marker_end;
        i = marker_end;
    }
    while i < bytes.len() {
        // Under a resolved `@md` mode the inline grammar gains markdown emphasis/
        // strong runs, and a backtick span is a *markdown* code span (projected to
        // `\code`/`\verb`) rather than a literal Rd backtick run. Without `@md` the
        // span set is the pure-Rd one (`*x*` and `` `x` `` stay literal prose).
        let span = match bytes[i] {
            b'`' if md => scan_inline_code(bytes, i).map(|end| (TokKind::RoxygenMdCode, end)),
            b'`' => scan_inline_code(bytes, i).map(|end| (TokKind::RoxygenCode, end)),
            b'*' | b'_' if md => scan_md_emphasis(bytes, i),
            b'\\' => scan_rd_macro(bytes, i).map(|end| (TokKind::RoxygenRdMacro, end)),
            b'!' if md => scan_md_image(bytes, i).map(|end| (TokKind::RoxygenMdImage, end)),
            b'[' if md => scan_md_link(bytes, i).map(|end| (TokKind::RoxygenMdLink, end)),
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

/// A markdown emphasis (`*…*`/`_…_`) or strong (`**…**`/`__…__`) span at
/// `bytes[i] in {*, _}`, recognized only under a resolved `@md` mode. Returns the
/// token kind (`RoxygenMdStrong` for a two-delimiter run, `RoxygenMdEmph` for a
/// one-delimiter run) and the index past the closing delimiter run, or `None`
/// when this is not a valid span (so it stays literal prose — losslessness holds
/// either way).
///
/// A pragmatic CommonMark subset sufficient for the inline foundation: the
/// opening run is 1 (emphasis) or 2 (strong) delimiters — a 3+ run is the
/// ambiguous combined form and bails. The opener must be left-flanking (followed
/// by a non-space) and the closer right-flanking (preceded by a non-space), and
/// an `_` run may not sit intraword (CommonMark forbids `snake_case` emphasis).
/// Nested/mismatched runs that don't satisfy this bail to text — a faithful
/// *under*-recognition, never a wrong structure.
fn scan_md_emphasis(bytes: &[u8], i: usize) -> Option<(TokKind, usize)> {
    let delim = bytes[i];
    let open_len = run_len(bytes, i, delim);
    if open_len >= 3 {
        return None; // combined emph+strong — out of foundation scope
    }
    let n = open_len; // 1 → emphasis, 2 → strong
    let content_start = i + n;
    // Opener must be left-flanking: a non-whitespace char follows the run.
    if bytes
        .get(content_start)
        .is_none_or(|b| b.is_ascii_whitespace())
    {
        return None;
    }
    // `_` cannot open intraword: the char before the run must not be alphanumeric.
    if delim == b'_' && i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
        return None;
    }
    let mut j = content_start;
    while j < bytes.len() {
        if bytes[j] == delim {
            let run = run_len(bytes, j, delim);
            // A closer of at least `n` delimiters that is right-flanking (the
            // preceding char is non-space) and, for `_`, not intraword.
            let close_end = j + n;
            if run >= n
                && j > content_start
                && !bytes[j - 1].is_ascii_whitespace()
                && (delim != b'_'
                    || bytes
                        .get(close_end)
                        .is_none_or(|b| !b.is_ascii_alphanumeric()))
            {
                let kind = if n == 2 {
                    TokKind::RoxygenMdStrong
                } else {
                    TokKind::RoxygenMdEmph
                };
                return Some((kind, close_end));
            }
            j += run;
        } else {
            j += utf8_len(bytes[j]);
        }
    }
    None
}

/// A markdown list-item marker at a line's content start: a bullet (`-`/`*`/`+`)
/// or an ordered marker (a run of up to nine ASCII digits then `.`/`)`), in
/// either case followed by a space/tab or the end of the line (CommonMark).
/// Returns the byte length of the marker *punctuation only* — the trailing space
/// is left in the following prose run, so a marker that turns out not to form a
/// list (the interrupt rule fails) reflows exactly like the plain text it stands
/// in for. `None` when the content does not open a list item.
fn scan_md_list_marker(bytes: &[u8], i: usize) -> Option<usize> {
    let marker_end = match bytes.get(i)? {
        b'-' | b'*' | b'+' => i + 1,
        b'0'..=b'9' => {
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j - i > 9 {
                return None; // CommonMark caps the start number at nine digits
            }
            match bytes.get(j) {
                Some(b'.' | b')') => j + 1,
                _ => return None,
            }
        }
        _ => return None,
    };
    match bytes.get(marker_end) {
        None | Some(b' ' | b'\t') => Some(marker_end),
        _ => None,
    }
}

/// A markdown inline image at `bytes[i] == b'!'`: `![alt](url "title")`. Requires
/// a `[` immediately after the `!`, a balanced `[…]` alt span, then a `(…)`
/// destination group. Returns the index past the closing `)`, or `None` when it is
/// not a complete inline image (so it stays literal prose — losslessness holds
/// either way). Only the **inline** form is recognized; a reference/shortcut image
/// (`![alt][ref]`/`![alt]`) is left to the prose path (un-handled-shape backlog).
fn scan_md_image(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.get(i + 1) != Some(&b'[') {
        return None;
    }
    let after_alt = scan_balanced(bytes, i + 1, b'[', b']')?;
    match bytes.get(after_alt) {
        Some(&b'(') => scan_balanced(bytes, after_alt, b'(', b')'),
        _ => None,
    }
}

/// A markdown link at `bytes[i] == b'['`: a balanced `[…]`, then either `(…)`
/// (inline link `[text](url)`), `[…]` (reference link `[text][ref]`), or — for a
/// bare `[…]` — a *shortcut* link `[dest]`. Returns the index past the link, or
/// `None` if it is not a recognized link shape.
///
/// roxygen2 turns **every** bracketed span into a link reference
/// (`get_md_linkrefs` in `markdown-link.R`: any non-empty bracket-free content,
/// not followed by `[` or `{`), so a bare `[note]`/`[see this]`/`[pkg::obj]`
/// resolves to `\link{…}` just like `[func()]`. The followed-by-`{` exclusion
/// keeps a pandoc-style `[x]{…}` (and a literal `\foo{…}` written under `@md`)
/// out — see [`is_shortcut_content`].
fn scan_md_link(bytes: &[u8], i: usize) -> Option<usize> {
    let after_text = scan_balanced(bytes, i, b'[', b']')?;
    match bytes.get(after_text) {
        Some(&b'(') => scan_balanced(bytes, after_text, b'(', b')'),
        Some(&b'[') => scan_balanced(bytes, after_text, b'[', b']'),
        // A bare `[…]` followed by `{` is not a link (roxygen2's lookahead).
        Some(&b'{') => None,
        _ => is_shortcut_content(&bytes[i + 1..after_text - 1]).then_some(after_text),
    }
}

/// Whether `content` (the bytes inside a bare `[…]`) is a markdown shortcut-link
/// reference. roxygen2's `get_md_linkrefs` regex accepts any non-empty span that
/// contains no brackets (`[^\]\[]+`), so spaces, digits, and `::` are all fine
/// (`[note]`, `[see this]`, `[pkg::obj]`); only an empty or bracket-bearing span
/// is rejected (the latter so nested `[a[b]c]` re-scans the inner `[b]`).
fn is_shortcut_content(content: &[u8]) -> bool {
    !content.is_empty() && !content.iter().any(|&b| matches!(b, b'[' | b']'))
}

/// An Rd macro at `bytes[i] == b'\\'`: `\name`, an optional balanced `[…]`, then
/// a required balanced `{…}` (and a second `{…}` for a two-argument macro like
/// `\item`). Returns the index past the last consumed `}`, or `None` when there
/// is no name or the first braces are unbalanced on the line.
pub(crate) fn scan_rd_macro(bytes: &[u8], i: usize) -> Option<usize> {
    let name_start = i + 1;
    let mut j = name_start;
    while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
        j += 1;
    }
    if j == name_start {
        return None; // `\\`, `\{`, `\n`, … are not macro calls
    }
    let name = std::str::from_utf8(&bytes[name_start..j]).unwrap_or_default();
    if bytes.get(j) == Some(&b'[') {
        j = scan_balanced(bytes, j, b'[', b']')?;
    }
    if bytes.get(j) != Some(&b'{') {
        return None;
    }
    let mut end = scan_balanced(bytes, j, b'{', b'}')?;
    // A two-argument macro pulls its adjacent second `{…}` group into the same
    // token; an unbalanced or absent second group leaves `end` after the first.
    if is_two_arg_rd_macro(name)
        && bytes.get(end) == Some(&b'{')
        && let Some(second) = scan_balanced(bytes, end, b'{', b'}')
    {
        end = second;
    }
    Some(end)
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
                        | TokKind::RoxygenMdImage
                        | TokKind::RoxygenMdEmph
                        | TokKind::RoxygenMdStrong
                        | TokKind::RoxygenMdCode
                        | TokKind::RoxygenMdListMarker
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
    fn md_inline_recognized_under_md_mode() {
        // With an `@md` directive in the block, emphasis/strong runs and a
        // markdown code span are carved out as their own leaves.
        let src = "#' a *one*, **two**, and `three` end.\n#' @md\n";
        assert_eq!(
            prose_texts(src),
            vec![
                (TokKind::RoxygenText, "a ".into()),
                (TokKind::RoxygenMdEmph, "*one*".into()),
                (TokKind::RoxygenText, ", ".into()),
                (TokKind::RoxygenMdStrong, "**two**".into()),
                (TokKind::RoxygenText, ", and ".into()),
                (TokKind::RoxygenMdCode, "`three`".into()),
                (TokKind::RoxygenText, " end.".into()),
            ]
        );
        assert_lossless(src);
    }

    #[test]
    fn md_list_marker_recognized_under_md_mode() {
        // A bullet or ordered marker at a line's content start is carved off as a
        // `RoxygenMdListMarker` (punctuation only; the trailing space stays in the
        // following text run).
        let bullet = "#' - first step\n#' @md\n";
        assert_eq!(
            prose_texts(bullet),
            vec![
                (TokKind::RoxygenMdListMarker, "-".into()),
                (TokKind::RoxygenText, " first step".into()),
            ]
        );
        assert_lossless(bullet);
        let ordered = "#' 1. one\n#' @md\n";
        assert_eq!(
            prose_texts(ordered),
            vec![
                (TokKind::RoxygenMdListMarker, "1.".into()),
                (TokKind::RoxygenText, " one".into()),
            ]
        );
        assert_lossless(ordered);
    }

    #[test]
    fn md_list_marker_off_without_md_directive() {
        // No `@md`: a leading `-` stays literal prose, no list marker token.
        assert_eq!(
            prose_texts("#' - first step\n"),
            vec![(TokKind::RoxygenText, "- first step".into())]
        );
    }

    #[test]
    fn md_list_marker_requires_space_and_is_not_emphasis() {
        // Under `@md`, a `*` at line start followed by a non-space is emphasis, not
        // a list marker; `-3` (no space) is plain text; `* item` is a bullet.
        let src = "#' * a *b* c\n#' @md\n";
        assert_eq!(
            prose_texts(src),
            vec![
                (TokKind::RoxygenMdListMarker, "*".into()),
                (TokKind::RoxygenText, " a ".into()),
                (TokKind::RoxygenMdEmph, "*b*".into()),
                (TokKind::RoxygenText, " c".into()),
            ]
        );
        assert_lossless(src);
        // A bare `-3` (no space after the marker) is not a list item.
        assert_eq!(
            prose_texts("#' -3 degrees\n#' @md\n"),
            vec![(TokKind::RoxygenText, "-3 degrees".into())]
        );
    }

    #[test]
    fn md_inline_off_without_md_directive() {
        // No `@md`: the markdown delimiters stay literal prose and a backtick span
        // is the pure-Rd `ROXYGEN_CODE`, not a markdown code span.
        assert_eq!(
            prose_texts("#' a *one* and `code` end\n"),
            vec![
                (TokKind::RoxygenText, "a *one* and ".into()),
                (TokKind::RoxygenCode, "`code`".into()),
                (TokKind::RoxygenText, " end".into()),
            ]
        );
    }

    #[test]
    fn md_emphasis_flanking_rejects_false_positives() {
        // Under `@md`, whitespace-flanked `*` and intraword `_` are not emphasis
        // (CommonMark flanking) --- they stay literal text, so the line is one run.
        let src = "#' a * b * c and snake_case_name here\n#' @md\n";
        assert_eq!(
            prose_texts(src),
            vec![(
                TokKind::RoxygenText,
                "a * b * c and snake_case_name here".into(),
            )]
        );
        assert_lossless(src);
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
            prose_texts("#' see [the docs](https://x.y) now\n#' @md\n"),
            vec![
                (TokKind::RoxygenText, "see ".into()),
                (TokKind::RoxygenMdLink, "[the docs](https://x.y)".into()),
                (TokKind::RoxygenText, " now".into()),
            ]
        );
        assert_lossless("#' see [the docs](https://x.y) now\n#' @md\n");
    }

    #[test]
    fn md_function_autolink() {
        assert_eq!(
            prose_texts("#' Call [func()] and [pkg::g()].\n#' @md\n"),
            vec![
                (TokKind::RoxygenText, "Call ".into()),
                (TokKind::RoxygenMdLink, "[func()]".into()),
                (TokKind::RoxygenText, " and ".into()),
                (TokKind::RoxygenMdLink, "[pkg::g()]".into()),
                (TokKind::RoxygenText, ".".into()),
            ]
        );
        assert_lossless("#' Call [func()] and [pkg::g()].\n#' @md\n");
    }

    #[test]
    fn md_reference_link() {
        assert_eq!(
            prose_texts("#' a [text][ref] b\n#' @md\n"),
            vec![
                (TokKind::RoxygenText, "a ".into()),
                (TokKind::RoxygenMdLink, "[text][ref]".into()),
                (TokKind::RoxygenText, " b".into()),
            ]
        );
        assert_lossless("#' a [text][ref] b\n#' @md\n");
    }

    #[test]
    fn link_shape_is_literal_text_without_md() {
        // A `[text](url)` shape is a markdown link only under `@md`; without it the
        // brackets are literal Rd prose, so no `RoxygenMdLink` is carved.
        assert_eq!(
            prose_texts("#' see [the docs](https://x.y) now\n"),
            vec![(
                TokKind::RoxygenText,
                "see [the docs](https://x.y) now".into()
            )]
        );
        assert_lossless("#' see [the docs](https://x.y) now\n");
    }

    #[test]
    fn bracketed_prose_is_literal_without_md() {
        // Without `@md`, brackets are literal Rd prose, not links — they stay one
        // prose run. (Under `@md` roxygen2 treats every `[…]` as a link; see
        // `md_shortcut_link`.)
        assert_eq!(
            prose_texts("#' see [1] and [a note]\n"),
            vec![(TokKind::RoxygenText, "see [1] and [a note]".into())]
        );
        assert_lossless("#' see [1] and [a note]\n");
    }

    #[test]
    fn md_shortcut_link() {
        // Under `@md`, any bracket-free `[…]` is a shortcut link — words, digits,
        // spaces, and `::` all qualify — but a `[…]{` is excluded.
        assert_eq!(
            prose_texts("#' see [note], [see this], [pkg::obj] but [x]{y}\n#' @md\n"),
            vec![
                (TokKind::RoxygenText, "see ".into()),
                (TokKind::RoxygenMdLink, "[note]".into()),
                (TokKind::RoxygenText, ", ".into()),
                (TokKind::RoxygenMdLink, "[see this]".into()),
                (TokKind::RoxygenText, ", ".into()),
                (TokKind::RoxygenMdLink, "[pkg::obj]".into()),
                (TokKind::RoxygenText, " but [x]{y}".into()),
            ]
        );
        assert_lossless("#' see [note], [see this], [pkg::obj] but [x]{y}\n#' @md\n");
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
