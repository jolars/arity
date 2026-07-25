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
pub(super) fn roxygen_md_directive(line: &str) -> Option<bool> {
    let after_hashes = line.trim_start_matches('#');
    let body = after_hashes.strip_prefix('\'')?.trim();
    match body {
        "@md" => Some(true),
        "@noMd" => Some(false),
        _ => None,
    }
}

/// If `line` (a roxygen line's text, starting at `#`, no trailing newline) opens
/// a tag, return the tag name (`@rawRd` → `"rawRd"`); otherwise `None` (a prose
/// or continuation line). Mirrors the marker → ws → `@name` scan in
/// [`lex_roxygen_line`]/[`lex_roxygen_tag`]. Used by the lexer driver to track
/// which tag's body a line belongs to, so a verbatim-Rd tag's content lexes as Rd
/// even inside an `@md` block (see [`is_raw_rd_tag`]).
pub(crate) fn roxygen_line_tag(line: &str) -> Option<&str> {
    let after_hashes = line.trim_start_matches('#');
    let body = after_hashes.strip_prefix('\'')?;
    let body = body.trim_start_matches([' ', '\t']);
    let rest = body.strip_prefix('@')?;
    let bytes = rest.as_bytes();
    // A tag opens with `@` immediately followed by a letter (matching the
    // `@@`/`@ `/`@1` exclusions in `lex_roxygen_line`); the name is
    // `[A-Za-z][A-Za-z0-9]*`.
    if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut end = 1;
    while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
        end += 1;
    }
    Some(&rest[..end])
}

/// Tags whose body roxygen2 injects verbatim and never markdown-processes
/// (`@rawRd` uses `tag_value`, not `tag_markdown`), so their content must lex as
/// Rd even inside an `@md` block — markdown leaves there would mis-project (a
/// `[bracket]` is literal Rd, not a `\link`). Only `@rawRd` is projector-relevant;
/// `@evalRd` (R code) and `@usage` (roclet-generated) share the non-markdown
/// semantics but are out of the projector's scope, so they are left out for now.
pub(crate) fn is_raw_rd_tag(name: &str) -> bool {
    name == "rawRd"
}

/// Tags whose body roxygen2 treats as verbatim R code (`tag_code`/`tag_examples`)
/// and never markdown-processes, so their content must not gain markdown leaves
/// even inside an `@md` block (`x <- `code`` in an `@examples` body is R, not a
/// markdown code span). This governs only the markdown grammar the `@md` mode
/// switches on; the Rd inline spans these bodies still tokenize as (a separate,
/// pre-existing gap) are unaffected.
pub(crate) fn is_code_tag(name: &str) -> bool {
    matches!(
        name,
        "examples" | "examplesIf" | "usage" | "eval" | "evalRd" | "evalNamespace"
    )
}

/// Whether a tag's body is never markdown-processed even inside an `@md` block —
/// the verbatim-Rd tag (`@rawRd`) or a verbatim-code tag (`@examples`, …). Keyed
/// by both the lexer driver and the inline span builder so `@md` markdown leaves
/// are suppressed consistently across such a tag's lines.
pub(crate) fn tag_body_skips_markdown(name: &str) -> bool {
    is_raw_rd_tag(name) || is_code_tag(name)
}

