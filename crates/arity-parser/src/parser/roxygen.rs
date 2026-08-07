//! Roxygen2 doc-comment recognition, sub-tokenization, and block structure.
//!
//! A roxygen line is a comment whose text matches `^#+'` (one-or-more `#`
//! followed by a single `'`). Such lines are sub-tokenized—rather than emitted
//! as one `COMMENT` token—so their structure (marker, tags, arguments, prose)
//! lives directly in the lossless CST. The sub-tokens' texts tile the line's
//! bytes exactly, preserving the round-trip invariant.
//!
//! The work is split into three phases, one module each, plus this parent which
//! owns the macro-classification layer (the `\macro` arity/verbatim tables that
//! both the lexer and the structure builder consult) and the shared
//! balanced-delimiter scan:
//!
//! * [`lex`] — sub-lexing: block-mode resolution + the per-line tokenizer
//!   (text → `Vec<Token>`).
//! * [`group`] — block grouping: wrapping a run of lines in a `ROXYGEN_BLOCK`
//!   and laying out its section/paragraph skeleton (`Vec<Token>` → `Vec<Event>`).
//! * [`build`] — structure building: the block-level Rd-macro and markdown
//!   constructs (`\itemize{…}`, `\describe{…}`, markdown lists) dispatched from
//!   the grouper.

mod build;
mod group;
mod inline;
mod lex;

pub(crate) use group::emit_roxygen_block;
pub use lex::{is_md_standalone_html_tag, md_fence_run_closes, starts_md_html_block};
pub(crate) use lex::{
    is_roxygen_comment, lex_roxygen_line, rd_backslash_is_escaped, resolve_roxygen_block,
    roxygen_line_tag, scan_rd_macro, tag_body_skips_markdown, tag_folds_prose_continuation,
};

/// Inline Rd macros whose `{…}` content is **verbatim** (`VERB` in
/// `tools::parse_Rd`): the body is raw text and nested `\macro` markup is *not*
/// parsed. Confirmed against `parse_Rd` (see the projector's `rd_macros` work).
/// Latexlike macros (`\code`, `\emph`, `\strong`, `\link`, …) are everything
/// else --- their content is sub-parsed, so nested macros become child nodes.
/// `\eqn`/`\deqn` (LaTeX math) and `\out` (raw output) are verbatim too:
/// `parse_Rd` yields `(\eqn (VERB "x \code{q} y"))`, the nested markup literal.
const VERBATIM_RD_MACROS: &[&str] = &[
    "url", "verb", "samp", "env", "kbd", "option", "eqn", "deqn", "out",
];

/// Whether the macro named `name` (without the leading `\`) takes verbatim
/// `{…}` content. Used both when building the CST (don't recurse into a verbatim
/// body) and when projecting it (emit `VERB`, not coalesced `TEXT`) — the
/// projector's block-form verbatim arm keys on this set, so it is `pub` like its
/// [`rd_macro_arity`] sibling (semver-loose, see the crate docs).
pub fn is_verbatim_rd_macro(name: &str) -> bool {
    VERBATIM_RD_MACROS.contains(&name)
}

/// Whether argument group `index` (0-based) of the macro named `name` takes
/// **verbatim** `{…}` content (`VERB` in `parse_Rd`: raw text, no nested markup).
/// A fully-verbatim macro (`\url`/`\verb`/…) is verbatim in its only argument;
/// `\href{url}{text}` is verbatim in its *first* argument (the URL) but latexlike
/// in its *second* (the link text, which is sub-parsed). Drives both the tree
/// builder (don't recurse into a verbatim arg) and, via the emitted `VERB` leaf,
/// the projector. Confirmed against `parse_Rd`: `\href`'s first arg is `VERB`.
pub(crate) fn is_verbatim_rd_arg(name: &str, index: usize) -> bool {
    is_verbatim_rd_macro(name) || (name == "href" && index == 0) || name == "figure"
}

