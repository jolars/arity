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
mod lex;

pub(crate) use group::emit_roxygen_block;
pub(crate) use lex::{is_roxygen_comment, lex_roxygen_line, resolve_roxygen_block, scan_rd_macro};

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
    is_verbatim_rd_macro(name) || (name == "href" && index == 0)
}

/// Inline Rd macros that take **two** adjacent `{…}` argument groups, the way
/// `tools::parse_Rd` does: `\item{term}{description}` (in `\describe`/`\value`/
/// `\arguments`) and `\tabular{format}{content}`. A one-argument macro like
/// `\code` consumes only its first group, so a trailing `\code{x}{y}`'s `{y}`
/// stays literal --- the arity is per macro. Also `\href{url}{text}`, whose first
/// argument is verbatim (see [`is_verbatim_rd_arg`]). Extensible (`\section`/… are
/// future targets, several of which surface as block macros instead). A braceless
/// `\item` (under `\itemize`/`\enumerate`) never reaches here: it has no `{`, so
/// it is not a macro token at all.
///
/// These are also the macros whose `{…}` arguments `parse_Rd` models as *list*
/// wrappers (so a multi-atom argument projects to a `(GRP …)`), as opposed to
/// latexlike macros (`\code`, `\emph`, …) whose single argument's content is
/// inlined directly. The projector keys its GRP rule on this set.
const TWO_ARG_RD_MACROS: &[&str] = &["item", "tabular", "href"];

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

/// Length in bytes of the UTF-8 char whose leading byte is `b`.
fn utf8_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}
