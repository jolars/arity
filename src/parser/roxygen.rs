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
pub(crate) use lex::{
    is_raw_rd_tag, is_roxygen_comment, lex_roxygen_line, resolve_roxygen_block, roxygen_line_tag,
    scan_rd_macro,
};

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

/// Inline Rd macros that take **two** adjacent `{…}` argument groups, the way
/// `tools::parse_Rd` does: `\item{term}{description}` (in `\describe`/`\value`/
/// `\arguments`) and `\tabular{format}{content}`. A one-argument macro like
/// `\code` consumes only its first group, so a trailing `\code{x}{y}`'s `{y}`
/// stays literal --- the arity is per macro. Also `\href{url}{text}`, whose first
/// argument is verbatim, and `\figure{path}{caption}` (both args verbatim --- see
/// [`is_verbatim_rd_arg`]). Extensible (`\section`/… are
/// future targets, several of which surface as block macros instead). A braceless
/// `\item` (under `\itemize`/`\enumerate`) never reaches here: it has no `{`, so
/// it is not a macro token at all.
///
/// These are also the macros whose `{…}` arguments `parse_Rd` models as *list*
/// wrappers (so a multi-atom argument projects to a `(GRP …)`), as opposed to
/// latexlike macros (`\code`, `\emph`, …) whose single argument's content is
/// inlined directly. The projector keys its GRP rule on this set.
const TWO_ARG_RD_MACROS: &[&str] = &["item", "tabular", "href", "figure"];

/// Whether the macro named `name` (without the leading `\`) takes two `{…}`
/// argument groups. Drives the lexer (consume the second group into one token),
/// the tree builder (emit both groups as children), and the projector (each
/// group is a list argument --- a multi-atom one becomes a `(GRP …)`).
pub(crate) fn is_two_arg_rd_macro(name: &str) -> bool {
    TWO_ARG_RD_MACROS.contains(&name)
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

/// The end index of an Rd macro name starting at `bytes[start]` (the byte *after*
/// the leading `\`). An Rd command name is `[A-Za-z][A-Za-z0-9]*`: a leading
/// letter then any letters or digits (e.g. `\linkS4class`). Returns `start` when
/// no valid name begins there (`\\`, `\{`, `\4`, end of input). The single source
/// of truth for where a `\name` ends, shared by the lexer and the tree builder.
pub(crate) fn rd_macro_name_end(bytes: &[u8], start: usize) -> usize {
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
pub(crate) fn is_known_rd_macro(name: &str) -> bool {
    KNOWN_RD_MACROS.contains(&name)
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
pub(crate) fn is_fragile_for_md(name: &str) -> bool {
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
pub(crate) fn resolve_md_inline(content: &str) -> crate::syntax::SyntaxNode {
    use crate::parser::events::Event;
    use crate::syntax::SyntaxKind;

    let mut tokens = Vec::new();
    lex::lex_roxygen_prose_fragment(&mut tokens, content, true);
    let mut events = Vec::with_capacity(tokens.len() + 2);
    events.push(Event::Start(SyntaxKind::ROXYGEN_PARAGRAPH));
    events.extend((0..tokens.len()).map(Event::Tok));
    events.push(Event::Finish);
    inline::resolve_emphasis(&tokens, &mut events);
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