/// The Rd macros whose adjacent-`{…}`-group count is **not one**, with that
/// count the way `tools::parse_Rd` counts it. Every name absent here takes
/// exactly one group, so a trailing `\code{x}{y}`'s `{y}` stays literal --- the
/// arity is per macro, and a group past the count is likewise literal
/// (`\ifelse{a}{b}{c}{d}` → `(\ifelse …)` plus a sibling `(LIST …)`). Probed
/// exhaustively against R 4.6 (every [`KNOWN_RD_MACROS`] name written
/// `\name{A}{B}{C}` in a `\details` section).
///
/// A trailing group is *optional*: `\eqn{x^2}` alone is a complete call, and the
/// lexer's consumption is present-and-adjacent-only, so the optionality costs
/// nothing. parse_Rd requires same-line adjacency --- `\eqn{a} {b}` and a
/// next-line `{b}` leave the group a literal brace `LIST`. A brace-less `\item`
/// (under `\itemize`/`\enumerate`) never reaches here: it has no `{`, so it is
/// not a macro token at all.
///
/// **Arity `0` is a complete call by name alone**: parse_Rd consumes no group at
/// all, so a written `{}`/`{x}` is a *following sibling* brace list, never an
/// argument (`\dots{}` → `(\dots)` plus `(LIST)`). See [`is_zero_arg_rd_macro`].
///
/// The trailing entries are **not** parse_Rd built-ins but R's *system user
/// macros* (`share/Rd/macros/system.Rd`, always loaded before every Rd file);
/// their arity is the largest `#N` in the definition --- zero when it has none
/// --- and parse_Rd consumes that many groups before expanding. The definitions
/// themselves live in the projector's `usermacro` module --- only the group
/// count is a parsing fact.
///
/// The arity-above-one names are also the macros whose `{…}` arguments
/// `parse_Rd` models as *list* wrappers (so a multi-atom argument projects to a
/// `(GRP …)`), as opposed to latexlike macros (`\code`, `\emph`, …) whose single
/// argument's content is inlined directly. The projector keys its GRP rule on
/// [`is_multi_arg_rd_macro`].
const RD_MACRO_ARITY: &[(&str, usize)] = &[
    ("if", 2),
    ("ifelse", 3),
    ("item", 2),
    ("tabular", 2),
    ("href", 2),
    ("figure", 2),
    ("eqn", 2),
    ("deqn", 2),
    ("enc", 2),
    ("method", 2),
    ("S3method", 2),
    ("S4method", 2),
    ("subsection", 2),
    // Zero-argument built-ins: `\cr`, `\dots`, … are complete by name alone.
    ("cr", 0),
    ("tab", 0),
    ("dots", 0),
    ("ldots", 0),
    ("R", 0),
    // R system user macros (see above).
    ("manual", 2),
    ("bibinfo", 3),
    ("sspace", 0),
    ("LaTeX", 0),
];

/// How many `{…}` argument groups the macro named `name` (without the leading
/// `\`) takes; `1` for every name outside [`RD_MACRO_ARITY`]. Drives the lexer
/// (consume that many groups into one token), the tree builder (emit them as
/// children), and the projector (each group of a multi-argument macro is a list
/// argument --- a multi-atom one becomes a `(GRP …)`).
pub fn rd_macro_arity(name: &str) -> usize {
    RD_MACRO_ARITY
        .iter()
        .find(|(n, _)| *n == name)
        .map_or(1, |(_, arity)| *arity)
}

/// Whether the macro named `name` (without the leading `\`) takes more than one
/// `{…}` argument group --- parse_Rd's *structural* macros, whose arguments are
/// list wrappers. See [`rd_macro_arity`].
pub fn is_multi_arg_rd_macro(name: &str) -> bool {
    rd_macro_arity(name) > 1
}

