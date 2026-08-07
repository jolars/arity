//! CST → Rd-tree projector: the primary roxygen2 conformance engine.
//!
//! [`project_to_rd`] walks arity's lossless CST and emits the **parser-owned**
//! Rd section subtrees in roxygen2's canonical S-expression shape --- the same
//! shape the R driver's `block-to-sections` op mints (`tests/oracle/
//! roxygen_oracle.R`). The projector-parity gate (`tests/roxygen_projector.rs`)
//! diffs this against a *pinned* `expected.rdtree` per corpus case, so it runs in
//! plain `cargo test` with no R, and **structural** divergences (a `\describe`
//! the CST never modeled as a block, a markdown list still flat prose) surface as
//! a mismatch. That is the signal that drives parser growth.
//!
//! ## What it projects, and what it deliberately does not
//!
//! It is a **faithful encoding translation**, never a roxygen2 roclet
//! reimplementation (RECAP's first invariant). It projects what the parser
//! models: the title/description derived from the intro paragraphs, and the
//! body of the prose section tags (`@details`, `@return` → `\value`,
//! `@seealso`, `@source`, `@format`, `@section`, …). It excludes everything
//! roxygen2 *generates* rather than parses --- `\name`/`\alias` (the object),
//! `\usage` (the formals), and the `\arguments` wrapper that groups `@param`
//! (the `block-to-sections` op drops the same set, so the two stay aligned).
//!
//! ## Current reach
//!
//! A section body is projected as a *sequence* of inline atoms: prose runs
//! coalesce into whitespace-normalized `(TEXT …)`, and inline Rd macros
//! (`\code`/`\link`/`\emph`/`\url`/…, including nesting, a dropped `[pkg]`
//! option, and verbatim `(VERB …)` bodies) surface as nested subtrees from the
//! CST's `ROXYGEN_RD_MACRO` nodes. A section the CST does not yet model
//! structurally --- a multi-line `\describe`/`\itemize`/`\tabular`, or markdown
//! that roxygen2 translates into nodes under a resolved `@md` mode (`*x*` →
//! `\emph{x}`) --- still projects as flat text and therefore **diverges**. Those
//! divergences are the backlog: each is closed by teaching the *parser* the
//! structure, then the projector grows a faithful arm for the new nodes. Never
//! patch the projector to make a case pass.

use std::borrow::Cow;

use rowan::NodeOrToken;

use crate::ast::{AstNode, RoxygenBlock, RoxygenParagraph, RoxygenSection, RoxygenTag};
use crate::parser::parse;
use crate::parser::roxygen::{
    MdArgPiece, advance_md_col, is_fragile_for_md, is_known_rd_macro, is_multi_arg_rd_macro,
    is_rd_braceless_drop_macro, is_verbatim_rd_macro, md_fence_run_closes, md_ws_gauge,
    rd_macro_arity, resolve_md_inline, resolve_md_inline_pieces, resolve_rd_inline,
    split_table_row_cells, sticky_braceless_code_mode,
};
use crate::roxygen::entities;
use crate::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

mod collect;
mod escapes;
mod linkrefs;
mod md_blocks;
mod md_links;
mod recovery;
mod section;
mod serialize;
mod sexpr;
#[cfg(test)]
mod tests;
mod text;
mod usermacro;

use self::collect::*;
use self::escapes::*;
use self::linkrefs::*;
use self::md_blocks::*;
use self::md_links::*;
use self::recovery::*;
use self::section::*;
use self::serialize::*;
use self::sexpr::*;
use self::text::*;
use self::usermacro::*;

