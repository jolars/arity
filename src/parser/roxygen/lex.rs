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
    // Under `@md`, a prose line whose content begins with a code fence (3+
    // backticks) carves the *whole* remaining line off as a `RoxygenMdFence`
    // leaf (an opener with its info string, or a bare closer). The block builder
    // pairs an opener with its closer into a `ROXYGEN_MD_CODE_BLOCK`; the leaf's
    // existence implies `@md` (the single mode source is the lexer), so the
    // builder keys off the token kind, never re-deriving mode.
    if md
        && line_start
        && let Some(fence_end) = scan_md_fence(bytes, pos)
    {
        push(
            out,
            TokKind::RoxygenMdFence,
            text,
            start,
            pos,
            fence_end - pos,
        );
        return;
    }
    // Under `@md`, a prose line whose content begins with a CommonMark HTML-block
    // start (condition 6 — a block-level tag) carves the *whole* remaining line off
    // as a `RoxygenMdHtmlBlock` opener leaf. The block builder gathers the opener
    // and the following lines (to the next blank line) into a `ROXYGEN_MD_HTML_BLOCK`;
    // the leaf's existence implies `@md`, so the builder keys off the token kind.
    if md
        && line_start
        && let Some(block_end) = scan_md_html_block(bytes, pos)
    {
        push(
            out,
            TokKind::RoxygenMdHtmlBlock,
            text,
            start,
            pos,
            block_end - pos,
        );
        return;
    }
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
        // Under `@md`, an inline link `[text](url)`: carve the `[` opener and the
        // `](url)` closer as neutral `RoxygenMdBracket` leaves and *recursively* lex
        // the link text in between, so emphasis/code spans inside it resolve. The
        // inline pass then assembles the matched pair into a `ROXYGEN_MD_LINK`
        // **node** whose display children are the resolved markdown. Reference and
        // shortcut links (`[t][r]`, `[t]`) stay opaque (the `scan_md_link` arm
        // below); only a *bracket-free* inline link text is split here.
        if md
            && bytes[i] == b'['
            && let Some((text_end, url_end)) = inline_link_span(bytes, i)
        {
            push(
                out,
                TokKind::RoxygenText,
                text,
                start,
                run_start,
                i - run_start,
            );
            push(out, TokKind::RoxygenMdBracket, text, start, i, 1);
            let inner = &text[i + 1..text_end - 1];
            lex_roxygen_prose(out, inner, start + i + 1, 0, md, false);
            push(
                out,
                TokKind::RoxygenMdBracket,
                text,
                start,
                text_end - 1,
                url_end - (text_end - 1),
            );
            i = url_end;
            run_start = i;
            continue;
        }
        // A *cross-line* inline-link opener: a `[` whose bracketed text is not
        // closed on this line (no same-line `]`, so the same-line link paths
        // above and `scan_md_link` below do not apply). Carve the `[` as a neutral
        // bracket opener leaf; the link text continues on following `#'` lines and
        // the inline pass pairs it with the later `](url)` closer over the
        // paragraph-granularity run (literal text if it never matches).
        if md && bytes[i] == b'[' && is_cross_line_link_opener(bytes, i) {
            push(
                out,
                TokKind::RoxygenText,
                text,
                start,
                run_start,
                i - run_start,
            );
            push(out, TokKind::RoxygenMdBracket, text, start, i, 1);
            i += 1;
            run_start = i;
            continue;
        }
        // A *cross-line* inline-link closer: a `](url)` whose matching `[` opened
        // on an earlier `#'` line. A same-line `[…](url)` is consumed whole by the
        // opener path above, so a bare `]` immediately followed by a balanced
        // `(url)` here has no same-line opener — carve it as a neutral bracket
        // closer leaf (the inline pass pairs it with the earlier opener, or leaves
        // it literal when unmatched).
        if md
            && bytes[i] == b']'
            && let Some(url_end) = cross_line_link_closer(bytes, i)
        {
            push(
                out,
                TokKind::RoxygenText,
                text,
                start,
                run_start,
                i - run_start,
            );
            push(out, TokKind::RoxygenMdBracket, text, start, i, url_end - i);
            i = url_end;
            run_start = i;
            continue;
        }
        // A *cross-line* reference-link closer: a bare `]` immediately followed by a
        // `[ref]` label, where the `]` closes a `[` that opened on an earlier `#'`
        // line (a `[text][ref]` reference link spanning lines). Carve only the lone
        // `]` as a neutral bracket leaf; the `[ref]` label is carved separately
        // ([`scan_md_link`], guaranteed a clean shortcut by `cross_line_ref_closer`).
        // The inline pass then either pairs the `]` with the earlier opener —
        // consuming the label as the dropped topic — or, with no opener, leaves the
        // `]` literal and the `[ref]` a standalone shortcut, so `a][b]` stays
        // `a]` + a `[b]` shortcut, matching roxygen2.
        if md && bytes[i] == b']' && cross_line_ref_closer(bytes, i) {
            push(
                out,
                TokKind::RoxygenText,
                text,
                start,
                run_start,
                i - run_start,
            );
            push(out, TokKind::RoxygenMdBracket, text, start, i, 1);
            i += 1;
            run_start = i;
            continue;
        }
        // Under a resolved `@md` mode the inline grammar gains markdown emphasis/
        // strong runs, and a backtick span is a *markdown* code span (projected to
        // `\code`/`\verb`) rather than a literal Rd backtick run. Without `@md` the
        // span set is the pure-Rd one (`*x*` and `` `x` `` stay literal prose).
        let span = match bytes[i] {
            b'`' if md => scan_inline_code(bytes, i).map(|end| (TokKind::RoxygenMdCode, end)),
            b'`' => scan_inline_code(bytes, i).map(|end| (TokKind::RoxygenCode, end)),
            // A `*`/`_` run under `@md` is carved *neutrally* as a maximal same-
            // char delimiter run (`RoxygenMdDelim`); the open/close decision and
            // matching are the inline pass's job (CommonMark delimiter stack), not
            // the lexer's. Always carves (losslessness holds — it is still text).
            b'*' | b'_' if md => Some((TokKind::RoxygenMdDelim, i + run_len(bytes, i, bytes[i]))),
            // A balanced inline `\name{…}` is its own span; an *unbalanced* `\name{`
            // opens a block macro that spans following `#'` lines. The opener runs
            // to the line end (its body is unclosed here), so carve `\name{…EOL` off
            // as its own `RoxygenText` token: at line start this reproduces the same
            // whole-line token, and mid-prose it splits the preceding run off so the
            // grouper can promote the opener to a block macro (when it later closes).
            b'\\' => scan_rd_macro(bytes, i)
                .map(|end| (TokKind::RoxygenRdMacro, end))
                .or_else(|| {
                    is_block_macro_opener_at(bytes, i)
                        .then_some((TokKind::RoxygenText, bytes.len()))
                }),
            b'!' if md => scan_md_image(bytes, i).map(|end| (TokKind::RoxygenMdImage, end)),
            b'[' if md => scan_md_link(bytes, i).map(|end| (TokKind::RoxygenMdLink, end)),
            b'<' if md => scan_md_autolink(bytes, i)
                .map(|end| (TokKind::RoxygenMdLink, end))
                .or_else(|| scan_md_html_inline(bytes, i).map(|end| (TokKind::RoxygenMdHtml, end))),
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

/// A markdown code fence at a line's content start: a run of three or more
/// backticks (the opener may carry an info string after the run). CommonMark
/// forbids a backtick inside a backtick fence's info string, so the whole
/// remaining line is the fence iff no backtick follows the opening run — which
/// also keeps an inline code span (`` `x` ``) that merely starts the line from
/// being mistaken for a fence. Returns the index past the fence (the end of the
/// line content), or `None` when the content does not open/close a fence.
fn scan_md_fence(bytes: &[u8], i: usize) -> Option<usize> {
    if run_len(bytes, i, b'`') < 3 {
        return None;
    }
    let info = i + run_len(bytes, i, b'`');
    if bytes[info..].contains(&b'`') {
        return None;
    }
    Some(bytes.len())
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
/// An inline markdown link `[text](url)` at `bytes[i] == b'['`: a balanced `[…]`
/// whose text is **bracket-free** (so it carries no nested link), immediately
/// followed by a balanced `(…)` destination. Returns `(text_end, url_end)` — the
/// index past the `]` and the index past the `)`. `None` when it is not a complete
/// bracket-free inline link, in which case the reference/shortcut path
/// ([`scan_md_link`]) applies and the link stays an opaque token.
///
/// The bracket-free restriction keeps the conservative opaque path for nested
/// brackets (`[a [b] c](url)`), whose CommonMark resolution (no nested links,
/// opener deactivation) the split path does not yet model.
fn inline_link_span(bytes: &[u8], i: usize) -> Option<(usize, usize)> {
    let text_end = scan_balanced(bytes, i, b'[', b']')?;
    if bytes[i + 1..text_end - 1]
        .iter()
        .any(|&b| matches!(b, b'[' | b']'))
    {
        return None;
    }
    if bytes.get(text_end) != Some(&b'(') {
        return None;
    }
    let url_end = scan_balanced(bytes, text_end, b'(', b')')?;
    Some((text_end, url_end))
}

/// Whether a `[` at `bytes[i]` opens a *cross-line* inline link: the remainder of
/// this line is bracket-free (no `[` or `]`), so its bracketed text is not closed
/// here and the matching `](url)` closer appears on a following `#'` line. A `[`
/// with a same-line `]` is left to the same-line link paths ([`inline_link_span`]
/// / [`scan_md_link`]); the bracket-free restriction keeps a nested-bracket text
/// on the conservative opaque path (matching the same-line split's own guard).
fn is_cross_line_link_opener(bytes: &[u8], i: usize) -> bool {
    !bytes[i + 1..].iter().any(|&b| matches!(b, b'[' | b']'))
}

/// The end index of a `](url)` inline-link closer at `bytes[i] == b']'`: the `]`
/// must be immediately followed by a balanced `(url)` destination. Returns the
/// index past the closing `)`, or `None` when it is not a closer. A same-line
/// `[…](url)` is consumed whole by the opener path, so a bare `]` reaching this
/// point has no same-line opener — the inline pass pairs it with a cross-line one.
fn cross_line_link_closer(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.get(i + 1) != Some(&b'(') {
        return None;
    }
    scan_balanced(bytes, i + 1, b'(', b')')
}

/// Whether a `]` at `bytes[i]` is a *cross-line* reference-link closer: it is
/// immediately followed by a balanced, bracket-free `[ref]` label that is a clean
/// shortcut (not itself followed by `(`, `[`, or `{`, the shapes that would make
/// [`scan_md_link`] reject or re-read it). Used to carve the lone `]` as a neutral
/// bracket leaf; the `[ref]` label is carved separately as a shortcut `MD_LINK`,
/// and the inline pass pairs the `]` with an earlier cross-line `[` opener (a
/// `[text][ref]` reference link spanning lines, the label consumed as the dropped
/// topic) or — with no opener — leaves the `]` literal and the `[ref]` a standalone
/// shortcut link (so `a][b]` stays `a]` + `\link{b}`). The clean-shortcut guard
/// keeps the lexer's lone-`]` carve in lockstep with the label that follows it, so
/// the inline pass never faces a lone `]` without its label token.
fn cross_line_ref_closer(bytes: &[u8], i: usize) -> bool {
    bytes.get(i + 1) == Some(&b'[')
        && scan_balanced(bytes, i + 1, b'[', b']').is_some_and(|end| {
            is_shortcut_content(&bytes[i + 2..end - 1])
                && !matches!(bytes.get(end), Some(b'(' | b'[' | b'{'))
        })
}

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

/// A CommonMark absolute-URI autolink at `bytes[i] == b'<'`: `<scheme:body>` where
/// the scheme is 2–32 chars beginning with an ASCII letter, then ASCII letters,
/// digits, `+`, `.`, or `-`; the body runs to the next `>` and may not contain a
/// space, `<`, or an ASCII control character. Returns the index past `>`, or
/// `None` when it is not a valid autolink — so raw HTML (`<p>`, `<img …>`, no
/// scheme `:`) and email autolinks (no `:`, out of scope) stay literal prose.
/// roxygen2's `mdxml_link` renders such a link (whose destination equals its text)
/// as `\url{…}`.
fn scan_md_autolink(bytes: &[u8], i: usize) -> Option<usize> {
    let scheme_start = i + 1;
    if !bytes.get(scheme_start).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut j = scheme_start + 1;
    while j < bytes.len()
        && matches!(bytes[j], b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'.' | b'-')
    {
        j += 1;
    }
    if !(2..=32).contains(&(j - scheme_start)) || bytes.get(j) != Some(&b':') {
        return None;
    }
    j += 1;
    while j < bytes.len() {
        match bytes[j] {
            b'>' => return Some(j + 1),
            b' ' | b'<' => return None,
            c if c.is_ascii_control() => return None,
            _ => j += 1,
        }
    }
    None
}

/// A CommonMark inline raw-HTML tag at `bytes[i] == b'<'`: an open tag
/// (`<name attrs… />`) or a closing tag (`</name >`), line-scoped. Returns the
/// index past the closing `>`, or `None` when this is not a well-formed tag (it
/// then stays literal prose — losslessness holds either way). The recognizer
/// mirrors the CommonMark "Raw HTML" grammar precisely so it never carves a span
/// `commonmark` (hence roxygen2) would keep literal; over-recognition would make
/// the projector emit a spurious `\out`. The comment / processing-instruction /
/// declaration / CDATA forms are **not** recognized (they stay literal — a
/// faithful under-handling, backlog).
///
/// roxygen2's `mdxml_html_inline` renders such a tag verbatim inside
/// `\if{html}{\out{…}}`.
fn scan_md_html_inline(bytes: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    let closing = bytes.get(j) == Some(&b'/');
    if closing {
        j += 1;
    }
    // Tag name: an ASCII letter then letters/digits/`-`.
    if !bytes.get(j).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    j += 1;
    while bytes
        .get(j)
        .is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'-')
    {
        j += 1;
    }
    if closing {
        // A closing tag takes no attributes: optional whitespace then `>`.
        j = skip_html_ws(bytes, j);
        return (bytes.get(j) == Some(&b'>')).then_some(j + 1);
    }
    // Zero or more attributes, each preceded by required whitespace.
    loop {
        let after_ws = skip_html_ws(bytes, j);
        if after_ws == j {
            break; // no whitespace ⇒ no further attribute
        }
        match scan_html_attribute(bytes, after_ws) {
            Some(end) => j = end,
            None => {
                j = after_ws;
                break;
            }
        }
    }
    // Optional whitespace, an optional self-closing `/`, then `>`.
    j = skip_html_ws(bytes, j);
    if bytes.get(j) == Some(&b'/') {
        j += 1;
    }
    (bytes.get(j) == Some(&b'>')).then_some(j + 1)
}

/// A CommonMark **HTML block start condition 6** at a line's content start: `<`
/// or `</`, then one of the block-level tag names (case-insensitive), then a
/// space/tab, `>`, `/>`, or the end of the line. Such a line opens an HTML block
/// that (per CommonMark) runs to the next blank line, so the whole remaining line
/// content is the opener leaf; this returns the index past it (the end of the
/// line). `None` otherwise.
///
/// Only condition 6 is recognized. Conditions 1–5 (`<script>`/`<pre>`/comments/
/// processing instructions/declarations/CDATA — each with its own non-blank-line
/// terminator) and condition 7 (a complete tag alone on a line) are faithful
/// under-handling: those forms stay literal prose or inline HTML (backlog), so the
/// modeled block is exactly the blank-line-terminated one roxygen2's corpus uses.
fn scan_md_html_block(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.get(i) != Some(&b'<') {
        return None;
    }
    let mut j = i + 1;
    if bytes.get(j) == Some(&b'/') {
        j += 1;
    }
    let name_start = j;
    while bytes.get(j).is_some_and(u8::is_ascii_alphanumeric) {
        j += 1;
    }
    if !is_html_block_tag(&bytes[name_start..j]) {
        return None;
    }
    match bytes.get(j) {
        None | Some(b' ' | b'\t' | b'>') => Some(bytes.len()),
        Some(b'/') if bytes.get(j + 1) == Some(&b'>') => Some(bytes.len()),
        _ => None,
    }
}

/// Whether `name` (ASCII, case-insensitive) is one of CommonMark's block-level
/// tag names for HTML-block start condition 6.
fn is_html_block_tag(name: &[u8]) -> bool {
    const BLOCK_TAGS: &[&str] = &[
        "address",
        "article",
        "aside",
        "base",
        "basefont",
        "blockquote",
        "body",
        "caption",
        "center",
        "col",
        "colgroup",
        "dd",
        "details",
        "dialog",
        "dir",
        "div",
        "dl",
        "dt",
        "fieldset",
        "figcaption",
        "figure",
        "footer",
        "form",
        "frame",
        "frameset",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "head",
        "header",
        "hr",
        "html",
        "iframe",
        "legend",
        "li",
        "link",
        "main",
        "menu",
        "menuitem",
        "nav",
        "noframes",
        "ol",
        "optgroup",
        "option",
        "p",
        "param",
        "search",
        "section",
        "summary",
        "table",
        "tbody",
        "td",
        "tfoot",
        "th",
        "thead",
        "title",
        "tr",
        "track",
        "ul",
    ];
    let Ok(name) = std::str::from_utf8(name) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    BLOCK_TAGS.contains(&lower.as_str())
}

/// Advance past a run of ASCII spaces and tabs (the line-scoped subset of
/// CommonMark "whitespace" — line endings cannot occur inside a sub-lexed line).
fn skip_html_ws(bytes: &[u8], i: usize) -> usize {
    let mut j = i;
    while bytes.get(j).is_some_and(|&b| matches!(b, b' ' | b'\t')) {
        j += 1;
    }
    j
}

/// A CommonMark HTML-tag attribute at `bytes[i]` (the byte after the required
/// whitespace): an attribute name, optionally followed by `=` and a quoted or
/// unquoted value. Returns the index past the attribute, or `None` when `i` does
/// not start a valid attribute name (or a present `=` lacks a valid value).
fn scan_html_attribute(bytes: &[u8], i: usize) -> Option<usize> {
    // Name: [A-Za-z_:][A-Za-z0-9_.:-]*
    if !bytes
        .get(i)
        .is_some_and(|&b| b.is_ascii_alphabetic() || matches!(b, b'_' | b':'))
    {
        return None;
    }
    let mut j = i + 1;
    while bytes
        .get(j)
        .is_some_and(|&b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
    {
        j += 1;
    }
    let after_name = j;
    // Optional value: `\s* = \s* value`.
    let eq = skip_html_ws(bytes, j);
    if bytes.get(eq) != Some(&b'=') {
        return Some(after_name);
    }
    j = skip_html_ws(bytes, eq + 1);
    match bytes.get(j) {
        Some(&q @ (b'\'' | b'"')) => {
            j += 1;
            while bytes.get(j).is_some_and(|&b| b != q) {
                j += 1;
            }
            (bytes.get(j) == Some(&q)).then_some(j + 1)
        }
        _ => {
            // Unquoted value: one or more chars excluding whitespace and
            // `"'=<>` `` ` ``.
            let start = j;
            while bytes.get(j).is_some_and(|&b| {
                !matches!(b, b' ' | b'\t' | b'"' | b'\'' | b'=' | b'<' | b'>' | b'`')
            }) {
                j += 1;
            }
            (j > start).then_some(j)
        }
    }
}

/// An Rd macro at `bytes[i] == b'\\'`: `\name`, an optional balanced `[…]`, then
/// a required balanced `{…}` (and a second `{…}` for a two-argument macro like
/// `\item`). Returns the index past the last consumed `}`, or `None` when there
/// is no name or the first braces are unbalanced on the line.
/// Whether `bytes[i] == b'\\'` begins an **unbalanced** `\name{` block-macro
/// opener — a `\name{` whose `{` group does not close within the line (so its
/// body spans following `#'` lines). The balanced inline case is handled by
/// [`scan_rd_macro`]; this is the residual the grouper promotes to a block macro.
fn is_block_macro_opener_at(bytes: &[u8], i: usize) -> bool {
    let k = super::rd_macro_name_end(bytes, i + 1);
    k > i + 1 && bytes.get(k) == Some(&b'{') && scan_balanced(bytes, k, b'{', b'}').is_none()
}

pub(crate) fn scan_rd_macro(bytes: &[u8], i: usize) -> Option<usize> {
    let name_start = i + 1;
    let mut j = super::rd_macro_name_end(bytes, name_start);
    if j == name_start {
        return None; // `\\`, `\{`, `\n`, … are not macro calls
    }
    let name = std::str::from_utf8(&bytes[name_start..j]).unwrap_or_default();
    // A brace-less `\word` that is **not** a known Rd macro is an `UNKNOWN` macro
    // token (parse_Rd tags any unrecognized `\word` `UNKNOWN`, even without a
    // group). A *known* name brace-less stays literal prose: a zero-arg macro's
    // name-only rendering and an arg-requiring macro's misuse are both backlog,
    // and leaving them as text keeps the existing tokenization (no regression).
    if bytes.get(j) != Some(&b'{') && bytes.get(j) != Some(&b'[') {
        return (!super::is_known_rd_macro(name)).then_some(j);
    }
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
                        | TokKind::RoxygenMdBracket
                        | TokKind::RoxygenMdImage
                        | TokKind::RoxygenMdDelim
                        | TokKind::RoxygenMdCode
                        | TokKind::RoxygenMdListMarker
                        | TokKind::RoxygenMdFence
                        | TokKind::RoxygenMdHtml
                        | TokKind::RoxygenMdHtmlBlock
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
        // With an `@md` directive in the block, each `*`/`_` run is carved as a
        // neutral `RoxygenMdDelim` (the inline pass decides open/close/match), and a
        // markdown code span is its own leaf. The lexer makes no flanking decision.
        let src = "#' a *one*, **two**, and `three` end.\n#' @md\n";
        assert_eq!(
            prose_texts(src),
            vec![
                (TokKind::RoxygenText, "a ".into()),
                (TokKind::RoxygenMdDelim, "*".into()),
                (TokKind::RoxygenText, "one".into()),
                (TokKind::RoxygenMdDelim, "*".into()),
                (TokKind::RoxygenText, ", ".into()),
                (TokKind::RoxygenMdDelim, "**".into()),
                (TokKind::RoxygenText, "two".into()),
                (TokKind::RoxygenMdDelim, "**".into()),
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
        // Under `@md`, a `*` at line start followed by a non-space carves a neutral
        // delimiter run (the inline pass decides emphasis), not a list marker; `-3`
        // (no space) is plain text; `* item` is a bullet.
        let src = "#' * a *b* c\n#' @md\n";
        assert_eq!(
            prose_texts(src),
            vec![
                (TokKind::RoxygenMdListMarker, "*".into()),
                (TokKind::RoxygenText, " a ".into()),
                (TokKind::RoxygenMdDelim, "*".into()),
                (TokKind::RoxygenText, "b".into()),
                (TokKind::RoxygenMdDelim, "*".into()),
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
    fn md_fence_recognized_under_md_mode() {
        // Under `@md`, a line whose content opens a code fence (3+ backticks)
        // carves the whole remaining content off as a `RoxygenMdFence` leaf — the
        // opener with its info string, and the bare closer.
        let opener = "#' ```r\n#' @md\n";
        assert_eq!(
            prose_texts(opener),
            vec![(TokKind::RoxygenMdFence, "```r".into())]
        );
        assert_lossless(opener);
        let closer = "#' ```\n#' @md\n";
        assert_eq!(
            prose_texts(closer),
            vec![(TokKind::RoxygenMdFence, "```".into())]
        );
        assert_lossless(closer);
    }

    #[test]
    fn md_fence_off_without_md_directive() {
        // No `@md`: a leading ```` ``` ```` stays literal prose (no fence leaf).
        assert_eq!(
            prose_texts("#' ```r\n"),
            vec![(TokKind::RoxygenText, "```r".into())]
        );
    }

    #[test]
    fn md_fence_requires_three_backticks_and_no_inner_backtick() {
        // A two-backtick run is not a fence; and a 3-backtick run followed by
        // another backtick is an inline code span at line start, not a fence
        // (CommonMark forbids a backtick in a backtick fence's info string).
        let two = "#' `` not a fence\n#' @md\n";
        assert_eq!(
            prose_texts(two),
            vec![(TokKind::RoxygenText, "`` not a fence".into())]
        );
        let inline = "#' ```code``` inline\n#' @md\n";
        assert_eq!(
            prose_texts(inline),
            vec![
                (TokKind::RoxygenMdCode, "```code```".into()),
                (TokKind::RoxygenText, " inline".into()),
            ]
        );
        assert_lossless(inline);
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
    fn md_emphasis_delims_carved_neutrally() {
        // Under `@md`, the lexer carves every `*`/`_` run as a neutral delimiter
        // leaf — it makes *no* flanking/open/close decision (that is the inline
        // pass's job). So whitespace-flanked `*` and intraword `_` still tokenize
        // as delimiter runs; whether they resolve to emphasis is decided later.
        let src = "#' a * b * c and snake_case_name here\n#' @md\n";
        assert_eq!(
            prose_texts(src),
            vec![
                (TokKind::RoxygenText, "a ".into()),
                (TokKind::RoxygenMdDelim, "*".into()),
                (TokKind::RoxygenText, " b ".into()),
                (TokKind::RoxygenMdDelim, "*".into()),
                (TokKind::RoxygenText, " c and snake".into()),
                (TokKind::RoxygenMdDelim, "_".into()),
                (TokKind::RoxygenText, "case".into()),
                (TokKind::RoxygenMdDelim, "_".into()),
                (TokKind::RoxygenText, "name here".into()),
            ]
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
        // An inline `[text](url)` link is split into neutral bracket leaves around
        // the recursively-lexed link text (here a single plain run), so emphasis or
        // code spans inside it can resolve in the inline pass.
        assert_eq!(
            prose_texts("#' see [the docs](https://x.y) now\n#' @md\n"),
            vec![
                (TokKind::RoxygenText, "see ".into()),
                (TokKind::RoxygenMdBracket, "[".into()),
                (TokKind::RoxygenText, "the docs".into()),
                (TokKind::RoxygenMdBracket, "](https://x.y)".into()),
                (TokKind::RoxygenText, " now".into()),
            ]
        );
        assert_lossless("#' see [the docs](https://x.y) now\n#' @md\n");
    }

    #[test]
    fn md_inline_link_text_emphasis() {
        // The link text is recursively lexed, so a `*…*` run inside it carves as a
        // neutral delimiter (resolved into emphasis by the inline pass), not literal.
        assert_eq!(
            prose_texts("#' [*x*](/u)\n#' @md\n"),
            vec![
                (TokKind::RoxygenMdBracket, "[".into()),
                (TokKind::RoxygenMdDelim, "*".into()),
                (TokKind::RoxygenText, "x".into()),
                (TokKind::RoxygenMdDelim, "*".into()),
                (TokKind::RoxygenMdBracket, "](/u)".into()),
            ]
        );
        assert_lossless("#' [*x*](/u)\n#' @md\n");
    }

    #[test]
    fn md_cross_line_inline_link() {
        // A `[` whose text is unclosed on its line carves a lone `[` opener leaf;
        // the `](url)` closer on a following line carves a lone closer leaf. The
        // inline pass pairs them over the paragraph run into one `ROXYGEN_MD_LINK`.
        assert_eq!(
            prose_texts("#' a [broken\n#' link](/u) b\n#' @md\n"),
            vec![
                (TokKind::RoxygenText, "a ".into()),
                (TokKind::RoxygenMdBracket, "[".into()),
                (TokKind::RoxygenText, "broken".into()),
                (TokKind::RoxygenText, "link".into()),
                (TokKind::RoxygenMdBracket, "](/u)".into()),
                (TokKind::RoxygenText, " b".into()),
            ]
        );
        assert_lossless("#' a [broken\n#' link](/u) b\n#' @md\n");
    }

    #[test]
    fn md_cross_line_link_brackets_only_under_md() {
        // Without `@md` the brackets stay literal prose (the cross-line carve is
        // mode-gated, like every other markdown inline recognizer).
        assert_eq!(
            prose_texts("#' a [broken\n#' link](/u) b\n"),
            vec![
                (TokKind::RoxygenText, "a [broken".into()),
                (TokKind::RoxygenText, "link](/u) b".into()),
            ]
        );
        assert_lossless("#' a [broken\n#' link](/u) b\n");
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
    fn md_url_autolink() {
        // A `<scheme:…>` autolink carves as a `RoxygenMdLink` under `@md`; a raw
        // HTML tag (no scheme `:`) carves as a `RoxygenMdHtml`.
        assert_eq!(
            prose_texts("#' see <https://x.y/a> and <p>lit</p>\n#' @md\n"),
            vec![
                (TokKind::RoxygenText, "see ".into()),
                (TokKind::RoxygenMdLink, "<https://x.y/a>".into()),
                (TokKind::RoxygenText, " and ".into()),
                (TokKind::RoxygenMdHtml, "<p>".into()),
                (TokKind::RoxygenText, "lit".into()),
                (TokKind::RoxygenMdHtml, "</p>".into()),
            ]
        );
        assert_lossless("#' see <https://x.y/a> and <p>lit</p>\n#' @md\n");
    }

    #[test]
    fn md_html_inline_tag() {
        // A raw inline-HTML tag (open tag with attributes) carves as a
        // `RoxygenMdHtml` span under `@md`; surrounding prose tiles around it.
        assert_eq!(
            prose_texts("#' before-<img src='foo.png'>-after\n#' @md\n"),
            vec![
                (TokKind::RoxygenText, "before-".into()),
                (TokKind::RoxygenMdHtml, "<img src='foo.png'>".into()),
                (TokKind::RoxygenText, "-after".into()),
            ]
        );
        assert_lossless("#' before-<img src='foo.png'>-after\n#' @md\n");
    }

    #[test]
    fn html_inline_is_literal_text_without_md() {
        // A raw HTML tag is recognized only under `@md`; without it `<` is literal
        // prose, so no `RoxygenMdHtml` is carved.
        assert_eq!(
            prose_texts("#' before-<img src='foo.png'>-after\n"),
            vec![(
                TokKind::RoxygenText,
                "before-<img src='foo.png'>-after".into()
            )]
        );
        assert_lossless("#' before-<img src='foo.png'>-after\n");
    }

    #[test]
    fn md_html_block_opener_carves_whole_line() {
        // A line whose content starts with a block-level tag (condition 6) carves
        // the whole remaining line as a `RoxygenMdHtmlBlock` opener under `@md`.
        assert_eq!(
            prose_texts("#' <p>a paragraph</p>\n#' @md\n"),
            vec![(TokKind::RoxygenMdHtmlBlock, "<p>a paragraph</p>".into())]
        );
        assert_lossless("#' <p>a paragraph</p>\n#' @md\n");
    }

    #[test]
    fn md_html_block_only_under_md() {
        // The HTML-block opener is recognized only under `@md`; without it the line
        // is ordinary prose (the inline `<p>` recognizer is also `@md`-gated).
        assert_eq!(
            prose_texts("#' <p>a paragraph</p>\n"),
            vec![(TokKind::RoxygenText, "<p>a paragraph</p>".into())]
        );
        assert_lossless("#' <p>a paragraph</p>\n");
    }

    #[test]
    fn md_non_block_tag_at_line_start_stays_inline() {
        // `<span>` is not a block-level tag, so a line starting with it does not
        // open an HTML block — it tiles as an inline `RoxygenMdHtml` span instead.
        assert_eq!(
            prose_texts("#' <span>x</span>\n#' @md\n"),
            vec![
                (TokKind::RoxygenMdHtml, "<span>".into()),
                (TokKind::RoxygenText, "x".into()),
                (TokKind::RoxygenMdHtml, "</span>".into()),
            ]
        );
        assert_lossless("#' <span>x</span>\n#' @md\n");
    }

    #[test]
    fn malformed_html_stays_literal() {
        // `<a b=>` has an `=` with no value → not a well-formed tag → literal prose
        // (an over-recognition would emit a spurious `\out`).
        assert_eq!(
            prose_texts("#' x <a b=> y\n#' @md\n"),
            vec![(TokKind::RoxygenText, "x <a b=> y".into())]
        );
        assert_lossless("#' x <a b=> y\n#' @md\n");
    }

    #[test]
    fn autolink_shape_is_literal_text_without_md() {
        // `<url>` is an autolink only under `@md`; without it, `<` is literal prose.
        assert_eq!(
            prose_texts("#' see <https://x.y/a> now\n"),
            vec![(TokKind::RoxygenText, "see <https://x.y/a> now".into())]
        );
        assert_lossless("#' see <https://x.y/a> now\n");
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