/// Split a GFM table row into its cells, honoring backslash-escaped pipes. One
/// optional leading and one optional trailing **unescaped** `|` are stripped (the
/// GFM leading/trailing pipe), then the remainder is split on each unescaped `|`.
/// Cells are returned untrimmed (callers trim). An escaped `\|` stays inside its
/// cell. Shared by the recognition gate (cell counting) and the projector (cell
/// rendering) so the two never disagree on where a cell begins.
///
/// GFM counts pipes **without** honoring code spans — a `|` inside `` `…` `` still
/// splits a cell — so this deliberately does not track backticks. That is what
/// makes `| ` + "`a|b`" + ` | y |` fail the header/delimiter cell-count match and
/// stay prose (verified against roxygen2).
pub fn split_table_row_cells(line: &str) -> Vec<&str> {
    let trimmed = line.trim();
    let bytes = trimmed.as_bytes();
    let start = usize::from(bytes.first() == Some(&b'|'));
    let end = if bytes.len() > start
        && bytes[bytes.len() - 1] == b'|'
        && !pipe_is_escaped(bytes, bytes.len() - 1)
    {
        bytes.len() - 1
    } else {
        bytes.len()
    };
    let inner = &trimmed[start..end];
    let ib = inner.as_bytes();
    let mut cells = Vec::new();
    let mut cell_start = 0;
    let mut i = 0;
    while i < ib.len() {
        match ib[i] {
            b'\\' => i += 2, // skip the escaped byte (an escaped `\|` stays in-cell)
            b'|' => {
                cells.push(&inner[cell_start..i]);
                i += 1;
                cell_start = i;
            }
            _ => i += 1,
        }
    }
    cells.push(&inner[cell_start.min(inner.len())..]);
    cells
}

/// The number of cells in a GFM table row (see [`split_table_row_cells`]). The
/// header row and the delimiter row form a table only when these are equal.
pub(crate) fn count_table_cells(line: &str) -> usize {
    split_table_row_cells(line).len()
}

/// Whether `line` is a GFM table **delimiter row**: it contains at least one
/// unescaped `|` (so a bare `---`, which is a setext underline, is *not* a
/// single-column table) and every cell (trimmed) is `:?-+:?` (optional leading
/// colon, one or more hyphens, optional trailing colon). The pipe requirement
/// mirrors cmark-gfm, which treats a pipeless dash run as a setext heading.
pub fn is_table_delim_row(line: &str) -> bool {
    let trimmed = line.trim();
    if !line_has_unescaped_pipe(trimmed) {
        return false;
    }
    let cells = split_table_row_cells(trimmed);
    !cells.is_empty() && cells.iter().all(|c| is_table_delim_cell(c.trim()))
}

/// Whether the byte at `idx` (a `|`) is backslash-escaped: preceded by an odd
/// run of `\`.
fn pipe_is_escaped(bytes: &[u8], idx: usize) -> bool {
    let mut k = idx;
    let mut count = 0;
    while k > 0 && bytes[k - 1] == b'\\' {
        count += 1;
        k -= 1;
    }
    count % 2 == 1
}

/// Whether `line` contains an unescaped `|`.
fn line_has_unescaped_pipe(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'|' => return true,
            _ => i += 1,
        }
    }
    false
}

/// Whether `cell` (already trimmed) is a valid delimiter cell: `:?-+:?`.
fn is_table_delim_cell(cell: &str) -> bool {
    let b = cell.as_bytes();
    let mut i = usize::from(b.first() == Some(&b':'));
    let dash_start = i;
    while i < b.len() && b[i] == b'-' {
        i += 1;
    }
    if i == dash_start {
        return false; // a delimiter cell needs at least one hyphen
    }
    if b.get(i) == Some(&b':') {
        i += 1;
    }
    i == b.len()
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

/// Advance a markdown value column across one whitespace character: a tab
/// expands to the next 4-column tab stop (CommonMark's rule — roxygen2 hands
/// cmark the tag value with tabs intact), any other character is one column.
/// Columns are zero-based **value** coordinates: column 0 is the first
/// character after the `#'` sigil plus the one whitespace character roxygen2's
/// tokenizer strips (`consumeWhitespace(1)`, parser2.cpp — a space *or* a tab).
/// The single source of column arithmetic for the block builder's indent gauge
/// and the projector's column stripping.
pub fn advance_md_col(col: usize, c: char) -> usize {
    if c == '\t' {
        col - col % 4 + 4
    } else {
        col + 1
    }
}

/// The one-based indent **gauge** of a marker→content whitespace run: the first
/// character (space or tab alike) is the one roxygen2 strips and maps to
/// gauge 1 (value column 0); the rest expand via [`advance_md_col`], so the
/// gauge is the content's value column plus one. An empty run gauges 0. For an
/// all-space run this equals the character count — the pre-tab behavior.
pub fn md_ws_gauge<'a>(texts: impl IntoIterator<Item = &'a str>) -> usize {
    let mut col = 0usize;
    let mut first = true;
    for text in texts {
        for c in text.chars() {
            if first {
                first = false;
                continue;
            }
            col = advance_md_col(col, c);
        }
    }
    if first { 0 } else { col + 1 }
}