/// Project `text` to the parser-owned Rd section subtrees, one canonical
/// S-expression per line, sorted --- byte-identical to the R driver's
/// `block-to-sections` output for the cases the projector models.
///
/// Sections are sorted (not in document order) because roxygen2's Rd emission
/// order is not the document order, and the projector does not replicate it; the
/// gate compares a *set* of section subtrees. Sections from every
/// `ROXYGEN_BLOCK` in `text` are merged into one sorted set.
///
/// Blocks that resolve to the **same topic** (`@name`/`@rdname` value) are one
/// Rd file in roxygen2, so their sections *merge* rather than sit side by side
/// (`RoxyTopic$add`): the merge combines each section type's value vector, and
/// the per-type `format` method then decides how those values render — `\title`
/// keeps only the first (`format_first`), while `\description`/`\details`/… join
/// with a paragraph break (`format_collapse`). See [`project_merged_topic`].
/// Blocks with no explicit topic name form their own singleton topic (arity does
/// not statically resolve an object's default topic, and distinct objects get
/// distinct files anyway), so the common single-block path is untouched.
pub fn project_to_rd(text: &str) -> String {
    let cst = parse(text).cst;
    let scan_end = roxygen_scan_end(&cst);
    // Group blocks by topic name, preserving first-seen document order. A named
    // topic accumulates every block that shares its name; an unnamed block is its
    // own group (never merged).
    let mut groups: Vec<Vec<RoxygenBlock>> = Vec::new();
    let mut named: Vec<(String, usize)> = Vec::new();
    for block in cst.descendants().filter_map(RoxygenBlock::cast) {
        if block.syntax().text_range().start() >= scan_end {
            continue;
        }
        match topic_name(&block) {
            Some(name) => match named.iter().find(|(n, _)| *n == name) {
                Some(&(_, idx)) => groups[idx].push(block),
                None => {
                    named.push((name, groups.len()));
                    groups.push(vec![block]);
                }
            },
            None => groups.push(vec![block]),
        }
    }

    let mut sections: Vec<String> = Vec::new();
    for group in &groups {
        match group.as_slice() {
            [block] => project_block(block, &mut sections),
            blocks => project_merged_topic(blocks, &mut sections),
        }
    }
    sections.sort();
    sections.join("\n")
}

/// End of roxygen2's comment-scan range. The regions `comments()` (tokenize.R)
/// builds tile `[byte 1, end of last top-level expression]` contiguously, so a
/// `#'` line starting past that end — after the last expression, or anywhere in
/// a file with no expression at all — is never tokenized. A block *inside* an
/// expression (a `#'` line in a function body) starts within the enclosing
/// expression's region, joins that region's block, and still renders. Bare
/// atoms (`NULL`, a lone literal) are tokens at top level, so both nodes and
/// non-structural tokens count; trivia, comments, semicolons, and the roxygen
/// blocks themselves do not.
fn roxygen_scan_end(root: &SyntaxNode) -> rowan::TextSize {
    root.children_with_tokens()
        .filter(|el| {
            !matches!(
                el.kind(),
                SyntaxKind::WHITESPACE
                    | SyntaxKind::NEWLINE
                    | SyntaxKind::COMMENT
                    | SyntaxKind::SEMICOLON
                    | SyntaxKind::ROXYGEN_BLOCK
            )
        })
        .last()
        .map(|el| el.text_range().end())
        .unwrap_or_default()
}

/// The topic key a block documents: the trimmed value of its `@name` (or, absent
/// that, `@rdname`) tag. Blocks sharing a key render into one Rd file and so merge
/// (`project_to_rd`). Returns `None` when the block names no topic — arity does
/// not statically derive an object's default topic, so such a block never merges.
fn topic_name(block: &RoxygenBlock) -> Option<String> {
    let mut rdname: Option<String> = None;
    for section in block.sections() {
        let Some(tag) = section.tag() else { continue };
        match tag.name().as_deref() {
            Some("name") => {
                let value = tag.text().map(|t| t.text().trim().to_string());
                if let Some(v) = value.filter(|v| !v.is_empty()) {
                    return Some(v);
                }
            }
            Some("rdname") if rdname.is_none() => {
                rdname = tag
                    .text()
                    .map(|t| t.text().trim().to_string())
                    .filter(|v| !v.is_empty());
            }
            _ => {}
        }
    }
    rdname
}