/// Whether a tag's same-line value is reflowable markdown/Rd **prose** whose field
/// spans its continuation lines — so a same-line value folds its contiguous
/// plain-prose continuations into the `ROXYGEN_TAG` node (see `emit_tag_line`),
/// letting an `@md` emphasis/link span resolve across the soft break. The
/// excluded tags carry a non-prose value that must keep its own line structure:
/// code/examples (`tag_code`/`tag_examples`), a single verbatim value
/// (`tag_value`), a token list (`tag_words` / namespace directives), a bare toggle,
/// `@section` (laid out specially from its `Title: body`), or verbatim Rd. Mirrors
/// the non-prose classes of the formatter's tag classifier; an unknown tag is
/// prose (roxygen2 defaults unknown tags to `tag_markdown`).
pub(crate) fn tag_folds_prose_continuation(name: &str) -> bool {
    !matches!(
        name,
        // tag_code / tag_examples
        "examples" | "examplesIf" | "usage" | "eval" | "evalRd" | "evalNamespace"
        // tag_value (single verbatim value; interior spaces significant)
        | "name" | "rdname" | "docType" | "encoding" | "family" | "concept"
        | "inheritParams" | "backref" | "exportClass" | "exportMethod" | "exportPattern"
        // tag_words / namespace directives (join to one line)
        | "keywords" | "aliases" | "import" | "importFrom" | "importClassesFrom"
        | "importMethodsFrom" | "exportS3Method" | "useDynLib" | "rawNamespace"
        // tag_toggle (no value anyway) + @section (special) + verbatim Rd
        | "export" | "noRd" | "md" | "noMd" | "section" | "rawRd"
    )
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

/// Sub-tokenize a bare prose **fragment** (no `#'` marker, not a line start) under
/// the given `md` mode. Used by the projector to re-lex the argument of a
/// non-fragile Rd macro under `@md` as a markdown inline run (`\emph{*x*}`'s `*x*`).
/// `line_start = false`, so the line-leading block recognizers (fence / HTML block /
/// list marker) never fire — a macro argument is inline content, not a block.
pub(super) fn lex_roxygen_prose_fragment(out: &mut Vec<Token>, text: &str, md: bool) {
    lex_roxygen_prose(out, text, 0, 0, md, false);
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

    // Under `@md`, a *prose* tag's same-line value is the start of the tag's
    // markdown document, so a value beginning with a CommonMark block start opens
    // that block exactly as at line start: carve the same leaves the line-start
    // path carves — a fence opener, an HTML-block opener (conditions 1–6), an ATX
    // heading, a thematic break, a block-quote opener, or a list marker (the
    // grouper closes the tag empty and gathers the block as a section sibling).
    // roxygen2 strips only the single separator
    // space after the tag head, so a deeper-indented value (>= 4 columns past
    // that space) is an indented code block whose content must lex as ordinary
    // tokens — every carve is gated on the indent.
    if md && tag_folds_prose_continuation(&name) {
        let ws_len = pos - text[..pos].trim_end_matches([' ', '\t']).len();
        if ws_len <= 4 {
            if let Some(fence_end) = scan_md_fence(bytes, pos) {
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
            if let Some(block_end) = scan_md_html_block(bytes, pos) {
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
            if is_atx_heading(bytes, pos) {
                push(
                    out,
                    TokKind::RoxygenMdHeading,
                    text,
                    start,
                    pos,
                    text.len() - pos,
                );
                return;
            }
            // A thematic break, checked before the list marker (a spaced form
            // like `- - -` or `* * *` starts with a valid bullet marker). The
            // value position is fresh (no preceding paragraph), so a contiguous
            // `---` is never a setext underline here — it carves directly as a
            // break, unlike at line start where setext takes precedence.
            if is_thematic_break(bytes, pos) {
                push(
                    out,
                    TokKind::RoxygenMdThematicBreak,
                    text,
                    start,
                    pos,
                    text.len() - pos,
                );
                return;
            }
            if is_block_quote_marker(bytes, pos) {
                push(
                    out,
                    TokKind::RoxygenMdBlockQuote,
                    text,
                    start,
                    pos,
                    text.len() - pos,
                );
                return;
            }
            if scan_md_list_marker(bytes, pos).is_some() {
                let content = carve_md_list_markers(out, text, start, pos);
                lex_roxygen_prose(out, text, start, content, md, false);
                return;
            }
        }
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
    // Under `@md`, a prose line whose whole content is a GFM table **delimiter
    // row** (`|---|:--:|`) carves it off as a `RoxygenMdTableDelim` leaf. The block
    // builder pairs it with the preceding header line (when their cell counts
    // match) into a `ROXYGEN_MD_TABLE`; an unmatched delimiter row stays literal
    // prose (the tree builder maps the kind to `ROXYGEN_TEXT`). The leaf's
    // existence implies `@md`, so the builder keys off the token kind, never
    // re-deriving mode.
    if md && line_start && super::is_table_delim_row(&text[pos..]) {
        push(
            out,
            TokKind::RoxygenMdTableDelim,
            text,
            start,
            pos,
            text.len() - pos,
        );
        return;
    }
    // Under `@md`, a prose line whose content is an ATX heading (`# Title`, up to
    // `######`) carves the *whole* remaining line off as a `RoxygenMdHeading` leaf.
    // The block builder wraps it in a single-line `ROXYGEN_MD_HEADING` node; the
    // leaf's existence implies `@md`, so the builder keys off the token kind.
    if md && line_start && is_atx_heading(bytes, pos) {
        push(
            out,
            TokKind::RoxygenMdHeading,
            text,
            start,
            pos,
            text.len() - pos,
        );
        return;
    }
    // Under `@md`, a line whose content is a **setext heading underline** (`===` or
    // `---`) carves the whole remaining line off as a `RoxygenMdSetextUnderline`
    // leaf. Whether it promotes the preceding paragraph into a heading is decided
    // later at block level (`emit_md_setext_heading`); an underline that heads
    // nothing stays literal prose (the tree builder maps the kind to `ROXYGEN_TEXT`).
    if md && line_start && is_setext_underline(bytes, pos) {
        push(
            out,
            TokKind::RoxygenMdSetextUnderline,
            text,
            start,
            pos,
            text.len() - pos,
        );
        return;
    }
    // Under `@md`, a line whose whole content is a CommonMark **thematic break**
    // (`***`/`___`, or a space-separated form like `- - -`) carves off as a
    // `RoxygenMdThematicBreak` leaf. A contiguous `---`/`===` run was already claimed
    // by the setext-underline check above (setext takes precedence), so this only
    // catches the `*`/`_`-based and spaced forms; a bare `---` that heads no paragraph
    // is promoted to a thematic break at block level. The block builder wraps the line
    // in a `ROXYGEN_MD_THEMATIC_BREAK`; the leaf implies `@md`, so the builder keys off
    // the token kind, never re-deriving mode.
    if md && line_start && is_thematic_break(bytes, pos) {
        push(
            out,
            TokKind::RoxygenMdThematicBreak,
            text,
            start,
            pos,
            text.len() - pos,
        );
        return;
    }
    // Under `@md`, a prose line whose content begins (after up to three spaces) with
    // a `>` carves the *whole* remaining line off as a `RoxygenMdBlockQuote` leaf. The
    // block builder gathers consecutive block-quote lines into a
    // `ROXYGEN_MD_BLOCK_QUOTE`; the leaf's existence implies `@md`, so the builder
    // keys off the token kind, never re-deriving mode.
    if md && line_start && is_block_quote_marker(bytes, pos) {
        push(
            out,
            TokKind::RoxygenMdBlockQuote,
            text,
            start,
            pos,
            text.len() - pos,
        );
        return;
    }
    // Under `@md`, a prose line whose content begins with a list marker carves it
    // off as a `RoxygenMdListMarker` leaf (the trailing space stays in the prose
    // run). Whether the marker actually forms a list is a block-level decision
    // (the CommonMark interrupt rule), made later in `emit_roxygen_block`.
    if md && line_start && scan_md_list_marker(bytes, pos).is_some() {
        let content = carve_md_list_markers(out, text, start, pos);
        run_start = content;
        i = content;
    }
    while i < bytes.len() {
        // Under `@md`, an inline link `[text](url)`: carve the `[` opener and the
        // `](url)` closer as neutral `RoxygenMdBracket` leaves and *recursively* lex
        // the link text in between, so emphasis/code spans inside it resolve. The
        // inline pass then assembles the matched pair into a `ROXYGEN_MD_LINK`
        // **node** whose display children are the resolved markdown. Shortcut and
        // reference links (`[t]`, `[t][r]`) carve onto neutral brackets too (the
        // `same_line_bracket_opener` arm below); only a `\`-bearing display still
        // falls to the opaque `scan_md_link` leaf. Here only a *bracket-free* inline
        // link text is split.
        if md
            && bytes[i] == b'['
            && !bracket_is_escaped(bytes, i)
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
        if md
            && bytes[i] == b'['
            && !bracket_is_escaped(bytes, i)
            && is_cross_line_link_opener(bytes, i)
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
            i += 1;
            run_start = i;
            continue;
        }
        // A *same-line* bracketed link span: a balanced, bracket-free `[…]` that is
        // not an inline link (handled above) and not a `[…]{…}` non-link. Carve the
        // `[` as a neutral bracket opener leaf and let the main loop lex the interior
        // and the closing `]` (the bare-`]`/`][ref]` carves below). This one carve
        // covers a shortcut display, a reference display (`[display][ref]`), and a
        // reference *label* (`[ref]`) alike; the inline pass pairs the brackets into
        // `ROXYGEN_MD_LINK` nodes and `classify_closer` reads a following neutral
        // `[ref]` off the arena lookahead. Only `!`/`\` displays stay on the opaque
        // `scan_md_link` leaf for now (see `same_line_bracket_opener`).
        if md
            && bytes[i] == b'['
            && !bracket_is_escaped(bytes, i)
            && same_line_bracket_opener(bytes, i)
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
            i += 1;
            run_start = i;
            continue;
        }
        // A *nested-bracket* link opener: a `[` whose balanced same-line interior
        // itself contains brackets, so the conservative same-line/opaque paths
        // above (which all require a bracket-free interior) do not apply. Carve the
        // `[` as a neutral opener and let the main loop carve the inner brackets and
        // the closer; the inline pass resolves the nesting with CommonMark opener
        // deactivation — the inner links win and this outer bracket stays literal
        // (`[a [b] c](url)`, `[foo [bar] baz]`, `[[x]](url)`).
        if md
            && bytes[i] == b'['
            && !bracket_is_escaped(bytes, i)
            && is_nested_bracket_opener(bytes, i)
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
        // `]` as a neutral bracket leaf; the `[ref]` label carves onto its own neutral
        // brackets (`same_line_bracket_opener`) and `classify_closer` reads it off the
        // lookahead. The inline pass then either pairs the `]` with the earlier opener
        // — folding the label in as `][ref]` — or, with no opener, leaves the `]`
        // literal and the `[ref]` a standalone shortcut, so `a][b]` stays `a]` + a
        // `[b]` shortcut, matching roxygen2.
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
        // A *collapsed* reference-link closer: a `]` immediately followed by an
        // empty `[]` label (CommonMark's collapsed form `[text][]`, whose reference
        // label is the display text itself). The empty label cannot carve on its own
        // (a bracket carve needs non-empty content), so carve the whole `][]` as one
        // neutral bracket closer leaf — like the `](url)` composite closer — and let
        // the inline pass pair it with an earlier opener (literal text when
        // unmatched, e.g. `[][]`). The projector resolves the label from the
        // display, so `[foo][]` links iff `foo` is defined in the refmap.
        if md
            && bytes[i] == b']'
            && bytes.get(i + 1) == Some(&b'[')
            && bytes.get(i + 2) == Some(&b']')
        {
            push(
                out,
                TokKind::RoxygenText,
                text,
                start,
                run_start,
                i - run_start,
            );
            push(out, TokKind::RoxygenMdBracket, text, start, i, 3);
            i += 3;
            run_start = i;
            continue;
        }
        // A *cross-line* shortcut-link closer: a lone `]` that closes a `[` opened on
        // an earlier `#'` line (a `[text]` shortcut spanning lines). Line-locally
        // *every* `]` is ambiguous — the lexer cannot see an earlier opener — so carve
        // any lone `]` that is not an inline (`](url)`) or reference (`][ref]`) closer
        // (handled above) and is not a non-link `]{…}` lookahead as a neutral bracket
        // leaf. The inline pass pairs it with an earlier cross-line opener (a shortcut
        // link) or, with no opener, re-emits it as literal text — so a truly stray `]`
        // is unchanged.
        if md
            && bytes[i] == b']'
            && !matches!(bytes.get(i + 1), Some(b'[' | b'{'))
            // A following `(…)` blocks the closer only when it is a *valid* inline
            // destination (a real `](url)`); an invalid `(…)` (`[t](a\ b)`) still
            // closes the shortcut and leaves the `(…)` literal prose.
            && !(bytes.get(i + 1) == Some(&b'(') && inline_dest_span(bytes, i + 1).is_some())
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
            // parse_Rd pairs backslashes left-to-right (`\\` is the literal-
            // backslash escape), so a `\` preceded by an odd-length backslash
            // run is consumed by its pair and can never begin a macro: `\\y`
            // is literal `\` + `y`, while `\\\y` re-forms `\y`. The parity
            // gate applies to both the inline carve and the block opener.
            b'\\' if !rd_backslash_is_escaped(bytes, i) => scan_rd_macro(bytes, i)
                .map(|end| (TokKind::RoxygenRdMacro, end))
                .or_else(|| {
                    is_block_macro_opener_at(bytes, i)
                        .then_some((TokKind::RoxygenText, bytes.len()))
                }),
            b'!' if md => scan_md_image(bytes, i).map(|end| (TokKind::RoxygenMdImage, end)),
            b'[' if md && !bracket_is_escaped(bytes, i) => {
                scan_md_link(bytes, i).map(|end| (TokKind::RoxygenMdLink, end))
            }
            b'<' if md => scan_md_autolink(bytes, i)
                .map(|end| (TokKind::RoxygenMdLink, end))
                .or_else(|| {
                    scan_md_email_autolink(bytes, i).map(|end| (TokKind::RoxygenMdLink, end))
                })
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

/// Whether the fence line `closer` closes a code block opened by the fence line
/// `opener` (both given as their fence-leaf text, starting at the fence run;
/// indentation is checked by the caller in its own coordinate system): the same
/// fence character, a run at least as long as the opener's, and nothing but
/// whitespace after the run — a closing fence has no info string (CommonMark
/// 4.5). A non-matching fence line is *content*.
pub(crate) fn md_fence_run_closes(opener: &str, closer: &str) -> bool {
    let (ob, cb) = (opener.as_bytes(), closer.as_bytes());
    let ch = match ob.first() {
        Some(&c @ (b'`' | b'~')) => c,
        _ => return false,
    };
    let run = run_len(cb, 0, ch);
    run >= run_len(ob, 0, ch) && closer[run..].trim_ascii().is_empty()
}

/// Count the run of consecutive `c` bytes starting at `i`.
pub(super) fn run_len(bytes: &[u8], i: usize, c: u8) -> usize {
    let mut j = i;
    while j < bytes.len() && bytes[j] == c {
        j += 1;
    }
    j - i
}

/// A CommonMark inline-code span at `bytes[i] == b'`'`: an opening backtick run
/// of length `n`, closed by the next run of *exactly* `n` backticks on the line.
/// Returns the index past the closing run, or `None` if unterminated.
pub(super) fn scan_inline_code(bytes: &[u8], i: usize) -> Option<usize> {
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
/// Whether the line content starting at `i` opens a CommonMark ATX heading: a run
/// of 1-6 `#` characters followed by a space, a tab, or the end of the line. (The
/// optional closing `#` sequence and title text are part of the same line and are
/// stripped by the projector, not here.) `#hashtag`, `#5x`, and a run of seven or
/// more `#` are not headings.
fn is_atx_heading(bytes: &[u8], i: usize) -> bool {
    let n = run_len(bytes, i, b'#');
    if !(1..=6).contains(&n) {
        return false;
    }
    matches!(bytes.get(i + n), None | Some(b' ' | b'\t'))
}

/// Whether the line content at `bytes[i..]` (positioned past the `#'` marker) is a
/// **setext heading underline**: after up to three spaces of leading indentation, a
/// non-empty run of `=` or a run of two-or-more `-`, then only trailing whitespace
/// to the line end. `=` gives a level-1 underline, `-` a level-2 one (the projector
/// reads the level from the leaf's first non-space byte).
///
/// A single `-`/`- ` is deliberately excluded here: it is indistinguishable from an
/// empty list-item bullet at the token level, so the lexer carves it as a
/// `RoxygenMdListMarker` and the *block* builder resolves it — a lone dash bullet
/// after a paragraph is a level-2 setext underline (`is_md_setext_dash_underline`),
/// but at a fresh position it opens an empty list.
fn is_setext_underline(bytes: &[u8], i: usize) -> bool {
    let mut j = i;
    let mut indent = 0;
    while indent < 3 && bytes.get(j) == Some(&b' ') {
        j += 1;
        indent += 1;
    }
    let ch = match bytes.get(j) {
        Some(&c @ (b'=' | b'-')) => c,
        _ => return false,
    };
    let run = run_len(bytes, j, ch);
    if ch == b'-' && run < 2 {
        return false; // a single `-` is an empty list bullet, not an underline
    }
    j += run;
    while let Some(&c) = bytes.get(j) {
        if c == b' ' || c == b'\t' {
            j += 1;
        } else {
            return false;
        }
    }
    true
}

/// Whether the whole line content at `bytes[i..]` (positioned past the `#'` marker)
/// is a CommonMark **thematic break**: after up to three spaces of leading
/// indentation, three or more of a single `*`/`-`/`_` character, with only spaces or
/// tabs allowed between and after, and nothing else to the line end.
///
/// The `---`/`===` contiguous forms are recognized by [`is_setext_underline`] first
/// (setext precedence), so in practice this catches the `*`- and `_`-based forms and
/// the space-separated variants (`- - -`); a bare `---` heading no paragraph is
/// promoted at block level. A run shorter than three, or a line mixing marker
/// characters or carrying other text, is not a thematic break.
fn is_thematic_break(bytes: &[u8], i: usize) -> bool {
    let mut j = i;
    let mut indent = 0;
    while indent < 3 && bytes.get(j) == Some(&b' ') {
        j += 1;
        indent += 1;
    }
    let ch = match bytes.get(j) {
        Some(&c @ (b'*' | b'-' | b'_')) => c,
        _ => return false,
    };
    let mut count = 0usize;
    while let Some(&c) = bytes.get(j) {
        if c == ch {
            count += 1;
            j += 1;
        } else if c == b' ' || c == b'\t' {
            j += 1;
        } else {
            return false;
        }
    }
    count >= 3
}

/// Whether the line content at `bytes[i..]` (positioned past the `#'` marker) opens
/// a **block quote**: after up to three spaces of leading indentation, a `>`. This
/// is the CommonMark block-quote marker (an optional single space after the `>` is
/// part of the marker, but its presence is not required to open a quote).
fn is_block_quote_marker(bytes: &[u8], i: usize) -> bool {
    let mut j = i;
    let mut indent = 0;
    while indent < 3 && bytes.get(j) == Some(&b' ') {
        j += 1;
        indent += 1;
    }
    bytes.get(j) == Some(&b'>')
}

/// A CommonMark code-fence line at `bytes[i..]`: a run of at least three
/// backticks or tildes, then the info string to the end of the line. A
/// *backtick* fence's info string may not contain a backtick; a *tilde* fence's
/// info string may contain both backticks and tildes (CommonMark 4.5).
fn scan_md_fence(bytes: &[u8], i: usize) -> Option<usize> {
    let ch = match bytes.get(i) {
        Some(&c @ (b'`' | b'~')) => c,
        _ => return None,
    };
    let run = run_len(bytes, i, ch);
    if run < 3 {
        return None;
    }
    if ch == b'`' && bytes[i + run..].contains(&b'`') {
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
/// Carve the list marker at `pos` and any **same-line consecutive markers**
/// after it: CommonMark opens a nested list when an item's content itself
/// begins with a list marker (`- - foo` is a bullet item holding a bullet
/// sublist; `1. - 2. foo` nests three deep), so each following marker
/// separated by one to four whitespace columns is carved as its own
/// `RoxygenMdListMarker` leaf, with the separating whitespace pushed as its
/// own all-whitespace prose run (the block builder keys the nested container
/// column off it). A `>` in the same one-to-four-column window opens a **block
/// quote at the item's content start** (`- > quoted`, cm-294/295): the rest of
/// the line carves as a `RoxygenMdBlockQuote` leaf after the separator run. An
/// ATX heading in that window (`- # Foo`, cm-302) likewise carves the rest of
/// the line as a `RoxygenMdHeading` leaf, and a code fence (`- ```` ``` ````,
/// cm-320/326) a `RoxygenMdFence` leaf.
/// Five or more separating columns are indented-code
/// territory, and a remainder that is a thematic break (`- * * *`) is a
/// different block, so both leave the remainder in the prose run. The caller
/// must have checked [`scan_md_list_marker`] at `pos`. Returns the position of
/// the content after the last carved marker (or the line end when the quote
/// leaf consumed the remainder).
fn carve_md_list_markers(out: &mut Vec<Token>, text: &str, start: usize, pos: usize) -> usize {
    let bytes = text.as_bytes();
    let mut p = pos;
    let mut end = scan_md_list_marker(bytes, p).expect("caller checked scan_md_list_marker");
    loop {
        push(out, TokKind::RoxygenMdListMarker, text, start, p, end - p);
        let mut q = end;
        while q < bytes.len() && (bytes[q] == b' ' || bytes[q] == b'\t') {
            q += 1;
        }
        if q == end || q >= bytes.len() || q - end > 4 || is_thematic_break(bytes, q) {
            return end;
        }
        if bytes[q] == b'>' {
            push(out, TokKind::RoxygenText, text, start, end, q - end);
            push(
                out,
                TokKind::RoxygenMdBlockQuote,
                text,
                start,
                q,
                text.len() - q,
            );
            return text.len();
        }
        // An ATX heading at the item's content start (`- # Foo`, cm-302): the
        // rest of the line carves as a `RoxygenMdHeading` leaf after the
        // separator run, exactly like the block-quote arm.
        if is_atx_heading(bytes, q) {
            push(out, TokKind::RoxygenText, text, start, end, q - end);
            push(
                out,
                TokKind::RoxygenMdHeading,
                text,
                start,
                q,
                text.len() - q,
            );
            return text.len();
        }
        // A code fence at the item's content start (`- ```, cm-320/326): the
        // rest of the line carves as a `RoxygenMdFence` leaf after the
        // separator run, exactly like the block-quote and heading arms.
        if scan_md_fence(bytes, q).is_some() {
            push(out, TokKind::RoxygenText, text, start, end, q - end);
            push(out, TokKind::RoxygenMdFence, text, start, q, text.len() - q);
            return text.len();
        }
        let Some(next_end) = scan_md_list_marker(bytes, q) else {
            return end;
        };
        push(out, TokKind::RoxygenText, text, start, end, q - end);
        p = q;
        end = next_end;
    }
}

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

/// A markdown image at `bytes[i] == b'!'`: the inline form `![alt](url "title")`,
/// the reference form `![alt][ref]`, the collapsed form `![alt][]`, or the
/// shortcut form `![alt]`. Requires a `[` immediately after the `!` and a balanced
/// `[…]` alt span; the arms below dispatch on what follows. Returns the index past
/// the image, or `None` when it is not a recognized image shape (so it stays
/// literal prose — losslessness holds either way).
fn scan_md_image(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.get(i + 1) != Some(&b'[') {
        return None;
    }
    let after_alt = scan_balanced(bytes, i + 1, b'[', b']')?;
    let alt = &bytes[i + 2..after_alt - 1];
    match bytes.get(after_alt) {
        // Inline image `![alt](url)`: a valid CommonMark destination. An invalid
        // `(…)` (`![z](a\ b)`) falls back to the bare shortcut `![alt]`, leaving the
        // `(…)` as literal prose — cmark keeps `![z]` a shortcut reference image.
        Some(&b'(') => inline_dest_span(bytes, after_alt)
            .or_else(|| is_shortcut_content(alt).then_some(after_alt)),
        // Reference image `![alt][ref]`: a bracket-free, non-empty `[ref]` label
        // (the alt must also be a shortcut candidate). A **collapsed** `![alt][]`
        // is carved too — cmark resolves it by the alt-as-label (a user
        // `[alt]: url` definition; undefined it stays literal, but that is the
        // projector's refmap decision, not a lexing one). A bracketed ref is not
        // carved (roxygen2 leaves it literal).
        Some(&b'[') => {
            if !is_shortcut_content(alt) {
                return None;
            }
            let ref_end = scan_balanced(bytes, after_alt, b'[', b']')?;
            let label = &bytes[after_alt + 1..ref_end - 1];
            (label.is_empty() || is_shortcut_content(label)).then_some(ref_end)
        }
        // A shortcut `![alt]` followed by `{` is not an image (candidate blocked by
        // roxygen2's `(?=[^\[{])` lookahead), matching the link shortcut rule.
        Some(&b'{') => None,
        // Shortcut image `![alt]`: a bracket-free, non-empty alt resolves against the
        // synthesized `[alt]: R:alt` reference definition.
        _ => is_shortcut_content(alt).then_some(after_alt),
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
    let url_end = inline_dest_span(bytes, text_end)?;
    Some((text_end, url_end))
}

/// The end index of a *valid* CommonMark inline link/image destination `(…)` at
/// `bytes[i] == b'('` — the index past the closing `)`, or `None` when the
/// parenthesized content is not a well-formed destination (optionally followed by
/// a title). Mirrors cmark's inline-link parse *after* roxygen2's
/// `double_escape_md`: every source backslash doubles, so cmark resolves each pair
/// back to a literal `\` and **no source backslash ever escapes** a paren, an
/// angle bracket, or a title quote. A bare destination runs to the first ASCII
/// whitespace or to the `)` at raw paren depth 0 (interior parens must balance by
/// raw count — `[t](foo\(bar)` never closes, not a link); an angle-bracketed
/// `<…>` destination runs to the first `>` and may contain spaces and parens.
/// After the destination, only trailing whitespace — or whitespace then a single
/// `"…"`/`'…'`/`(…)` title then trailing whitespace — may precede the closing
/// `)`. Whitespace is ASCII-only throughout (a U+00A0 is destination content). So
/// a bare destination with an interior ASCII space then non-title text
/// (`[t](a\ b)`) is **not** a link: cmark leaves the `[t]` a shortcut reference
/// and the `(a\ b)` literal prose.
fn inline_dest_span(bytes: &[u8], i: usize) -> Option<usize> {
    debug_assert_eq!(bytes.get(i), Some(&b'('));
    let mut j = i + 1;
    while bytes.get(j).is_some_and(u8::is_ascii_whitespace) {
        j += 1;
    }
    if bytes.get(j) == Some(&b'<') {
        // Angle-bracketed destination: to the first `>`; no raw `<`/newline inside.
        j += 1;
        loop {
            match bytes.get(j) {
                None | Some(b'<') | Some(b'\n') => return None,
                Some(b'>') => {
                    j += 1;
                    break;
                }
                Some(_) => j += 1,
            }
        }
    } else {
        // Bare destination: to the first ASCII whitespace or the depth-0 `)`.
        let mut depth = 0usize;
        loop {
            match bytes.get(j) {
                None => return None,
                Some(&b) if b.is_ascii_whitespace() => break,
                Some(b'(') => {
                    depth += 1;
                    j += 1;
                }
                Some(b')') if depth == 0 => break,
                Some(b')') => {
                    depth -= 1;
                    j += 1;
                }
                Some(_) => j += 1,
            }
        }
    }
    let ws_start = j;
    while bytes.get(j).is_some_and(u8::is_ascii_whitespace) {
        j += 1;
    }
    match bytes.get(j) {
        Some(b')') => return Some(j + 1),
        None => return None,
        // Content abutting the destination (no separating whitespace).
        _ if j == ws_start => return None,
        _ => {}
    }
    let nested_open = bytes[j];
    let close = match nested_open {
        b'"' => b'"',
        b'\'' => b'\'',
        b'(' => b')',
        _ => return None,
    };
    j += 1;
    // The title, unlike the destination, is scanned longest-match (cmark's re2c
    // title pattern): after doubling, a source-backslash-preceded closer is
    // *optionally* escaped, so the title runs to the first closer NOT preceded
    // by a `\` — or, when every closer is `\`-preceded, to the last one. An
    // interior `(` in a `(…)` title is likewise allowed only when `\`-preceded.
    let mut escapable_close = None;
    loop {
        match bytes.get(j) {
            // No unpreceded closer: longest-match falls back to the last
            // backslash-preceded one.
            None => {
                j = escapable_close? + 1;
                break;
            }
            Some(&c) if c == close => {
                if bytes[j - 1] == b'\\' {
                    escapable_close = Some(j);
                    j += 1;
                } else {
                    j += 1;
                    break;
                }
            }
            Some(&c) if nested_open == b'(' && c == b'(' => {
                if bytes[j - 1] == b'\\' {
                    j += 1;
                } else {
                    return None;
                }
            }
            Some(_) => j += 1,
        }
    }
    while bytes.get(j).is_some_and(u8::is_ascii_whitespace) {
        j += 1;
    }
    (bytes.get(j) == Some(&b')')).then(|| j + 1)
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

/// Whether a `[` at `bytes[i]` opens a *same-line* bracketed link span carved onto
/// the arena: a balanced, bracket-free `[…]` (`is_shortcut_content`) that is not an
/// inline link (`[…](url)`, handled by [`inline_link_span`]) and not a `[…]{…}`
/// non-link. Carving the `[` as a neutral `RoxygenMdBracket` opener routes it
/// through the inline pass (uniform with cross-line spans and inline links) instead
/// of the opaque [`scan_md_link`] leaf, so the bracket recognizers converge on the
/// arena stack.
///
/// This one predicate carves three roles that the arena then distinguishes:
/// - a **shortcut** display `[text]` (the `]` followed by neither `(` nor `[`),
/// - a **reference** display `[display][ref]` (the `]` followed by `[`), and
/// - a reference **label** `[ref]` (the `[` immediately preceded by `]`).
///
/// `classify_closer` reads a following neutral `[ref]` label off the lookahead, so
/// the label no longer needs its own opaque token (the preceded-by-`]` guard and
/// the not-followed-by-`[` restriction the earlier shortcut/reference predicates
/// carried are gone). A plain display resolves to a single `Inline::Text` whose
/// text equals the raw interior, so the projector's node path is byte-identical to
/// the old leaf path; a marked-up display (`[*foo*]`, `` [`x` `y`] ``) resolves its
/// children so the projector can mirror roxygen2's `parse_link` (a sole code span
/// links, any richer display is dropped). A `\`-bearing display routes through here
/// too: the main loop lexes its interior, so a backslash-word (`[a\b]`, an Rd macro
/// to parse_Rd) carves as a `ROXYGEN_RD_MACRO` child and the projector renders it
/// inside the `\link` (markdown-level plain text — the backslash escapes nothing, so
/// roxygen2 keeps the link). Only an `!` display (a possible image marker) stays on
/// the opaque [`scan_md_link`] leaf for now.
fn same_line_bracket_opener(bytes: &[u8], i: usize) -> bool {
    let Some(close) = scan_balanced(bytes, i, b'[', b']') else {
        return false;
    };
    let content = &bytes[i + 1..close - 1];
    is_shortcut_content(content)
        && !content.contains(&b'!')
        && bytes.get(close) != Some(&b'{')
        // A following `(…)` blocks the shortcut carve only when it is a *valid*
        // inline destination (a real `[…](url)` link, handled by `inline_link_span`);
        // an invalid `(…)` (`[t](a\ b)`) leaves `[t]` a shortcut and `(…)` literal.
        && !(bytes.get(close) == Some(&b'(') && inline_dest_span(bytes, close).is_some())
}

/// Whether a `[` at `bytes[i]` opens a *nested-bracket* same-line link: its
/// balanced `[…]` interior itself contains a `[` or `]`. The bracket-free same-line
/// paths ([`inline_link_span`], [`same_line_bracket_opener`]) and the opaque
/// [`scan_md_link`] all require a bracket-free interior, so only a nested interior
/// reaches here. Carving the outer `[` as a neutral opener (rather than one opaque
/// link token) lets the inline pass resolve the nesting with CommonMark opener
/// deactivation: the inner links win and this outer bracket stays literal.
fn is_nested_bracket_opener(bytes: &[u8], i: usize) -> bool {
    scan_balanced(bytes, i, b'[', b']').is_some_and(|close| {
        bytes[i + 1..close - 1]
            .iter()
            .any(|&b| matches!(b, b'[' | b']'))
    })
}

/// Whether a markdown `[` at `bytes[i]` is *backslash-escaped* (CommonMark `\[`),
/// so it cannot open a link and stays literal prose. roxygen2 honors an escaped
/// open bracket via `double_escape_md`, which special-cases brackets — it reverts
/// `\\[`→`\[` after doubling every other backslash, so a `[` reaches cmark escaped
/// whenever *any* backslash run immediately precedes it. A single adjacent `\`
/// therefore already neutralizes the opener (verified against the oracle for one to
/// three leading backslashes: `\[`, `\\[`, `\\\[` all stay literal). Only the
/// *opener* is guarded; an escaped closing `]` instead triggers roxygen2's
/// link-reference machinery (a synthesized `[…]: …` linkref) and is left as backlog.
fn bracket_is_escaped(bytes: &[u8], i: usize) -> bool {
    i > 0 && bytes[i - 1] == b'\\'
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
    inline_dest_span(bytes, i + 1)
}

/// Whether a `]` at `bytes[i]` is a *cross-line* reference-link closer: it is
/// immediately followed by a balanced, bracket-free `[ref]` label that is a clean
/// shortcut (not itself followed by `(` or `{`, the shapes that would make
/// [`scan_md_link`] reject or re-read it). A following `[` does NOT block the
/// closer: cmark consumes the label regardless of what comes after it, so
/// `[foo][bar][baz]` pairs left-to-right — `[foo][bar]` a reference link, `[baz]`
/// a separate shortcut (cm-572). Used to carve the lone `]` as a neutral bracket
/// leaf; the `[ref]` label is carved separately as a shortcut `MD_LINK`,
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
                && !matches!(bytes.get(end), Some(b'(' | b'{'))
        })
}

fn scan_md_link(bytes: &[u8], i: usize) -> Option<usize> {
    let after_text = scan_balanced(bytes, i, b'[', b']')?;
    match bytes.get(after_text) {
        // A valid inline `(url)` destination, else fall back to the bare shortcut
        // `[…]` (leaving the invalid `(…)` as literal prose, like cmark).
        Some(&b'(') => inline_dest_span(bytes, after_text)
            .or_else(|| is_shortcut_content(&bytes[i + 1..after_text - 1]).then_some(after_text)),
        Some(&b'[') => scan_balanced(bytes, after_text, b'[', b']'),
        // A bare `[…]` followed by `{` is not a link (roxygen2's lookahead).
        Some(&b'{') => None,
        _ => is_shortcut_content(&bytes[i + 1..after_text - 1]).then_some(after_text),
    }
}

/// Whether `content` (the bytes inside a bare `[…]`) is a markdown shortcut-link
/// reference or label. cmark accepts any non-empty span whose brackets are all
/// backslash-escaped, so spaces, digits, `::`, and an escaped `\[` are all fine
/// (`[note]`, `[see this]`, `[pkg::obj]`, `[ref\[]`); an empty span or a *bare*
/// `[` is rejected (the latter so nested `[a[b]c]` re-scans the inner `[b]`).
/// The escaped-`[` rule mirrors [`bracket_is_escaped`] (a single adjacent `\`
/// suffices — `double_escape_md`'s `\\[`→`\[` revert keeps the escape live).
/// Any `]` is still rejected, even escaped: an escaped-*close* `\]` engages
/// roxygen2's linkref leak machinery and stays backlog. Note this is cmark's
/// *label-content* rule, not `get_md_linkrefs`' bracket-free candidate regex —
/// the synthesized-def mirror lives in the projector (`md_linkref_scan`).
fn is_shortcut_content(content: &[u8]) -> bool {
    !content.is_empty()
        && content.iter().enumerate().all(|(k, &b)| match b {
            b']' => false,
            b'[' => k > 0 && content[k - 1] == b'\\',
            _ => true,
        })
}

/// A CommonMark absolute-URI autolink at `bytes[i] == b'<'`: `<scheme:body>` where
/// the scheme is 2–32 chars beginning with an ASCII letter, then ASCII letters,
/// digits, `+`, `.`, or `-`; the body runs to the next `>` and may not contain a
/// space, `<`, or an ASCII control character. Returns the index past `>`, or
/// `None` when it is not a valid autolink — so raw HTML (`<p>`, `<img …>`, no
/// scheme `:`) falls through (to [`scan_md_email_autolink`], then raw HTML).
/// roxygen2's `mdxml_link` renders such a link (whose destination equals its text)
/// as `\url{…}`.
pub(super) fn scan_md_autolink(bytes: &[u8], i: usize) -> Option<usize> {
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

/// A CommonMark email autolink at `bytes[i] == b'<'`: `<addr>` where `addr`
/// matches the spec's email regex --- a local part of one or more
/// ``[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]``, an `@`, then one or more `.`-separated
/// domain labels, each an ASCII letter/digit optionally followed by up to 62
/// letters/digits/hyphens and ending in a letter/digit (no leading/trailing
/// hyphen; max label length 63). Returns the index past `>`, or `None` when it is
/// not a valid email autolink (it then falls through to raw HTML / literal prose).
/// A `scheme:`-bearing address is handled by [`scan_md_autolink`] first --- the two
/// forms are disjoint, an email address has no `:`. roxygen2's `mdxml_link` renders
/// this as `\href{mailto:addr}{addr}`.
pub(super) fn scan_md_email_autolink(bytes: &[u8], i: usize) -> Option<usize> {
    fn is_local(b: u8) -> bool {
        matches!(b,
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9'
            | b'.' | b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+'
            | b'/' | b'=' | b'?' | b'^' | b'_' | b'`' | b'{' | b'|' | b'}' | b'~' | b'-')
    }
    let mut j = i + 1;
    let local_start = j;
    while bytes.get(j).is_some_and(|&b| is_local(b)) {
        j += 1;
    }
    if j == local_start || bytes.get(j) != Some(&b'@') {
        return None;
    }
    j += 1; // consume '@'
    // One or more `.`-separated domain labels.
    loop {
        let label_start = j;
        if !bytes.get(j).is_some_and(u8::is_ascii_alphanumeric) {
            return None;
        }
        let mut last = j; // index of the last letter/digit seen in this label
        j += 1;
        while bytes
            .get(j)
            .is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'-')
        {
            if bytes[j] != b'-' {
                last = j;
            }
            j += 1;
        }
        // The label must end in a letter/digit (no trailing hyphen) and be at
        // most 63 characters long.
        if last + 1 != j || j - label_start > 63 {
            return None;
        }
        match bytes.get(j) {
            Some(&b'.') => j += 1, // another label follows
            Some(&b'>') => return Some(j + 1),
            _ => return None,
        }
    }
}

/// A CommonMark inline raw-HTML span at `bytes[i] == b'<'`: a tag, comment,
/// processing instruction, declaration, or CDATA section, line-scoped. Returns
/// the index past the span, or `None` when no form matches (it then stays
/// literal prose — losslessness holds either way). Each recognizer mirrors the
/// engine's "Raw HTML" grammar precisely so it never carves a span `commonmark`
/// (hence roxygen2) would keep literal; over-recognition would make the
/// projector emit a spurious `\out`.
///
/// roxygen2's `mdxml_html_inline` renders every form verbatim inside
/// `\if{html}{\out{…}}`.
///
/// Also the scanner for **multi-line** spans: the inline pass calls it on a
/// paragraph's joined logical text (soft breaks as `\n`), where every form's
/// text/whitespace loops cross line boundaries exactly as the engine's do.
pub(super) fn scan_md_html_inline(bytes: &[u8], i: usize) -> Option<usize> {
    match bytes.get(i + 1) {
        Some(&b'!') => scan_md_html_inline_bang(bytes, i),
        Some(&b'?') => scan_md_html_inline_pi(bytes, i),
        _ => scan_md_html_inline_tag(bytes, i),
    }
}

/// The `<!`-headed inline raw-HTML forms: a comment (`<!-- … -->`), a CDATA
/// section (`<![CDATA[ … ]]>`), or a declaration (`<!NAME …>`). The three are
/// disjoint (`<!--`/`<![` never satisfy the declaration's letter test).
fn scan_md_html_inline_bang(bytes: &[u8], i: usize) -> Option<usize> {
    let rest = &bytes[i..];
    if rest.starts_with(b"<!--") {
        return scan_md_html_inline_comment(bytes, i);
    }
    if is_html_cdata_opener(rest) {
        return scan_md_html_inline_cdata(bytes, i);
    }
    scan_md_html_inline_declaration(bytes, i)
}

/// An inline HTML comment at `bytes[i..] == b"<!--…"`. The engine follows the
/// relaxed HTML-spec rule (CommonMark 0.31): the empty forms `<!-->`/`<!--->`
/// are comments, otherwise the closer is the **first `-->` not preceded by a
/// `-`** in the text — interior `--`, `->`, `>`, and even a dash-blocked `-->`
/// are all comment text (`<!-- x ---> b -->` closes at the second `-->`), while
/// a comment whose only `-->` abuts a dash run (`<!-- x --->`) stays literal.
/// Returns the index past the closing `-->`.
fn scan_md_html_inline_comment(bytes: &[u8], i: usize) -> Option<usize> {
    let rest = &bytes[i..];
    if rest.starts_with(b"<!-->") {
        return Some(i + 5);
    }
    if rest.starts_with(b"<!--->") {
        return Some(i + 6);
    }
    let text_start = i + 4;
    let mut j = text_start;
    loop {
        if bytes.get(j)? == &b'-'
            && bytes.get(j + 1) == Some(&b'-')
            && bytes.get(j + 2) == Some(&b'>')
            // A `-->` preceded by a text `-` is comment text, not the closer
            // (the opener's own dashes don't count — an empty text is fine).
            && !(j > text_start && bytes[j - 1] == b'-')
        {
            return Some(j + 3);
        }
        j += 1;
    }
}

/// An inline CDATA section at `bytes[i..] == b"<![CDATA[…"` (the keyword is
/// case-insensitive, per [`is_html_cdata_opener`]). The text is
/// `("]" [^\]] | "]]" [^>] | [^\]])*`, then `]]>` closes — so `]]]>` never
/// closes a section (the `]]` pair consumes the following `]`), matching the
/// engine. Returns the index past the closing `]]>`.
fn scan_md_html_inline_cdata(bytes: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 9;
    loop {
        match bytes.get(j)? {
            b']' if bytes.get(j + 1) == Some(&b']') => match bytes.get(j + 2)? {
                b'>' => return Some(j + 3),
                _ => j += 3, // `]]` + a non-`>` byte
            },
            b']' => {
                bytes.get(j + 1)?; // `]` + a non-`]` byte (must exist)
                j += 2;
            }
            _ => j += 1,
        }
    }
}

/// An inline HTML declaration at `bytes[i..] == b"<!…"`: one or more
/// **uppercase** ASCII letters (the engine keeps the pre-0.31 uppercase-only
/// rule — `<!doctype` stays literal), **required** whitespace, then anything up
/// to the first `>`. Returns the index past it.
fn scan_md_html_inline_declaration(bytes: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 2;
    let name_start = j;
    while bytes.get(j).is_some_and(u8::is_ascii_uppercase) {
        j += 1;
    }
    if j == name_start {
        return None;
    }
    let after_ws = skip_html_ws(bytes, j);
    if after_ws == j {
        return None; // `<!DOCTYPE>` (no whitespace) stays literal
    }
    j = after_ws;
    while let Some(&b) = bytes.get(j) {
        if b == b'>' {
            return Some(j + 1);
        }
        j += 1;
    }
    None
}

/// An inline processing instruction at `bytes[i..] == b"<?…"`: anything (empty
/// included) up to the first `?>`. Returns the index past it.
fn scan_md_html_inline_pi(bytes: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 2;
    loop {
        if bytes.get(j)? == &b'?' && bytes.get(j + 1) == Some(&b'>') {
            return Some(j + 2);
        }
        j += 1;
    }
}

/// Whether `text` placed at a markdown line's content start would open an HTML
/// block (any of the start conditions 1–6 [`scan_md_html_block`] recognizes).
/// HTML blocks interrupt a paragraph, so the formatter must never reflow such
/// text onto a line start — an *inline* comment/PI/CDATA/declaration/block-tag
/// (or literal prose that merely looks like an opener, e.g. an unterminated
/// `<!--`) would reparse as a block and change the rendered Rd.
pub(crate) fn starts_md_html_block(text: &str) -> bool {
    scan_md_html_block(text.as_bytes(), 0).is_some()
}

/// Whether `text` is a complete standalone HTML tag — one open/closing/
/// self-closing tag followed by nothing but spaces or tabs. Placed alone at a
/// markdown line's content start at a fresh (non-paragraph) position, such a
/// line opens an HTML block (CommonMark start condition 7, blank-terminated).
/// Condition 7 cannot *interrupt* a paragraph, so mid-paragraph line starts are
/// safe — the formatter guards only a paragraph's **first** line with this.
pub(crate) fn is_md_standalone_html_tag(text: &str) -> bool {
    let bytes = text.as_bytes();
    scan_md_html_inline_tag(bytes, 0)
        .is_some_and(|end| bytes[end..].iter().all(|&b| b == b' ' || b == b'\t'))
}

/// Whether `rest` begins with a CDATA opener `<![CDATA[`. The `CDATA` keyword
/// is **case-insensitive** (cmark's scanner spells it as a re2c case-insensitive
/// literal), so `<![cdata[` opens too — both inline and at block level
/// (condition 5).
fn is_html_cdata_opener(rest: &[u8]) -> bool {
    rest.len() >= 9
        && rest.starts_with(b"<![")
        && rest[3..8].eq_ignore_ascii_case(b"cdata")
        && rest[8] == b'['
}

/// A CommonMark inline raw-HTML tag at `bytes[i] == b'<'`: an open tag
/// (`<name attrs… />`) or a closing tag (`</name >`), line-scoped. Returns the
/// index past the closing `>`, or `None` when this is not a well-formed tag.
fn scan_md_html_inline_tag(bytes: &[u8], i: usize) -> Option<usize> {
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

/// A CommonMark **HTML block start condition 1 or 6** at a line's content start:
/// `<` or `</`, then a tag name (case-insensitive), then a space/tab, `>`, `/>`,
/// or the end of the line. Condition 6 is one of the block-level tag names (open
/// or close form); condition 1 is one of the *verbatim* tags `<pre>`/`<script>`/
/// `<style>`/`<textarea>` in **opening** form (a close tag never starts condition
/// 1, and `/>` is condition 7, not 1). Either way the whole remaining line content
/// is the opener leaf; this returns the index past it (the end of the line). `None`
/// otherwise. The two conditions differ only in their **terminator** (blank line
/// vs a line containing a matching close tag), which the block builder re-derives
/// from the opener text — see [`super::build::emit_md_html_block`].
///
/// Conditions 2–5 are handled too, each carving the whole line off as an opener
/// leaf; their distinct terminators (`-->`/`?>`/`>`/`]]>`) are re-derived by the
/// block builder from the opener text:
///
/// * **Condition 2** — a line beginning `<!--` (an HTML comment).
/// * **Condition 3** — a line beginning `<?` (a processing instruction).
/// * **Condition 4** — a line beginning `<!` then an **uppercase** ASCII letter
///   (a declaration, e.g. `<!DOCTYPE`; the engine keeps the pre-0.31
///   uppercase-only rule).
/// * **Condition 5** — a line beginning `<![CDATA[` (the keyword is
///   case-insensitive).
///
/// Condition 7 (a complete tag alone on a line) is **not** carved here: its
/// opener lexes as an ordinary inline `RoxygenMdHtml` tag leaf, and the block
/// builder recognizes the standalone-tag line structurally
/// ([`super::build::is_md_html_block7_line`]) — condition 7 cannot interrupt a
/// paragraph, so the decision needs the paragraph state only the builder has.
fn scan_md_html_block(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.get(i) != Some(&b'<') {
        return None;
    }
    // Conditions 2–5 (comment / CDATA / declaration / processing instruction). All
    // carve the whole line; the terminator is re-derived in the block builder. The
    // forms are disjoint (`<!--`/`<![CDATA[` never satisfy the condition-4 letter
    // test), so their relative order here is immaterial.
    let rest = &bytes[i..];
    if rest.starts_with(b"<!--") // condition 2
        || is_html_cdata_opener(rest) // condition 5 (`CDATA` is case-insensitive)
        // condition 3
        || rest.starts_with(b"<?")
        // condition 4: `<!` then an **uppercase** ASCII letter (a declaration;
        // the engine keeps the pre-0.31 uppercase-only rule, so `<!doctype`
        // stays prose).
        || (rest.starts_with(b"<!") && rest.get(2).is_some_and(u8::is_ascii_uppercase))
    {
        return Some(bytes.len());
    }
    let mut j = i + 1;
    let closing = bytes.get(j) == Some(&b'/');
    if closing {
        j += 1;
    }
    let name_start = j;
    while bytes.get(j).is_some_and(u8::is_ascii_alphanumeric) {
        j += 1;
    }
    let name = &bytes[name_start..j];
    let verbatim = !closing && is_html_verbatim_block_tag(name);
    if !is_html_block_tag(name) && !verbatim {
        return None;
    }
    match bytes.get(j) {
        None | Some(b' ' | b'\t' | b'>') => Some(bytes.len()),
        // `/>` opens a condition-6 block (a complete/void tag) but not a verbatim
        // condition-1 one (CommonMark condition 1 requires whitespace, `>`, or EOL).
        Some(b'/') if !verbatim && bytes.get(j + 1) == Some(&b'>') => Some(bytes.len()),
        _ => None,
    }
}

/// The four CommonMark **HTML block start condition 1** tag names: their content
/// is verbatim (`<pre>`/`<script>`/`<style>`/`<textarea>`) and the block runs to a
/// line containing a matching close tag, not to a blank line. Single source of
/// truth, shared with the block builder's terminator ([`super::build`]).
pub(super) const HTML_VERBATIM_TAGS: &[&str] = &["pre", "script", "style", "textarea"];

/// Whether `name` (ASCII, case-insensitive) is one of the condition-1 verbatim tag
/// names ([`HTML_VERBATIM_TAGS`]).
fn is_html_verbatim_block_tag(name: &[u8]) -> bool {
    let Ok(name) = std::str::from_utf8(name) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    HTML_VERBATIM_TAGS.contains(&lower.as_str())
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

/// Advance past a run of CommonMark HTML whitespace: ASCII spaces, tabs, and
/// line endings. A sub-lexed *line* never contains a `\n`, so the line-scoped
/// scanners are unaffected; the newline arm serves the inline pass's joined
/// paragraph text, where a soft break is whitespace inside a tag or declaration
/// (`<span`⏎`class="v">`, `<!A`⏎`y>` — both engine-probed).
fn skip_html_ws(bytes: &[u8], i: usize) -> usize {
    let mut j = i;
    while bytes
        .get(j)
        .is_some_and(|&b| matches!(b, b' ' | b'\t' | b'\n'))
    {
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
/// Whether the backslash at `bytes[i]` is itself escaped: the length of the
/// maximal backslash run immediately before it is odd, so parse_Rd's
/// left-to-right pairing consumes it (`\\` → literal `\`) and it cannot begin
/// a macro.
pub(crate) fn rd_backslash_is_escaped(bytes: &[u8], i: usize) -> bool {
    let mut k = 0usize;
    while k < i && bytes[i - 1 - k] == b'\\' {
        k += 1;
    }
    k % 2 == 1
}

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
    // A zero-argument macro is a complete call by its name alone (`\dots` →
    // `(\dots)`); parse_Rd never consumes a following group for it, so carve
    // name-only even when a `{`/`[` follows.
    if super::is_zero_arg_rd_macro(name) {
        return Some(j);
    }
    // A brace-less `\word` that is **not** a known Rd macro is an `UNKNOWN` macro
    // token (parse_Rd tags any unrecognized `\word` `UNKNOWN`, even without a
    // group). Any other *known* name brace-less stays literal prose: parse_Rd's
    // drop-recovery for an arg-requiring macro's misuse (`\emph z`) is backlog,
    // and leaving it as text keeps the existing tokenization (no regression).
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
                        | TokKind::RoxygenMdTableDelim
                        | TokKind::RoxygenMdHeading
                        | TokKind::RoxygenMdSetextUnderline
                        | TokKind::RoxygenMdBlockQuote
                        | TokKind::RoxygenMdThematicBreak
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
    fn md_consecutive_list_markers_carve_a_same_line_nested_item() {
        // `- - foo`: an item whose content itself begins with a list marker
        // opens a nested list (cm-300), so every same-line consecutive marker
        // is carved, with the separating whitespace as its own prose run.
        let src = "#' - - foo\n#' @md\n";
        assert_eq!(
            prose_texts(src),
            vec![
                (TokKind::RoxygenMdListMarker, "-".into()),
                (TokKind::RoxygenText, " ".into()),
                (TokKind::RoxygenMdListMarker, "-".into()),
                (TokKind::RoxygenText, " foo".into()),
            ]
        );
        assert_lossless(src);
        // `1. - 2. foo` nests three deep (cm-301) — ordered markers carve too.
        assert_eq!(
            prose_texts("#' 1. - 2. foo\n#' @md\n"),
            vec![
                (TokKind::RoxygenMdListMarker, "1.".into()),
                (TokKind::RoxygenText, " ".into()),
                (TokKind::RoxygenMdListMarker, "-".into()),
                (TokKind::RoxygenText, " ".into()),
                (TokKind::RoxygenMdListMarker, "2.".into()),
                (TokKind::RoxygenText, " foo".into()),
            ]
        );
        // A remainder that is a thematic break (`* * *`) is a different block,
        // not a nested marker — no second `RoxygenMdListMarker` is carved (the
        // `*`s fall to the neutral emphasis-delim carve, space-flanked so they
        // never resolve; unchanged from before the consecutive-marker carve).
        assert!(
            !prose_texts("#' - * * *\n#' @md\n")[1..]
                .iter()
                .any(|(k, _)| *k == TokKind::RoxygenMdListMarker)
        );
        assert_eq!(
            prose_texts("#' -     - foo\n#' @md\n"),
            vec![
                (TokKind::RoxygenMdListMarker, "-".into()),
                (TokKind::RoxygenText, "     - foo".into()),
            ]
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
    fn md_table_delim_recognized_under_md_mode() {
        // Under `@md`, a line whose whole content is a GFM delimiter row carves off
        // as a single `RoxygenMdTableDelim` leaf (with or without outer pipes, and
        // with alignment colons). The header/body rows stay ordinary text.
        for delim in ["|---|---|", "| :-- | :-: | --: |", "---|---", "| --- |"] {
            let src = format!("#' {delim}\n#' @md\n");
            assert_eq!(
                prose_texts(&src),
                vec![(TokKind::RoxygenMdTableDelim, delim.into())],
                "delimiter row {delim:?}"
            );
            assert_lossless(&src);
        }
    }

    #[test]
    fn md_table_delim_rejects_non_delimiters() {
        // A colon-only cell (`| : |`, no hyphen) is not a delimiter row — it stays
        // literal prose. (A pipeless `---` is a *setext underline* leaf now, covered
        // by `md_setext_underline_recognized`.)
        let text = "| : | - |";
        let src = format!("#' {text}\n#' @md\n");
        assert_eq!(
            prose_texts(&src),
            vec![(TokKind::RoxygenText, text.into())],
            "non-delimiter {text:?}"
        );
    }

    #[test]
    fn md_setext_underline_recognized() {
        // Under `@md`, a line whose content is a run of `=` or two-or-more `-` (with
        // optional up-to-3-space indent and trailing whitespace) carves off as a
        // single `RoxygenMdSetextUnderline` leaf — the block builder decides whether
        // it promotes a preceding paragraph into a heading.
        for underline in ["===", "=", "---", "--", "=== "] {
            let src = format!("#' text\n#' {underline}\n#' @md\n");
            let toks = prose_texts(&src);
            assert_eq!(
                toks.last(),
                Some(&(TokKind::RoxygenMdSetextUnderline, underline.into())),
                "underline {underline:?} -> {toks:?}"
            );
            assert_lossless(&src);
        }
    }

    #[test]
    fn md_setext_underline_rejects_non_underlines() {
        // A single `-`/`- ` (an empty list bullet), a mixed `=-=`, a `=== x` with
        // trailing text, and an underline without `@md` all stay non-underline: the
        // single dash is a list marker, the rest literal prose.
        let single_dash = prose_texts("#' text\n#' - \n#' @md\n");
        assert!(
            single_dash
                .iter()
                .any(|t| t.0 == TokKind::RoxygenMdListMarker && t.1 == "-"),
            "single `- ` is a list bullet: {single_dash:?}"
        );
        assert!(
            !single_dash
                .iter()
                .any(|t| t.0 == TokKind::RoxygenMdSetextUnderline),
            "single `- ` is not a setext underline: {single_dash:?}"
        );
        for text in ["=-=", "=== x"] {
            let src = format!("#' first\n#' {text}\n#' @md\n");
            assert_eq!(
                prose_texts(&src).last(),
                Some(&(TokKind::RoxygenText, text.into())),
                "non-underline {text:?}"
            );
        }
        assert_eq!(
            prose_texts("#' text\n#' ===\n").last(),
            Some(&(TokKind::RoxygenText, "===".into())),
            "no `@md`: an underline-looking line is literal prose"
        );
    }

    #[test]
    fn md_block_quote_recognized_under_md_mode() {
        // Under `@md`, a line whose content opens with `>` carves off as a single
        // `RoxygenMdBlockQuote` leaf, the whole line included. The `#'` marker→content
        // whitespace is trivia, so any leading indentation is stripped before the leaf
        // (a `>` after 4+ spaces is roxygen2's *indented code block*, not a quote — an
        // unmodeled gap shared with the fence/heading/HTML-block recognizers).
        for quote in ["> quoted", ">tight"] {
            let src = format!("#' {quote}\n#' @md\n");
            assert_eq!(
                prose_texts(&src).first(),
                Some(&(TokKind::RoxygenMdBlockQuote, quote.into())),
                "block quote {quote:?}"
            );
            assert_lossless(&src);
        }
    }

    #[test]
    fn md_block_quote_rejects_non_quotes() {
        // A `>` not at content start stays literal prose; a `>` line without `@md`
        // likewise stays literal (the recognizer is mode-gated).
        let src = "#' a > b\n#' @md\n";
        assert!(
            !prose_texts(src)
                .iter()
                .any(|t| t.0 == TokKind::RoxygenMdBlockQuote),
            "a `>` mid-text is not a block quote"
        );
        assert_eq!(
            prose_texts("#' > quoted\n").first(),
            Some(&(TokKind::RoxygenText, "> quoted".into())),
            "no `@md`: a `>` line is literal prose"
        );
    }

    #[test]
    fn md_thematic_break_recognized_under_md_mode() {
        // Under `@md`, a line whose whole content is a thematic break carves off as a
        // single `RoxygenMdThematicBreak` leaf. The lexer owns the `*`/`_`-based and
        // space-separated forms; a contiguous `---` is claimed by the setext-underline
        // path first (setext precedence), so it is not tested here.
        for brk in ["***", "___", "* * *", "- - -", "_ _ _", "****", "*** "] {
            let src = format!("#' text\n#' {brk}\n#' @md\n");
            let toks = prose_texts(&src);
            assert_eq!(
                toks.last(),
                Some(&(TokKind::RoxygenMdThematicBreak, brk.into())),
                "thematic break {brk:?} -> {toks:?}"
            );
            assert_lossless(&src);
        }
    }

    #[test]
    fn md_thematic_break_rejects_non_breaks() {
        // A run shorter than three (`**`, emphasis delimiters), a line with trailing
        // text (`*** x`), a mixed run (`*-*`), and a break without `@md` all stay
        // non-break: none carves a `RoxygenMdThematicBreak` leaf.
        for text in ["** ", "*** x", "*-*"] {
            let src = format!("#' first\n#' {text}\n#' @md\n");
            assert!(
                !prose_texts(&src)
                    .iter()
                    .any(|t| t.0 == TokKind::RoxygenMdThematicBreak),
                "non-break {text:?} must not carve a thematic break"
            );
        }
        assert!(
            !prose_texts("#' text\n#' ***\n")
                .iter()
                .any(|t| t.0 == TokKind::RoxygenMdThematicBreak),
            "no `@md`: a `***` line is literal prose"
        );
    }

    #[test]
    fn md_heading_recognized_under_md_mode() {
        // Under `@md`, a line whose content opens an ATX heading (1-6 `#` then a
        // space, up to the line end) carves off as a single `RoxygenMdHeading` leaf,
        // whole line (title and any closing `#` sequence included).
        for heading in ["# Title", "###### Deep", "# Closing #", "#"] {
            let src = format!("#' {heading}\n#' @md\n");
            assert_eq!(
                prose_texts(&src),
                vec![(TokKind::RoxygenMdHeading, heading.into())],
                "heading {heading:?}"
            );
            assert_lossless(&src);
        }
    }

    #[test]
    fn md_heading_rejects_non_headings() {
        // `#hashtag` (no space after the run), a run of seven `#` (over the CommonMark
        // limit of six), and `#` off without `@md` all stay literal prose.
        for text in ["#hashtag", "####### seven"] {
            let src = format!("#' {text}\n#' @md\n");
            assert_eq!(
                prose_texts(&src),
                vec![(TokKind::RoxygenText, text.into())],
                "non-heading {text:?}"
            );
        }
        assert_eq!(
            prose_texts("#' # Title\n"),
            vec![(TokKind::RoxygenText, "# Title".into())],
            "no `@md`: a hash line is literal prose"
        );
    }

    #[test]
    fn md_table_delim_off_without_md_directive() {
        // No `@md`: a delimiter-looking row stays literal prose (no delim leaf).
        assert_eq!(
            prose_texts("#' |---|---|\n"),
            vec![(TokKind::RoxygenText, "|---|---|".into())]
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
    fn md_fence_recognizes_tilde_runs() {
        // A run of three-plus tildes opens a fence; its info string may contain
        // backticks and tildes (CommonMark 4.5 — the backtick-in-info rule is
        // backtick-fence-specific). A two-tilde run is not a fence.
        assert_eq!(
            prose_texts("#' ~~~ aa ``` ~~~\n#' @md\n"),
            vec![(TokKind::RoxygenMdFence, "~~~ aa ``` ~~~".into())]
        );
        assert_eq!(
            prose_texts("#' ~~ not a fence\n#' @md\n"),
            vec![(TokKind::RoxygenText, "~~ not a fence".into())]
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
    fn md_collapsed_ref_closer_carves_whole() {
        // A collapsed reference `[text][]` carves its `][]` closer as one neutral
        // bracket leaf (the empty label cannot carve on its own); the inline pass
        // pairs it with the `[` opener into a `ROXYGEN_MD_LINK` node whose label
        // resolves from the display.
        assert_eq!(
            prose_texts("#' a [foo][] b\n#' @md\n"),
            vec![
                (TokKind::RoxygenText, "a ".into()),
                (TokKind::RoxygenMdBracket, "[".into()),
                (TokKind::RoxygenText, "foo".into()),
                (TokKind::RoxygenMdBracket, "][]".into()),
                (TokKind::RoxygenText, " b".into()),
            ]
        );
        assert_lossless("#' a [foo][] b\n#' @md\n");
        // Without an opener the composite closer stays literal (`[][]` never
        // links: an empty display carves no opener, and the unmatched `][]`
        // re-emits as text).
        assert_lossless("#' a [][] b\n#' @md\n");
    }

    #[test]
    fn md_function_autolink() {
        // A same-line plain-text shortcut `[func()]` carves as neutral bracket
        // leaves (opener `[`, closer `]`) for the inline pass to pair, not the
        // opaque `RoxygenMdLink` leaf (see `same_line_bracket_opener`).
        assert_eq!(
            prose_texts("#' Call [func()] and [pkg::g()].\n#' @md\n"),
            vec![
                (TokKind::RoxygenText, "Call ".into()),
                (TokKind::RoxygenMdBracket, "[".into()),
                (TokKind::RoxygenText, "func()".into()),
                (TokKind::RoxygenMdBracket, "]".into()),
                (TokKind::RoxygenText, " and ".into()),
                (TokKind::RoxygenMdBracket, "[".into()),
                (TokKind::RoxygenText, "pkg::g()".into()),
                (TokKind::RoxygenMdBracket, "]".into()),
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
    fn md_email_autolink() {
        // A CommonMark email autolink `<addr>` carves as a `RoxygenMdLink` under
        // `@md`; an address with an invalid domain label (a leading hyphen here)
        // is not an autolink and stays literal prose.
        assert_eq!(
            prose_texts("#' mail <a.b+c@x.co> not <foo@-ex.com>\n#' @md\n"),
            vec![
                (TokKind::RoxygenText, "mail ".into()),
                (TokKind::RoxygenMdLink, "<a.b+c@x.co>".into()),
                (TokKind::RoxygenText, " not <foo@-ex.com>".into()),
            ]
        );
        assert_lossless("#' mail <a.b+c@x.co> not <foo@-ex.com>\n#' @md\n");
        // Without `@md`, `<` is literal prose --- no autolink recognition.
        assert_eq!(
            prose_texts("#' mail <a@b.com>\n"),
            vec![(TokKind::RoxygenText, "mail <a@b.com>".into())]
        );
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
        // A reference link carves entirely onto neutral bracket leaves: the display
        // `[text]` and the label `[ref]` are each a `[`/`]` pair; the inline pass
        // pairs them into a `ROXYGEN_MD_LINK` node and reads `[ref]` off the lookahead.
        assert_eq!(
            prose_texts("#' a [text][ref] b\n#' @md\n"),
            vec![
                (TokKind::RoxygenText, "a ".into()),
                (TokKind::RoxygenMdBracket, "[".into()),
                (TokKind::RoxygenText, "text".into()),
                (TokKind::RoxygenMdBracket, "]".into()),
                (TokKind::RoxygenMdBracket, "[".into()),
                (TokKind::RoxygenText, "ref".into()),
                (TokKind::RoxygenMdBracket, "]".into()),
                (TokKind::RoxygenText, " b".into()),
            ]
        );
        assert_lossless("#' a [text][ref] b\n#' @md\n");
    }

    #[test]
    fn md_reference_link_with_markup_display_carves_onto_arena() {
        // A markup-bearing reference display (`[*foo*][r1]`) carves the same way as a
        // plain one (`[plain][r2]`): the `[` opener, the closing `]`, and the `[ref]`
        // label are all neutral bracket leaves, so the inline pass resolves the
        // emphasis and the projector applies roxygen2's drop rule.
        assert_eq!(
            prose_texts("#' a [*foo*][r1] and [plain][r2] b\n#' @md\n"),
            vec![
                (TokKind::RoxygenText, "a ".into()),
                (TokKind::RoxygenMdBracket, "[".into()),
                (TokKind::RoxygenMdDelim, "*".into()),
                (TokKind::RoxygenText, "foo".into()),
                (TokKind::RoxygenMdDelim, "*".into()),
                (TokKind::RoxygenMdBracket, "]".into()),
                (TokKind::RoxygenMdBracket, "[".into()),
                (TokKind::RoxygenText, "r1".into()),
                (TokKind::RoxygenMdBracket, "]".into()),
                (TokKind::RoxygenText, " and ".into()),
                (TokKind::RoxygenMdBracket, "[".into()),
                (TokKind::RoxygenText, "plain".into()),
                (TokKind::RoxygenMdBracket, "]".into()),
                (TokKind::RoxygenMdBracket, "[".into()),
                (TokKind::RoxygenText, "r2".into()),
                (TokKind::RoxygenMdBracket, "]".into()),
                (TokKind::RoxygenText, " b".into()),
            ]
        );
        assert_lossless("#' a [*foo*][r1] and [plain][r2] b\n#' @md\n");
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
        // Under `@md`, a same-line bracket-free plain-text `[…]` shortcut — words,
        // digits, spaces, and `::` all qualify — carves as neutral bracket leaves
        // (opener/closer) for the inline pass; a `[…]{` is excluded and stays prose.
        assert_eq!(
            prose_texts("#' see [note], [see this], [pkg::obj] but [x]{y}\n#' @md\n"),
            vec![
                (TokKind::RoxygenText, "see ".into()),
                (TokKind::RoxygenMdBracket, "[".into()),
                (TokKind::RoxygenText, "note".into()),
                (TokKind::RoxygenMdBracket, "]".into()),
                (TokKind::RoxygenText, ", ".into()),
                (TokKind::RoxygenMdBracket, "[".into()),
                (TokKind::RoxygenText, "see this".into()),
                (TokKind::RoxygenMdBracket, "]".into()),
                (TokKind::RoxygenText, ", ".into()),
                (TokKind::RoxygenMdBracket, "[".into()),
                (TokKind::RoxygenText, "pkg::obj".into()),
                (TokKind::RoxygenMdBracket, "]".into()),
                (TokKind::RoxygenText, " but [x]{y}".into()),
            ]
        );
        assert_lossless("#' see [note], [see this], [pkg::obj] but [x]{y}\n#' @md\n");
    }

    #[test]
    fn nested_bracket_link_carves_outer_opener() {
        // A nested-bracket link `[a [b] c](url)` carves *every* bracket as a neutral
        // leaf (the outer opener via `is_nested_bracket_opener`, the inner `[b]` via
        // the same-line shortcut path, and the `](url)` inline closer), so the inline
        // pass can resolve the nesting with opener deactivation. The opaque
        // `scan_md_link` no longer swallows the whole span.
        assert_eq!(
            prose_texts("#' x [a [b] c](https://o.org) y\n#' @md\n"),
            vec![
                (TokKind::RoxygenText, "x ".into()),
                (TokKind::RoxygenMdBracket, "[".into()),
                (TokKind::RoxygenText, "a ".into()),
                (TokKind::RoxygenMdBracket, "[".into()),
                (TokKind::RoxygenText, "b".into()),
                (TokKind::RoxygenMdBracket, "]".into()),
                (TokKind::RoxygenText, " c".into()),
                (TokKind::RoxygenMdBracket, "](https://o.org)".into()),
                (TokKind::RoxygenText, " y".into()),
            ]
        );
        assert_lossless("#' x [a [b] c](https://o.org) y\n#' @md\n");
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