/// The end index of an Rd macro name starting at `bytes[start]` (the byte *after*
/// the leading `\`). An Rd command name is `[A-Za-z][A-Za-z0-9]*`: a leading
/// letter then any letters or digits (e.g. `\linkS4class`). Returns `start` when
/// no valid name begins there (`\\`, `\{`, `\4`, end of input). The single source
/// of truth for where a `\name` ends, shared by the lexer and the tree builder.
pub fn rd_macro_name_end(bytes: &[u8], start: usize) -> usize {
    let mut k = start;
    if bytes.get(k).is_some_and(u8::is_ascii_alphabetic) {
        k += 1;
        while k < bytes.len() && bytes[k].is_ascii_alphanumeric() {
            k += 1;
        }
    }
    k
}

/// The built-in Rd macro names `tools::parse_Rd` recognizes (without the leading
/// `\`). A `\word` *not* in this set is an **unknown** macro: `parse_Rd` tags it
/// `UNKNOWN` (warning "unknown macro '\word'"), even brace-less. Used to gate
/// brace-less macro recognition in the lexer (only an unknown name is carved as a
/// token; a known name brace-less stays literal prose --- its name-only/expanded
/// rendering is backlog) and the projector's name-only classification (a known
/// list child like `\item`/`\cr` → `(\name)`, an unknown one → `(UNKNOWN …)`).
///
/// The set is parse_Rd's static keyword table, verified against R 4.5; it
/// deliberately excludes package/user-defined macros (`\CRANpkg`, `\doi`, …),
/// which `parse_Rd` *expands* rather than parses (out of scope for a static
/// projector --- they surface as faithful divergences).
const KNOWN_RD_MACROS: &[&str] = &[
    // Sectioning / structural commands.
    "name",
    "alias",
    "title",
    "description",
    "usage",
    "arguments",
    "value",
    "details",
    "references",
    "note",
    "author",
    "seealso",
    "examples",
    "keyword",
    "concept",
    "section",
    "subsection",
    "docType",
    "encoding",
    "Rdversion",
    "format",
    "source",
    "synopsis",
    "figure",
    "item",
    "describe",
    "itemize",
    "enumerate",
    "tabular",
    "method",
    "S3method",
    "S4method",
    "newcommand",
    "renewcommand",
    "Sexpr",
    "RdOpts",
    "if",
    "ifelse",
    "out",
    "enc",
    "href",
    // Inline text / cross-reference / math macros.
    "emph",
    "strong",
    "bold",
    "code",
    "preformatted",
    "kbd",
    "samp",
    "pkg",
    "file",
    "email",
    "url",
    "var",
    "env",
    "option",
    "command",
    "dfn",
    "cite",
    "acronym",
    "dQuote",
    "sQuote",
    "verb",
    "link",
    "linkS4class",
    "eqn",
    "deqn",
    // Zero-argument / escape / examples-only commands.
    "cr",
    "tab",
    "dots",
    "ldots",
    "R",
    "dontrun",
    "donttest",
    "dontshow",
    "testonly",
];

/// Whether `name` (without the leading `\`) is a built-in Rd macro `parse_Rd`
/// recognizes. The single source of truth for the known/unknown split, shared by
/// the lexer (gate brace-less recognition) and the projector (name-only → `(\name)`
/// vs `(UNKNOWN …)`). See [`KNOWN_RD_MACROS`].
pub fn is_known_rd_macro(name: &str) -> bool {
    KNOWN_RD_MACROS.contains(&name)
}