/// One inline element of a section body: a run of prose text (coalesced and
/// whitespace-normalized at serialization) or an Rd macro node (projected as a
/// nested subtree). Modeling the body as a *sequence* — rather than one flat
/// string — is what lets inline `\code`/`\link`/… surface as structure.
#[derive(Clone)]
enum Inline {
    Text(String),
    Macro(SyntaxNode),
    /// The `USERMACRO` leaf parse_Rd emits ahead of a **system Rd macro's**
    /// expansion (`\doi`, `\CRANpkg`, …), carrying the macro's raw definition
    /// body followed by its argument text. The expansion itself follows as
    /// ordinary inlines, so a definition that expands to plain text (`\I`)
    /// coalesces with the surrounding prose. See [`usermacro`].
    UserMacro(String),
    /// A markdown inline leaf resolved under `@md` mode — emphasis, strong, or a
    /// code span — carrying its delimiter-stripped inner content. Emphasis/strong
    /// project to `\emph`/`\strong` over `(TEXT …)`; a code span projects to
    /// `\code` or `\verb` per roxygen2's R-parseability rule (see [`md_code_atom`]).
    MdCode(String),
    /// A resolved markdown emphasis (`strong = false`) or strong (`strong = true`)
    /// **node** (`ROXYGEN_MD_EMPH`/`ROXYGEN_MD_STRONG`), carrying its inner inline
    /// run (the delimiter leaves stripped). Projects to `\emph`/`\strong` over the
    /// recursively-serialized children — so nesting (`*foo **bar***`) surfaces as
    /// structure, not flattened text.
    MdEmphasis {
        strong: bool,
        children: Vec<Inline>,
    },
    /// A markdown block list resolved under `@md` mode (a `ROXYGEN_MD_LIST` node).
    /// Projects to `\itemize`/`\enumerate` with a name-only `\item` per item ahead
    /// of its content (see [`serialize_md_list`]).
    MdList(SyntaxNode),
    /// A markdown block list whose item contents have been rewritten by the
    /// whole-field link-reference pipeline ([`apply_user_linkrefs`]) — a user
    /// `[ref]: url` definition resolves a referencing link inside a list item, or a
    /// definition that *sits* in a list item is consumed (leaving the item empty).
    /// Carries each item's resolved inline run instead of the opaque node, so it
    /// projects from those (see [`serialize_md_list_resolved`]); produced only when
    /// some item actually changed, so a list with no link-reference work keeps its
    /// `MdList(node)` form and its byte-identical serialization.
    MdListResolved {
        ordered: bool,
        items: Vec<Vec<Inline>>,
    },
    /// A markdown link resolved under `@md` mode, carrying the raw leaf text. The
    /// inline `[text](url)` form projects to `\href{url}{text}`; the reference
    /// (`[text][ref]`) and shortcut (`[dest]`) forms resolve to an `\link`/
    /// `\linkS4class` (optionally `\code`-wrapped) per roxygen2's `parse_link`
    /// (see [`resolve_md_link`]). This is the opaque-leaf form (a `<url>` autolink,
    /// a reference/shortcut link, or a bracketed-text inline link).
    MdLink(String),
    /// An inline `[text](url)` link resolved into a `ROXYGEN_MD_LINK` **node** by
    /// the inline pass: `url` is the destination and `display` the recursively
    /// resolved link text (so emphasis/code spans inside it surface as structure).
    /// Projects to `\href{url}{display}` (the display GRP-wrapped when it is more
    /// than one atom, since `\href` is a two-argument structural macro), or to
    /// `\url{text}` when the destination is empty or equals the text.
    MdInlineLink {
        url: String,
        display: Vec<Inline>,
    },
    /// A reference link `[text][ref]` resolved into a `ROXYGEN_MD_LINK` **node** by
    /// the inline pass (its closer leaf is `][ref]`): `display` is the recursively
    /// resolved link text and `dest` the reference label. roxygen2's section
    /// serializer drops the `[ref]` topic option, so this projects to `\link{display}`
    /// (`\code`-wrapped when the display is a single code span), falling back to the
    /// shortcut path when the display text equals the label (see [`ref_link_node_atom`]).
    MdRefLink {
        dest: String,
        display: Vec<Inline>,
    },
    /// A shortcut link `[text]` resolved into a `ROXYGEN_MD_LINK` **node** by the
    /// inline pass (its closer leaf is a bare `]`): the display text *is* the link
    /// destination. Projects to `\link`/`\linkS4class` per the destination shape
    /// (`-class`, `pkg::`, `()`), `\code`-wrapped for a code-span display — the node
    /// analog of the opaque shortcut leaf (see [`shortcut_link_node_atom`]).
    MdShortcutLink {
        display: Vec<Inline>,
    },
    /// A markdown image resolved under `@md` mode, carrying the raw leaf text
    /// `![alt](url "title")`. Projects to `\figure{url}{title}` — wrapped in
    /// `\if{html}{…}`/`\if{pdf}{…}` per roxygen2's extension-keyed image-format
    /// rule (see [`resolve_md_image`]).
    MdImage(String),
    /// A markdown fenced code block resolved under `@md` mode (a
    /// `ROXYGEN_MD_CODE_BLOCK` node). Projects to roxygen2's three-atom
    /// `\if{html}{\out{<div…>}}` / `\preformatted{…}` / `\if{html}{\out{</div>}}`
    /// sequence (see [`serialize_md_code_block`]).
    MdCodeBlock(SyntaxNode),
    /// A markdown indented code block resolved under `@md` mode (a
    /// `ROXYGEN_MD_INDENTED_CODE` node). Projects to the same three-atom
    /// `\if{html}{\out{<div…>}}` / `\preformatted{…}` / `\if{html}{\out{</div>}}`
    /// sequence as a fenced code block, but with a bare `sourceCode` class and each
    /// line's leading indentation stripped (see [`serialize_md_indented_code`]).
    MdIndentedCode(SyntaxNode),
    /// A raw inline-HTML tag resolved under `@md` mode, carrying the verbatim tag
    /// text (`<img …>`, `</span>`). Projects to roxygen2's
    /// `\if{html}{\out{<tag>}}` (`mdxml_html_inline`; see [`html_inline_atom`]).
    MdHtml(String),
    /// A markdown HTML block resolved under `@md` mode (a `ROXYGEN_MD_HTML_BLOCK`
    /// node). Projects to roxygen2's `\if{html}{\out{…}}` with one verbatim line
    /// per `VERB` (`mdxml_html_block`; see [`serialize_md_html_block`]).
    MdHtmlBlock(SyntaxNode),
    /// A GFM table resolved under `@md` mode (a `ROXYGEN_MD_TABLE` node). Projects
    /// to roxygen2's `\tabular{<align>}{<cells>}`: the delimiter row gives the
    /// per-column alignment (`l`/`c`/`r`), and the header and body rows fill one
    /// `GRP` with each cell's markdown-resolved content, `\tab` between cells and
    /// `\cr` ending each row (see [`serialize_md_table`]).
    MdTable(SyntaxNode),
    /// A markdown ATX heading resolved under `@md` mode (a `ROXYGEN_MD_HEADING`
    /// node). Unlike the other block inlines it does not serialize in place: it is
    /// a **structural marker** that splits an `@description`/`@details` body into
    /// roxygen2's `\section` (level 1) / `\subsection` (level >= 2) outline, hoisting
    /// level-1 headings to top-level Rd sections (see [`emit_section_with_headings`]).
    /// The projected title (level and text) is read from the node with
    /// [`parse_md_heading`].
    MdHeading(SyntaxNode),
    /// A markdown block quote resolved under `@md` mode (a `ROXYGEN_MD_BLOCK_QUOTE`
    /// node). roxygen2 does not support block quotes: it warns and renders the node's
    /// *flattened plain text* (`escape_comment(xml_text)` — the `>` markers and inner
    /// markdown dropped, descendant text concatenated with no separator). The
    /// Its flattened text glues onto adjacent prose with no paragraph separator
    /// (see [`block_quote_flat_text`]), pushed as a `Final` run segment.
    MdBlockQuote(SyntaxNode),
    /// A bare `{…}` brace group in prose — *not* a macro's argument delimiters,
    /// which live inside the macro's CST node. parse_Rd models an unescaped brace
    /// pair in prose text as a `LIST` node over its parsed contents, so `a {b c} d`
    /// projects `(TEXT "a") (LIST (TEXT "b c")) (TEXT "d")`. Groups nest, span
    /// macros and soft breaks, and carry the recursively-grouped inner run;
    /// projects to `(LIST <children…>)` (empty group → `(LIST)`). Produced only by
    /// [`group_brace_lists`] on a **balanced** prose run (both markdown modes).
    BraceGroup(Vec<Inline>),
    /// A brace-less `\item` in prose text, projecting to `(UNKNOWN "\item")`.
    /// `\item` is the one [`STICKY_BRACELESS_RD_MACROS`](crate::parser::roxygen)
    /// name whose brace-less misuse is neither a clean text drop (the other known
    /// brace-required macros) nor a code/verbatim-mode swallow (`\code`/`\verb`/…):
    /// out of list context parse_Rd tags it an unknown-macro node and the
    /// surrounding prose continues (`a \item b` → `(TEXT "a") (UNKNOWN "\item")
    /// (TEXT "b")`). Produced only by [`split_braceless_items`] on a prose run
    /// (both markdown modes; the `@md` pipeline is a net no-op on a backslash run
    /// before a letter, so parse_Rd sees the same brace-less `\item` either way).
    BracelessItem,
    /// The swallowed tail of a brace-less sticky code/verbatim Rd macro
    /// (`\code`/`\verb`/…, see [`sticky_braceless_code_mode`]). Out of an argument
    /// context parse_Rd's "expecting `{`" recovery leaves its lexer in R-code
    /// (`code = true`) or verbatim (`code = false`) mode, so everything from the
    /// dropped `\name` to section end becomes one atom **per physical source
    /// line**: `\code z here` (line-wrapped) → `(RCODE " z here\n") (RCODE
    /// "continued\n")`. `lines` holds each physical line's verbatim content (no
    /// trailing `\n`); it projects to `(RCODE …)`/`(VERB …)`, one per line.
    /// Produced only by [`split_sticky_braceless_swallow`] on an explicit prose
    /// tag's body whose tail is a single-paragraph plain-text run (no
    /// macro/list/emphasis, and no `\ { } %` or paragraph break — those cases are
    /// withheld).
    StickyVerbatim {
        code: bool,
        lines: Vec<String>,
    },
}