/// Whether the macro named `name` (without the leading `\`) takes **no**
/// argument group --- `parse_Rd`'s zero-argument built-ins (`\dots`, `\cr`, …)
/// and the zero-placeholder system user macros (`\sspace`, `\LaTeX`). Such an
/// occurrence is a *complete* call by its name alone, so the lexer carves it
/// name-only even when a `{` follows (the group is then a sibling brace list) —
/// unlike other known names, whose brace-less misuse triggers parse_Rd's
/// drop-recovery. One source of truth with the rest of the group count:
/// [`RD_MACRO_ARITY`].
pub(crate) fn is_zero_arg_rd_macro(name: &str) -> bool {
    rd_macro_arity(name) == 0
}

/// Known Rd macros whose brace-less misuse does **not** recover as a clean text
/// drop: by the time parse_Rd's "expecting `{`" error fires, its lexer has
/// already switched to the macro's argument mode, and the recovery leaves that
/// state in place — everything to *section end* (crossing paragraph breaks)
/// becomes per-line `RCODE`/`VERB` atoms (`\code z` → `(RCODE " z after\n") …`).
/// `\item` is special the other way: out of list context parse_Rd tags it an
/// `(UNKNOWN "\item")` node and the text continues. Probed exhaustively against
/// R 4.5 (every [`KNOWN_RD_MACROS`] name as `before \name z after`); every name
/// *not* here (and not zero-arg) drops cleanly instead — see
/// [`is_rd_braceless_drop_macro`].
const STICKY_BRACELESS_RD_MACROS: &[&str] = &[
    // RCODE-argument macros: recovery leaves parse_Rd in R-code mode.
    "code",
    "donttest",
    "dontshow",
    "testonly",
    // VERB-argument macros: recovery leaves parse_Rd in verbatim mode.
    "verb",
    "url",
    "samp",
    "env",
    "kbd",
    "option",
    "out",
    "eqn",
    "deqn",
    "href",
    "figure",
    "preformatted",
    "dontrun",
    "newcommand",
    "renewcommand",
    // Out-of-list `\item` becomes an `(UNKNOWN …)` node, not a drop.
    "item",
];

/// Whether a brace-less `\name` in prose is dropped by parse_Rd's recovery with
/// the surrounding text continuing as plain `TEXT` (the "unexpected TEXT/section
/// header, expecting `{`" recovery): a known, non-zero-arg macro outside
/// [`STICKY_BRACELESS_RD_MACROS`]. Zero-arg names are complete calls (carved by
/// the lexer), unknown names are `(UNKNOWN …)` nodes (also carved), and the
/// sticky set leaves parse_Rd in a code/verbatim mode. Of that set, brace-less
/// `\item` is projected as an `(UNKNOWN "\item")` node (`split_braceless_items` in
/// the projector); the code/verbatim mode-flip names (`\code`/`\verb`/…) are still
/// left literal (backlog). Mode-independent: the `@md` pipeline is a net no-op on a
/// backslash run before a letter, so parse_Rd sees the same brace-less macro either
/// way.
pub fn is_rd_braceless_drop_macro(name: &str) -> bool {
    is_known_rd_macro(name)
        && !is_zero_arg_rd_macro(name)
        && !STICKY_BRACELESS_RD_MACROS.contains(&name)
}

/// For a brace-less sticky Rd macro name (`\code`/`\verb`/…, see
/// [`STICKY_BRACELESS_RD_MACROS`]), which parse_Rd lexer state its "expecting `{`"
/// recovery leaves in place: `Some(true)` for the **R-code** macros (the recovery
/// stays in R-code mode → per-line `RCODE` atoms) and `Some(false)` for the
/// **verbatim** macros (verbatim mode → per-line `VERB` atoms). `None` for a
/// non-sticky name and for `item`, whose out-of-list misuse is instead an
/// `(UNKNOWN "\item")` node (handled by the projector's `split_braceless_items`).
/// The two sets are exactly [`STICKY_BRACELESS_RD_MACROS`] minus `item`, probed
/// against R 4.5 (see that list's doc).
pub fn sticky_braceless_code_mode(name: &str) -> Option<bool> {
    match name {
        "code" | "donttest" | "dontshow" | "testonly" => Some(true),
        "verb" | "url" | "samp" | "env" | "kbd" | "option" | "out" | "eqn" | "deqn" | "href"
        | "figure" | "preformatted" | "dontrun" | "newcommand" | "renewcommand" => Some(false),
        _ => None,
    }
}

/// Rd macros whose `{…}` content roxygen2 **protects** from the markdown parser
/// (`escaped_for_md` in roxygen2's `R/markdown-escaping.R`): under `@md`,
/// `escape_rd_for_md` swaps the whole `\tag{…}` span out for a placeholder before
/// running cmark, so markdown inside such a macro stays literal Rd
/// (`\code{*x*}` → `\code{*x*}`, not `\code{\emph{x}}`). Every *other* macro keeps
/// only its backslash-word as literal text while its argument **is** markdown-
/// processed (`\emph{*x*}` → `\emph{\emph{x}}`), so the projector resolves the arg
/// of a non-fragile, known, single-argument macro as a markdown inline run.
const FRAGILE_FOR_MD_RD_MACROS: &[&str] = &[
    "acronym",
    "code",
    "command",
    "CRANpkg",
    "deqn",
    "doi",
    "dontrun",
    "dontshow",
    "donttest",
    "email",
    "env",
    "eqn",
    "figure",
    "file",
    "if",
    "ifelse",
    "kbd",
    "link",
    "linkS4class",
    "method",
    "mjeqn",
    "mjdeqn",
    "mjseqn",
    "mjsdeqn",
    "mjteqn",
    "mjtdeqn",
    "newcommand",
    "option",
    "out",
    "packageAuthor",
    "packageDescription",
    "packageDESCRIPTION",
    "packageIndices",
    "packageMaintainer",
    "packageTitle",
    "pkg",
    "PR",
    "preformatted",
    "renewcommand",
    "S3method",
    "S4method",
    "samp",
    "special",
    "testonly",
    "url",
    "var",
    "verb",
];

/// Whether the macro named `name` (without the leading `\`) has its `{…}` content
/// **protected** from markdown under `@md` (roxygen2's `escaped_for_md`). A fragile
/// macro keeps its argument literal; a non-fragile one has it markdown-processed.
/// See [`FRAGILE_FOR_MD_RD_MACROS`].
pub fn is_fragile_for_md(name: &str) -> bool {
    FRAGILE_FOR_MD_RD_MACROS.contains(&name)
}

/// Resolve a bare prose `content` string as a `@md` markdown **inline run** and
/// return the resulting `ROXYGEN_PARAGRAPH` node, whose children are the resolved
/// inline elements (text, emphasis/strong nodes, links, code spans, nested Rd
/// macros). Drives the projector's translation of a non-fragile Rd macro's argument
/// under `@md` (`\emph{*x*}` → `\emph{\emph{x}}`): the projector slices out the raw
/// argument text and feeds it here, reusing the **real** inline pass (the
/// delimiter-stack arena) rather than a second markdown scanner — so nesting,
/// links, and code spans resolve exactly as in ordinary `@md` prose. A nested
/// fragile macro stays an opaque `ROXYGEN_RD_MACRO` token here; the projector keeps
/// its argument literal by recursing with the same fragility check.
pub fn resolve_md_inline(content: &str) -> crate::syntax::SyntaxNode {
    let mut tokens = Vec::new();
    lex::lex_roxygen_prose_fragment(&mut tokens, content, true);
    resolve_md_inline_tokens(tokens, true)
}

/// Non-markdown twin of [`resolve_md_inline`]: lex `content` as a plain **Rd**
/// prose fragment (no markdown recognizers; `\name{…}` macros still carve, gated
/// by the backslash-pairing parity) and return the `ROXYGEN_PARAGRAPH` node.
/// Drives the projector's re-parse of a *demoted* markdown span's rendered body,
/// which `tools::parse_Rd` reads as ordinary Rd text once the generating macro's
/// backslash has been paired away.
pub fn resolve_rd_inline(content: &str) -> crate::syntax::SyntaxNode {
    let mut tokens = Vec::new();
    lex::lex_roxygen_prose_fragment(&mut tokens, content, false);
    resolve_md_inline_tokens(tokens, false)
}

/// One piece of a **structural** Rd-macro argument fed to
/// [`resolve_md_inline_pieces`]: either raw prose `Text` (markdown-lexed) or a
/// pre-parsed nested `Macro` (its raw `\name{…}`/`\name` source, kept opaque).
pub enum MdArgPiece {
    /// Raw prose text, lexed as a markdown inline fragment.
    Text(String),
    /// A nested Rd macro's raw source (`\strong{y}`, `\tab`, `\cr`, …), emitted as
    /// one opaque `RoxygenRdMacro` token so emphasis/links span across it.
    Macro(String),
}

/// Resolve a **structural** Rd-macro argument (`\item`/`\tabular`/`\href` under
/// `@md`) as a single markdown inline run from its already-carved `pieces`,
/// returning the resolved `ROXYGEN_PARAGRAPH` node (same shape as
/// [`resolve_md_inline`]).
///
/// roxygen2 markdown-processes a structural argument as **one** cmark run: a nested
/// Rd macro is opaque text to cmark (reconstituted afterward), so an emphasis or
/// link span crosses it. Re-lexing the raw argument string cannot reproduce this
/// faithfully — the prose fragment lexer leaves a *brace-less* known macro
/// (`\tab`/`\cr`, the table separators) literal. Instead each pre-parsed macro
/// child (carved by the block-macro grouper) is emitted as one opaque
/// `RoxygenRdMacro` token, which [`build_rd_macro`](crate::parser::tree_builder)
/// re-expands into a faithful node (a brace-less `\tab` → a name-only `\tab` node).
/// The prose pieces between them lex as ordinary markdown fragments, so the
/// delimiter-stack arena spans emphasis across the macros exactly as cmark does.
pub fn resolve_md_inline_pieces(pieces: &[MdArgPiece]) -> crate::syntax::SyntaxNode {
    use crate::parser::lexer::{TokKind, Token};

    let mut tokens = Vec::new();
    for piece in pieces {
        match piece {
            MdArgPiece::Text(t) => lex::lex_roxygen_prose_fragment(&mut tokens, t, true),
            // Offsets are unused by the inline pass and the tree builder (which key
            // off `kind`/`text` only), so a synthetic macro token needs no real span.
            MdArgPiece::Macro(m) => tokens.push(Token {
                kind: TokKind::RoxygenRdMacro,
                text: m.clone(),
                start: 0,
                end: 0,
            }),
        }
    }
    resolve_md_inline_tokens(tokens, true)
}

/// Wrap `tokens` in a paragraph, run the emphasis/inline pass, and return the
/// resolved `ROXYGEN_PARAGRAPH` node. Shared by [`resolve_md_inline`],
/// [`resolve_rd_inline`], and [`resolve_md_inline_pieces`]. `md` is the mode the
/// tokens were lexed under; a non-md fragment carries no markdown delimiters, so
/// the inline pass only threads its tokens through.
fn resolve_md_inline_tokens(
    tokens: Vec<crate::parser::lexer::Token>,
    md: bool,
) -> crate::syntax::SyntaxNode {
    use crate::parser::events::Event;
    use crate::syntax::SyntaxKind;

    let mut events = Vec::with_capacity(tokens.len() + 2);
    events.push(Event::Start(SyntaxKind::ROXYGEN_PARAGRAPH));
    events.extend((0..tokens.len()).map(Event::Tok));
    events.push(Event::Finish);
    // Fragments carry no soft breaks, so the multi-line HTML resolution is
    // inert here.
    inline::resolve_emphasis(&tokens, &mut events, md);
    let root = crate::parser::tree_builder::build_tree(&tokens, &events);
    root.first_child()
        .expect("build_tree always wraps the paragraph in ROOT")
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
