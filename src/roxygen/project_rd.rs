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
    MdArgPiece, is_fragile_for_md, is_known_rd_macro, is_rd_braceless_drop_macro,
    is_two_arg_rd_macro, resolve_md_inline, resolve_md_inline_pieces, split_table_row_cells,
    sticky_braceless_code_mode,
};
use crate::roxygen::entities;
use crate::syntax::{SyntaxKind, SyntaxNode};

/// Project `text` to the parser-owned Rd section subtrees, one canonical
/// S-expression per line, sorted --- byte-identical to the R driver's
/// `block-to-sections` output for the cases the projector models.
///
/// Sections are sorted (not in document order) because roxygen2's Rd emission
/// order is not the document order, and the projector does not replicate it; the
/// gate compares a *set* of section subtrees. Sections from every
/// `ROXYGEN_BLOCK` in `text` are merged into one sorted set.
pub fn project_to_rd(text: &str) -> String {
    let cst = parse(text).cst;
    let mut sections: Vec<String> = Vec::new();
    for block in cst.descendants().filter_map(RoxygenBlock::cast) {
        project_block(&block, &mut sections);
    }
    sections.sort();
    sections.join("\n")
}

/// One inline element of a section body: a run of prose text (coalesced and
/// whitespace-normalized at serialization) or an Rd macro node (projected as a
/// nested subtree). Modeling the body as a *sequence* — rather than one flat
/// string — is what lets inline `\code`/`\link`/… surface as structure.
#[derive(Clone)]
enum Inline {
    Text(String),
    Macro(SyntaxNode),
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

/// One topic's worth of sections from a single roxygen block.
///
/// The block already owns logical structure: its children are `ROXYGEN_SECTION`s
/// (the intro, then one per `@tag`), each holding a `ROXYGEN_TAG` heading and/or
/// `ROXYGEN_PARAGRAPH`s. So the projector is a direct walk — the line-reassembly
/// state machine the line-flat CST forced is gone. A tag section's body is the
/// tag's own inline prose followed by its paragraphs (continuation and
/// paragraph-break both collapse to a single space under `norm_ws`).
/// The block's resolved markdown mode, mirroring the lexer's
/// `resolve_roxygen_block`: a standalone `@md` directive turns it on, `@noMd` off,
/// the last one in the block winning; off by default (Rd-first). A directive is a
/// tag named `md`/`noMd` with no argument or prose value (roxygen2 errors on a
/// directive line carrying other content).
fn block_md(block: &RoxygenBlock) -> bool {
    let mut md = false;
    for section in block.sections() {
        if let Some(tag) = section.tag()
            && tag.arg().is_none()
            && tag.text().is_none()
        {
            match tag.name().as_deref() {
                Some("md") => md = true,
                Some("noMd") => md = false,
                _ => {}
            }
        }
    }
    md
}

fn project_block(block: &RoxygenBlock, out: &mut Vec<String>) {
    // Resolve the block's markdown mode the way the lexer's `resolve_roxygen_block`
    // does (a standalone `@md`/`@noMd` directive line, last one wins, default off).
    // Plain prose text leaves carry no mode (their kind is identical in both modes),
    // so the projector re-derives it here: it keys whether prose is literal Rd
    // (where an unescaped `%` is a comment) or escaped markdown (where it survives).
    let md = block_md(block);
    // The section strings this block contributes; a `@md` block's `TEXT` leaves get
    // their escaped braces (`\{`/`\}`) resolved to bare braces once every section
    // (and its `rdComplete` drop decision) is built (see `resolve_md_text_braces`).
    let block_start = out.len();
    let mut intro_paras: Vec<Vec<Inline>> = Vec::new();
    let mut tag_sections: Vec<(String, Vec<Inline>)> = Vec::new();
    // `@slot` (S4) and `@field` (reference class) each aggregate every tag of a
    // topic into one Slots/Fields section, so they are collected here as
    // (name, definition) pairs rather than projected per-tag.
    let mut slots: Vec<(String, Vec<Inline>)> = Vec::new();
    let mut fields: Vec<(String, Vec<Inline>)> = Vec::new();
    // `@examples`/`@examplesIf` is an aggregating field: every examples tag of a
    // topic concatenates into a single `\examples` section. The body is
    // reformatted R, so the projector only records *that* one exists.
    let mut has_examples = false;

    for section in block.sections() {
        if let Some(tag) = section.tag() {
            let name = tag.name().map(|n| n.to_string()).unwrap_or_default();
            let mut body = tag_inlines(&tag);
            for part in section_body_parts(&section) {
                // A part that leads with a block quote carries no separator: its
                // flattened text glues onto whatever precedes (roxygen2 emits no
                // paragraph break around an unsupported block quote). Any other part
                // is a fresh roxygen paragraph, joined by a line break (norm_ws
                // collapses it to a space, but it bounds an Rd `%` comment in
                // non-markdown prose).
                if !body.is_empty() && !matches!(part.first(), Some(Inline::MdBlockQuote(_))) {
                    body.push(Inline::Text("\n".to_string()));
                }
                body.extend(part);
            }
            match name.as_str() {
                "slot" | "field" => {
                    // roxygen2 parses `@slot`/`@field` with `tag_two_part`, which
                    // runs `rdComplete(x$raw, is_code = FALSE)` on the *raw* tag
                    // value (name + description, before markdown) and drops the
                    // whole tag to NULL on a brace imbalance — mode-independently,
                    // so a dropped tag contributes no `\describe` item (and an
                    // all-dropped Slots/Fields aggregate emits no section at all).
                    // The raw value's only rd_complete-relevant chars (`{}`, `\`,
                    // `%`) never appear in the `#'`/`@slot` scaffolding, and
                    // `is_code = FALSE` ignores quotes, so the section's full source
                    // text scans identically to roxygen2's `x$raw`.
                    if !rd_complete(&section.syntax().text().to_string()) {
                        continue;
                    }
                    let arg = tag.arg().map(|t| t.text().to_string()).unwrap_or_default();
                    if name == "slot" {
                        slots.push((arg, body));
                    } else {
                        fields.push((arg, body));
                    }
                }
                // `@section` uses `tag_markdown` (`sections = FALSE`), so it never
                // gets the per-section `rdComplete` drop under markdown. But in
                // markdown-OFF mode `markdown_if_active`'s else-branch runs
                // `rdComplete(x$raw)` unconditionally on the whole `title: body`
                // value and replaces it with "" on a brace imbalance. `roxy_tag_rd`
                // then splits "" on its first `:` → title="", content=NA, rendering
                // `\section{}{NA}` → `(\section (TEXT "NA"))`. As with `@slot`/
                // `@field`, the raw value's only rd_complete-relevant chars
                // (`{}`, `\`, `%`) never appear in the scaffolding and `is_code =
                // FALSE` ignores quotes, so the full section source scans
                // identically to `x$raw`.
                "section" if !md && !rd_complete(&section.syntax().text().to_string()) => {
                    out.push("(\\section (TEXT \"NA\"))".to_string());
                }
                "examples" | "examplesIf" => has_examples = true,
                _ => tag_sections.push((name, body)),
            }
        } else {
            intro_paras.extend(section_body_parts(&section));
        }
    }

    // roxygen2's `parse_description` (R/block.R) splits the intro prose by
    // paragraph: 1st = title, 2nd = description, the rest = details (merged with
    // any explicit @details). A tag whose value is the literal "NULL" is the
    // `rd_section()` suppression sentinel (`R/field.R`), so it does not count as
    // an explicit title/description — a suppressed `@description NULL` re-triggers
    // the title-as-description fallback (`topics_add_default_description`).
    let has_explicit_title = tag_sections
        .iter()
        .any(|(n, b)| n == "title" && !is_null_section(b, md));
    let has_explicit_desc = tag_sections
        .iter()
        .any(|(n, b)| n == "description" && !is_null_section(b, md));
    let explicit_title_body = tag_sections
        .iter()
        .find(|(n, b)| n == "title" && !is_null_section(b, md))
        .map(|(_, b)| b.clone());

    // 1st intro paragraph = title. An explicit @title claims the role and leaves
    // the intro paragraphs to shift down into description/details.
    let mut cursor = 0usize;
    let intro_title = if has_explicit_title {
        None
    } else {
        intro_paras.get(cursor).inspect(|_| cursor += 1).cloned()
    };
    // 2nd intro paragraph = description (unless an explicit @description claims it).
    let intro_desc = if has_explicit_desc {
        None
    } else {
        intro_paras.get(cursor).inspect(|_| cursor += 1).cloned()
    };
    // Everything remaining = details, merged with any explicit @details — but
    // roxygen2 only folds @details in when there *are* leftover intro paragraphs;
    // otherwise @details stands alone (emitted by the tag loop below).
    let intro_details = &intro_paras[cursor..];
    let merge_details = !intro_details.is_empty();

    if let Some(title) = &intro_title {
        // The intro title is re-emitted as a `tag_markdown` (`sections = FALSE`)
        // tag, which does not get the per-section `rdComplete` drop.
        push_section(out, "title", title, md, false);
    }

    // Description: the intro's 2nd paragraph, else roxygen2's
    // title-as-description fallback — when no description exists anywhere, the
    // title value (intro title, else explicit @title) is reused.
    let description = match intro_desc {
        Some(d) => Some(d),
        None if has_explicit_desc => None, // emitted by the tag loop below
        None => intro_title.clone().or_else(|| explicit_title_body.clone()),
    };
    if let Some(description) = description {
        emit_section_with_headings(out, "description", &description, md, true);
    }

    // The intro-derived details (and any folded-in @details).
    if merge_details {
        let mut body = join_paras(intro_details);
        for (_, ed) in tag_sections.iter().filter(|(n, _)| n == "details") {
            body.push(Inline::Text("\n".to_string()));
            body.extend(join_paras(std::slice::from_ref(ed)));
        }
        emit_section_with_headings(out, "details", &body, md, true);
    }

    for (name, body) in &tag_sections {
        // A folded-in @details was emitted above; skip the standalone section.
        if merge_details && name == "details" {
            continue;
        }
        project_tag_section(name, body, out, md);
    }

    // roxygen2's `topics_add_default_description` runs *after* the roclet has
    // processed every tag, so an explicit `@description` whose content hoisted
    // entirely into top-level `\section`s (a leading `# heading`, line-start or
    // as the tag's same-line value) leaves the topic without a `\description`
    // and the title fallback re-fires. A *dropped* description (`rdComplete`)
    // still emits an empty `(\description)` atom — the section object exists —
    // so it correctly suppresses the fallback here.
    if !out.iter().any(|s| s.starts_with("(\\description"))
        && let Some(title) = intro_title.as_ref().or(explicit_title_body.as_ref())
    {
        emit_section_with_headings(out, "description", title, md, true);
    }

    // The aggregated `@slot`/`@field` sections (roxygen2's Slots/Fields).
    if !slots.is_empty() {
        out.push(describe_section("Slots", &slots, md));
    }
    if !fields.is_empty() {
        out.push(describe_section("Fields", &fields, md));
    }

    // The single aggregated `\examples` section (body reformatted R → placeholder).
    if has_examples {
        out.push("(\\examples ...)".to_string());
    }

    // parse_Rd renders a `@md` prose escape `\{`/`\}` as a bare brace in the final
    // TEXT node. This runs after every section (and its `rdComplete` drop) is built
    // so the drop scan still weighs the escaped brace (an unbalanced *escaped* brace
    // does not drop the section), while the rendered TEXT carries the bare brace.
    if md {
        for section in &mut out[block_start..] {
            *section = resolve_md_text_braces(section);
        }
    }
}

/// Project the aggregated `@slot`/`@field` tags of a topic into a single
/// `\section{<title>}{\describe{\item{\code{name}}{def}…}}`. roxygen2 collects
/// every `@slot` (S4) and `@field` (reference class) into one Slots/Fields
/// section; each tag becomes a `\describe` item whose term is the verbatim
/// `\code{name}` (the name is R-code, tagged `RCODE` like a `\code` body) and
/// whose definition is the tag's prose.
fn describe_section(title: &str, items: &[(String, Vec<Inline>)], md: bool) -> String {
    let mut item_atoms: Vec<String> = Vec::new();
    for (name, def) in items {
        let code_atoms = rcode_atoms(name);
        let term = if code_atoms.is_empty() {
            "(\\code)".to_string()
        } else {
            format!("(\\code {})", code_atoms.join(" "))
        };
        // The definition is `\item`'s second (structural) argument: a multi-atom
        // prose+macro run is `(GRP …)`-wrapped, a single atom stays bare. It is a
        // `markdown_if_active` field (the description half of `tag_two_part`), so it
        // carries the `add_linkrefs_to_md` poisoning like any other prose section.
        let mut parts = vec![term];
        let def_arg = grp_arg(&serialize_prose_with_linkrefs(def, md));
        if !def_arg.is_empty() {
            parts.push(def_arg);
        }
        item_atoms.push(format!("(\\item {})", parts.join(" ")));
    }
    format!(
        "(\\section (TEXT {}) (\\describe {}))",
        encode_text(title),
        item_atoms.join(" ")
    )
}

/// Flatten paragraphs into a single inline run, with a space between each (the
/// canonical serializer collapses the paragraph break to one space anyway).
fn join_paras(paras: &[Vec<Inline>]) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    for (i, p) in paras.iter().enumerate() {
        if i > 0 {
            // A paragraph break is ≥1 line break; norm_ws collapses it to a space,
            // but it bounds an Rd `%` comment in non-markdown prose.
            out.push(Inline::Text("\n".to_string()));
        }
        out.extend(p.iter().cloned());
    }
    out
}

/// Map a tag to its Rd section macro and push the projected subtree. Tags that
/// roxygen2 does not turn into a parser-owned section (`@param` feeds the excluded
/// `\arguments`; `@export`/`@md`/`@name`/… are directives) are skipped. The
/// aggregating `@slot`/`@field` tags are handled by [`describe_section`], not here.
fn project_tag_section(name: &str, body: &[Inline], out: &mut Vec<String>, md: bool) {
    // roxygen2's `rd_section()` drops any section whose value is the literal
    // string "NULL" (`R/field.R`), a sentinel to suppress that field (e.g.
    // `@format NULL` to override an auto-generated data `\format`). This applies
    // to every prose tag that maps to a plain-string `rd_section`; `@section`
    // (a two-part value) and the excluded `@param`/… are unaffected.
    if NULL_SUPPRESSIBLE.contains(&name) && is_null_section(body, md) {
        return;
    }
    // A brace-less sticky code/verbatim macro (`\code z`/`\verb z`/…) swallows its
    // tail into per-line `RCODE`/`VERB` atoms. It is a plain-prose-section effect,
    // so it applies to the ordinary prose tags but not `@rawRd` (raw Rd, injected
    // verbatim) or `@section` (a two-part `title: body` value); the intro paragraph
    // (out of scope) is excluded structurally — it never reaches this function.
    let swallowed = (!matches!(name, "rawRd" | "section"))
        .then(|| split_sticky_braceless_swallow(body, md))
        .flatten();
    let body = swallowed.as_deref().unwrap_or(body);
    match name {
        // `@rawRd` injects its content verbatim into the Rd file at top level;
        // roxygen2 does not wrap it in a section macro. parse_Rd then splits it
        // into a sequence of top-level Rd nodes, each a section in its own right.
        // arity's roxygen lexer already recognizes inline Rd macros in prose, so
        // serializing the body yields the same atom granularity (a prose run, a
        // `\emph`, …); each atom is pushed as a bare top-level section. The
        // content is raw Rd, never markdown: the lexer keys `@rawRd` bodies to
        // non-markdown even inside an `@md` block (`is_raw_rd_tag`), so a
        // `[bracket]`/`*star*` here stays literal Rd text rather than a spurious
        // `\link`/`\emph`.
        "rawRd" => {
            for atom in serialize_inlines(body, md) {
                out.push(atom);
            }
        }
        // Direct prose → section-macro mappings. `@description`/`@details` are
        // `tag_markdown_with_sections` (`sections = TRUE`), so a brace-incomplete
        // rendered section is dropped to empty; the rest are plain `tag_markdown`.
        "description" => emit_section_with_headings(out, "description", body, md, true),
        "details" => emit_section_with_headings(out, "details", body, md, true),
        "return" => push_section(out, "value", body, md, false),
        "seealso" => push_section(out, "seealso", body, md, false),
        "source" => push_section(out, "source", body, md, false),
        "format" => push_section(out, "format", body, md, false),
        "references" => push_section(out, "references", body, md, false),
        "note" => push_section(out, "note", body, md, false),
        "author" => push_section(out, "author", body, md, false),
        "title" => push_section(out, "title", body, md, false),
        // `@section Title: body` → \section{Title}{body}. roxygen2 splits the
        // field value on its first `:`; parse_Rd then models `\section` as a
        // two-arg structural macro, so each side sub-parses inline macros/markdown
        // and a multi-atom argument is `(GRP …)`-wrapped while a single-atom one
        // stays bare (the same rule `serialize_macro` applies to `\item`/`\tabular`).
        "section" => {
            // roxygen2 `markdown_if_active`-processes the **whole** `title: body`
            // (so the link-reference pipeline — user `[ref]: url` defs, undefined-
            // label demotion, and `add_linkrefs_to_md` poisoning — spans both
            // halves), then splits the rendered Rd on the first `:`. Run the whole
            // pipeline on the whole body, split the result, and append any leaked
            // definitions to the content — they render at the very end of the
            // field, i.e. after the `:`.
            let transformed = md.then(|| resolve_linkrefs(body)).flatten();
            let body = transformed.as_deref().unwrap_or(body);
            let (heading, content) = split_section_title(body);
            let title = serialize_inlines(&heading, md);
            let mut content_atoms = serialize_inlines(&content, md);
            if md {
                for leaked in leaked_linkref_text(&inline_source_skeleton(body)) {
                    append_rendered_text(&mut content_atoms, &leaked);
                }
            }
            let mut inner = grp_arg(&title);
            if !content_atoms.is_empty() {
                if !inner.is_empty() {
                    inner.push(' ');
                }
                inner.push_str(&grp_arg(&content_atoms));
            }
            out.push(format!("(\\section{})", prefix_space(&inner)));
        }
        // `@examples`/`@examplesIf` is an aggregating field, emitted once by
        // `project_block`, so it never reaches this per-tag dispatch.
        // Everything else is roclet scaffolding or an excluded section.
        _ => {}
    }
}

/// The prose tags whose section is a plain-string `rd_section` and is therefore
/// suppressed when its value is the literal "NULL" (roxygen2's `R/field.R`
/// sentinel). `@section` is excluded: its value is a (title, body) pair, never the
/// bare string "NULL".
const NULL_SUPPRESSIBLE: &[&str] = &[
    "description",
    "details",
    "return",
    "seealso",
    "source",
    "format",
    "references",
    "note",
    "author",
    "title",
];

/// Whether a tag body is roxygen2's "NULL" suppression sentinel: it coalesces to
/// exactly one `(TEXT "NULL")` atom (a plain-string value of "NULL", any
/// surrounding whitespace already normalized away), with no macro or markdown
/// structure that would make the value something other than that string.
fn is_null_section(body: &[Inline], md: bool) -> bool {
    let atoms = serialize_inlines(body, md);
    atoms.len() == 1 && atoms[0] == "(TEXT \"NULL\")"
}

/// Push `(\<macro> <atoms…>)` for a prose section, or `(\<macro>)` when the body
/// has no content (after coalescing). Under `@md`, roxygen2's `add_linkrefs_to_md`
/// may append leaked link-reference definitions to the field text (see
/// [`leaked_linkref_text`]); they coalesce into the section's trailing prose.
///
/// `drop_on_incomplete` mirrors roxygen2's `markdown_if_active` per-section drop —
/// but the rule is **mode-dependent**. With markdown **on**, only the
/// `sections = TRUE` tags (`@description`/`@details`, including the intro paragraphs
/// `parse_description` re-emits as those tags) run `rdComplete` and replace the body
/// with `""` on a brace imbalance; the other prose tags use plain `tag_markdown`
/// (`sections = FALSE`) and do not drop, so they pass `false`. With markdown **off**,
/// `markdown_if_active`'s else-branch runs `rdComplete(text)` *unconditionally*, so
/// **every** prose section it produces (title included) drops to empty on a brace
/// imbalance regardless of `drop_on_incomplete` (`R/markdown.R`, `src/isComplete.cpp`).
/// (`@field`/`@slot` use `tag_two_part`, a separate whole-tag drop, not modeled here.)
fn push_section(
    out: &mut Vec<String>,
    macro_name: &str,
    body: &[Inline],
    md: bool,
    drop_on_incomplete: bool,
) {
    let atoms = serialize_prose_with_linkrefs(body, md);
    let check_drop = if md { drop_on_incomplete } else { true };
    if check_drop && !section_rd_complete(body, md) {
        out.push(format!("(\\{macro_name})"));
        return;
    }
    if atoms.is_empty() {
        out.push(format!("(\\{macro_name})"));
    } else {
        out.push(format!("(\\{macro_name} {})", atoms.join(" ")));
    }
}

/// One node in a markdown-heading outline: an enclosing tag (level 0, no title) or
/// a heading (level 1-6). `body` is the prose before the frame's first child
/// heading; `children` index into the flat frame arena in source order.
struct HeadingFrame {
    level: usize,
    title: Vec<Inline>,
    body: Vec<Inline>,
    children: Vec<usize>,
}

/// Emit a `@description`/`@details` section (macro `macro_name`) as roxygen2's
/// markdown-heading outline. Prose before the first heading — plus any level->=2
/// heading with no enclosing level-1 heading — stays inside the enclosing
/// `\<macro_name>`; each level-1 heading hoists to a **top-level** `\section`
/// sibling; a deeper heading nests as `\subsection` under the nearest shallower
/// heading. Falls back to a plain [`push_section`] when the body has no heading.
///
/// roxygen2 markdown-processes the whole field as one document (so link references
/// span it) and only *then* splits it into sections; arity splits first and
/// resolves each piece independently, so a link reference cannot yet cross a
/// heading (backlog). The common case — self-contained heading sections — is exact.
/// The per-section `rdComplete` drop is likewise not applied to the hoisted
/// `\section`/`\subsection`s (their bodies are balanced in practice; backlog).
fn emit_section_with_headings(
    out: &mut Vec<String>,
    macro_name: &str,
    body: &[Inline],
    md: bool,
    drop_on_incomplete: bool,
) {
    // Segment the body: the leading run, then one (heading, following-run) per
    // heading marker in source order.
    let mut segments: Vec<(Option<SyntaxNode>, Vec<Inline>)> = vec![(None, Vec::new())];
    for inl in body {
        if let Inline::MdHeading(node) = inl {
            segments.push((Some(node.clone()), Vec::new()));
        } else {
            segments.last_mut().unwrap().1.push(inl.clone());
        }
    }
    if segments.len() == 1 {
        // No heading — the ordinary prose section path.
        push_section(out, macro_name, body, md, drop_on_incomplete);
        return;
    }

    // Build the outline. Frame 0 is the enclosing tag (level 0); each heading's
    // parent is the nearest open frame of a strictly lower level.
    let mut frames: Vec<HeadingFrame> = vec![HeadingFrame {
        level: 0,
        title: Vec::new(),
        body: std::mem::take(&mut segments[0].1),
        children: Vec::new(),
    }];
    let mut stack = vec![0usize];
    for (node, run) in segments.into_iter().skip(1) {
        let node = node.expect("a non-leading segment always carries a heading");
        let (level, title_text) = parse_md_heading(&node);
        let title = resolve_macro_arg_inlines(&title_text);
        while frames[*stack.last().unwrap()].level >= level {
            stack.pop();
        }
        let parent = *stack.last().unwrap();
        let idx = frames.len();
        frames.push(HeadingFrame {
            level,
            title,
            body: run,
            children: Vec::new(),
        });
        frames[parent].children.push(idx);
        stack.push(idx);
    }

    // The enclosing tag section: its leading prose plus any level->=2 children that
    // hang directly off it (headings before the first level-1 heading), each a
    // nested `\subsection`. A level-1 child hoists out (below). Omit the enclosing
    // section entirely when it has no content.
    let mut inner = serialize_prose_with_linkrefs(&frames[0].body, md);
    for &c in &frames[0].children {
        if frames[c].level >= 2 {
            inner.push(render_heading_frame(&frames, c, md, "subsection"));
        }
    }
    if !inner.is_empty() {
        out.push(format!("(\\{macro_name} {})", inner.join(" ")));
    }

    // Each level-1 child is a top-level `\section` sibling in the output.
    for &c in &frames[0].children {
        if frames[c].level == 1 {
            out.push(render_heading_frame(&frames, c, md, "section"));
        }
    }
}

/// Render a heading frame as `(\<macro_name> <title> <body>)` — a two-arg
/// structural macro (like `@section`): the title, then the body. Each argument is
/// bare when a single atom, `(GRP …)`-wrapped when several, absent when empty. The
/// frame's own prose comes first; every child heading nests one level deeper as a
/// `\subsection`, appended in source order.
fn render_heading_frame(frames: &[HeadingFrame], idx: usize, md: bool, macro_name: &str) -> String {
    let f = &frames[idx];
    // A bare `{…}` in the heading title is an Rd `LIST` group, exactly as in prose
    // and macro args (`# H {a b}` → `(GRP (TEXT "H") (LIST (TEXT "a b")))`).
    let title_atoms = serialize_inlines(&group_brace_lists(&f.title, md), md);
    let mut body_atoms = serialize_prose_with_linkrefs(&f.body, md);
    for &c in &f.children {
        body_atoms.push(render_heading_frame(frames, c, md, "subsection"));
    }
    let mut inner = grp_arg(&title_atoms);
    let body_arg = grp_arg(&body_atoms);
    if !body_arg.is_empty() {
        if !inner.is_empty() {
            inner.push(' ');
        }
        inner.push_str(&body_arg);
    }
    format!("(\\{macro_name}{})", prefix_space(&inner))
}

/// A markdown heading node's level (1-6) and title text (markdown source, resolved
/// later by the caller). Handles both node shapes that reach `ROXYGEN_MD_HEADING`:
///
/// - **ATX** (`# Title`): a single line; the level is the leading `#` run, the title
///   is the rest with the optional closing `#` sequence stripped.
/// - **Setext** (`Title` / `===`): two or more `#'` lines whose last is a `===`/`---`
///   underline; the level comes from the underline (`=` -> 1, `-` -> 2) and the
///   title from the joined prose lines above it (soft breaks become spaces, matching
///   a coalesced paragraph).
fn parse_md_heading(node: &SyntaxNode) -> (usize, String) {
    let text = node.text().to_string();
    let lines: Vec<&str> = text.split('\n').map(strip_marker).collect();
    if lines.len() >= 2
        && let Some(level) = setext_underline_level(lines.last().unwrap())
    {
        let title = lines[..lines.len() - 1]
            .iter()
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join(" ");
        return (level, title);
    }
    // An ATX heading opening as a tag's same-line value has a marker-less line
    // (the `#'` belongs to the enclosing tag): drop only the single separator
    // space — `strip_marker` would eat the heading's own `#` run.
    let from_value = node
        .first_token()
        .is_some_and(|t| t.kind() != SyntaxKind::ROXYGEN_MARKER);
    let line = if from_value {
        text.strip_prefix([' ', '\t']).unwrap_or(&text).trim_start()
    } else {
        strip_marker(&text).trim_start()
    };
    let level = line.bytes().take_while(|&b| b == b'#').count().clamp(1, 6);
    let rest = line.get(level..).unwrap_or("").trim();
    (level, strip_atx_closing(rest).to_string())
}

/// The setext heading level of a marker-stripped line, or `None` when it is not a
/// setext underline: a non-empty run of `=` (level 1) or `-` (level 2) with only
/// surrounding whitespace. Mirrors the lexer's `is_setext_underline` (which carved
/// the underline leaf), so a line that reached here as the last child of a heading
/// node always matches.
fn setext_underline_level(line: &str) -> Option<usize> {
    let s = line.trim();
    let ch = s.bytes().next()?;
    if (ch == b'=' || ch == b'-') && s.bytes().all(|b| b == ch) {
        return Some(if ch == b'=' { 1 } else { 2 });
    }
    None
}

/// Strip a CommonMark ATX **closing sequence** — a trailing run of `#` preceded by
/// a space/tab (or forming the whole title) — from a heading title, trimming the
/// remaining trailing whitespace. `foo ###` -> `foo`; `foo#` (no preceding space)
/// stays `foo#`; `###` (empty heading with only a closing run) -> ``.
fn strip_atx_closing(s: &str) -> &str {
    let t = s.trim_end();
    let hashes = t.len() - t.trim_end_matches('#').len();
    if hashes == 0 {
        return t;
    }
    let before = &t[..t.len() - hashes];
    if before.is_empty() || before.ends_with([' ', '\t']) {
        before.trim_end()
    } else {
        t
    }
}

/// Whether roxygen2 would keep (not drop) a prose section, i.e. its Rd is
/// brace-complete under [`rd_complete`] — but from the text roxygen2 *actually*
/// scans, which differs by mode.
///
/// **Markdown on:** roxygen2 runs `rdComplete(markdown(text))` — the
/// markdown-*rendered* Rd, whose structural braces (`\emph{…}`, the
/// trailing-`\` `\emph{\}` case) come from cmark. That render is what the
/// canonical *ungrouped* atoms reconstruct ([`section_atoms_rd_complete`]); the
/// scan re-serializes with grouping off ([`serialize_prose`]) because a bare
/// `{…}` `LIST` group loses the brace-abutting backslash parity — the flat atoms
/// *are* `markdown(text)`.
///
/// **Markdown off:** roxygen2 runs `rdComplete(x$raw)` on the *raw* tag value,
/// where an escaped brace `\{`/`\}` is still backslash-escaped and therefore
/// **not counted**. The atoms are the wrong text here: escape resolution has
/// already collapsed `\{` → a bare `{` (see [`resolve_rd_text_escapes`]), so
/// reconstructing from them would count the brace and false-drop the section
/// (`a \{ b`, which roxygen2 renders "a { b", is complete). Scan the
/// pre-resolution raw text ([`section_raw_rd`]) instead — raw ≈ rendered for the
/// only chars `rd_complete` weighs (`{}`, `\`, `%`), and md-off never synthesizes
/// the cmark-derived braces that make the atoms necessary.
fn section_rd_complete(body: &[Inline], md: bool) -> bool {
    if md {
        // Scan the *ungrouped* atoms: they reconstruct `markdown(text)` faithfully,
        // whereas grouping a balanced brace pair into a `LIST` loses the backslash
        // parity rd_complete weighs (see [`serialize_prose`]). First drop each
        // odd-run `\%` comment region ([`strip_scan_percent_comments`]): the output
        // swallow keeps `ceil(k/2)` backslashes (parse_Rd's rendered text), which
        // can leave a dangling trailing escape that false-drops a kept section.
        let stripped = strip_scan_percent_comments(body);
        let scan_body = stripped.as_deref().unwrap_or(body);
        section_atoms_rd_complete(&serialize_prose(scan_body, md, false), md)
    } else {
        rd_complete(&section_raw_rd(body))
    }
}

/// Whether a section's projected atoms reconstruct to brace-complete Rd, i.e.
/// roxygen2 would *not* drop the section. Rebuilds the pre-parse Rd string from
/// the canonical S-expression atoms ([`sexpr_to_rd`]) and runs [`rd_complete`]
/// (the `is_code = false` form `markdown_if_active` uses).
fn section_atoms_rd_complete(atoms: &[String], md: bool) -> bool {
    let mut rd = String::new();
    for atom in atoms {
        sexpr_to_rd(atom, md, &mut rd);
    }
    rd_complete(&rd)
}

/// Reconstruct the raw Rd text roxygen2 runs `rdComplete` on for a markdown-OFF
/// prose section: the tag value verbatim, *before* any escape resolution. A
/// prose leaf contributes its raw source (so `\{`/`\}` keep the backslash that
/// makes `rd_complete` treat the brace as escaped, not structural); an explicit
/// Rd macro contributes its verbatim source (`\emph{x}`, balanced by
/// construction). Markdown-off bodies hold only these two inline kinds — the
/// cmark-synthesized inlines are md-only — so any other variant would be a
/// contradiction and contributes nothing. The soft-wrap sentinel
/// ([`SOFT_BREAK`]) is restored to a real newline so an unescaped `%` comment
/// stays scoped to its physical line exactly as it is in roxygen2's `x$raw`.
fn section_raw_rd(body: &[Inline]) -> String {
    let mut s = String::new();
    for inl in body {
        match inl {
            Inline::Text(t) => {
                for c in t.chars() {
                    s.push(if c == SOFT_BREAK { '\n' } else { c });
                }
            }
            Inline::Macro(n) => s.push_str(&n.text().to_string()),
            _ => {}
        }
    }
    s
}

/// Reconstruct the pre-parse Rd string from one projected S-expression atom,
/// appending to `out`. Node atoms are balanced by construction --- a
/// `(\macro c1 c2 …)` renders `\macro{c1}{c2}…` and a `(GRP …)` concatenates its
/// children (the wrapping braces come from its parent), so the only brace
/// imbalance can come from a leaf's decoded text (a trailing `\` escaping the next
/// `}`, exactly roxygen2's `\emph{\}` case). A leaf (`TEXT`/`RCODE`/`VERB`/
/// `UNKNOWN`) contributes its decoded content; under `@md` every `%` is re-escaped
/// to `\%` to mirror roxygen2's markdown render (which escapes `%` in prose, URLs,
/// verbatim, and code alike, so none opens an Rd comment), keeping the count
/// faithful.
fn sexpr_to_rd(atom: &str, md: bool, out: &mut String) {
    let bytes = atom.as_bytes();
    let mut i = 0;
    render_sexpr(bytes, &mut i, md, false, out);
}

/// `verbatim` marks a subtree living inside a **fragile** macro (`\verb`, `\code`,
/// `\link`, `\url`, `\preformatted`, …). markdown() keeps such a macro's argument
/// **raw** — an escaped `\{`/`\}` stays backslash-escaped — so it contributes a
/// brace-balanced `{…}` pair to the `rdComplete` scan regardless of its interior.
/// The projected atoms, however, have already resolved `\{` → a bare `{` (see
/// [`resolve_rd_arg_escapes`]), so counting those bare braces would false-drop a
/// section whose fragile argument merely holds an odd escaped brace
/// (`\verb{d \{ e}`). Neutralizing the leaf braces inside a fragile subtree
/// ([`append_leaf_text`] drops them) restores the balanced count markdown()
/// actually presents. Cmark-*derived* braces (`*\**` → `\emph{\}`) are outside
/// any fragile macro, so they still count and drop correctly.
fn render_sexpr(bytes: &[u8], i: &mut usize, md: bool, verbatim: bool, out: &mut String) {
    if bytes.get(*i) != Some(&b'(') {
        return;
    }
    *i += 1; // consume '('
    let head_start = *i;
    while let Some(&c) = bytes.get(*i) {
        if c == b' ' || c == b')' {
            break;
        }
        *i += 1;
    }
    let head = &bytes[head_start..*i];
    let is_leaf = matches!(head, b"TEXT" | b"RCODE" | b"VERB" | b"UNKNOWN");
    // Under `@md`, roxygen2 escapes every `%` to `\%` in the rendered Rd --- in
    // prose, URLs, verbatim, and code alike --- so none opens an Rd comment.
    let escape_percent = md;
    if is_leaf {
        skip_spaces(bytes, i);
        if bytes.get(*i) == Some(&b'"') {
            let text = read_quoted(bytes, i);
            append_leaf_text(&text, escape_percent, verbatim, out);
        }
        // consume the closing ')'
        while let Some(&c) = bytes.get(*i) {
            *i += 1;
            if c == b')' {
                break;
            }
        }
        return;
    }
    let is_grp = head == b"GRP";
    // A fragile macro keeps its argument raw in markdown() output, so its whole
    // subtree stays brace-balanced regardless of the resolved leaf braces beneath.
    let child_verbatim = verbatim
        || (!is_grp
            && is_fragile_for_md(
                std::str::from_utf8(head)
                    .unwrap_or("")
                    .trim_start_matches('\\'),
            ));
    if !is_grp {
        // A macro head: `\name`. Its leading backslash escapes the first name
        // letter for `rd_complete`, which is harmless (a letter, never a brace).
        out.push_str(std::str::from_utf8(head).unwrap_or(""));
    }
    loop {
        skip_spaces(bytes, i);
        match bytes.get(*i) {
            None => break,
            Some(&b')') => {
                *i += 1;
                break;
            }
            Some(_) => {
                if is_grp {
                    render_sexpr(bytes, i, md, child_verbatim, out);
                } else {
                    out.push('{');
                    render_sexpr(bytes, i, md, child_verbatim, out);
                    out.push('}');
                }
            }
        }
    }
}

fn skip_spaces(bytes: &[u8], i: &mut usize) {
    while bytes.get(*i) == Some(&b' ') {
        *i += 1;
    }
}

/// Read and decode a `"…"` quoted leaf string at `bytes[*i]` (which must be the
/// opening quote), inverting [`encode_text`] (`\\`→`\`, `\"`→`"`, `\n`→newline).
/// Leaves `*i` just past the closing quote.
fn read_quoted(bytes: &[u8], i: &mut usize) -> String {
    *i += 1; // consume opening quote
    let mut out = String::new();
    while let Some(&c) = bytes.get(*i) {
        if c == b'\\' {
            *i += 1;
            match bytes.get(*i) {
                Some(b'n') => out.push('\n'),
                Some(&other) => out.push(other as char),
                None => out.push('\\'),
            }
            *i += 1;
        } else if c == b'"' {
            *i += 1; // consume closing quote
            break;
        } else {
            // Copy a full UTF-8 char so multibyte content survives.
            let start = *i;
            *i += 1;
            while bytes.get(*i).is_some_and(|b| b & 0xC0 == 0x80) {
                *i += 1;
            }
            out.push_str(std::str::from_utf8(&bytes[start..*i]).unwrap_or(""));
        }
    }
    out
}

/// Append a leaf's decoded text to the reconstructed Rd, re-escaping `%`→`\%` when
/// `escape_percent` (any leaf under `@md`, where roxygen2 escapes `%` so it never
/// opens an Rd comment). Other special chars (`{`/`}`/`\`) pass through verbatim —
/// they are exactly what `rd_complete` must weigh against the structural braces.
fn append_leaf_text(text: &str, escape_percent: bool, verbatim: bool, out: &mut String) {
    if verbatim {
        // A fragile macro's argument is raw and brace-balanced in markdown()
        // output, so it contributes exactly one balanced `{…}` pair (the enclosing
        // delimiters) to the `rd_complete` scan regardless of its interior. The
        // projected atom, however, may carry either **resolved** braces (a literal
        // `\verb{d \{ e}` -> bare `{`, via [`resolve_rd_arg_escapes`]) or
        // **escaped** ones (a markdown code span -> `\{`, kept verbatim), so neither
        // re-escaping nor pass-through is uniformly right. Drop every
        // `rd_complete`-significant char (`{`, `}`, `\`, `%`) so the interior is
        // inert and only the enclosing pair counts.
        for c in text.chars() {
            if !matches!(c, '{' | '}' | '\\' | '%') {
                out.push(c);
            }
        }
    } else if escape_percent {
        // Under `@md`, roxygen2 escapes every `%` to `\%` in the rendered Rd, so
        // none opens an Rd comment against the brace count.
        for c in text.chars() {
            if c == '%' {
                out.push('\\');
            }
            out.push(c);
        }
    } else {
        out.push_str(text);
    }
}

/// Port of roxygen2's `rdComplete(string, is_code = FALSE)` (`src/isComplete.cpp`):
/// a brace-balance scan where `\` escapes the next char and `%` starts a comment to
/// end of line. The string is complete iff braces net to zero and the scan does not
/// end mid-escape. (The `is_code = TRUE` string/raw-string handling is unused by
/// `markdown_if_active`, so it is not modeled.)
fn rd_complete(s: &str) -> bool {
    #[derive(PartialEq)]
    enum State {
        Rd,
        RdEscape,
        RdComment,
    }
    let mut state = State::Rd;
    let mut braces: i64 = 0;
    for c in s.chars() {
        match state {
            State::Rd => match c {
                '{' => braces += 1,
                '}' => braces -= 1,
                '\\' => state = State::RdEscape,
                '%' => state = State::RdComment,
                _ => {}
            },
            State::RdEscape => state = State::Rd,
            State::RdComment => {
                if c == '\n' {
                    state = State::Rd;
                }
            }
        }
    }
    braces == 0 && state != State::RdEscape
}

/// Serialize a prose body into canonical atoms, applying roxygen2's
/// `add_linkrefs_to_md` poisoning model under `@md`. Every field text that
/// roxygen2 runs through `markdown_if_active` is subject to the same leak — a
/// prose section ([`push_section`]), each `@field`/`@slot` definition
/// ([`describe_section`]), and the `@section` body — so they share this path.
///
/// Two steps, both `@md`-only: a leaked link-reference block de-links the
/// shortcut/reference links in its poisoned tail, so [`demote_poisoned_links`]
/// rewrites them to literal bracket text *first* (keeping body and leaked
/// definitions consistent), then [`leaked_linkref_text`] appends the leaked
/// definitions to the trailing prose.
fn serialize_prose_with_linkrefs(body: &[Inline], md: bool) -> Vec<String> {
    serialize_prose(body, md, true)
}

/// The shared prose serializer behind [`serialize_prose_with_linkrefs`]. `group`
/// controls whether a balanced bare `{…}` run is partitioned into an Rd `LIST`
/// group ([`group_brace_lists`]).
///
/// **Output uses `group = true`** — a bare group renders `(LIST …)` in both
/// markdown modes. **The md `rdComplete` drop scan uses `group = false`**: roxygen2
/// decides the drop on `markdown(text)`, whose braces are the flat cmark-rendered
/// ones, and grouping loses the backslash parity that scan weighs. A structural
/// brace comes from an *even* (non-escaping) run, which [`collapse_md_backslash_runs`]
/// keeps verbatim while it abuts the brace; grouping consumes the brace, so the run
/// collapses early (`\\` → `\`) and a trailing `\` before the `LIST`'s brace would
/// read as a spurious escape in the reconstruction. Scanning the ungrouped flat
/// atoms sidesteps that — they *are* `markdown(text)`.
fn serialize_prose(body: &[Inline], md: bool, group: bool) -> Vec<String> {
    let transformed = md.then(|| resolve_linkrefs(body)).flatten();
    let body = transformed.as_deref().unwrap_or(body);
    // The md leaked-linkref scan reconstructs the raw markdown source, so it reads
    // the *ungrouped* body — brace groups are Rd `LIST` structure, not markdown, and
    // never affect the link-reference candidate scan. Capture it before grouping.
    let leaked = if md {
        leaked_linkref_text(&inline_source_skeleton(body))
    } else {
        Vec::new()
    };
    // A bare `{…}` in prose is an Rd `LIST` group in both modes. The brace/comment
    // parity is shared (an odd backslash run escapes the brace, an even run opens
    // it); only the `%`-comment trigger differs by mode (`group_brace_lists` handles
    // that internally).
    let grouped = group.then(|| group_brace_lists(body, md));
    let scan = grouped.as_deref().unwrap_or(body);
    // A brace-less `\item` in prose is parse_Rd's out-of-list recovery: an
    // `(UNKNOWN "\item")` node splitting the surrounding text. Carve it out of the
    // (already brace-grouped) run before serializing. Output path only: the
    // `group = false` md `rdComplete` scan reads the raw `\item` text, which counts
    // no braces either way, so it needs no split.
    let split = group.then(|| split_braceless_items(scan)).flatten();
    let scan = split.as_deref().unwrap_or(scan);
    let mut atoms = serialize_inlines(scan, md);
    for l in leaked {
        append_rendered_text(&mut atoms, &l);
    }
    atoms
}

/// Apply roxygen2's full markdown link-reference pipeline to a prose body,
/// returning the rewritten inline run (or `None` when nothing changed). The
/// caller has already checked that markdown is active. Three composing stages,
/// each turning links into other links or literal text:
///
/// 1. **User definitions** (`[ref]: url`): a referencing shortcut/reference link
///    whose label is defined becomes a `\href{url}{display}` (display kept), and
///    the definition lines are consumed. Runs on the original body so the refmap
///    below still sees every bracket the way roxygen2's raw-source scan does.
/// 2. **Undefined-label demotion** (the `(?<!\])`/`(?=[^\[{])` link-reference-map
///    gap): a shortcut/reference link whose label roxygen2 never defines demotes
///    to literal bracket text.
/// 3. **Positional poisoning** (`add_linkrefs_to_md`): a valid candidate whose
///    synthesized definition leaks demotes its tail.
///
/// Both demotions only turn links into literal text, so order is immaterial to
/// correctness; the refmap (stage 2) runs after stage 1 so it sees every bracket
/// the user defs left behind.
fn resolve_linkrefs(body: &[Inline]) -> Option<Vec<Inline>> {
    let mut urls: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    collect_user_linkrefs_tree(body, &mut urls);
    let resolved = (!urls.is_empty())
        .then(|| apply_user_linkrefs(body, &urls))
        .flatten();
    let b1 = resolved.as_deref().unwrap_or(body);
    let undefined = demote_undefined_links(b1, &linkref_keys(b1));
    let b2 = undefined.as_deref().unwrap_or(b1);
    let demoted = demote_poisoned_links(b2);
    // Materialize an owned body only when some stage actually rewrote it.
    if resolved.is_some() || undefined.is_some() || demoted.is_some() {
        Some(demoted.unwrap_or_else(|| b2.to_vec()))
    } else {
        None
    }
}

/// A pending piece of the `(TEXT …)` atom [`serialize_inlines`] is coalescing.
/// Ordinary prose is `Raw` (source text awaiting the markdown/comment pipeline);
/// a block quote's already-flattened text is `Final` (pre-processed) so it *glues*
/// into the surrounding atom instead of splitting off as its own — roxygen2 emits
/// no paragraph separator around an unsupported block quote, so its text runs
/// straight onto adjacent prose (`before` + `> q` → `beforeq`).
enum RunSeg {
    Raw(String),
    Final(String),
}

/// Append raw source text to the pending run, coalescing into a trailing `Raw`
/// segment so a contiguous prose run stays one segment (processed as a whole).
fn push_raw(run: &mut Vec<RunSeg>, s: &str) {
    match run.last_mut() {
        Some(RunSeg::Raw(last)) => last.push_str(s),
        _ => run.push(RunSeg::Raw(s.to_string())),
    }
}

/// Drop trailing whitespace (spaces, source line breaks, `SOFT_BREAK`s) from the
/// pending run, popping now-empty trailing `Raw` segments. Used before a block
/// quote glues on, so the preceding paragraph's trailing break does not survive as
/// a separating space (`norm_ws` would collapse it to one). A `Final` segment (an
/// already-flattened block quote) is left untouched — its own whitespace is fixed.
fn trim_trailing_run_ws(run: &mut Vec<RunSeg>) {
    while let Some(RunSeg::Raw(last)) = run.last_mut() {
        let trimmed = last.trim_end_matches(is_posix_space);
        if trimmed.len() == last.len() {
            break;
        }
        last.truncate(trimmed.len());
        if last.is_empty() {
            run.pop();
        } else {
            break;
        }
    }
}

/// Finalize the pending run into one coalesced `(TEXT …)` atom (or `None` when it
/// normalizes to empty), clearing it. Each `Raw` segment runs through the prose
/// pipeline ([`process_prose`]: markdown escaping or Rd `%`-comment stripping)
/// *without* normalizing whitespace; a `Final` (pre-flattened block quote) segment
/// passes through verbatim; the concatenation is whitespace-normalized once, so a
/// boundary line break collapses to a single space while a glued block quote stays
/// seamless.
fn flush_run(run: &mut Vec<RunSeg>, md: bool) -> Option<String> {
    if run.is_empty() {
        return None;
    }
    let mut combined = String::new();
    for seg in run.iter() {
        match seg {
            RunSeg::Raw(s) => combined.push_str(&process_prose(s, md)),
            RunSeg::Final(s) => combined.push_str(s),
        }
    }
    run.clear();
    text_atom(&combined)
}

/// Partition a non-`@md` prose run's bare `{…}` brace groups into [`Inline::BraceGroup`]
/// nodes. parse_Rd treats an unescaped brace pair in prose text as a `LIST`
/// delimiter (a macro's own braces live inside its CST node, so only *bare* text
/// braces reach here): `a {b c} d` → `(TEXT "a") (LIST (TEXT "b c")) (TEXT "d")`.
/// Groups nest, span macros (an `Inline::Macro` between the braces lands inside the
/// group), and cross soft breaks; the inner text pieces keep their raw form so the
/// downstream [`process_prose`] still resolves escapes and strips `%` comments.
///
/// Brace parity mirrors [`resolve_rd_text_escapes`] and is mode-independent: an
/// odd-length backslash run escapes the following brace (`\{`/`\}` stay literal, no
/// group), an even run leaves it bare (`\\{` opens a group). Under `@md` a source
/// `\\{y}` is exactly what parse_Rd receives from `markdown(text)` — cmark pairs the
/// doubled run back to `\` and leaves the brace bare — so the same parity applies.
///
/// The **`%` comment trigger is inverted between modes**, mirroring
/// [`md_percent_swallow`]. Non-md: a bare `%` opens an Rd comment that hides braces
/// to the physical line end (an escaped `\%` was already consumed by the backslash
/// arm, so only a bare `%` reaches the `%` arm). Md: roxygen2 escapes a rendered `%`
/// to `\%`, so a bare/even-preceded `%` stays literal and does **not** hide braces;
/// only a `%` preceded by an **odd** backslash run renders bare and opens a comment
/// (the escaping backslash collides). Either way a comment's text is copied verbatim
/// (the prose pipeline drops it later) without treating its braces as delimiters.
///
/// Only a **balanced** run is grouped; an unbalanced one is returned unchanged (its
/// section drops via `rdComplete` before the atoms are used — see
/// [`section_rd_complete`]), so the pass never models parse_Rd's error recovery. A
/// brace-free run is likewise returned as-is, keeping its byte-identical
/// serialization.
fn group_brace_lists(body: &[Inline], md: bool) -> Vec<Inline> {
    // `stack[0]` is the output level; each deeper frame is an open `{` group.
    let mut stack: Vec<Vec<Inline>> = vec![Vec::new()];
    let mut buf = String::new();
    let mut grouped = false;
    let flush = |stack: &mut Vec<Vec<Inline>>, buf: &mut String| {
        if !buf.is_empty() {
            stack
                .last_mut()
                .unwrap()
                .push(Inline::Text(std::mem::take(buf)));
        }
    };
    for inl in body {
        let Inline::Text(s) = inl else {
            // A non-text inline (macro, resolved md node) is opaque to brace
            // scanning; flush the pending text and drop it into the current group.
            flush(&mut stack, &mut buf);
            stack.last_mut().unwrap().push(inl.clone());
            continue;
        };
        let bytes = s.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => {
                    let start = i;
                    while i < bytes.len() && bytes[i] == b'\\' {
                        i += 1;
                    }
                    let run_len = i - start;
                    buf.push_str(&s[start..i]);
                    // An odd run escapes the next char (brace/letter): copy it
                    // verbatim so it is never read as a group delimiter. Under `@md`
                    // a `%` is *not* escape-consumed here — its comment-ness is
                    // decided by the `%` arm (odd preceding run → bare `%` → comment).
                    if run_len % 2 == 1 && i < bytes.len() && !(md && bytes[i] == b'%') {
                        let mut end = i + 1;
                        while !s.is_char_boundary(end) {
                            end += 1;
                        }
                        buf.push_str(&s[i..end]);
                        i = end;
                    }
                }
                b'%' => {
                    // Whether this `%` opens an Rd comment (hiding following braces to
                    // the physical line end). Non-md: a bare `%` always does (an
                    // escaped `\%` was consumed by the backslash arm). Md: only a `%`
                    // rendered bare — one preceded by an *odd* backslash run — does
                    // (roxygen2 escapes an even/zero-run `%` to `\%`); mirrors
                    // [`md_percent_swallow`].
                    let opens_comment = if md {
                        let mut k = 0usize;
                        while k < i && bytes[i - 1 - k] == b'\\' {
                            k += 1;
                        }
                        k % 2 == 1
                    } else {
                        true
                    };
                    if opens_comment {
                        // Copy verbatim to the physical line end (a real newline or a
                        // SOFT_BREAK); its braces are inert.
                        let start = i;
                        while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'\x0c' {
                            i += 1;
                        }
                        buf.push_str(&s[start..i]);
                    } else {
                        buf.push('%');
                        i += 1;
                    }
                }
                b'{' => {
                    flush(&mut stack, &mut buf);
                    stack.push(Vec::new());
                    grouped = true;
                    i += 1;
                }
                b'}' if stack.len() > 1 => {
                    flush(&mut stack, &mut buf);
                    let g = stack.pop().unwrap();
                    stack.last_mut().unwrap().push(Inline::BraceGroup(g));
                    grouped = true;
                    i += 1;
                }
                _ => {
                    let start = i;
                    i += 1;
                    while i < bytes.len() && !matches!(bytes[i], b'\\' | b'%' | b'{' | b'}') {
                        i += 1;
                    }
                    buf.push_str(&s[start..i]);
                }
            }
        }
        flush(&mut stack, &mut buf);
    }
    // An unclosed group means the braces are unbalanced: leave the run flat (the
    // section drops anyway), never a partial tree.
    if !grouped || stack.len() != 1 {
        return body.to_vec();
    }
    stack.pop().unwrap()
}

/// Serialize an inline run into the canonical atom sequence: maximal prose runs
/// coalesce into one whitespace-normalized `(TEXT …)`, and each macro becomes a
/// nested subtree — mirroring the R driver's `serialize_children`. `md` is the
/// block's resolved markdown mode: with markdown off a prose run is literal Rd, so
/// `process_prose` strips its `%` line comments.
fn serialize_inlines(body: &[Inline], md: bool) -> Vec<String> {
    let mut atoms: Vec<String> = Vec::new();
    let mut run: Vec<RunSeg> = Vec::new();
    for inl in body {
        match inl {
            Inline::Text(s) => push_raw(&mut run, s),
            Inline::Macro(node) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(serialize_macro(node, md));
            }
            Inline::MdCode(content) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(md_code_atom(content));
            }
            Inline::MdEmphasis { strong, children } => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                // Recurse into the inner inline run (nesting projects as structure),
                // then wrap. The block's `@md` mode holds inside an emphasis span.
                let inner = serialize_inlines(children, md).join(" ");
                let head = if *strong { "\\strong" } else { "\\emph" };
                atoms.push(if inner.is_empty() {
                    format!("({head})")
                } else {
                    format!("({head} {inner})")
                });
            }
            Inline::MdList(node) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(serialize_md_list(node));
            }
            Inline::MdListResolved { ordered, items } => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(serialize_md_list_resolved(*ordered, items));
            }
            Inline::MdLink(raw) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(resolve_md_link(raw).unwrap_or_default());
            }
            Inline::MdInlineLink { url, display } => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(inline_link_node_atom(url, display, md));
            }
            // A reference/shortcut link whose display is not plain text is *dropped*
            // by roxygen2's `parse_link` ("markdown links must contain plain text").
            // The dropped link contributes nothing and — like roxygen2's `""` — does
            // not break the surrounding text run, so it is skipped without flushing
            // (the run keeps accumulating, coalescing the text on either side).
            Inline::MdRefLink { dest, display } => {
                if link_display_is_droppable(display) {
                    continue;
                }
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(ref_link_node_atom(display, dest));
            }
            Inline::MdShortcutLink { display } => {
                if link_display_is_droppable(display) {
                    continue;
                }
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(shortcut_link_node_atom(display));
            }
            Inline::MdImage(raw) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                if let Some(atom) = resolve_md_image(raw) {
                    atoms.push(atom);
                }
            }
            Inline::MdCodeBlock(node) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.extend(serialize_md_code_block(node));
            }
            Inline::MdIndentedCode(node) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.extend(serialize_md_indented_code(node));
            }
            Inline::MdHtml(raw) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(html_inline_atom(raw));
            }
            Inline::MdHtmlBlock(node) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(serialize_md_html_block(node));
            }
            Inline::BracelessItem => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(format!("(UNKNOWN {})", encode_text("\\item")));
            }
            // A brace-less sticky code/verbatim macro's swallowed tail: one verbatim
            // `(RCODE …)`/`(VERB …)` atom per physical source line, each carrying its
            // own trailing `\n` (`\code z here` line-wrapped → `(RCODE " z here\n")
            // (RCODE "continued\n")`).
            Inline::StickyVerbatim { code, lines } => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                let head = if *code { "RCODE" } else { "VERB" };
                for line in lines {
                    atoms.push(format!("({head} {})", encode_text(&format!("{line}\n"))));
                }
            }
            Inline::BraceGroup(children) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                let inner = serialize_inlines(children, md).join(" ");
                atoms.push(if inner.is_empty() {
                    "(LIST)".to_string()
                } else {
                    format!("(LIST {inner})")
                });
            }
            Inline::MdBlockQuote(node) => {
                // roxygen2 has no block-quote support: it renders the flattened
                // plain text with *no* surrounding paragraph separator, so the text
                // glues straight onto adjacent prose (`before` + `> q` → `beforeq`).
                // Push it as a pre-flattened `Final` segment so it coalesces into
                // the current `(TEXT …)` atom instead of splitting off as its own.
                // The preceding node keeps a trailing line break (its own newline,
                // or the part-join break) which `norm_ws` would otherwise turn into a
                // separating space, so drop that trailing whitespace first — cmark
                // strips a paragraph's trailing whitespace before the quote appends.
                let flat = block_quote_flat_text(node);
                if !flat.is_empty() {
                    trim_trailing_run_ws(&mut run);
                    run.push(RunSeg::Final(flat));
                }
            }
            Inline::MdTable(node) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(serialize_md_table(node));
            }
            // A heading is normally consumed by the outline builder before it
            // reaches here (`emit_section_with_headings`). Reaching this arm means a
            // heading in a context roxygen2 does not turn into a section (e.g.
            // `@seealso`, where roxygen2 errors on a level-1 heading) — out of scope
            // for the projector. Fall back to rendering the title text inline so the
            // walk never panics; such a case is never pinned in the corpus.
            Inline::MdHeading(node) => {
                let (_, title) = parse_md_heading(node);
                for atom in serialize_inlines(&resolve_macro_arg_inlines(&title), md) {
                    if let Some(prose) = flush_run(&mut run, md) {
                        atoms.push(prose);
                    }
                    atoms.push(atom);
                }
            }
        }
    }
    if let Some(atom) = flush_run(&mut run, md) {
        atoms.push(atom);
    }
    atoms
}

/// Project one `ROXYGEN_RD_MACRO` node into `(\name <children…>)`: the `[opt]` and
/// `{`/`}` delimiters are dropped, prose text coalesces into `(TEXT …)`, verbatim
/// content becomes `(VERB …)` (no whitespace collapse), and nested macros recurse.
///
/// A *structural* macro (`\item`, `\tabular` --- [`is_two_arg_rd_macro`]) models
/// each `{…}` argument as a list, so a multi-atom argument projects to a
/// `(GRP …)` wrapper (`\tabular{rl}{a \tab b}` → `(\tabular (TEXT "rl") (GRP …))`)
/// while a single-atom argument unwraps (`\item{a}{first}` → `(\item (TEXT "a")
/// (TEXT "first"))`). A latexlike macro (`\code`, `\emph`, …) inlines its single
/// argument's atoms directly, never wrapping.
///
/// Under `@md`, a **non-fragile** inline text macro (`\emph`, `\strong`, `\sQuote`,
/// …) has its argument **markdown-processed** ([`is_md_inline_text_macro`]):
/// roxygen2 protects only its `escaped_for_md` set from cmark, so a non-fragile
/// macro's `{…}` body is parsed as a markdown inline run (`\emph{*x*}` →
/// `\emph{\emph{x}}`). A fragile nested macro (`\code`/`\link`/…) stays literal —
/// this resolves recursively, so each macro re-checks its own fragility. A
/// non-fragile **structural** macro (`\item`, `\tabular`, `\href` —
/// [`is_md_structural_macro`]) likewise markdown-processes each of its arguments:
/// the `md_structural` flag below routes prose runs through the inline pass while
/// the loop's existing arms keep nested macros (`\tab`/`\cr`), verbatim args (the
/// `\href` URL), and the per-argument `(GRP …)` wrap intact.
fn serialize_macro(node: &SyntaxNode, md: bool) -> String {
    // `\preformatted` is a *verbatim block* macro: parse_Rd keeps its body
    // verbatim (no whitespace collapse, no nested-macro / markdown parsing) and
    // splits it at newlines into one `(VERB …)` per line — the same shape as an
    // `\out` body. The run/flush prose model below normalizes whitespace, so this
    // macro takes a dedicated verbatim arm instead.
    if macro_head(node).trim_start_matches('\\') == "preformatted" {
        let atoms = preformatted_atoms(node);
        return if atoms.is_empty() {
            "(\\preformatted)".to_string()
        } else {
            format!("(\\preformatted {})", atoms.join(" "))
        };
    }
    let head_full = macro_head(node);
    let name = head_full.trim_start_matches('\\');
    if md
        && is_md_inline_text_macro(name)
        && let Some(content) = macro_single_arg_content(node)
    {
        // A bare `{…}` in the argument is an Rd `LIST` group, exactly as in prose
        // (`\emph{a {b} c}` → `(\emph (TEXT "a") (LIST (TEXT "b")) (TEXT "c"))`).
        let grouped = group_brace_lists(&resolve_macro_arg_inlines(&content), md);
        let atoms = serialize_inlines(&grouped, md);
        return if atoms.is_empty() {
            format!("({head_full})")
        } else {
            format!("({head_full} {})", atoms.join(" "))
        };
    }
    // A structural two-arg macro (`\item`, `\tabular`, `\href`) under `@md` has
    // each non-verbatim argument markdown-processed as **one** cmark run (so an
    // emphasis/link span crosses a nested macro). That needs a whole-argument
    // resolution from the pre-carved children, handled by a dedicated walk.
    if md && is_md_structural_macro(name) {
        return serialize_md_structural_macro(node, &head_full);
    }
    let mut head = String::new();
    let mut structural = false;
    let mut out_atoms: Vec<String> = Vec::new();
    // Per-argument pieces: raw prose text (escapes unresolved) and already-serialized
    // atoms (nested macros, verbatim `(VERB …)`). Collected between the argument's
    // `{`…`}` so a bare `{…}` brace group can be folded across a nested macro (see
    // [`finalize_macro_arg`] / [`group_arg_pieces`]).
    let mut pieces: Vec<ArgPiece> = Vec::new();
    let mut text_buf = String::new();
    // Push the pending prose run as one text piece, coalescing contiguous text tokens
    // (parse_Rd models one `(TEXT …)` per uninterrupted run).
    let flush_text = |text_buf: &mut String, pieces: &mut Vec<ArgPiece>| {
        if !text_buf.is_empty() {
            pieces.push(ArgPiece::Text(std::mem::take(text_buf)));
        }
    };
    for el in node.children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_RD_MACRO_NAME => {
                head = el
                    .as_token()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();
                structural = is_two_arg_rd_macro(head.trim_start_matches('\\'));
            }
            SyntaxKind::ROXYGEN_RD_MACRO_VERB => {
                flush_text(&mut text_buf, &mut pieces);
                let raw = el
                    .as_token()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();
                pieces.push(ArgPiece::Atom(format!(
                    "(VERB {})",
                    encode_text(&resolve_rd_arg_escapes(&raw))
                )));
            }
            SyntaxKind::ROXYGEN_RD_MACRO => {
                flush_text(&mut text_buf, &mut pieces);
                if let Some(n) = el.as_node() {
                    pieces.push(ArgPiece::Atom(serialize_macro(n, md)));
                }
            }
            // A closing `}` ends an argument group: flush the run, then atomize the
            // argument's pieces (folding bare `{…}` groups into `(LIST …)`) and
            // finalize (GRP-wrapping a structural macro's multi-atom argument). The
            // opening `{` carries no content.
            SyntaxKind::ROXYGEN_RD_MACRO_DELIM => {
                if el.as_token().is_some_and(|t| t.text() == "}") {
                    flush_text(&mut text_buf, &mut pieces);
                    finalize_macro_arg(&mut pieces, head == "\\code", structural, &mut out_atoms);
                }
            }
            // The dropped option and the `#'` markers threaded into a multi-line
            // block macro carry no projected content; any other leaf (text, and
            // the collapsed newline/whitespace trivia) is prose.
            SyntaxKind::ROXYGEN_RD_MACRO_OPT | SyntaxKind::ROXYGEN_MARKER => {}
            _ => {
                if let Some(t) = el.as_token() {
                    text_buf.push_str(t.text());
                }
            }
        }
    }
    // Defensive: trailing content with no closing brace (a malformed macro).
    flush_text(&mut text_buf, &mut pieces);
    finalize_macro_arg(&mut pieces, head == "\\code", structural, &mut out_atoms);
    if out_atoms.is_empty() {
        // A name-only macro node (no `{…}` content). A known zero-argument macro
        // (`\cr`, or a list child `\item` under `\itemize`) renders name-only;
        // an **unknown** brace-less `\word` is tagged `UNKNOWN` by parse_Rd.
        let name = head.trim_start_matches('\\');
        if is_known_rd_macro(name) {
            format!("({head})")
        } else {
            format!("(UNKNOWN {})", encode_text(&head))
        }
    } else {
        format!("({head} {})", out_atoms.join(" "))
    }
}

/// A piece of a non-`@md` (or fragile) macro argument while folding bare `{…}`
/// brace groups: raw prose text (escapes unresolved) or an already-serialized atom
/// (a nested macro or a verbatim `(VERB …)`), opaque to the brace scan.
enum ArgPiece {
    Text(String),
    Atom(String),
}

/// Atomize one macro argument's [`ArgPiece`]s into `out`, folding bare `{…}` groups
/// into `(LIST …)` atoms and GRP-wrapping a structural macro's multi-atom argument.
///
/// A **verbatim** argument (`\code`'s RCODE body, `code == true`) is never grouped:
/// its braces are literal R code, so each text piece splits into `(RCODE …)` line
/// atoms and nested macros splice in. A **prose** argument folds bare groups via
/// [`group_arg_pieces`]; when that finds no group (or the braces are unbalanced —
/// the section drops via `rdComplete`), each text piece coalesces into one
/// whitespace-normalized `(TEXT …)`, byte-identical to the ungrouped path.
///
/// Finalization matches parse_Rd: a structural two-arg macro (`\item`/`\tabular`/
/// `\href`) wraps a multi-atom argument in `(GRP …)` (a bare group counts as one
/// atom); a single-atom argument or a latexlike macro's inlined content splices in.
fn finalize_macro_arg(
    pieces: &mut Vec<ArgPiece>,
    code: bool,
    structural: bool,
    out: &mut Vec<String>,
) {
    if pieces.is_empty() {
        return;
    }
    let atoms = if code {
        let mut v = Vec::new();
        for p in pieces.drain(..) {
            match p {
                ArgPiece::Text(s) => v.extend(rcode_atoms(&resolve_rd_arg_escapes(&s))),
                ArgPiece::Atom(a) => v.push(a),
            }
        }
        v
    } else if let Some(a) = group_arg_pieces(pieces) {
        pieces.clear();
        a
    } else {
        let mut v = Vec::new();
        for p in pieces.drain(..) {
            match p {
                ArgPiece::Text(s) => {
                    if let Some(a) = text_atom(&resolve_rd_arg_escapes(&s)) {
                        v.push(a);
                    }
                }
                ArgPiece::Atom(a) => v.push(a),
            }
        }
        v
    };
    if structural && atoms.len() > 1 {
        out.push(format!("(GRP {})", atoms.join(" ")));
    } else {
        out.extend(atoms);
    }
}

/// Fold a **prose** macro argument's [`ArgPiece`]s into serialized atoms, turning
/// each bare `{…}` brace pair into a `(LIST …)` atom (empty group → `(LIST)`).
/// parse_Rd lexes a braced argument with the same bare-group rule as prose text —
/// an unescaped `{`/`}` is a `LIST` delimiter — so `\emph{a {b} c}` projects
/// `(\emph (TEXT "a") (LIST (TEXT "b")) (TEXT "c"))`; groups nest and span nested
/// macros (an opaque `Atom` lands inside the group). Mirrors [`group_brace_lists`]
/// but on already-carved pieces, and unlike prose text a braced argument has **no**
/// `%` comment (an in-arg `%` is literal) and no brace-less-macro drop, so the scan
/// only weighs backslash-escaping and braces.
///
/// Brace parity matches [`resolve_rd_arg_escapes`]: an odd-length backslash run
/// escapes the following brace (`\{`/`\}` stay literal, no group), an even run opens
/// it. Text pieces keep their raw backslashes; [`text_atom`] resolves them once each
/// run flushes.
///
/// Returns `None` when the argument holds no bare group (so the caller keeps the
/// byte-identical ungrouped atomization) or the braces are unbalanced (the section
/// drops via `rdComplete` before these atoms are used — never a partial tree).
fn group_arg_pieces(pieces: &[ArgPiece]) -> Option<Vec<String>> {
    // `stack[0]` is the argument's output level; each deeper frame is an open `{`.
    let mut stack: Vec<Vec<String>> = vec![Vec::new()];
    let mut buf = String::new();
    let mut grouped = false;
    let flush = |stack: &mut Vec<Vec<String>>, buf: &mut String| {
        if !buf.is_empty() {
            if let Some(a) = text_atom(&resolve_rd_arg_escapes(buf)) {
                stack.last_mut().unwrap().push(a);
            }
            buf.clear();
        }
    };
    for piece in pieces {
        match piece {
            // A nested macro / verbatim atom is opaque to the brace scan: flush the
            // pending text and drop it into the current group.
            ArgPiece::Atom(a) => {
                flush(&mut stack, &mut buf);
                stack.last_mut().unwrap().push(a.clone());
            }
            ArgPiece::Text(s) => {
                let bytes = s.as_bytes();
                let mut i = 0usize;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => {
                            let start = i;
                            while i < bytes.len() && bytes[i] == b'\\' {
                                i += 1;
                            }
                            let run_len = i - start;
                            buf.push_str(&s[start..i]);
                            // An odd run escapes the next char (a brace stays literal):
                            // copy it verbatim so it is never read as a delimiter.
                            if run_len % 2 == 1 && i < bytes.len() {
                                let mut end = i + 1;
                                while !s.is_char_boundary(end) {
                                    end += 1;
                                }
                                buf.push_str(&s[i..end]);
                                i = end;
                            }
                        }
                        b'{' => {
                            flush(&mut stack, &mut buf);
                            stack.push(Vec::new());
                            grouped = true;
                            i += 1;
                        }
                        b'}' if stack.len() > 1 => {
                            flush(&mut stack, &mut buf);
                            let g = stack.pop().unwrap();
                            let inner = g.join(" ");
                            stack.last_mut().unwrap().push(if inner.is_empty() {
                                "(LIST)".to_string()
                            } else {
                                format!("(LIST {inner})")
                            });
                            grouped = true;
                            i += 1;
                        }
                        _ => {
                            let start = i;
                            i += 1;
                            while i < bytes.len() && !matches!(bytes[i], b'\\' | b'{' | b'}') {
                                i += 1;
                            }
                            buf.push_str(&s[start..i]);
                        }
                    }
                }
            }
        }
    }
    flush(&mut stack, &mut buf);
    // No bare group, or unbalanced braces (the section drops anyway): keep the run
    // flat, never a partial tree.
    if !grouped || stack.len() != 1 {
        return None;
    }
    Some(stack.pop().unwrap())
}

/// Project a **structural** two-arg macro (`\item`/`\tabular`/`\href`) under `@md`,
/// markdown-processing each non-verbatim argument as a single cmark run.
///
/// roxygen2 resolves a structural argument as **one** markdown run, so an emphasis
/// or link span crosses a nested Rd macro (`*a \strong{x} b*` →
/// `\emph{a \strong{x} b}`, and even `*a \tab b*` →
/// `\emph{a \tab b}` across a brace-less separator). The general `serialize_macro`
/// loop resolves each prose run between the carved macros *separately*, which
/// leaves the unmatched `*` delimiters literal. Here each argument group's
/// already-carved children are collected into [`MdArgPiece`]s — prose runs as
/// markdown-lexed text, every nested macro (braced `\strong`, brace-less
/// `\tab`/`\cr`) as one opaque token — and resolved together by
/// [`resolve_md_inline_pieces`], so the delimiter-stack arena spans the macros.
///
/// A *verbatim* argument (the `\href` URL) keeps its `(VERB …)` projection
/// untouched. A multi-atom argument `(GRP …)`-wraps (parse_Rd models it as a list);
/// a single-atom argument (e.g. one `\emph` owning the whole argument) stays bare.
fn serialize_md_structural_macro(node: &SyntaxNode, head_full: &str) -> String {
    let mut out_atoms: Vec<String> = Vec::new();
    let mut pieces: Vec<MdArgPiece> = Vec::new();
    let mut run = String::new();
    // A verbatim argument projects as a single `(VERB …)`, never markdown.
    let mut verb: Option<String> = None;

    // Flush the pending prose run into a markdown-lexed text piece.
    let flush = |run: &mut String, pieces: &mut Vec<MdArgPiece>| {
        if !run.is_empty() {
            pieces.push(MdArgPiece::Text(std::mem::take(run)));
        }
    };

    for el in node.children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_RD_MACRO_NAME => {}
            SyntaxKind::ROXYGEN_RD_MACRO_VERB => {
                let raw = el
                    .as_token()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();
                verb = Some(format!("(VERB {})", encode_text(&raw)));
            }
            // A nested macro is opaque to the markdown run: emit its raw source as
            // one piece so emphasis/links span across it.
            SyntaxKind::ROXYGEN_RD_MACRO => {
                flush(&mut run, &mut pieces);
                if let Some(n) = el.as_node() {
                    pieces.push(MdArgPiece::Macro(n.text().to_string()));
                }
            }
            // The closing `}` of an argument group: resolve its pieces as one
            // markdown run (or emit the verbatim atom), GRP-wrapping a multi-atom
            // result. The opening `{` carries no content.
            SyntaxKind::ROXYGEN_RD_MACRO_DELIM => {
                if el.as_token().is_some_and(|t| t.text() == "}") {
                    flush(&mut run, &mut pieces);
                    if let Some(v) = verb.take() {
                        out_atoms.push(v);
                    } else {
                        let para = resolve_md_inline_pieces(&pieces);
                        // Fold bare `{…}` groups (Rd `LIST`s) in the resolved run.
                        let grouped = group_brace_lists(&para_to_inlines(&para), true);
                        let atoms = serialize_inlines(&grouped, true);
                        match atoms.len() {
                            0 => {}
                            1 => out_atoms.push(atoms.into_iter().next().unwrap()),
                            _ => out_atoms.push(format!("(GRP {})", atoms.join(" "))),
                        }
                    }
                    pieces.clear();
                }
            }
            SyntaxKind::ROXYGEN_RD_MACRO_OPT | SyntaxKind::ROXYGEN_MARKER => {}
            _ => {
                if let Some(t) = el.as_token() {
                    run.push_str(t.text());
                }
            }
        }
    }
    if out_atoms.is_empty() {
        format!("({head_full})")
    } else {
        format!("({head_full} {})", out_atoms.join(" "))
    }
}

/// Whether macro `name` (without the leading `\`) has its single argument
/// **markdown-processed** when it appears inline under `@md`. roxygen2 protects
/// only its `escaped_for_md` set ([`is_fragile_for_md`]) from cmark, so *every*
/// other macro's argument is markdown — but arity already models the block and
/// structural macros (`\itemize`/`\describe`/`\tabular`/`\Sexpr`/…) as their own
/// constructs, so resolving their bodies as inline prose would be wrong; they are
/// excluded here. The remainder are the inline text macros (`\emph`, `\strong`,
/// `\sQuote`, `\value`, …) whose body is a latexlike inline run.
fn is_md_inline_text_macro(name: &str) -> bool {
    is_known_rd_macro(name)
        && !is_fragile_for_md(name)
        && !is_two_arg_rd_macro(name)
        && !matches!(
            name,
            "itemize" | "enumerate" | "describe" | "Sexpr" | "RdOpts" | "enc"
        )
}

/// Whether macro `name` (without the leading `\`) is a **structural** two-argument
/// macro whose arguments are markdown-processed when it appears under `@md`. These
/// are the non-fragile members of [`is_two_arg_rd_macro`] (`\item`, `\tabular`,
/// `\href`) --- `\figure` is fragile ([`is_fragile_for_md`]), so it stays literal.
/// Unlike a latexlike single-arg macro ([`is_md_inline_text_macro`]), each `{…}`
/// argument is resolved independently and a multi-atom one wraps in `(GRP …)`.
fn is_md_structural_macro(name: &str) -> bool {
    is_known_rd_macro(name) && !is_fragile_for_md(name) && is_two_arg_rd_macro(name)
}

/// The raw source text of a single-argument macro's `{…}` content (everything
/// between the first `{` delimiter and its matching `}`), or `None` if the macro
/// has no argument group. Nested macros contribute their *source* (their own
/// braces live inside the child node, not as direct delimiters), so the result
/// re-lexes faithfully; threaded `#'` markers are dropped (defensive — an inline
/// macro carries none).
fn macro_single_arg_content(node: &SyntaxNode) -> Option<String> {
    let mut content = String::new();
    let mut opened = false;
    let mut inside = false;
    for el in node.children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_RD_MACRO_DELIM => {
                let text = el
                    .as_token()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();
                if text == "{" && !opened {
                    opened = true;
                    inside = true;
                } else if text == "}" && inside {
                    inside = false;
                }
            }
            SyntaxKind::ROXYGEN_MARKER => {}
            _ if inside => match el {
                NodeOrToken::Node(n) => content.push_str(&n.text().to_string()),
                NodeOrToken::Token(t) => content.push_str(t.text()),
            },
            _ => {}
        }
    }
    opened.then_some(content)
}

/// Resolve a non-fragile macro's raw argument `content` as a `@md` markdown inline
/// run, returning the projected inline elements. Reuses the real inline pass
/// ([`resolve_md_inline`]) and the ordinary inline collector, so emphasis, links,
/// code spans, and nested macros resolve exactly as in `@md` prose.
fn resolve_macro_arg_inlines(content: &str) -> Vec<Inline> {
    para_to_inlines(&resolve_md_inline(content))
}

/// Collect a resolved `ROXYGEN_PARAGRAPH` node's children into projected inline
/// elements: the threaded `#'` markers drop and a soft `NEWLINE` becomes a space
/// (norm_ws-equivalent), everything else projects via [`push_inline`].
fn para_to_inlines(para: &SyntaxNode) -> Vec<Inline> {
    let mut out = Vec::new();
    for el in para.children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_MARKER => {}
            SyntaxKind::NEWLINE => out.push(Inline::Text(SOFT_BREAK.to_string())),
            _ => push_inline(&mut out, el),
        }
    }
    out
}

/// The macro head (`\name`, with the leading `\`) of a `ROXYGEN_RD_MACRO` node,
/// or `""` if it has no name leaf.
fn macro_head(node: &SyntaxNode) -> String {
    node.children_with_tokens()
        .find(|el| el.kind() == SyntaxKind::ROXYGEN_RD_MACRO_NAME)
        .and_then(|el| el.as_token().map(|t| t.text().to_string()))
        .unwrap_or_default()
}

/// The per-line `(VERB …)` atoms of a `\preformatted` block macro. The body is
/// the verbatim text between the opening `{` and closing `}`; each continuation
/// `#'` line has its marker (and the single following space) stripped, the lines
/// rejoin with `\n`, and [`verb_atoms`] splits at newlines exactly as parse_Rd
/// does for a verbatim macro body. (A `\preformatted` body never nests another
/// macro or a markdown construct, so reconstructing from the node text — rather
/// than walking typed children — stays faithful and mirrors
/// [`serialize_md_html_block`].)
fn preformatted_atoms(node: &SyntaxNode) -> Vec<String> {
    let text = node.text().to_string();
    let (Some(open), Some(close)) = (text.find('{'), text.rfind('}')) else {
        return Vec::new();
    };
    if close <= open {
        return Vec::new();
    }
    // The opener-line remainder keeps its leading space verbatim; later lines drop
    // only the `#'` marker (and one conventional space).
    let mut body = String::new();
    for (idx, line) in text[open + 1..close].split('\n').enumerate() {
        if idx == 0 {
            body.push_str(line);
        } else {
            body.push('\n');
            body.push_str(strip_marker(line));
        }
    }
    // parse_Rd resolves the Rd-string escapes inside a `\preformatted` body exactly
    // as it does for any other verbatim macro argument (`\{` -> `{`, `\%` -> `%`,
    // `\\` -> `\`), so the projected `(VERB …)` matches — and the rd_complete scan
    // (which neutralizes fragile-macro braces) counts a balanced pair.
    verb_atoms(&resolve_rd_arg_escapes(&body))
}

/// Split an `@section` body at roxygen2's title separator (the first literal `:`,
/// which lives in a prose `Inline::Text` run) into `(title, content)` inline runs.
/// The `:` is dropped; everything before it is the heading, everything after the
/// body. Macros/markdown carry no `:` separator, so only `Inline::Text` is scanned.
fn split_section_title(body: &[Inline]) -> (Vec<Inline>, Vec<Inline>) {
    let mut title: Vec<Inline> = Vec::new();
    let mut content: Vec<Inline> = Vec::new();
    let mut split = false;
    for inl in body {
        if split {
            content.push(inl.clone());
            continue;
        }
        if let Inline::Text(t) = inl
            && let Some(idx) = t.find(':')
        {
            if idx > 0 {
                title.push(Inline::Text(t[..idx].to_string()));
            }
            let after = &t[idx + 1..];
            if !after.is_empty() {
                content.push(Inline::Text(after.to_string()));
            }
            split = true;
            continue;
        }
        title.push(inl.clone());
    }
    (title, content)
}

/// Render a structural macro argument from its serialized atoms: a multi-atom
/// argument is `(GRP …)`-wrapped (parse_Rd models it as a list), a single-atom one
/// stays bare, and an empty one yields nothing. Mirrors `serialize_macro`'s
/// `finalize`, used for the `\section` title/body arguments.
fn grp_arg(atoms: &[String]) -> String {
    match atoms {
        [] => String::new(),
        [one] => one.clone(),
        many => format!("(GRP {})", many.join(" ")),
    }
}

fn prefix_space(s: &str) -> String {
    if s.is_empty() {
        String::new()
    } else {
        format!(" {s}")
    }
}

/// A `(TEXT "…")` atom with the body whitespace-normalized (matching the R
/// driver's `norm_ws`), or `None` if the body is blank.
fn text_atom(body: &str) -> Option<String> {
    let t = norm_ws(body);
    (!t.is_empty()).then(|| format!("(TEXT {})", encode_text(&t)))
}

/// Apply roxygen2's prose-text pipeline to a raw source run *without* normalizing
/// whitespace — the caller normalizes once over the fully coalesced atom (see
/// [`flush_run`]), so a `Final` block-quote segment can glue seamlessly onto the
/// processed prose on either side. With markdown off the run is literal Rd, where
/// an unescaped `%` begins a comment to end of line (parse_Rd's rule), so the
/// comment is stripped per physical line; with markdown on roxygen2 escapes `%`
/// (`\%`) so it survives and the markdown escapes (backslash runs, `[`/`]`, HTML
/// entities) resolve instead. (Source line breaks stay as `\n`, which the caller's
/// `norm_ws` later collapses, so a comment's end-of-line is honored either way.)
fn process_prose(run: &str, md: bool) -> String {
    if md {
        // cmark decodes HTML entities (`&amp;`, `&copy;`, `&#65;`) as the final
        // text transform: they are inert with respect to the `%`-swallow, bracket,
        // and backslash rules (an entity-produced `[`/`%`/`\` is literal text, not a
        // delimiter), so decode after those run on the raw source.
        decode_html_entities(&unescape_md_brackets(&collapse_md_backslash_runs(
            &md_percent_swallow(run),
        )))
    } else {
        resolve_rd_text_escapes(&strip_rd_comments(run))
    }
}

/// Resolve parse_Rd's literal-text escapes in non-`@md` prose. Backslashes pair
/// left-to-right (`\\` → one literal `\`); an unpaired trailing backslash
/// before one of the Rd escape characters `%`, `{`, `}` is consumed with the
/// escape resolved (`\%` → `%`, `\{` → `{`); an unpaired backslash before a
/// brace-required known macro name not followed by `{` re-forms the macro whose
/// missing argument triggers parse_Rd's drop-recovery — the `\name` vanishes
/// and the text continues (`\emph z` → ` z`; see
/// [`is_rd_braceless_drop_macro`], which excludes the sticky code/verbatim-mode
/// names left literal as backlog); an unpaired backslash before anything else
/// stays literal (`a \ b` keeps its backslash). Runs before `%` interact with
/// the line comment: [`strip_rd_comments`] runs first with the same pairing
/// (its `escaped` flip-flop), so a `%` that survives it is always
/// escape-consumed here.
fn resolve_rd_text_escapes(run: &str) -> String {
    let bytes = run.as_bytes();
    let mut out = String::with_capacity(run.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'\\' {
                i += 1;
            }
            out.push_str(&run[start..i]);
            continue;
        }
        let mut k = 0usize;
        while i < bytes.len() && bytes[i] == b'\\' {
            i += 1;
            k += 1;
        }
        for _ in 0..k / 2 {
            out.push('\\');
        }
        if k % 2 == 1 {
            match bytes.get(i) {
                Some(b'%' | b'{' | b'}') => {
                    out.push(bytes[i] as char);
                    i += 1;
                }
                _ => {
                    if let Some(end) = braceless_drop_name_end(run, i) {
                        i = end;
                    } else {
                        out.push('\\');
                    }
                }
            }
        }
    }
    out
}

/// Resolve parse_Rd's Rd-string escapes inside a **literal Rd macro's braced
/// argument** (`\code{…}`, `\verb{…}`, `\emph{…}`, `\link{…}`, `\url{…}`, …).
/// parse_Rd lexes every braced argument — verbatim `RCODE`/`VERB` or prose `TEXT`
/// alike — with the same escape rules: backslashes pair left-to-right (`\\` → one
/// literal `\`), and an unpaired trailing backslash before one of the Rd
/// metacharacters `{`, `}`, `%` is consumed with the character rendered bare
/// (`\{` → `{`, `\}` → `}`, `\%` → `%`); any other unpaired backslash stays literal.
///
/// Unlike [`resolve_rd_text_escapes`] this does **no** brace-less-macro drop
/// recovery (an in-argument `\word` is a real nested macro, already carved into a
/// child node, so the run between children is pure argument text) and **no**
/// `%`-comment stripping (`%` inside a braced argument is literal, never a comment).
///
/// Mode-independent: parse_Rd resolves these escapes in a fragile macro's argument
/// under `@md` exactly as it does with markdown off (engine-probed). A markdown
/// code span or fence keeps its `\{` because it projects through a *different* path
/// ([`md_code_atom`]/[`serialize_md_code_block`]/[`verb_atoms`]), never this one.
fn resolve_rd_arg_escapes(run: &str) -> String {
    let bytes = run.as_bytes();
    let mut out = String::with_capacity(run.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'\\' {
                i += 1;
            }
            out.push_str(&run[start..i]);
            continue;
        }
        let mut k = 0usize;
        while i < bytes.len() && bytes[i] == b'\\' {
            i += 1;
            k += 1;
        }
        for _ in 0..k / 2 {
            out.push('\\');
        }
        if k % 2 == 1 {
            match bytes.get(i) {
                Some(b'%' | b'{' | b'}') => {
                    out.push(bytes[i] as char);
                    i += 1;
                }
                _ => out.push('\\'),
            }
        }
    }
    out
}

/// The end index of a brace-required known macro name at `run[start..]` whose
/// brace-less misuse parse_Rd drop-recovers, or `None` when the bytes there are
/// not such a name (or a `{` follows, making it a real macro call — those are
/// carved by the lexer and never reach prose text, but the guard keeps the
/// transform faithful on any input). Shared by the non-`@md` escape resolution
/// and the `@md` backslash-run collapse: the preceding unpaired `\` plus this
/// name vanish from the rendered text.
fn braceless_drop_name_end(run: &str, start: usize) -> Option<usize> {
    let bytes = run.as_bytes();
    let end = crate::parser::roxygen::rd_macro_name_end(bytes, start);
    (end > start && is_rd_braceless_drop_macro(&run[start..end]) && bytes.get(end) != Some(&b'{'))
        .then_some(end)
}

/// Split each unescaped, brace-less `\item` in a prose inline run into its own
/// [`Inline::BracelessItem`] node, mirroring parse_Rd's out-of-list recovery
/// (`a \item b` → `(TEXT "a") (UNKNOWN "\item") (TEXT "b")`). Recurses into bare
/// brace groups (`{a \item b}` → `(LIST (TEXT "a") (UNKNOWN "\item") (TEXT "b"))`)
/// and emphasis spans so a nested `\item` splits too. Returns `None` when the run
/// contains no brace-less `\item` (the caller keeps the original body). Runs after
/// [`group_brace_lists`], so a following `{…}` has already become an
/// `Inline::BraceGroup` (`\item{x}` → `(UNKNOWN "\item") (LIST (TEXT "x"))`,
/// matching parse_Rd, which never binds the group to the unknown macro).
fn split_braceless_items(body: &[Inline]) -> Option<Vec<Inline>> {
    let mut changed = false;
    let mut out: Vec<Inline> = Vec::with_capacity(body.len());
    for inl in body {
        match inl {
            Inline::Text(s) => match split_item_text(s) {
                Some(pieces) => {
                    changed = true;
                    out.extend(pieces);
                }
                None => out.push(inl.clone()),
            },
            Inline::BraceGroup(children) => match split_braceless_items(children) {
                Some(new) => {
                    changed = true;
                    out.push(Inline::BraceGroup(new));
                }
                None => out.push(inl.clone()),
            },
            Inline::MdEmphasis { strong, children } => match split_braceless_items(children) {
                Some(new) => {
                    changed = true;
                    out.push(Inline::MdEmphasis {
                        strong: *strong,
                        children: new,
                    });
                }
                None => out.push(inl.clone()),
            },
            _ => out.push(inl.clone()),
        }
    }
    changed.then_some(out)
}

/// Partition one prose text string at each unescaped, brace-less `\item` (see
/// [`split_braceless_items`]). The interleaved `Inline::Text` (raw, escapes
/// unresolved for the downstream [`process_prose`]) and [`Inline::BracelessItem`]
/// pieces, or `None` when there is no split. Parity-gated like every `\`-carve: a
/// backslash run of even length keeps the final `\` escaped (`\\item` stays literal
/// `\item` text), so only a run of odd length that abuts the exact name `item`
/// begins the macro. The name must be exactly `item` (a longer `\itemize`/`\itemx`
/// is a different macro — the lexer or the drop-recovery handles those).
fn split_item_text(s: &str) -> Option<Vec<Inline>> {
    let bytes = s.as_bytes();
    let mut out: Vec<Inline> = Vec::new();
    let mut seg_start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            i += 1;
            continue;
        }
        let run_start = i;
        while i < bytes.len() && bytes[i] == b'\\' {
            i += 1;
        }
        // An even run pairs off entirely (the `\item` is escaped); an odd run leaves
        // the final `\` (at `i - 1`) unescaped, opening a macro at `run[i..]`.
        if (i - run_start) % 2 == 1 {
            let name_end = crate::parser::roxygen::rd_macro_name_end(bytes, i);
            if &s[i..name_end] == "item" {
                let before = &s[seg_start..i - 1];
                if !before.is_empty() {
                    out.push(Inline::Text(before.to_string()));
                }
                out.push(Inline::BracelessItem);
                seg_start = name_end;
                i = name_end;
            }
        }
    }
    if out.is_empty() {
        return None;
    }
    let tail = &s[seg_start..];
    if !tail.is_empty() {
        out.push(Inline::Text(tail.to_string()));
    }
    Some(out)
}

/// Rewrite an explicit prose tag's body when a brace-less sticky code/verbatim Rd
/// macro (`\code`/`\verb`/…, see [`sticky_braceless_code_mode`]) triggers
/// parse_Rd's argument-mode swallow, or `None` when no such trigger applies (the
/// caller keeps the body). The dropped `\name` and everything after it, to section
/// end, becomes an [`Inline::StickyVerbatim`] — one `RCODE`/`VERB` atom per
/// physical source line — while the prose *before* the trigger stays ordinary text
/// (`a \code z here` → `(TEXT "a") (RCODE " z here\n")`).
///
/// **Scope (this slice): an explicit prose tag with a single-paragraph plain-text
/// tail.** The swallow crosses paragraph breaks in roxygen2, but arity's paragraph
/// model collapses blank-line *counts* (many blanks → one part boundary), so a
/// cross-paragraph tail cannot be reconstructed faithfully from the inline run and
/// is withheld. The tail must also be free of macros/lists/emphasis (they still
/// parse inside the swallow, splitting the RCODE — `\code z \emph{x}` →
/// `(RCODE " z ") (\emph …) …`) and of the raw chars `\ { } %` (a bare `{`/`}`
/// breaks the section's field braces, a `%` acts as an Rd line comment, a stray
/// `\` risks a nested carve) — all withheld as backlog. Withholding leaves the
/// `\code` literal (its current projection), never a wrong shape.
///
/// The intro paragraph is deliberately excluded: its swallow crosses roxygen2's
/// generated field braces (`{`/`}` scaffolding leaks into the atoms), which is
/// unmodelable at section granularity. Only [`project_tag_section`] calls this, so
/// the intro (emitted directly in [`project_block`]) never reaches it.
fn split_sticky_braceless_swallow(body: &[Inline], md: bool) -> Option<Vec<Inline>> {
    for (idx, inl) in body.iter().enumerate() {
        let Inline::Text(s) = inl else { continue };
        let Some((backslash, name_end, code)) = find_sticky_trigger(s) else {
            continue;
        };
        // Reject a same-line `%` before the trigger: a non-`@md` Rd comment there
        // would swallow the `\name` itself (there is no `SOFT_BREAK` between them in
        // one text piece, so the whole pre-trigger portion is one physical line).
        if s[..backslash].contains('%') {
            return None;
        }
        // The tail = this text from `name_end`, plus every following inline. A
        // single-paragraph plain-text tail is all `Inline::Text` with none of the
        // section-breaking / mode-shifting chars; anything else is withheld.
        let mut content = s[name_end..].to_string();
        for later in &body[idx + 1..] {
            match later {
                Inline::Text(t) => content.push_str(t),
                _ => return None,
            }
        }
        if content.contains(['\n', '\\', '{', '}', '%']) {
            return None;
        }
        let mut out: Vec<Inline> = body[..idx].to_vec();
        let before = &s[..backslash];
        if !before.is_empty() {
            out.push(Inline::Text(before.to_string()));
        }
        out.push(Inline::StickyVerbatim {
            code,
            lines: sticky_swallow_lines(&content, md),
        });
        return Some(out);
    }
    None
}

/// Find the first brace-less sticky code/verbatim macro trigger in one prose text
/// piece: `(backslash index, name end, code)` where `backslash` is the position of
/// the opening `\`, `name_end` is one past the macro name, and `code` is
/// [`sticky_braceless_code_mode`]'s R-code/verbatim flag. Parity-gated like every
/// `\`-carve — only an odd-length backslash run opens a macro (`\\code` is an
/// escaped literal) — and brace-less only (a `{` follows a real macro call, carved
/// by the lexer and never reaching prose text; guarded regardless).
fn find_sticky_trigger(s: &str) -> Option<(usize, usize, bool)> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            i += 1;
            continue;
        }
        let run_start = i;
        while i < bytes.len() && bytes[i] == b'\\' {
            i += 1;
        }
        if (i - run_start) % 2 == 1 {
            let name_end = crate::parser::roxygen::rd_macro_name_end(bytes, i);
            if name_end > i
                && bytes.get(name_end) != Some(&b'{')
                && let Some(code) = sticky_braceless_code_mode(&s[i..name_end])
            {
                return Some((i - 1, name_end, code));
            }
        }
    }
    None
}

/// Split a sticky swallow's tail into its per-physical-line contents. Physical
/// lines are the [`SOFT_BREAK`] boundaries of the folded run; the first line (the
/// trigger line's remainder) keeps its leading whitespace verbatim
/// (`(RCODE " z here\n")`) in both modes.
///
/// Continuation-line leading whitespace differs by mode. **Non-`@md`:** roxygen2
/// strips only the `#'` comment prefix and one following space, so a `#'   cont`
/// line surfaces as `"  cont"` (two spaces surviving). **`@md`:** cmark
/// additionally strips a paragraph continuation line's remaining leading
/// whitespace before the swallow captures the rendered text, so `#'   cont`
/// surfaces as `"cont"` (fully flush). Trailing `\n` is added per line by the
/// serializer.
fn sticky_swallow_lines(content: &str, md: bool) -> Vec<String> {
    content
        .split(SOFT_BREAK)
        .enumerate()
        .map(|(i, line)| {
            if i == 0 {
                line.to_string()
            } else if md {
                line.trim_start_matches(is_posix_space).to_string()
            } else {
                line.strip_prefix(' ').unwrap_or(line).to_string()
            }
        })
        .collect()
}

/// In `@md` prose, roxygen2 honors a CommonMark backslash escape for the square
/// brackets `[`/`]` only: an escaped `\[`/`\]` is literal (never a link delimiter)
/// *and the backslash is consumed* (`\[`→`[`, `\]`→`]`). This is unique to
/// brackets — roxygen2's `double_escape_md` doubles every backslash but then
/// reverts `\\[`→`\[` and `\\]`→`\]`, so only the bracket escape survives cmark;
/// every other punctuation escape (`\*`, `` \` ``, `\%`, …) keeps its backslash
/// because the doubling neutralizes it. The lexer already suppresses the link at an
/// escaped `[` ([`bracket_is_escaped`](crate::parser::roxygen)); this drops the
/// now-redundant backslash so the projected literal text matches roxygen2.
///
/// Only a *single* adjacent backslash is consumed (`\\[`→`\[`); deeper backslash
/// runs follow `double_escape_md`'s non-overlapping `gsub` semantics and are left
/// as backlog (a `\\\[` run is rare in real docs).
fn unescape_md_brackets(run: &str) -> String {
    let mut out = String::with_capacity(run.len());
    let mut chars = run.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && matches!(chars.peek(), Some('[' | ']')) {
            out.push(chars.next().expect("peeked bracket"));
        } else {
            out.push(c);
        }
    }
    out
}

/// Resolve `parse_Rd`'s escaped-brace rendering in the **projected `@md` TEXT**
/// leaves. The leaf arrives carrying the cmark-stage backslash runs
/// ([`collapse_md_backslash_runs`] leaves a run abutting `{`/`}` verbatim), so a run
/// of `k` backslashes before a brace is exactly what parse_Rd receives from
/// `markdown(text)`: it pairs the backslashes left-to-right into `floor(k/2)` literal
/// backslashes, and for **odd** `k` the trailing unpaired `\` escapes the brace to a
/// **bare** literal (`\{` → `{`, `\\\{` → `\{`, `\\\\\{` → `\\{`). A run before any
/// non-brace character — or a bare brace with no preceding backslash — is untouched.
///
/// This runs **after** the section's `rdComplete` drop decision (see
/// [`resolve_md_text_braces`]), which is why it lives here and not in
/// [`process_prose`]: the drop scan must weigh the *escaped* (pre-resolution) brace
/// (roxygen2 runs `rdComplete(markdown(text))` where `\{` is still escaped, so an
/// unbalanced *escaped* brace does not drop the section — `a \{ b` is kept), whereas
/// the rendered TEXT wants the bare brace. Resolving before the scan would count the
/// bare brace and false-drop.
///
/// An **even** `k` pairs off entirely and leaves the brace *unescaped* — a real Rd
/// brace group parse_Rd models as `(LIST …)`. arity keeps flat `TEXT` there (the
/// bare-brace-group model is separate backlog); this transform still halves the
/// backslashes to `k/2` and copies the brace bare, so the projection stays divergent
/// for that shape without compounding the backslash count.
fn resolve_md_brace_runs(run: &str) -> String {
    let bytes = run.as_bytes();
    let mut out = String::with_capacity(run.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            // Measure the maximal backslash run.
            let mut k = 0usize;
            while i + k < bytes.len() && bytes[i + k] == b'\\' {
                k += 1;
            }
            if matches!(bytes.get(i + k), Some(b'{' | b'}')) {
                // parse_Rd pairs the run into `floor(k/2)` literal backslashes; an
                // odd trailing `\` escapes the brace bare, an even run leaves it bare
                // too (the `(LIST …)` group backlog). Either way: emit the bare brace.
                for _ in 0..k / 2 {
                    out.push('\\');
                }
                out.push(bytes[i + k] as char);
                i += k + 1;
                continue;
            }
            for _ in 0..k {
                out.push('\\');
            }
            i += k;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i] != b'\\' {
            i += 1;
        }
        out.push_str(&run[start..i]);
    }
    out
}

/// Apply [`resolve_md_brace_runs`] to every `(TEXT "…")` leaf in a projected
/// `@md` section string, leaving every other leaf verbatim. The escaped-brace
/// resolution is a **`@md`-mode, prose-TEXT-only** encoding difference: a code
/// span's verbatim `VERB` keeps its `\{` (roxygen2 renders `\verb` content
/// literally), a data-name `RCODE`/`\code` body resolves the brace but is left as
/// backlog here, and non-`@md` TEXT already had its escapes resolved upstream
/// ([`resolve_rd_text_escapes`]). So the transform is gated to `@md` sections by
/// its single caller ([`project_block`]) and scoped to `TEXT` heads here.
///
/// The scan tracks quote state so a literal `(TEXT "` *inside* another leaf's
/// string (e.g. a code span) is copied as data, never mistaken for a leaf opener.
fn resolve_md_text_braces(sexpr: &str) -> String {
    let bytes = sexpr.as_bytes();
    let mut out = String::with_capacity(sexpr.len());
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                out.push('(');
                i += 1;
                // Read the head token (up to a space, ')', or '"').
                let head_start = i;
                while i < bytes.len() && !matches!(bytes[i], b' ' | b')' | b'"') {
                    i += 1;
                }
                let head = &sexpr[head_start..i];
                out.push_str(head);
                if head == "TEXT" {
                    // Copy the single separating space, then transform the quoted
                    // content (a `TEXT` leaf is always `(TEXT "…")`).
                    while i < bytes.len() && bytes[i] == b' ' {
                        out.push(' ');
                        i += 1;
                    }
                    if bytes.get(i) == Some(&b'"') {
                        let text = read_quoted(bytes, &mut i);
                        out.push_str(&encode_text(&resolve_md_brace_runs(&text)));
                    }
                }
            }
            b'"' => {
                // A leaf's quoted string in non-`TEXT` position: copy it verbatim,
                // honoring `\"`/`\\` escapes so an interior `(` or `"` is data.
                let start = i;
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => i += if i + 1 < bytes.len() { 2 } else { 1 },
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
                out.push_str(&sexpr[start..i]);
            }
            _ => {
                let start = i;
                while i < bytes.len() && !matches!(bytes[i], b'(' | b'"') {
                    i += 1;
                }
                out.push_str(&sexpr[start..i]);
            }
        }
    }
    out
}

/// In `@md` prose, model roxygen2's `%`-swallow. `%` is the Rd comment character,
/// so roxygen2's markdown→Rd pass escapes a rendered `%` to `\%`; but when the
/// markdown already places a literal backslash immediately before the `%`, that
/// escaping backslash collides with the literal one and the `%` is left **bare** in
/// the Rd, starting a comment that eats to end of line. Whether the collision
/// happens is keyed on the **parity of the source backslash run** before the `%`
/// (`double_escape_md` doubles the run to `2k`, cmark resolves the `\\` pairs, and
/// the emitted Rd carries the `k` literal backslashes plus the one escaping the
/// `%` — a run of `k + 1`, which parse_Rd leaves a trailing bare `%` iff `k` is
/// odd):
///
/// - `k` **odd** (`\%`, `\\\%`, …): the `%` comments to end of line. The `k`
///   backslashes are kept (later halved to `ceil(k/2)` by
///   [`collapse_md_backslash_runs`]) and everything from the `%` to the physical
///   line's end is dropped.
/// - `k` **even** (bare `%`, `\\%`, `\\\\%`, …): the `%` survives as a literal
///   percent; the run keeps its `ceil(k/2)` backslashes and the `%`.
///
/// The swallow is line-scoped (roxygen2's comment ends at the newline, and a
/// soft-wrapped continuation on the next `#'` line survives), mirroring the
/// non-`@md` [`strip_rd_comments`]. It runs **before** [`collapse_md_backslash_runs`]
/// so the odd/even decision reads the original run length, not its halved form.
fn md_percent_swallow(run: &str) -> String {
    physical_lines(run)
        .map(md_percent_swallow_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The prefix of `line` up to (not including) the first `%` whose preceding
/// maximal backslash run has **odd** length (the whole line if none); the kept
/// backslashes are retained for [`collapse_md_backslash_runs`] to halve.
fn md_percent_swallow_line(line: &str) -> &str {
    let bytes = line.as_bytes();
    for (i, _) in line.char_indices().filter(|&(i, _)| bytes[i] == b'%') {
        let mut k = 0usize;
        while i > k && bytes[i - 1 - k] == b'\\' {
            k += 1;
        }
        if k % 2 == 1 {
            return &line[..i];
        }
    }
    line
}

/// Strip each odd-run `\%` comment region from a prose `TEXT` leaf **for the
/// `@md` `rdComplete` drop scan only**. Per physical line, the backslash run, the
/// `%`, and everything to the line's end are dropped.
///
/// roxygen2 decides the drop on `rdComplete(markdown(text))`. In `markdown(text)`
/// an odd source backslash run before a `%` renders an *even* run plus a bare
/// comment `%` (`\%` → `\\%`, `\\\%` → `\\\%`): the even run pairs cleanly and the
/// bare `%` comments to end of line, so the region contributes nothing to the
/// brace balance and never leaves a dangling escape. The **output** serializer
/// models the same swallow via [`md_percent_swallow`], but it keeps `ceil(k/2)`
/// backslashes — parse_Rd's rendered text (`y \% …` → `y \`) — which can leave an
/// *odd* trailing backslash at the section's end. Reconstructing the scan from
/// those output atoms then reads a dangling escape and false-drops a section
/// roxygen2 keeps (`@details y \% {z} end.`). Dropping the whole region here (not
/// just the tail) removes that backslash so the scan matches `markdown(text)`. An
/// **even**-run `%` — a genuine literal percent roxygen2 escapes to `\%` — is left
/// untouched for render-time re-escaping ([`append_leaf_text`]).
fn strip_scan_percent_comment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for seg in text.split_inclusive(['\n', SOFT_BREAK]) {
        match seg.chars().next_back() {
            Some(c @ ('\n' | SOFT_BREAK)) => {
                out.push_str(scan_line_before_odd_percent(
                    &seg[..seg.len() - c.len_utf8()],
                ));
                out.push(c);
            }
            _ => out.push_str(scan_line_before_odd_percent(seg)),
        }
    }
    out
}

/// The prefix of `line` up to (not including) the backslash run of the first `%`
/// whose preceding maximal backslash run has **odd** length (the whole line if
/// none). Unlike [`md_percent_swallow_line`], the backslashes are dropped too, so
/// no trailing escape survives into the [`rd_complete`] scan.
fn scan_line_before_odd_percent(line: &str) -> &str {
    let bytes = line.as_bytes();
    for (i, _) in line.char_indices().filter(|&(i, _)| bytes[i] == b'%') {
        let mut k = 0usize;
        while i > k && bytes[i - 1 - k] == b'\\' {
            k += 1;
        }
        if k % 2 == 1 {
            return &line[..i - k];
        }
    }
    line
}

/// Rewrite a prose body's top-level `TEXT` leaves for the `@md` `rdComplete` drop
/// scan, applying [`strip_scan_percent_comment`]. Returns `None` when no leaf
/// changed (the common case — no odd-run `\%`), so the scan reuses the original
/// body. Only top-level prose is stripped: a `%`-comment inside a balanced macro
/// argument contributes a balanced pair regardless (backlog).
fn strip_scan_percent_comments(body: &[Inline]) -> Option<Vec<Inline>> {
    let mut changed = false;
    let new: Vec<Inline> = body
        .iter()
        .map(|inl| match inl {
            Inline::Text(t) => {
                let stripped = strip_scan_percent_comment(t);
                changed |= stripped != *t;
                Inline::Text(stripped)
            }
            other => other.clone(),
        })
        .collect();
    changed.then_some(new)
}

/// In `@md` prose, a run of literal backslashes collapses per CommonMark's
/// backslash escaping. roxygen2's `double_escape_md` doubles every backslash
/// (`k` → `2k`), cmark then resolves each `\\` pair to one literal backslash
/// (`2k` → `k`), and finally `parse_Rd` collapses the rendered `\\` pairs again
/// (`k` → `ceil(k/2)`, the trailing odd backslash escaping the next character).
/// The net effect on the parsed text is that a run of `k` source backslashes
/// renders as `ceil(k/2)` backslashes: a lone `\` (`\*`, `` \` ``, `\_`, …) keeps
/// its single backslash (`ceil(1/2) == 1`, a no-op), while `\\` → `\`,
/// `\\\\` → `\\`, and so on.
///
/// A run immediately before `[`/`]` is left untouched — those bracket escapes
/// follow `double_escape_md`'s revert (`\\[` → `\[`) and are resolved separately
/// by [`unescape_md_brackets`], which runs after this. A run before `{`/`}` is
/// **also** left untouched, at cmark's `k`-backslash stage: parse_Rd's brace
/// resolution is parity-dependent and is deferred to the post-pass
/// ([`resolve_md_brace_runs`]) so the `rdComplete` scan can weigh the still-escaped
/// brace first (halving here would destroy the parity it needs). Runs before `%`
/// (the Rd comment character) are also left to the separate `%`-swallow modeling (a
/// lone `\%` keeps its backslash but the bare `%` still comments to end of line);
/// `ceil(k/2)` is a no-op for the common `k == 1` case there anyway.
///
/// An **odd** run before a brace-required known macro name not followed by `{`
/// is the brace-less misuse: the `k` source backslashes reach parse_Rd intact
/// (double → cmark halves), which pairs them into `k/2` literal backslashes and
/// re-forms the trailing `\name` — whose missing argument drop-recovers, so the
/// name vanishes (`\emph z` → ` z`, `\\\link q` → `\ q`; see
/// [`braceless_drop_name_end`]). Mirrors the non-`@md`
/// [`resolve_rd_text_escapes`].
fn collapse_md_backslash_runs(run: &str) -> String {
    let bytes = run.as_bytes();
    let mut out = String::with_capacity(run.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'\\' {
                i += 1;
            }
            out.push_str(&run[start..i]);
            continue;
        }
        // Consume a maximal run of backslashes.
        let mut k = 0usize;
        while i < bytes.len() && bytes[i] == b'\\' {
            i += 1;
            k += 1;
        }
        // A run abutting a square bracket is a bracket escape: leave it verbatim
        // for `unescape_md_brackets` (its `\\[` → `\[` revert is a distinct path).
        if matches!(bytes.get(i), Some(b'[' | b']')) {
            for _ in 0..k {
                out.push('\\');
            }
        } else if matches!(bytes.get(i), Some(b'{' | b'}')) {
            // A run abutting a brace is left at cmark's stage: `double_escape_md`
            // doubles the run and cmark halves it back, so the atom carries the
            // same `k` backslashes roxygen2's `rdComplete` scans in `markdown(text)`
            // (a brace's escape does not drop the section — the scan sees `\{`).
            // parse_Rd's brace resolution is parity-dependent (odd `k` escapes the
            // brace bare, even `k` leaves an Rd group), and the general `ceil(k/2)`
            // halving would destroy that parity, so it is deferred to the post-pass
            // ([`resolve_md_text_braces`] → [`resolve_md_brace_runs`]).
            for _ in 0..k {
                out.push('\\');
            }
        } else if k % 2 == 1
            && let Some(end) = braceless_drop_name_end(run, i)
        {
            // Odd run + brace-less drop macro: the unpaired `\` re-forms the
            // macro, whose drop-recovery consumes `\name`.
            for _ in 0..k / 2 {
                out.push('\\');
            }
            i = end;
        } else {
            for _ in 0..k.div_ceil(2) {
                out.push('\\');
            }
        }
    }
    out
}

/// Reconstruct a field's markdown **source** text for link-reference scanning:
/// the literal prose (`Inline::Text`) verbatim, with every *resolved* inline (a
/// link/code/macro/emphasis arity already turned into structure) flattened to a
/// single space. A resolved inline is never a leaked-definition candidate —
/// roxygen2 either linkified it (a valid, consumed definition) or it is a non-link
/// macro — and a space keeps it a neutral boundary that cannot fuse adjacent
/// brackets into a spurious span.
fn inline_source_skeleton(body: &[Inline]) -> String {
    let mut s = String::new();
    for inl in body {
        s.push_str(&inline_skeleton_fragment(inl));
    }
    s
}

/// The markdown-source fragment an inline contributes to [`inline_source_skeleton`]:
/// the literal prose (`Inline::Text`) verbatim, and a single space for every
/// *resolved* inline arity already turned into structure.
///
/// **Inline links are the exception.** roxygen2's `get_md_linkrefs` synthesizes a
/// `[text]: R:text` reference definition for an inline `[text](url)` link too (its
/// `[text]` is a bracket-free shortcut candidate followed by `(`, which the regex
/// lookahead allows), so in a poisoned tail that definition leaks even though the
/// `\href` itself survives (the link carries its own destination). The skeleton
/// therefore exposes the link's bracketed display text — `[text]` followed by a
/// space placeholder for the consumed `(url)` — so the link-reference scan sees the
/// candidate. **Images are handled the same way:** an image `![alt](url)`'s `[alt]`
/// is a bracket-free candidate too (the `[` is preceded by `!`, allowed, and
/// followed by `(`, lookahead-allowed), so its synthesized `[alt]: R:alt`
/// definition leaks in a poisoned tail even though the `\figure` survives (it
/// carries its own destination). The node-form (`MdInlineLink`/`MdImage`) is
/// handled; an **opaque** inline-link leaf is handled too — a nested-bracket
/// display (`[a [b] c](url)`) keeps the link opaque (the lexer only nodes a
/// bracket-free display), yet `get_md_linkrefs` still finds the *inner*
/// bracket-free `[b]` candidate, so the display is exposed verbatim. Autolinks
/// (`<url>`) carry no bracket candidate, so they stay a single space.
fn inline_skeleton_fragment(inl: &Inline) -> Cow<'_, str> {
    match inl {
        Inline::Text(t) => Cow::Borrowed(t),
        Inline::MdInlineLink { display, .. } => {
            Cow::Owned(format!("[{}] ", inline_plain_text(display)))
        }
        Inline::MdImage(raw) => match image_alt_text(raw) {
            Some(alt) => Cow::Owned(format!("[{alt}] ")),
            None => Cow::Borrowed(" "),
        },
        // An opaque inline-link leaf (nested-bracket display): expose the raw
        // display so its inner bracket-free candidate surfaces, with a space
        // placeholder for the consumed `(url)`. A shortcut/reference leaf is
        // handled by demotion, an autolink carries no candidate — both a space.
        Inline::MdLink(raw) => match opaque_inline_link_display(raw) {
            Some(display) => Cow::Owned(format!("[{display}] ")),
            None => Cow::Borrowed(" "),
        },
        // A markdown list is part of the same cmark document, so an escaped-close
        // candidate (or a post-demotion literal bracket) inside a list item must
        // surface in the whole-field poisoning skeleton. Recurse into each item,
        // space-guarded per item (the raw source separates items with newlines, so
        // a `[` opening an item is never seen as preceded by the previous item's
        // `]`) — the same shape as `linkref_skeleton_push`, and the offset walk in
        // `demote_poisoned_walk` stays byte-aligned with this.
        Inline::MdList(node) => {
            let mut s = String::new();
            for item in node
                .children()
                .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
            {
                s.push(' ');
                for child in md_list_item_inlines(&item) {
                    s.push_str(&inline_skeleton_fragment(&child));
                }
            }
            s.push(' ');
            Cow::Owned(s)
        }
        Inline::MdListResolved { items, .. } => {
            let mut s = String::new();
            for item in items {
                s.push(' ');
                for child in item {
                    s.push_str(&inline_skeleton_fragment(child));
                }
            }
            s.push(' ');
            Cow::Owned(s)
        }
        _ => Cow::Borrowed(" "),
    }
}

/// The verbatim bracketed display of an **opaque inline-link** leaf
/// (`[display](url)` → `display`), or `None` for a shortcut/reference leaf (no `(`
/// after the balanced display) or an autolink (`<url>`). The display is taken
/// verbatim because it is what roxygen2's `get_md_linkrefs` scans for *inner*
/// bracket-free candidates — a nested-bracket display `[a [b] c]` yields a `[b]`
/// candidate (the outer `[a [b] c]` is not a candidate: its content has brackets).
/// Only nested-bracket displays reach here; a bracket-free inline link is a
/// `ROXYGEN_MD_LINK` *node* (`MdInlineLink`), handled above.
fn opaque_inline_link_display(raw: &str) -> Option<&str> {
    let bytes = raw.as_bytes();
    if bytes.first() == Some(&b'<') {
        return None; // autolink
    }
    let text_end = scan_delimited(bytes, 0, b'[', b']')?;
    // Inline link iff the balanced display is immediately followed by `(`.
    (bytes.get(text_end) == Some(&b'(')).then(|| &raw[1..text_end - 1])
}

/// The literal alt-text span of an image leaf (`![alt](url)` → `alt`), or `None`
/// if the leaf does not open with a balanced `![…]`. The alt is taken verbatim
/// from the source (it is what roxygen2's `get_md_linkrefs` scans for a `[alt]`
/// candidate), so it is not markdown-resolved.
fn image_alt_text(raw: &str) -> Option<&str> {
    let bytes = raw.as_bytes();
    // The leaf always begins `![`; the alt span is `[…]` starting at index 1.
    let alt_end = scan_delimited(bytes, 1, b'[', b']')?;
    Some(&raw[2..alt_end - 1])
}

/// The **leaked** link-reference definitions roxygen2's `add_linkrefs_to_md`
/// appends to a markdown field's rendered text (`markdown-link.R`).
///
/// roxygen2 scans the (double-escaped) field text with `get_md_linkrefs` and, for
/// **every** bracket-free `[…]` shortcut candidate, appends a synthesized
/// `[label]: R:URLencode(label)` reference definition so cmark resolves the
/// shortcut as a link to `R:label`. A candidate whose closing bracket is
/// **backslash-escaped** (`[text\]`) yields a definition whose own label never
/// closes — *not* a valid CommonMark link reference definition — so cmark leaves it
/// as **literal text**, and it leaks into the rendered Rd as trailing prose
/// (`… [text]: R:text%5C`). arity already renders such a shortcut literally (the
/// lexer never pairs an escaped-close bracket); this models the leaked *definition*.
///
/// **Whole-field poisoning.** The synthesized definitions are appended as one
/// block (each candidate on its own line, in source order). cmark parses it
/// top-down: a *valid* candidate's definition closes and is consumed (the shortcut
/// becomes a link, which arity resolves via its own link path). But the **first
/// invalid** (escaped-close) candidate's label never closes — it runs into the next
/// line's `[`, which is illegal inside a link label — so that definition *and every
/// definition after it* fail to parse and leak as literal text (a definition cannot
/// interrupt the paragraph the failed one started). So the leaked block begins at
/// the first invalid candidate and runs to the end, **valid candidates included**.
/// Correspondingly, a shortcut/reference link whose definition is in that leaked
/// tail is de-linked in the body — handled upstream by [`demote_poisoned_links`],
/// which rewrites those links to literal bracket text *before* the skeleton is
/// built, so they reappear here as the candidates whose definitions leak.
///
/// Returns the cmark-rendered leaked definition lines (already final text), in
/// document order. `@md` only; empty when no candidate is invalid.
fn leaked_linkref_text(source: &str) -> Vec<String> {
    let escaped = double_escape_md(source);
    let labels = md_linkref_labels(&escaped);
    let Some(first_invalid) = labels.iter().position(|label| !linkref_label_closes(label)) else {
        return Vec::new();
    };
    labels[first_invalid..]
        .iter()
        .map(|label| cmark_unescape(&format!("[{label}]: R:{}", url_encode(label))))
        .collect()
}

/// Rewrite the shortcut/reference links that a leaked link-reference block
/// de-links (see [`leaked_linkref_text`]) into literal bracket text.
///
/// roxygen2's `add_linkrefs_to_md` appends one synthesized `[label]: R:…`
/// definition per bracket-free `[…]` candidate. The **first invalid** (escaped-close)
/// candidate poisons every definition after it, so any shortcut or reference link
/// occurring after that point loses its definition and renders literally. The
/// escaped-close candidate itself is already literal text (the lexer never pairs an
/// escaped-close bracket), so it lives in a `Text` inline; once such a `Text` is
/// seen, every following shortcut/reference link node is demoted to its source
/// bracket text. Demoting *before* the skeleton is built means those links reappear
/// as candidates, so their now-leaked definitions surface naturally.
///
/// Inline links (`[text](url)`), autolinks (`<url>`), and code spans do **not**
/// need a reference definition, so they survive poisoning and are left untouched.
/// `@md` only; returns `None` (no rewrite) when the body has no invalid candidate.
fn demote_poisoned_links(body: &[Inline]) -> Option<Vec<Inline>> {
    // The poisoning boundary is found on the whole-body skeleton (an escaped-close
    // candidate can straddle several inlines — the lexer splits `[stop\]` into a
    // `Text` plus a leftover `]` delimiter), then mapped back by skeleton offset.
    // The skeleton descends into list items (see `inline_skeleton_fragment`), so
    // the demotion walk must descend identically to keep offsets aligned.
    let skeleton = inline_source_skeleton(body);
    let boundary = first_invalid_linkref_offset(&skeleton)?;
    let mut offset = 0;
    let mut changed = false;
    let out = demote_poisoned_walk(body, boundary, &mut offset, &mut changed);
    Some(relink_demoted_inline_links(out))
}

/// Recursive offset-threaded walk for [`demote_poisoned_links`]: demote every
/// shortcut/reference link whose skeleton offset starts after `boundary` to its
/// literal bracket source, descending into list items with the same per-item
/// space-guard offset accounting as [`inline_skeleton_fragment`]'s list arms (so
/// the boundary maps back consistently). `offset` advances inline-by-inline;
/// `changed` is set when any inline is rewritten. A list whose items change becomes
/// an `MdListResolved`; an untouched list keeps its opaque `MdList` form (so its
/// serialization stays byte-identical).
fn demote_poisoned_walk(
    body: &[Inline],
    boundary: usize,
    offset: &mut usize,
    changed: &mut bool,
) -> Vec<Inline> {
    let mut out = Vec::with_capacity(body.len());
    for inl in body {
        match inl {
            Inline::MdList(node) => {
                let items: Vec<Vec<Inline>> = node
                    .children()
                    .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
                    .map(|item| md_list_item_inlines(&item))
                    .collect();
                let mut item_changed = false;
                let new_items = demote_poisoned_items(&items, boundary, offset, &mut item_changed);
                if item_changed {
                    *changed = true;
                    out.push(Inline::MdListResolved {
                        ordered: md_list_is_ordered(node),
                        items: new_items,
                    });
                } else {
                    out.push(inl.clone());
                }
            }
            Inline::MdListResolved { ordered, items } => {
                let mut item_changed = false;
                let new_items = demote_poisoned_items(items, boundary, offset, &mut item_changed);
                if item_changed {
                    *changed = true;
                    out.push(Inline::MdListResolved {
                        ordered: *ordered,
                        items: new_items,
                    });
                } else {
                    out.push(inl.clone());
                }
            }
            _ => {
                let start = *offset;
                *offset += skeleton_len(inl);
                if start > boundary
                    && let Some(text) = demoted_link_source(inl)
                {
                    *changed = true;
                    out.push(Inline::Text(text));
                } else {
                    out.push(inl.clone());
                }
            }
        }
    }
    out
}

/// Walk a list's items for [`demote_poisoned_walk`], advancing `offset` with the
/// per-item leading space and overall trailing space that
/// [`inline_skeleton_fragment`]'s list arm emits, so offsets stay byte-aligned.
fn demote_poisoned_items(
    items: &[Vec<Inline>],
    boundary: usize,
    offset: &mut usize,
    item_changed: &mut bool,
) -> Vec<Vec<Inline>> {
    let mut new_items = Vec::with_capacity(items.len());
    for item in items {
        *offset += 1; // the per-item leading space guard
        new_items.push(demote_poisoned_walk(item, boundary, offset, item_changed));
    }
    *offset += 1; // the list's trailing space
    new_items
}

/// Re-form enclosing inline links that a poisoning demotion exposes.
///
/// When a nested link's inner shortcut is de-linked by poisoning (the arena
/// resolves the optimistic CommonMark structure — inner link wins, outer bracket
/// literal — so [`demote_poisoned_links`] rewrites the now-dead inner shortcut to
/// literal text), the enclosing brackets are no longer deactivated by a live inner
/// link, so roxygen2 (cmark) resolves the *outer* `[…](url)` as an inline link
/// instead. Concretely `[a [b] c](url)` parses to literal `[a `, `\link{b}`,
/// literal ` c](url)`; once `[b]` is demoted to text the whole span is consecutive
/// literal text and re-forms as `\href{url}{a [b] c}`.
///
/// Scans each maximal run of consecutive `Inline::Text` for an *unescaped*
/// `[display](url)` inline-link pattern and splits it into `Text` + `MdInlineLink`.
/// The consecutive-text constraint is what scopes this to the poisoned case: a
/// surviving inner inline link is a node that interrupts the run (so a nested
/// inline-in-inline like `[a [b](u) c](o)` keeps its outer bracket literal, exactly
/// as CommonMark does), and a non-poisoned nested link still has its inner `\link`
/// node interrupting the run. An escaped `\[` keeps its backslash in the text
/// inline (unescaping happens later in `process_prose`), so it is skipped here and
/// stays literal — `\[bracket](x)` never relinks. Shortcuts/references in a poisoned
/// tail are dead, so only inline `(url)` links re-form; the re-formed display is
/// therefore plain literal text.
fn relink_demoted_inline_links(body: Vec<Inline>) -> Vec<Inline> {
    let mut out = Vec::with_capacity(body.len());
    let mut text_run = String::new();
    for inl in body {
        match inl {
            Inline::Text(s) => text_run.push_str(&s),
            other => {
                relink_text_run(&text_run, &mut out);
                text_run.clear();
                out.push(other);
            }
        }
    }
    relink_text_run(&text_run, &mut out);
    out
}

/// Push `s` to `out`, re-forming any unescaped `[display](url)` inline link into an
/// `Inline::MdInlineLink` (display as plain text — see [`relink_demoted_inline_links`]).
fn relink_text_run(s: &str, out: &mut Vec<Inline>) {
    if s.is_empty() {
        return;
    }
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut run_start = 0;
    while i < bytes.len() {
        if bytes[i] == b'['
            && !(i > 0 && bytes[i - 1] == b'\\')
            && let Some(text_end) = scan_delimited(bytes, i, b'[', b']')
            && bytes.get(text_end) == Some(&b'(')
            && let Some(url_end) = scan_delimited(bytes, text_end, b'(', b')')
        {
            if run_start < i {
                out.push(Inline::Text(s[run_start..i].to_string()));
            }
            let display = s[i + 1..text_end - 1].to_string();
            let url = s[text_end + 1..url_end - 1].to_string();
            out.push(Inline::MdInlineLink {
                url,
                display: vec![Inline::Text(display)],
            });
            i = url_end;
            run_start = i;
            continue;
        }
        i += 1;
    }
    if run_start < s.len() {
        out.push(Inline::Text(s[run_start..].to_string()));
    }
}

/// An inline's byte length in [`inline_source_skeleton`] — the length of its
/// [`inline_skeleton_fragment`], so the poisoning-boundary offset maps back onto
/// the body inline-by-inline.
fn skeleton_len(inl: &Inline) -> usize {
    inline_skeleton_fragment(inl).len()
}

/// The literal source bracket text for a shortcut or reference link that a leaked
/// definition block de-links, or `None` for any inline that survives poisoning (an
/// inline link, autolink, code span, macro, or plain text — none of which depend on
/// a reference definition). Used by [`demote_poisoned_links`].
fn demoted_link_source(inl: &Inline) -> Option<String> {
    match inl {
        Inline::MdShortcutLink { display } => Some(format!("[{}]", link_label_text(display))),
        Inline::MdRefLink { dest, display } => {
            Some(format!("[{}][{}]", link_label_text(display), dest))
        }
        // The opaque same-line leaf is its own verbatim source; demote only the
        // shortcut/reference forms (an inline link or autolink survives).
        Inline::MdLink(raw) if opaque_link_is_shortcut_or_ref(raw) => Some(raw.clone()),
        _ => None,
    }
}

/// Whether an opaque `ROXYGEN_MD_LINK` leaf is a shortcut (`[dest]`) or reference
/// (`[text][ref]`) link — the forms that depend on a reference definition and are
/// thus de-linked by poisoning — rather than an inline link (`[text](url)`) or
/// autolink (`<url>`), which carry their own destination. Mirrors the closer
/// dispatch in [`resolve_md_link`].
fn opaque_link_is_shortcut_or_ref(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    if bytes.first() == Some(&b'<') {
        return false; // autolink
    }
    let Some(text_end) = scan_delimited(bytes, 0, b'[', b']') else {
        return false;
    };
    // `(` → inline link (own destination); `[` → reference; nothing → shortcut.
    !matches!(bytes.get(text_end), Some(&b'('))
}

/// The set of normalized link-reference labels roxygen2 **defines** for a markdown
/// field — its *link-reference map*.
///
/// roxygen2's `add_linkrefs_to_md` synthesizes a `[label]: R:…` definition for
/// every bracket-free `[…]` shortcut candidate found by `get_md_linkrefs`, and
/// cmark resolves a shortcut or reference link only when its (normalized) label is
/// one of those definitions. The candidate scan's `(?<!\])` lookbehind skips a `[`
/// immediately preceded by `]` and its `(?=[^\[{])` lookahead skips one followed by
/// `[`/`{`, so a bracketed span in those positions defines nothing — and a link
/// using such a label stays literal unless the label is *also* defined by some
/// other candidate in the same field (e.g. a standalone `[b]` elsewhere defines `b`
/// for an `a][b]`). arity's arena resolves every shortcut optimistically, so
/// [`demote_undefined_links`] uses this map to drop the ones roxygen2 leaves literal.
///
/// The map is built from a faithful reconstruction of the field's raw markdown
/// source ([`linkref_source_skeleton`]) — every link/image bracket re-exposed — so
/// the candidate scan ([`md_linkref_scan`], the same port the poisoning path uses)
/// sees what roxygen2 saw before parsing.
fn linkref_keys(body: &[Inline]) -> std::collections::HashSet<String> {
    md_linkref_scan(&linkref_source_skeleton(body))
        .into_iter()
        .map(|(label, _)| normalize_linkref_label(&label))
        .collect()
}

/// Reconstruct a markdown field's raw source from its resolved inline body,
/// re-exposing the bracket text of every link and image so the link-reference
/// candidate scan ([`md_linkref_scan`]) sees the same `[…]` spans roxygen2 scanned
/// before parsing. Unlike [`inline_source_skeleton`] — which renders a *resolved*
/// shortcut/reference link as a single space because the poisoning path handles it
/// positionally — this exposes those brackets so [`linkref_keys`] can decide which
/// labels are defined at all.
///
/// The reconstruction is faithful with respect to the candidate scan's lookbehind
/// and lookahead: a shortcut/reference link re-emits its trailing `]` (so a span
/// right after it is correctly seen as preceded by `]`), an inline link/image
/// re-emits `[display] `/`[alt] ` (a space stands in for the consumed `(url)`,
/// non-blocking like the real `)`), and emphasis children recurse between space
/// guards (the dropped `*`/`_` markers were non-blocking too). A code span and
/// other resolved inlines contribute a single space.
fn linkref_source_skeleton(body: &[Inline]) -> String {
    let mut s = String::new();
    for inl in body {
        linkref_skeleton_push(inl, &mut s);
    }
    s
}

fn linkref_skeleton_push(inl: &Inline, s: &mut String) {
    match inl {
        Inline::Text(t) => s.push_str(t),
        Inline::MdShortcutLink { display } => {
            s.push('[');
            s.push_str(&link_label_text(display));
            s.push(']');
        }
        Inline::MdRefLink { dest, display } => {
            s.push('[');
            s.push_str(&link_label_text(display));
            s.push_str("][");
            s.push_str(dest);
            s.push(']');
        }
        Inline::MdInlineLink { display, .. } => {
            s.push('[');
            s.push_str(&inline_plain_text(display));
            s.push_str("] ");
        }
        Inline::MdImage(raw) => match image_alt_text(raw) {
            Some(alt) => {
                s.push('[');
                s.push_str(alt);
                s.push_str("] ");
            }
            None => s.push(' '),
        },
        // An opaque leaf is its own verbatim source — a same-line shortcut/reference
        // (`[*foo*]`/`[t][r]`), a nested-bracket inline link (whose inner `[b]` is a
        // candidate), or an autolink (no bracket). All are faithful as-is.
        Inline::MdLink(raw) => s.push_str(raw),
        Inline::MdEmphasis { children, .. } => {
            s.push(' ');
            for child in children {
                linkref_skeleton_push(child, s);
            }
            s.push(' ');
        }
        // A markdown list is part of the same cmark document, so its items'
        // brackets are link-reference candidates for the whole-field refmap (a
        // label defined inside a list resolves a reference anywhere in the field,
        // and vice versa). Each item is space-guarded — the raw source separates
        // items with newlines, so a `[` opening an item is never seen as preceded
        // by the previous item's closing `]`.
        Inline::MdList(node) => {
            for item in node
                .children()
                .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
            {
                s.push(' ');
                for child in md_list_item_inlines(&item) {
                    linkref_skeleton_push(&child, s);
                }
            }
            s.push(' ');
        }
        Inline::MdListResolved { items, .. } => {
            for item in items {
                s.push(' ');
                for child in item {
                    linkref_skeleton_push(child, s);
                }
            }
            s.push(' ');
        }
        _ => s.push(' '),
    }
}

/// Normalize a link-reference label for matching, mirroring CommonMark's
/// case-insensitive, whitespace-collapsing comparison: trim, fold internal
/// whitespace runs to one space, and lowercase. (Full Unicode case-folding is
/// approximated by `to_lowercase`; sufficient for the labels roxygen2 produces.)
fn normalize_linkref_label(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The resolution label of a shortcut or reference link — the label that must be
/// defined in the field's link-reference map for the link to resolve — or `None`
/// for any inline that does not depend on a reference definition (an inline link or
/// autolink carries its own destination; text/code/macros are not links). For a
/// reference link the label is the `[ref]` topic; for a shortcut it is the display.
fn link_ref_label(inl: &Inline) -> Option<String> {
    match inl {
        Inline::MdShortcutLink { display } => Some(link_label_text(display)),
        Inline::MdRefLink { dest, .. } => Some(dest.clone()),
        Inline::MdLink(raw) => opaque_link_ref_label(raw),
        _ => None,
    }
}

/// The resolution label of an *opaque* shortcut/reference `ROXYGEN_MD_LINK` leaf
/// (`[dest]` → `dest`, `[text][ref]` → `ref`), or `None` for an inline link
/// (own destination) or autolink. Mirrors the closer dispatch in [`resolve_md_link`].
fn opaque_link_ref_label(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    if bytes.first() == Some(&b'<') {
        return None; // autolink
    }
    let text_end = scan_delimited(bytes, 0, b'[', b']')?;
    match bytes.get(text_end) {
        Some(&b'(') => None, // inline link — own destination
        Some(&b'[') => {
            let ref_end = scan_delimited(bytes, text_end, b'[', b']')?;
            Some(raw[text_end + 1..ref_end - 1].to_string())
        }
        _ => Some(raw[1..text_end - 1].to_string()), // shortcut
    }
}

/// Demote shortcut/reference links whose label is absent from the field's
/// link-reference map (`keys`) to their literal bracket source — roxygen2 leaves
/// such links unresolved (see [`linkref_keys`]). Returns `None` when nothing is
/// demoted (the common case, so the body is reused unchanged). `@md` only.
fn demote_undefined_links(
    body: &[Inline],
    keys: &std::collections::HashSet<String>,
) -> Option<Vec<Inline>> {
    let mut changed = false;
    let out: Vec<Inline> = body
        .iter()
        .map(|inl| {
            if let Some(label) = link_ref_label(inl)
                && !keys.contains(&normalize_linkref_label(&label))
                && let Some(text) = demoted_link_source(inl)
            {
                changed = true;
                return Inline::Text(text);
            }
            // Descend into list items: a referencing link inside a list item is
            // checked against the same whole-field refmap (`keys`), since roxygen2
            // runs the entire tag value through cmark as one document. A list whose
            // items actually changed becomes an `MdListResolved` carrying the
            // rewritten runs; an untouched list keeps its opaque form (byte-identical
            // serialization). Mirrors the descent in `apply_user_linkrefs`.
            match demote_undefined_in_list(inl, keys) {
                Some(resolved) => {
                    changed = true;
                    resolved
                }
                None => inl.clone(),
            }
        })
        .collect();
    changed.then_some(out)
}

/// If `inl` is a markdown list (opaque `MdList` or already-`MdListResolved`),
/// run undefined-label demotion over each item against the whole-field refmap
/// `keys`, returning a rewritten `Inline::MdListResolved` when any item changed
/// (else `None`). Not a list, or no change ⇒ `None`. Helper for
/// [`demote_undefined_links`].
fn demote_undefined_in_list(
    inl: &Inline,
    keys: &std::collections::HashSet<String>,
) -> Option<Inline> {
    let (ordered, items): (bool, Vec<Vec<Inline>>) = match inl {
        Inline::MdList(node) => (
            md_list_is_ordered(node),
            node.children()
                .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
                .map(|item| md_list_item_inlines(&item))
                .collect(),
        ),
        Inline::MdListResolved { ordered, items } => (*ordered, items.clone()),
        _ => return None,
    };
    let mut new_items = Vec::with_capacity(items.len());
    let mut item_changed = false;
    for item in &items {
        match demote_undefined_links(item, keys) {
            Some(rewritten) => {
                new_items.push(rewritten);
                item_changed = true;
            }
            None => new_items.push(item.clone()),
        }
    }
    item_changed.then_some(Inline::MdListResolved {
        ordered,
        items: new_items,
    })
}

/// Collect user-written CommonMark link-reference definitions (`[ref]: url`) across
/// the **whole field**, recursing into list items, into a global (normalized-label →
/// destination) map. roxygen2 runs the entire tag value through cmark as one
/// document, so a definition in any paragraph or list item resolves a referencing
/// link anywhere else in the field. First definition of a label wins (cmark),
/// approximated by `or_insert` over a top-level-then-descend walk (a duplicate label
/// split across a list and prose — vanishingly rare — is the only case where strict
/// document order would differ; backlog).
fn collect_user_linkrefs_tree(
    body: &[Inline],
    urls: &mut std::collections::HashMap<String, String>,
) {
    let (level, _dropped) = collect_user_linkrefs(body);
    for (label, url) in level {
        urls.entry(label).or_insert(url);
    }
    for inl in body {
        match inl {
            Inline::MdList(node) => {
                for item in node
                    .children()
                    .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
                {
                    collect_user_linkrefs_tree(&md_list_item_inlines(&item), urls);
                }
            }
            Inline::MdListResolved { items, .. } => {
                for item in items {
                    collect_user_linkrefs_tree(item, urls);
                }
            }
            _ => {}
        }
    }
}

/// Apply the field's global user link-reference map (`urls`, from
/// [`collect_user_linkrefs_tree`]) to a body, recursing into list items: consume
/// definition runs (they render nothing) and rewrite each referencing shortcut or
/// reference link whose label is defined to an [`Inline::MdInlineLink`] — so an
/// `[*foo*][r1]` with a `[r1]: url` definition renders `\href{url}{\emph{foo}}`
/// (display **kept**, unlike the R-topic `\link` path that drops a non-plain
/// display). The user definition wins over roxygen2's synthesized `[r1]: R:r1`
/// because cmark keeps the first definition and the synthesized block is appended
/// last.
///
/// Returns `None` when nothing in this subtree changed, so a list with no
/// link-reference work keeps its opaque [`Inline::MdList`] form (and thus its
/// byte-identical serialization); a list that *did* change becomes an
/// [`Inline::MdListResolved`] carrying its rewritten items.
///
/// Scope (this slice): single-`Text`-node definitions at a true block start (a
/// definition cannot interrupt a paragraph), bare or `<…>` destinations, an optional
/// same-line title — now resolved across paragraphs and list items of the same
/// field. Multi-line definitions, titles spanning lines, and URL normalization
/// (percent-encoding, entities) are backlog.
fn apply_user_linkrefs(
    body: &[Inline],
    urls: &std::collections::HashMap<String, String>,
) -> Option<Vec<Inline>> {
    let (_, dropped) = collect_user_linkrefs(body);
    let mut out = Vec::with_capacity(body.len());
    let mut changed = !dropped.is_empty();
    for (i, inl) in body.iter().enumerate() {
        if dropped.contains(&i) {
            continue;
        }
        if let Some(label) = link_ref_label(inl)
            && let Some(url) = urls.get(&normalize_linkref_label(&label))
            && let Some(display) = link_display_inlines(inl)
        {
            out.push(Inline::MdInlineLink {
                url: url.clone(),
                display,
            });
            changed = true;
            continue;
        }
        if let Inline::MdList(node) = inl {
            let items: Vec<Vec<Inline>> = node
                .children()
                .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
                .map(|item| md_list_item_inlines(&item))
                .collect();
            let mut new_items = Vec::with_capacity(items.len());
            let mut item_changed = false;
            for item in &items {
                match apply_user_linkrefs(item, urls) {
                    Some(rewritten) => {
                        new_items.push(rewritten);
                        item_changed = true;
                    }
                    None => new_items.push(item.clone()),
                }
            }
            if item_changed {
                out.push(Inline::MdListResolved {
                    ordered: md_list_is_ordered(node),
                    items: new_items,
                });
                changed = true;
                continue;
            }
        }
        out.push(inl.clone());
    }
    changed.then_some(out)
}

/// Scan a field body for user link-reference definitions, returning the
/// (normalized-label → destination) map and the set of body indices that are part
/// of a definition (and so must be dropped from the rendered output). A definition
/// run is consumed only at a *block start* (the body start, or right after a `Text`
/// containing a newline — a paragraph break): a definition cannot interrupt a
/// paragraph (CommonMark). Within a run, consecutive definitions are separated by a
/// whitespace-only `Text` (a soft line break), which is also dropped.
fn collect_user_linkrefs(
    body: &[Inline],
) -> (
    std::collections::HashMap<String, String>,
    std::collections::BTreeSet<usize>,
) {
    let mut urls: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut dropped = std::collections::BTreeSet::new();
    let mut i = 0;
    let mut block_start = true;
    while i < body.len() {
        if block_start && let Some(end) = scan_linkref_run(body, i, &mut urls, &mut dropped) {
            i = end;
            // The remainder of this block (if any) is prose, not definitions.
            block_start = false;
            continue;
        }
        block_start = matches!(&body[i], Inline::Text(t) if t.contains('\n'));
        i += 1;
    }
    (urls, dropped)
}

/// Consume a run of consecutive link-reference definitions beginning at a block
/// start (`start`), recording each into `urls` (first definition of a label wins,
/// per cmark) and its inlines into `dropped`. Tolerates CommonMark's
/// leading-whitespace indentation and the whitespace-only soft breaks that separate
/// stacked definitions (both dropped). Returns the exclusive end index of the run,
/// or `None` when no definition begins the block. Whitespace *after* the last
/// definition is left untouched (it belongs to the following prose).
fn scan_linkref_run(
    body: &[Inline],
    start: usize,
    urls: &mut std::collections::HashMap<String, String>,
    dropped: &mut std::collections::BTreeSet<usize>,
) -> Option<usize> {
    let mut end = start;
    let mut any = false;
    loop {
        // Skip whitespace-only (non-paragraph-break) Text — leading indentation or a
        // soft break between definitions.
        let mut k = end;
        while let Some(Inline::Text(t)) = body.get(k) {
            if t.is_empty() || t.contains('\n') || !t.chars().all(char::is_whitespace) {
                break;
            }
            k += 1;
        }
        let Some((label, url, def_end)) = match_linkref_def(body, k) else {
            break;
        };
        urls.entry(normalize_linkref_label(&label)).or_insert(url);
        for idx in end..def_end {
            dropped.insert(idx);
        }
        any = true;
        end = def_end;
    }
    any.then_some(end)
}

/// Match a single link-reference definition at body index `j`: a shortcut-shaped
/// label link (`[label]`) followed by `Text` of the form `: <destination> [title]`.
/// The destination (and optional title) may continue across soft line breaks, so
/// the trailing `Text` run is concatenated — joining continuation lines — up to (but
/// not including) a paragraph break (a `Text` containing `\n`) or a non-`Text`
/// inline. Returns `(label, destination, def_end)` where `def_end` is the exclusive
/// body index just past the consumed definition, or `None` when the shape does not
/// hold.
fn match_linkref_def(body: &[Inline], j: usize) -> Option<(String, String, usize)> {
    let label = linkref_def_label(body.get(j)?)?;
    let mut text = String::new();
    let mut k = j + 1;
    while let Some(Inline::Text(t)) = body.get(k) {
        if t.contains('\n') {
            break; // a paragraph break ends the definition's block
        }
        text.push_str(t);
        k += 1;
    }
    if k == j + 1 {
        return None; // no trailing text — not a definition
    }
    let (url, line_closed) = parse_linkref_def_dest(&text)?;
    // The `Text` run stops at the first non-`Text` inline (a macro, an inline link,
    // the next definition's label, …). When that inline sits on the *same physical
    // line* as the destination, it is trailing content — CommonMark forbids anything
    // but whitespace after the destination (and optional title), so this is not a
    // definition (e.g. `[foo]: url \emph{bar}`). But when a line boundary (a
    // `SOFT_BREAK`) already closed the destination's line, the inline begins a new
    // block (the next stacked `[r2]: …` definition, say) and is fine. Only end-of-body
    // or a paragraph break (a `Text` carrying `\n`) may otherwise follow.
    if !line_closed && !matches!(body.get(k), None | Some(Inline::Text(_))) {
        return None;
    }
    Some((label, url, k))
}

/// The label of a shortcut-shaped link (`[label]`) — the form a link-reference
/// definition's leading token takes (a `[label]` followed by `:` is a shortcut, not
/// a reference or inline link). `None` for any other inline.
fn linkref_def_label(inl: &Inline) -> Option<String> {
    match inl {
        Inline::MdShortcutLink { display } => Some(inline_plain_text(display)),
        Inline::MdLink(raw) => {
            let bytes = raw.as_bytes();
            if bytes.first() != Some(&b'[') {
                return None;
            }
            let end = scan_delimited(bytes, 0, b'[', b']')?;
            (end == bytes.len()).then(|| raw[1..end - 1].to_string())
        }
        _ => None,
    }
}

/// Parse a link-reference definition's destination from the `Text` that follows the
/// label (`: <destination> [title]`). The destination is angle-bracketed (`<…>`,
/// brackets stripped) or a non-whitespace run; an optional title (`"…"`, `'…'`, or
/// `(…)`) may follow. Returns `(destination, line_closed)` — where `line_closed`
/// reports whether a line boundary ([`SOFT_BREAK`]) follows the destination/title
/// (so a subsequent inline begins a *new* block, not trailing content) — or `None`
/// when the text is not a clean single-node definition (trailing non-title content
/// makes it a paragraph, not a definition). `text` never carries `\n` (the caller's
/// loop stops at a paragraph break), so a line boundary here is always a soft wrap.
fn parse_linkref_def_dest(text: &str) -> Option<(String, bool)> {
    let rest = text.strip_prefix(':')?.trim_start();
    let (url, after) = if let Some(r) = rest.strip_prefix('<') {
        let close = r.find('>')?;
        (r[..close].to_string(), &r[close + 1..])
    } else {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        (rest[..end].to_string(), &rest[end..])
    };
    if url.is_empty() {
        return None;
    }
    // cmark entity-decodes link destinations (`&amp;` → `&`), so a defined href
    // carries the decoded URL. (The destination is otherwise verbatim — no
    // percent re-encoding.)
    let url = decode_html_entities(&url);
    if after.trim_start().is_empty() {
        return Some((url, after.contains(SOFT_BREAK)));
    }
    // An optional title; anything else means this is not a valid definition.
    let after = after.trim_start();
    let close = match after.as_bytes()[0] {
        b'"' => '"',
        b'\'' => '\'',
        b'(' => ')',
        _ => return None,
    };
    let title_rest = &after[1..];
    let end = title_rest.find(close)?;
    let residual = &title_rest[end + 1..];
    residual
        .trim()
        .is_empty()
        .then(|| (url.clone(), residual.contains(SOFT_BREAK)))
}

/// Decode the HTML character references cmark resolves: every semicolon-terminated
/// HTML5 named entity (`&amp;`, `&copy;`, `&hellip;`, …) and numeric references
/// (`&#NN;`, `&#xHH;`). CommonMark requires the trailing `;`, so a bare `&amp` (no
/// semicolon) is left verbatim, as is an unrecognized name (`&nope;`). The fast
/// path returns the input unchanged when it has no `&`, so text without entities is
/// byte-identical. Used both for a markdown link destination and, via
/// [`process_prose`], for `@md` prose text (cmark decodes entities everywhere
/// except code spans/blocks, which the projector keeps as separate verbatim leaves).
fn decode_html_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s.as_bytes()[i] == b'&'
            && let Some(rel) = s[i + 1..].find(';')
            && decode_entity(&s[i + 1..i + 1 + rel], &mut out)
        {
            i += 1 + rel + 1;
            continue;
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Resolve one character-reference body (the text between `&` and `;`), appending
/// its replacement to `out` and returning `true` on success. A named entity is
/// looked up in the full HTML5 table ([`entities::HTML5_ENTITIES`]); a numeric
/// reference decodes its decimal (`#NN`) or hexadecimal (`#xHH`) code point, mapping
/// U+0000, a surrogate, or an out-of-range value to the replacement character
/// U+FFFD (cmark's rule). Returns `false` for an unrecognized name or a malformed
/// numeric body, leaving the source `&…;` verbatim.
fn decode_entity(body: &str, out: &mut String) -> bool {
    if let Some(num) = body.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => num.parse::<u32>().ok(),
        };
        let Some(code) = code else { return false };
        out.push(
            char::from_u32(code)
                .filter(|&c| c != '\0')
                .unwrap_or('\u{FFFD}'),
        );
        return true;
    }
    match entities::HTML5_ENTITIES.binary_search_by_key(&body, |&(name, _)| name) {
        Ok(idx) => {
            out.push_str(entities::HTML5_ENTITIES[idx].1);
            true
        }
        Err(_) => false,
    }
}

/// The display inlines of a referencing link — what `\href{url}{…}` renders when the
/// link's label resolves to a user destination. For a shortcut/reference *node* this
/// is the resolved `display`; for an opaque shortcut/reference leaf the bracketed
/// text becomes one `Text` (plain by construction — marked-up displays nodeify).
/// `None` for an inline link or autolink (own destination, not reference-resolved).
fn link_display_inlines(inl: &Inline) -> Option<Vec<Inline>> {
    match inl {
        Inline::MdShortcutLink { display } | Inline::MdRefLink { display, .. } => {
            Some(display.clone())
        }
        Inline::MdLink(raw) => {
            let bytes = raw.as_bytes();
            if bytes.first() == Some(&b'<') {
                return None; // autolink
            }
            let text_end = scan_delimited(bytes, 0, b'[', b']')?;
            match bytes.get(text_end) {
                Some(&b'(') => None, // inline link — own destination
                _ => Some(vec![Inline::Text(raw[1..text_end - 1].to_string())]),
            }
        }
        _ => None,
    }
}

/// roxygen2's `double_escape_md` (`markdown-link.R`): double every backslash, then
/// revert the two bracket escapes (`\\[`→`\[`, `\\]`→`\]`) — so only a bracket
/// escape survives cmark, every other punctuation escape is neutralized. The
/// `gsub(fixed = TRUE)` passes are non-overlapping left-to-right, matching
/// `str::replace`.
fn double_escape_md(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace("\\\\[", "\\[")
        .replace("\\\\]", "\\]")
}

/// Scan double-escaped markdown text for `get_md_linkrefs` shortcut candidates,
/// returning each candidate's reference **label** (`refs[,3]`: the second `[…]`
/// group if present, else the first). Ports roxygen2's `get_md_linkrefs` regex
/// (`markdown-link.R`): a bracket-free `[content]`, optionally followed by a
/// bracket-free `[ref]`, **not** preceded by `]`/`\` and **not** followed by
/// `[`/`{`. Matches are non-overlapping, left to right.
fn md_linkref_labels(text: &str) -> Vec<String> {
    md_linkref_scan(text).into_iter().map(|(l, _)| l).collect()
}

/// Scan `text` for `get_md_linkrefs` candidates, returning each candidate's
/// reference **label** (see [`md_linkref_labels`]) paired with the **byte offset of
/// its opening `[`**. The position lets the poisoning boundary
/// ([`first_invalid_linkref_offset`]) map back into the body skeleton.
fn md_linkref_scan(text: &str) -> Vec<(String, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // Lookbehind: a `[` not preceded by `]` or `\`.
        if bytes[i] != b'[' || (i > 0 && matches!(bytes[i - 1], b']' | b'\\')) {
            i += 1;
            continue;
        }
        let Some((content, content_end)) = bracket_free_group(bytes, i) else {
            i += 1;
            continue;
        };
        // Optional second `[ref]` (a non-empty bracket-free group right after).
        let (label, match_end) = match bracket_free_group(bytes, content_end) {
            Some((reff, ref_end)) => (reff, ref_end),
            None => (content, content_end),
        };
        // Lookahead: not immediately followed by `[` or `{`.
        if matches!(bytes.get(match_end), Some(b'[' | b'{')) {
            i += 1;
            continue;
        }
        out.push((String::from_utf8_lossy(label).into_owned(), i));
        i = match_end;
    }
    out
}

/// The byte offset (in `skeleton`, the body's reconstructed markdown source) of the
/// opening `[` of the first **invalid** (escaped-close) link-reference candidate, or
/// `None` if every candidate closes. This is where leaked-definition poisoning
/// begins — every shortcut/reference link after it is de-linked (see
/// [`demote_poisoned_links`]). The skeleton carries raw (single) backslashes, so a
/// candidate is invalid exactly when its label ends with a backslash:
/// `double_escape_md` turns any non-empty trailing backslash run into an odd run
/// (`2k-1`) that fails to close (`linkref_label_closes`), so any trailing backslash
/// poisons — matching the escaped-label classification the leak itself uses.
fn first_invalid_linkref_offset(skeleton: &str) -> Option<usize> {
    md_linkref_scan(skeleton)
        .into_iter()
        .find(|(label, _)| label.ends_with('\\'))
        .map(|(_, start)| start)
}

/// If `bytes[open]` is `[`, return the bracket-free content (`[^\]\[]+`, ≥1 byte,
/// no interior `[`/`]`) and the index just past its closing `]`. `None` when there
/// is no such group (empty content, an interior `[`, or no closing `]`).
fn bracket_free_group(bytes: &[u8], open: usize) -> Option<(&[u8], usize)> {
    if bytes.get(open) != Some(&b'[') {
        return None;
    }
    let start = open + 1;
    let mut j = start;
    while j < bytes.len() && !matches!(bytes[j], b'[' | b']') {
        j += 1;
    }
    (bytes.get(j) == Some(&b']') && j > start).then_some((&bytes[start..j], j + 1))
}

/// Whether the synthesized definition `[label]: …` is a valid CommonMark link
/// reference definition (its label's closing `]` is *not* backslash-escaped). The
/// label is bracket-free, so the only failure is a trailing odd run of backslashes
/// escaping the `]`. A valid definition is consumed by cmark (the shortcut becomes
/// a link, handled by arity's own link path); an invalid one leaks.
fn linkref_label_closes(label: &str) -> bool {
    label.bytes().rev().take_while(|&b| b == b'\\').count() % 2 == 0
}

/// R's `URLencode(x, reserved = FALSE)`: keep ASCII alphanumerics and the
/// unreserved/sub-delim set, percent-encode every other byte as `%XX`
/// (uppercase). Matches roxygen2's `map_chr(refs, URLencode)` for the synthesized
/// `R:label` destinations (e.g. `\`→`%5C`, space→`%20`).
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'!' | b'#'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b'-'
                    | b'.'
                    | b'/'
                    | b':'
                    | b';'
                    | b'='
                    | b'?'
                    | b'@'
                    | b'['
                    | b']'
                    | b'_'
                    | b'~'
            )
        {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Resolve CommonMark backslash escapes in `s`: a `\` before an ASCII-punctuation
/// char is dropped (the char stays literal); any other `\` is kept. Renders the
/// *leaked* (invalid) link-reference definition the way cmark renders it as
/// paragraph text (`[text\]: R:text%5C` → `[text]: R:text%5C`).
fn cmark_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek().is_some_and(char::is_ascii_punctuation) {
            out.push(chars.next().expect("peeked punctuation"));
        } else {
            out.push(c);
        }
    }
    out
}

/// Append already-rendered prose `extra` to a section's atom list, coalescing it
/// into a trailing `(TEXT …)` atom (the way roxygen2's appended link-ref text
/// coalesces with the section's prose under the canonical TEXT-run merge), or
/// pushing a fresh `(TEXT …)` when the last atom is not prose.
fn append_rendered_text(atoms: &mut Vec<String>, extra: &str) {
    if let Some(last) = atoms.last_mut()
        && let Some(text) = decode_text_atom(last)
        && let Some(merged) = text_atom(&format!("{text} {extra}"))
    {
        *last = merged;
        return;
    }
    if let Some(atom) = text_atom(extra) {
        atoms.push(atom);
    }
}

/// Reverse [`encode_text`] for a `(TEXT "…")` atom, returning the decoded inner
/// string, or `None` if `atom` is not a text atom.
fn decode_text_atom(atom: &str) -> Option<String> {
    let inner = atom.strip_prefix("(TEXT \"")?.strip_suffix("\")")?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some(other) => out.push(other), // `\\` → `\`, `\"` → `"`
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// Strip Rd `%` line comments from literal-Rd prose: on each physical line, an
/// unescaped `%` (one not preceded by a `\`) begins a comment that runs to the end
/// of that line. Lines are rejoined with `\n` (collapsed downstream by `norm_ws`).
fn strip_rd_comments(s: &str) -> String {
    physical_lines(s)
        .map(strip_rd_line_comment)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The prefix of `line` before its first unescaped `%` (the whole line if none).
fn strip_rd_line_comment(line: &str) -> &str {
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '%' {
            return &line[..i];
        }
    }
    line
}

/// The verbatim `(RCODE …)` atoms for a `\code` body. parse_Rd keeps `\code`
/// content verbatim (no whitespace collapse) but splits it at newlines, attaching
/// each `\n` to the atom it ends (`\code{a\nb}` → `(RCODE "a\n") (RCODE "b")`). An
/// empty body yields no atom.
fn rcode_atoms(body: &str) -> Vec<String> {
    let mut atoms = Vec::new();
    let mut rest = body;
    while let Some(idx) = rest.find('\n') {
        let (seg, tail) = rest.split_at(idx + 1);
        atoms.push(format!("(RCODE {})", encode_text(seg)));
        rest = tail;
    }
    if !rest.is_empty() {
        atoms.push(format!("(RCODE {})", encode_text(rest)));
    }
    atoms
}

/// Collapse every whitespace run to a single space and trim (the R `norm_ws`,
/// `gsub("[[:space:]]+", " ")` then `trimws`).
///
/// The R driver's `[[:space:]]` is **ASCII-only** even in a UTF-8 locale, so
/// non-ASCII Unicode whitespace (NBSP `U+00A0`, NEL `U+0085`, the `Zs`
/// separators) is *preserved verbatim*, never collapsed. This matters for
/// flanking-rejected emphasis: `*\u{a0}a\u{a0}*` stays the literal text
/// `*\u{a0}a\u{a0}*` (a NBSP can't flank, so no `\emph`), and the NBSP must
/// survive the projection. Rust's `split_whitespace`/`char::is_whitespace` is
/// Unicode-aware and would wrongly fold NBSP to a plain space, so we classify
/// against the ASCII POSIX-space set instead.
fn norm_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.chars() {
        if is_posix_space(c) {
            pending_space = true;
        } else {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(c);
        }
    }
    out
}

/// The C-locale POSIX `[[:space:]]` set: space, tab, newline, vertical tab,
/// form feed, and carriage return. ASCII-only by design (see `norm_ws`).
fn is_posix_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r')
}

/// A sentinel marking a **physical soft-wrap** line break inside a prose run: the
/// point where one `#'` source line ends and the next continues the *same*
/// roxygen2 paragraph. It is distinct from a paragraph break (`\n`) on purpose.
/// An Rd `%` comment (and the `@md` `%`-swallow) ends at the *physical* source
/// line, so [`strip_rd_comments`]/[`md_percent_swallow`] must stop at a
/// `SOFT_BREAK` just as they stop at a `\n`; but the link-reference block
/// machinery ([`collect_user_linkrefs`]/[`scan_linkref_run`]) treats **only** a
/// `\n` as a paragraph break — a definition may not interrupt a soft-wrapped
/// paragraph. `SOFT_BREAK` is ASCII whitespace (so `norm_ws` collapses it to a
/// single space; a soft-wrapped paragraph with no comment renders identically)
/// but is not `\n` (so it never reads as a paragraph break).
const SOFT_BREAK: char = '\u{c}'; // form feed

/// Split a prose run at every **physical** source-line boundary — a paragraph
/// break (`\n`) or a soft-wrap ([`SOFT_BREAK`]). Used by the `%`-comment
/// strippers, whose comments end at the physical line.
fn physical_lines(run: &str) -> impl Iterator<Item = &str> {
    run.split(['\n', SOFT_BREAK])
}

/// Escape a string the way the R driver's `encode_text` does (`\`, `"`, `\n`).
fn encode_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The body parts of a section, grouped into roxygen2 *paragraphs* (its blank-
/// line-delimited prose blocks), excluding its `@tag` heading. roxygen2 splits
/// the section text on `\n\n`, so a block macro or markdown list that directly
/// follows a prose line — with no blank `#'` line between — belongs to the same
/// paragraph as that prose; a blank `#'` line (a *section-level* `ROXYGEN_MARKER`,
/// as opposed to the per-line markers nested inside each node) starts a new
/// paragraph. Each returned `Vec<Inline>` is one such paragraph: a prose
/// `ROXYGEN_PARAGRAPH` contributes its inline run, a block `ROXYGEN_RD_MACRO`
/// (a multi-line `\itemize`/`\describe`/…) an `Inline::Macro`, and a
/// `ROXYGEN_MD_LIST` an `Inline::MdList`, with adjacent nodes joined by a space.
fn section_body_parts(section: &RoxygenSection) -> Vec<Vec<Inline>> {
    let mut groups: Vec<Vec<Inline>> = Vec::new();
    let mut cur: Vec<Inline> = Vec::new();
    for el in section.syntax().children_with_tokens() {
        match el.kind() {
            // An ATX heading is a structural break: it ends the current part and
            // stands alone as its own part (a single `Inline::MdHeading` marker), so
            // the description/details outline builder can split cleanly on it.
            SyntaxKind::ROXYGEN_MD_HEADING => {
                let Some(node) = el.into_node() else { continue };
                if !cur.is_empty() {
                    groups.push(std::mem::take(&mut cur));
                }
                groups.push(vec![Inline::MdHeading(node)]);
            }
            SyntaxKind::ROXYGEN_PARAGRAPH
            | SyntaxKind::ROXYGEN_RD_MACRO
            | SyntaxKind::ROXYGEN_MD_LIST
            | SyntaxKind::ROXYGEN_MD_CODE_BLOCK
            | SyntaxKind::ROXYGEN_MD_INDENTED_CODE
            | SyntaxKind::ROXYGEN_MD_HTML_BLOCK
            | SyntaxKind::ROXYGEN_MD_BLOCK_QUOTE
            | SyntaxKind::ROXYGEN_MD_TABLE => {
                let Some(node) = el.into_node() else { continue };
                let kind = node.kind();
                let inlines = match kind {
                    SyntaxKind::ROXYGEN_PARAGRAPH => RoxygenParagraph::cast(node)
                        .map(|p| paragraph_inlines(&p))
                        .unwrap_or_default(),
                    SyntaxKind::ROXYGEN_MD_LIST => vec![Inline::MdList(node)],
                    SyntaxKind::ROXYGEN_MD_CODE_BLOCK => vec![Inline::MdCodeBlock(node)],
                    SyntaxKind::ROXYGEN_MD_INDENTED_CODE => vec![Inline::MdIndentedCode(node)],
                    SyntaxKind::ROXYGEN_MD_HTML_BLOCK => vec![Inline::MdHtmlBlock(node)],
                    SyntaxKind::ROXYGEN_MD_BLOCK_QUOTE => vec![Inline::MdBlockQuote(node)],
                    SyntaxKind::ROXYGEN_MD_TABLE => vec![Inline::MdTable(node)],
                    _ => vec![Inline::Macro(node)],
                };
                // A block quote carries no separator: roxygen2 flattens it with no
                // surrounding paragraph break, so its text glues onto the preceding
                // node (`before` + `> q` → `beforeq`). Every other node joins with a
                // space (a roxygen paragraph break, collapsed by `norm_ws`).
                if !cur.is_empty() && kind != SyntaxKind::ROXYGEN_MD_BLOCK_QUOTE {
                    cur.push(Inline::Text(" ".to_string()));
                }
                cur.extend(inlines);
            }
            // A thematic break (`***`/`---`/`___`) renders empty in roxygen2 (it has
            // no thematic-break support and emits `escape_comment(xml_text)` = ""), so
            // it contributes nothing. It still separates roxygen paragraphs, so it
            // ends the current part the way a blank line does.
            SyntaxKind::ROXYGEN_MD_THEMATIC_BREAK if !cur.is_empty() => {
                groups.push(std::mem::take(&mut cur));
            }
            // A section-level `#'` marker is a blank doc-comment line: it ends the
            // current paragraph (per-line markers live *inside* the nodes above).
            SyntaxKind::ROXYGEN_MARKER if !cur.is_empty() => {
                groups.push(std::mem::take(&mut cur));
            }
            _ => {}
        }
    }
    if !cur.is_empty() {
        groups.push(cur);
    }
    groups
}

/// The inline elements of a prose paragraph: its text and inline Rd-macro
/// content, with the threaded `#'` markers dropped and inter-line newlines turned
/// into a joining space (continuation lines fold into one run). An Rd macro
/// becomes an `Inline::Macro`; all other content (plain text and — in the absence
/// of resolved markdown — inline code and link spans, which are literal Rd prose)
/// becomes `Inline::Text`. Whitespace is collapsed downstream by `norm_ws`.
fn paragraph_inlines(para: &RoxygenParagraph) -> Vec<Inline> {
    let mut out = Vec::new();
    for el in para.syntax().children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_MARKER => {} // trivia: never prose
            SyntaxKind::NEWLINE => out.push(Inline::Text(SOFT_BREAK.to_string())), // line join
            _ => push_inline(&mut out, el),
        }
    }
    out
}

/// The inline elements of a tag line: everything after the `@`, the tag name, and
/// an arg-bearing tag's argument (and the leading whitespace before the prose).
///
/// A tag with a same-line prose value folds its contiguous plain-prose
/// continuation lines into the tag node (see `emit_tag_line`), so the tag may
/// carry the threaded `#'` markers and inter-line newlines of those continuations
/// — dropped and turned into a joining soft break exactly as `paragraph_inlines`
/// does, so the folded value reads as one run (`@details *a` \n `b*` →
/// `\emph{a b}`).
fn tag_inlines(tag: &RoxygenTag) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut seen_prose = false;
    for el in tag.syntax().children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_AT | SyntaxKind::ROXYGEN_TAG_NAME | SyntaxKind::ROXYGEN_TAG_ARG => {
                continue;
            }
            // A threaded continuation marker is trivia (never prose); an inter-line
            // newline joins the continuation into one run (norm_ws collapses the
            // soft break, which still bounds a non-markdown `%` comment).
            SyntaxKind::ROXYGEN_MARKER => {}
            SyntaxKind::NEWLINE => {
                if seen_prose {
                    out.push(Inline::Text(SOFT_BREAK.to_string()));
                }
            }
            SyntaxKind::WHITESPACE => {
                if seen_prose {
                    push_inline(&mut out, el);
                }
            }
            _ => {
                seen_prose = true;
                push_inline(&mut out, el);
            }
        }
    }
    out
}

/// Append `el` to an inline run: a macro node as `Inline::Macro`, anything else
/// as `Inline::Text` of its source text.
fn push_inline(out: &mut Vec<Inline>, el: NodeOrToken<SyntaxNode, crate::syntax::SyntaxToken>) {
    match el {
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_RD_MACRO => {
            out.push(Inline::Macro(n));
        }
        // A nested `ROXYGEN_MD_LIST` (a sublist inside a list item) projects as
        // its own `\itemize`/`\enumerate`, the way a top-level list does.
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_MD_LIST => {
            out.push(Inline::MdList(n));
        }
        // A `ROXYGEN_MD_CODE_BLOCK` folded into a list item (a fenced code block
        // at the item's content column) projects to roxygen2's three-atom
        // `\if{html}…\preformatted…\if{html}` sequence, the same as a
        // section-level fenced block.
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_MD_CODE_BLOCK => {
            out.push(Inline::MdCodeBlock(n));
        }
        // A resolved emphasis/strong *node* (the inline pass's output): recurse
        // into its inner inline run, skipping the opener/closer delimiter leaves
        // (and any inter-line trivia), so nesting projects as structure.
        NodeOrToken::Node(n)
            if matches!(
                n.kind(),
                SyntaxKind::ROXYGEN_MD_EMPH | SyntaxKind::ROXYGEN_MD_STRONG
            ) =>
        {
            let strong = n.kind() == SyntaxKind::ROXYGEN_MD_STRONG;
            // The span's *own* delimiters are the first child (opener) and the last
            // child (closer); they are skipped. Any *interior* `ROXYGEN_MD_DELIM` is
            // an unmatched delimiter and stays literal text (`_foo_bar_baz_` →
            // `\emph` over `foo_bar_baz`), reached via push_inline's text fallback.
            let kids: Vec<_> = n.children_with_tokens().collect();
            let interior = kids.len().saturating_sub(1);
            let mut children = Vec::new();
            for child in kids.into_iter().take(interior).skip(1) {
                match child.kind() {
                    SyntaxKind::ROXYGEN_MARKER => {} // threaded trivia: never prose
                    SyntaxKind::NEWLINE => children.push(Inline::Text(SOFT_BREAK.to_string())),
                    _ => push_inline(&mut children, child),
                }
            }
            out.push(Inline::MdEmphasis { strong, children });
        }
        // A resolved inline-link *node* (`ROXYGEN_MD_LINK`): the inline pass's
        // output for a bracket-paired link. The first child is the `[` opener leaf
        // and the last child the closer leaf; the display in between recurses (so
        // emphasis/code spans inside the link surface as structure). The closer text
        // distinguishes the three paired forms: `](url)` is an inline link carrying
        // its destination (`\href`/`\url`); `][ref]` is a *reference* link whose
        // `[ref]` topic option is dropped, projecting to `\link{display}`; a bare `]`
        // is a *shortcut* link whose display is the destination.
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_MD_LINK => {
            let kids: Vec<_> = n.children_with_tokens().collect();
            let closer = kids.last().map(|c| c.to_string()).unwrap_or_default();
            let interior = kids.len().saturating_sub(1);
            let mut display = Vec::new();
            for child in kids.into_iter().take(interior).skip(1) {
                match child.kind() {
                    SyntaxKind::ROXYGEN_MARKER => {} // threaded trivia: never prose
                    SyntaxKind::NEWLINE => display.push(Inline::Text(SOFT_BREAK.to_string())),
                    _ => push_inline(&mut display, child),
                }
            }
            if closer == "]" {
                // A bare `]` closer: a cross-line *shortcut* link `[text]`.
                out.push(Inline::MdShortcutLink { display });
            } else if let Some(dest) = closer.strip_prefix("][").and_then(|s| s.strip_suffix(']')) {
                out.push(Inline::MdRefLink {
                    dest: dest.to_string(),
                    display,
                });
            } else {
                out.push(Inline::MdInlineLink {
                    url: inline_link_dest(&closer),
                    display,
                });
            }
        }
        // A **multi-line** raw inline-HTML span — a `ROXYGEN_MD_HTML` *node* the
        // inline pass resolved across soft breaks. Its rendered text is the
        // span's cmark-visible content ([`md_html_node_text`]): the `#'` markers
        // and continuation indentation strip, the soft breaks stay as real
        // newlines (the span text is verbatim `\out` content, one VERB per line).
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_MD_HTML => {
            out.push(Inline::MdHtml(md_html_node_text(&n)));
        }
        // A **multi-line** code span — a `ROXYGEN_MD_CODE` *node* the inline
        // pass resolved across soft breaks. Same cmark-visible walk, then the
        // ordinary code-span strip: [`strip_code_span`] converts the interior
        // line endings to spaces (CommonMark: a code span's line endings are
        // treated like spaces) before its single-space trim.
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_MD_CODE => {
            out.push(Inline::MdCode(strip_code_span(&md_html_node_text(&n))));
        }
        NodeOrToken::Node(n) => out.push(Inline::Text(n.text().to_string())),
        // Markdown inline leaves (emitted only under `@md`): carve off their
        // delimiters and carry the inner content; the kind chooses the Rd macro.
        NodeOrToken::Token(t) if t.kind() == SyntaxKind::ROXYGEN_MD_CODE => {
            out.push(Inline::MdCode(strip_code_span(t.text())));
        }
        // A markdown link leaf: the inline `[text](url)` form projects to `\href`;
        // the reference (`[text][ref]`) and shortcut (`[dest]`) forms resolve to an
        // `\link`/`\linkS4class` (optionally `\code`-wrapped) per roxygen2's
        // `parse_link` (see [`resolve_md_link`]). A leaf that resolves to nothing
        // (an unrecognized shape) falls through to literal prose.
        NodeOrToken::Token(t)
            if t.kind() == SyntaxKind::ROXYGEN_MD_LINK && resolve_md_link(t.text()).is_some() =>
        {
            out.push(Inline::MdLink(t.text().to_string()));
        }
        // A markdown image leaf `![alt](url "title")` → `\figure` (see
        // [`resolve_md_image`]). A leaf that resolves to nothing falls through to
        // literal prose.
        NodeOrToken::Token(t)
            if t.kind() == SyntaxKind::ROXYGEN_MD_IMAGE && resolve_md_image(t.text()).is_some() =>
        {
            out.push(Inline::MdImage(t.text().to_string()));
        }
        // A raw inline-HTML leaf `<tag>` → `\if{html}{\out{<tag>}}` (see
        // [`html_inline_atom`]).
        NodeOrToken::Token(t) if t.kind() == SyntaxKind::ROXYGEN_MD_HTML => {
            out.push(Inline::MdHtml(t.text().to_string()));
        }
        // A `ROXYGEN_MD_LIST_MARKER` that reached an inline run (rather than a
        // `ROXYGEN_MD_LIST`) is a marker that did not form a list — the CommonMark
        // interrupt rule kept it inline. roxygen2 renders it as literal text.
        NodeOrToken::Token(t) => out.push(Inline::Text(t.text().to_string())),
    }
}

/// The cmark-visible text of a multi-line raw inline-HTML span
/// (`ROXYGEN_MD_HTML` node): content children verbatim (a covered Rd macro or
/// carved leaf contributes its raw source — roxygen2 restores a placeholder to
/// its original text in the rendered output), a soft break as a real `\n`, and
/// the threaded `#'` markers and marker→content indentation dropped (roxygen2
/// strips `#' `, and cmark strips a paragraph continuation line's remaining
/// leading whitespace — engine-probed: `#'    y -->` renders `y -->`).
fn md_html_node_text(node: &SyntaxNode) -> String {
    let mut s = String::new();
    for el in node.children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_MARKER | SyntaxKind::WHITESPACE => {}
            SyntaxKind::NEWLINE => s.push('\n'),
            _ => match el {
                NodeOrToken::Token(t) => s.push_str(t.text()),
                NodeOrToken::Node(n) => s.push_str(&n.text().to_string()),
            },
        }
    }
    s
}

/// The content of a markdown code span: drop the matched backtick runs, then
/// apply CommonMark's single-space trim (if the inner text both starts and ends
/// with a space but is not all spaces, one space is removed from each end).
fn strip_code_span(text: &str) -> String {
    let ticks = text.bytes().take_while(|&b| b == b'`').count();
    let inner = text
        .get(ticks..text.len() - ticks)
        .unwrap_or("")
        .replace('\n', " ");
    if inner.len() >= 2
        && inner.starts_with(' ')
        && inner.ends_with(' ')
        && !inner.trim().is_empty()
    {
        inner[1..inner.len() - 1].to_string()
    } else {
        inner
    }
}

/// Resolve a `ROXYGEN_MD_LINK` leaf into its Rd atom, mirroring roxygen2's
/// `parse_link` (`markdown-link.R`). Three forms:
///
/// - **inline** `[text](url)` → `(\href (VERB url) (TEXT text))`;
/// - **reference** `[text][ref]` → `(\link (TEXT text))` — the has-link-text
///   branch (always `\link`, `\code`-wrapped iff the display text is a code span);
/// - **shortcut** `[dest]` → `(\link …)`/`(\linkS4class …)`, `\code`-wrapped when
///   `dest` is a code span or ends in `()`.
///
/// The `\link[…]`/`\linkS4class[…]` *topic option* is dropped by roxygen2's
/// section serializer, so only the macro head, the display text, and the
/// `\code`-wrap survive. Package resolution (`resolve_link_package`) is inherently
/// non-static, so the projector models exactly what roxygen2 does with no
/// resolvable package context (the corpus's `current_package == ""`): a package
/// prefix in the display text comes only from an explicit `pkg::` in the link.
///
/// Returns `None` for an unrecognized shape (the leaf then stays literal prose).
fn resolve_md_link(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    // A CommonMark autolink `<…>`. cmark has two disjoint forms:
    //   * a URI autolink `<scheme:…>` whose destination equals its text →
    //     `\url{…}` (roxygen2's `mdxml_link` `dest == xml_text(xml)` branch);
    //   * an email autolink `<addr>` (no URI scheme), for which cmark sets the
    //     destination to `mailto:addr` → `\href{mailto:addr}{addr}` (the address
    //     is both destination and display).
    // The lexer only carves a valid autolink here, so distinguishing the two
    // reduces to whether a URI scheme is present ([`autolink_has_uri_scheme`]).
    if bytes.first() == Some(&b'<') {
        let inner = raw.strip_prefix('<')?.strip_suffix('>')?;
        return Some(if autolink_has_uri_scheme(inner) {
            url_atom(inner)
        } else {
            href_atom(inner, &format!("mailto:{inner}"))
        });
    }
    let text_end = scan_delimited(bytes, 0, b'[', b']')?;
    let text = &raw[1..text_end - 1];
    match bytes.get(text_end) {
        Some(&b'(') => {
            let url_end = scan_delimited(bytes, text_end, b'(', b')')?;
            (url_end == bytes.len())
                .then(|| inline_link_atom(text, &raw[text_end + 1..url_end - 1]))
        }
        Some(&b'[') => {
            let ref_end = scan_delimited(bytes, text_end, b'[', b']')?;
            (ref_end == bytes.len()).then(|| ref_link_atom(text, &raw[text_end + 1..ref_end - 1]))
        }
        // A bare `[dest]` is the whole leaf (the lexer carves nothing after it).
        None => Some(shortcut_link_atom(text)),
        _ => None,
    }
}

/// The destination of an inline-link closer leaf `](dest)`: the text between the
/// parentheses (verbatim, mirroring the opaque path's `&raw[text_end+1..url_end-1]`).
fn inline_link_dest(close: &str) -> String {
    close
        .strip_prefix("](")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or("")
        .to_string()
}

/// Project a `ROXYGEN_MD_LINK` node `[display](url)`: `\href{url}{display}` with
/// the display GRP-wrapped when it is more than one atom (`\href` is a two-argument
/// structural macro), falling back to `\url{text}` when the destination is empty or
/// equals the link text (roxygen2's `mdxml_link`, the auto-generated-destination
/// branch). Mirrors [`inline_link_atom`], but the display renders the link's
/// resolved markdown *children* rather than a flat string.
fn inline_link_node_atom(url: &str, display: &[Inline], md: bool) -> String {
    let display_text = inline_plain_text(display);
    if url.is_empty() || norm_ws(url) == norm_ws(&display_text) {
        return url_atom(&display_text);
    }
    let arg = grp_arg(&serialize_inlines(display, md));
    format!("(\\href (VERB {}){})", encode_text(url), prefix_space(&arg))
}

/// Project a `ROXYGEN_MD_LINK` node with a reference closer (`[display][ref]`):
/// `\link{display}` — roxygen2's section serializer drops the `[ref]` topic option,
/// so only the `\link` head and the display survive, `\code`-wrapped when the
/// display is a single code span. When the display text equals the reference label,
/// roxygen2 treats the text as auto-generated and falls back to the shortcut path.
/// Mirrors [`ref_link_atom`], but the display is the link's resolved markdown
/// *children* rather than a flat string.
fn ref_link_node_atom(display: &[Inline], dest: &str) -> String {
    let display_text = inline_plain_text(display);
    if norm_ws(&display_text) == norm_ws(dest) {
        return shortcut_link_atom(dest);
    }
    if display_has_macro(display) {
        return link_over_display(display);
    }
    let (inner, is_code) = match display {
        [Inline::MdCode(content)] => (content.clone(), true),
        _ => (display_text, false),
    };
    code_wrap(
        format!("(\\link {})", text_atom(&inner).unwrap_or_default()),
        is_code,
    )
}

/// Whether a resolved link display carries a `ROXYGEN_RD_MACRO` child (a
/// backslash-word or `\name{…}` written in the markdown source). Such a display is
/// rendered as `\link` over the serialized display atoms ([`link_over_display`])
/// rather than collapsed to a flat destination string, so the macro surfaces as a
/// nested Rd subtree the way parse_Rd reads it (`[a\b]` → `(\link (TEXT "a")
/// (UNKNOWN "\\b"))`).
fn display_has_macro(display: &[Inline]) -> bool {
    display.iter().any(|inl| matches!(inl, Inline::Macro(_)))
}

/// Render `\link` over a macro-bearing display: the topic is the serialized display
/// atoms (text runs plus each Rd macro as a nested subtree), mirroring roxygen2's
/// `\link{<markdown display>}` whose body parse_Rd then parses. The `\linkS4class` /
/// `pkg::` / `()` shortcut-destination refinements operate on a flat string and so do
/// not apply to a macro-bearing destination (a vanishingly-rare combination — left as
/// backlog).
fn link_over_display(display: &[Inline]) -> String {
    let body = serialize_inlines(display, true).join(" ");
    format!("(\\link {body})")
}

/// Project a `ROXYGEN_MD_LINK` node with a bare `]` shortcut closer (`[display]`):
/// the display text *is* the destination, so this mirrors [`shortcut_link_atom`] but
/// takes the code-span-ness from the resolved children rather than from backticks in
/// a raw string. A single code-span display re-wraps its content in backticks so the
/// shared resolver detects it (`\code`-wrapped `\link`); any other display passes its
/// coalesced plain text through unchanged.
fn shortcut_link_node_atom(display: &[Inline]) -> String {
    match display {
        [Inline::MdCode(content)] => shortcut_link_atom(&format!("`{content}`")),
        _ if display_has_macro(display) => link_over_display(display),
        _ => shortcut_link_atom(&inline_plain_text(display)),
    }
}

/// Whether roxygen2's `parse_link` would *drop* a shortcut/reference link with this
/// resolved display, rendering nothing ("markdown links must contain plain text").
/// `parse_link` first unwraps a display that is a *single* code span (which then
/// links as `\code{\link{…}}`) and otherwise requires every child to be text (a
/// softbreak/linebreak, both projected as `Inline::Text(" ")`, also count): any
/// emphasis, a second code span, an image, an autolink, or raw HTML makes the link
/// non-plain and roxygen2 discards it. (An inline `[text](url)` link is never
/// subject to this — it carries its own destination and renders `\href`.)
///
/// An `Inline::Macro` child counts as **plain text** *unless* its argument carries
/// cmark-active markdown: a bare backslash-word (`\b`) or a `\name{…}` whose body is
/// literal (`\emph{x}`, or any fragile macro like `\code{*x*}`) is literal text to
/// cmark (a backslash escapes only punctuation, macro braces are literal), so the
/// link is kept and parse_Rd reinterprets it as an Rd macro. But a non-fragile
/// macro whose argument *is* markdown-processed and resolves to active markup
/// (`\emph{*x*}`, `\emph{a \strong{*x*}}`, `` \emph{`c`} ``) makes the display
/// non-plain to cmark, so roxygen2 drops the link ([`macro_arg_has_active_markdown`]).
fn link_display_is_droppable(display: &[Inline]) -> bool {
    if matches!(display, [Inline::MdCode(_)]) {
        return false;
    }
    !display.iter().all(|inl| match inl {
        Inline::Text(_) => true,
        Inline::Macro(n) => !macro_arg_has_active_markdown(n),
        _ => false,
    })
}

/// Whether a non-fragile Rd macro's markdown-processed argument resolves to any
/// **cmark-active** markup (emphasis, a code span, a link, an image, raw HTML, or a
/// nested non-fragile macro whose own argument is active). Such a macro is *not*
/// plain text to cmark, so a link display containing it is dropped. A fragile macro
/// (`\code`/`\link`/…) or a non-fragile one with a literal argument (`\emph{x}`) is
/// inert. Mirrors the resolution [`serialize_macro`] performs, so the drop decision
/// and the render agree.
fn macro_arg_has_active_markdown(node: &SyntaxNode) -> bool {
    let head = macro_head(node);
    let name = head.trim_start_matches('\\');
    is_md_inline_text_macro(name)
        && macro_single_arg_content(node).is_some_and(|content| {
            inlines_have_active_markdown(&resolve_macro_arg_inlines(&content))
        })
}

/// Whether a resolved inline run carries cmark-active markup: any element that is
/// neither plain text nor an inert macro (see [`macro_arg_has_active_markdown`],
/// which recurses through nested non-fragile macros).
fn inlines_have_active_markdown(inlines: &[Inline]) -> bool {
    inlines.iter().any(|inl| match inl {
        Inline::Text(_) => false,
        Inline::Macro(n) => macro_arg_has_active_markdown(n),
        _ => true,
    })
}

/// A best-effort plain-text rendering of a resolved inline run, used only to test a
/// link's destination against its text (the `\url` auto-destination branch). Rich
/// inlines (emphasis, code) contribute their textual content; non-textual inlines
/// contribute nothing — a link whose destination equals such a text is vanishingly
/// rare, and the `\href` branch is the safe default.
fn inline_plain_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for inl in inlines {
        match inl {
            Inline::Text(t) => s.push_str(t),
            Inline::MdCode(t) => s.push_str(t),
            Inline::MdEmphasis { children, .. } => s.push_str(&inline_plain_text(children)),
            Inline::MdInlineLink { display, .. } => s.push_str(&inline_plain_text(display)),
            Inline::MdShortcutLink { display } => s.push_str(&inline_plain_text(display)),
            _ => {}
        }
    }
    s
}

/// The text of a link's display for **link-reference purposes** — its resolution
/// label ([`link_ref_label`]), its candidate in the refmap skeleton
/// ([`linkref_skeleton_push`]), and the literal a demoted link rewrites to
/// ([`demoted_link_source`]). Identical to [`inline_plain_text`] except an
/// `Inline::Macro` contributes its **verbatim source** (`\emph{*x*}`) rather than
/// nothing: a pure-macro display (`[\emph{*x*}]`) would otherwise produce the empty
/// label `""`, whose `[]` candidate registers no refmap key
/// ([`bracket_free_group`]) — so the link was spuriously demoted to a literal `[]`
/// instead of reaching the drop/keep decision in [`serialize_inlines`]. The macro
/// source is also exactly what roxygen2's own `get_md_linkrefs` candidate scan sees,
/// and keeping the skeleton and the resolution label both routed through this helper
/// keeps them self-consistent (so a defined label never spuriously demotes).
fn link_label_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for inl in inlines {
        match inl {
            Inline::Text(t) => s.push_str(t),
            Inline::MdCode(t) => s.push_str(t),
            Inline::MdEmphasis { children, .. } => s.push_str(&link_label_text(children)),
            Inline::MdInlineLink { display, .. } => s.push_str(&link_label_text(display)),
            Inline::MdShortcutLink { display } => s.push_str(&link_label_text(display)),
            Inline::Macro(n) => s.push_str(&n.text().to_string()),
            _ => {}
        }
    }
    s
}

/// An inline `[text](url)` link, mirroring roxygen2's `mdxml_link`: an empty
/// destination — or one equal to the rendered link text — projects to `\url{text}`
/// (the destination is auto-generated from the text); otherwise `\href{url}{text}`.
fn inline_link_atom(text: &str, url: &str) -> String {
    if url.is_empty() || norm_ws(url) == norm_ws(text) {
        url_atom(text)
    } else {
        href_atom(text, url)
    }
}

/// Whether an autolink's inner text (the `…` in `<…>`) is a CommonMark **URI**
/// autolink rather than an **email** autolink: an ASCII letter, then 1–31 more of
/// letter/digit/`+`/`.`/`-`, then `:` (scheme length 2–32). Mirrors the scheme
/// check in the parser's `scan_md_autolink`; the two autolink forms are disjoint
/// (an email address has no `:`), so a `false` here means an email autolink.
fn autolink_has_uri_scheme(inner: &str) -> bool {
    let b = inner.as_bytes();
    if !b.first().is_some_and(u8::is_ascii_alphabetic) {
        return false;
    }
    let mut j = 1;
    while j < b.len()
        && matches!(b[j], b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'.' | b'-')
    {
        j += 1;
    }
    (2..=32).contains(&j) && b.get(j) == Some(&b':')
}

/// A bare URL → `(\url (VERB url))` (roxygen2's `\url{…}`; the URL is verbatim).
fn url_atom(url: &str) -> String {
    format!("(\\url (VERB {}))", encode_text(url))
}

/// An inline `[text](url)` link → `(\href (VERB url) <text>)`: the URL is verbatim
/// (no whitespace collapse), the display rendered by [`link_display_atom`] (a code
/// span sub-renders to `\verb`/`\code`; other text is whitespace-normalized prose,
/// an empty display contributing no atom).
fn href_atom(text: &str, url: &str) -> String {
    let mut atoms = vec![format!("(VERB {})", encode_text(url))];
    if let Some(atom) = link_display_atom(text) {
        atoms.push(atom);
    }
    format!("(\\href {})", atoms.join(" "))
}

/// The display-text atom for an inline `[text](url)` link. roxygen2 renders the
/// link's markdown *children*, so a single code-span text becomes `\verb`/`\code`
/// (via [`md_code_atom`], mirroring `mdxml_code`) rather than literal prose; any
/// other text is whitespace-normalized `(TEXT …)` (`None` when blank). General
/// inline sub-rendering of *mixed* markdown in link text (e.g. emphasis) is not
/// yet modeled — such a text stays plain prose (faithful under-handling, backlog).
fn link_display_atom(text: &str) -> Option<String> {
    let (inner, is_code) = unwrap_code_span(text);
    if is_code {
        Some(md_code_atom(inner))
    } else {
        text_atom(text)
    }
}

/// A reference link `[text][ref]` (explicit link text) → always `\link` over the
/// display text, `\code`-wrapped iff the display is a single code span. When the
/// display text equals the destination, roxygen2 treats the text as
/// auto-generated and falls back to the shortcut path.
fn ref_link_atom(text: &str, dest: &str) -> String {
    let (display, is_code) = unwrap_code_span(text);
    if norm_ws(display) == norm_ws(dest) {
        return shortcut_link_atom(dest);
    }
    code_wrap(
        format!("(\\link {})", text_atom(display).unwrap_or_default()),
        is_code,
    )
}

/// A shortcut link `[dest]` (no explicit link text) → roxygen2's `!has_link_text`
/// branch: `\linkS4class` for an `-class` destination without a package, else
/// `\link`; `\code`-wrapped when the destination is a code span or a `()` call.
/// The display text is `pkg::` + the object (with any `-class` suffix dropped).
fn shortcut_link_atom(dest: &str) -> String {
    let (dest, code_span) = unwrap_code_span(dest);
    let is_code = code_span || dest.ends_with("()");
    let (pkg, fun) = match dest.rsplit_once("::") {
        Some((p, f)) => (Some(p), f),
        None => (None, dest),
    };
    let s4 = dest.ends_with("-class");
    let body = if s4 {
        fun.strip_suffix("-class").unwrap_or(fun)
    } else {
        fun
    };
    let head = if s4 && pkg.is_none() {
        "\\linkS4class"
    } else {
        "\\link"
    };
    let display = match pkg {
        Some(p) => format!("{p}::{body}"),
        None => body.to_string(),
    };
    code_wrap(
        format!("({head} {})", text_atom(&display).unwrap_or_default()),
        is_code,
    )
}

/// Resolve a `ROXYGEN_MD_IMAGE` leaf `![alt](url "title")` into its Rd atom,
/// mirroring roxygen2's `mdxml_image` (`markdown.R`). The alt text is *dropped*
/// (roxygen2 uses only the destination and title); the result is
/// `(\figure (VERB url) [(VERB title)])`, wrapped in `(\if (TEXT "html") …)` or
/// `(\if (TEXT "pdf") …)` per the extension-keyed `get_image_format` rule. Returns
/// `None` for an unrecognized shape (the leaf then stays literal prose).
fn resolve_md_image(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    // The leaf always begins `![`; the alt span is `[…]` starting at index 1.
    let alt_end = scan_delimited(bytes, 1, b'[', b']')?;
    if bytes.get(alt_end) != Some(&b'(') {
        return None;
    }
    let dest_end = scan_delimited(bytes, alt_end, b'(', b')')?;
    if dest_end != bytes.len() {
        return None;
    }
    let (url, title) = split_image_dest(&raw[alt_end + 1..dest_end - 1]);
    Some(figure_atom(url, title))
}

/// Split a CommonMark image destination `url "title"` into `(url, title)`. The URL
/// is angle-bracketed (`<…>`) or runs to the first ASCII whitespace; the optional
/// title that follows is wrapped in `"…"`, `'…'`, or `(…)`. A missing title is an
/// empty string.
fn split_image_dest(dest: &str) -> (&str, &str) {
    let dest = dest.trim();
    let (url, rest) = if dest.as_bytes().first() == Some(&b'<') {
        match dest.find('>') {
            Some(close) => (&dest[1..close], &dest[close + 1..]),
            None => (dest, ""),
        }
    } else {
        match dest.find(char::is_whitespace) {
            Some(sp) => (&dest[..sp], &dest[sp..]),
            None => (dest, ""),
        }
    };
    (url, strip_title_delims(rest.trim()))
}

/// Strip the surrounding title delimiters from a CommonMark image title
/// (`"…"`/`'…'`/`(…)`); return the input unchanged when it is not delimited.
fn strip_title_delims(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2
        && matches!(
            (b[0], b[b.len() - 1]),
            (b'"', b'"') | (b'\'', b'\'') | (b'(', b')')
        )
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Build the `\figure` atom for an image, applying roxygen2's `get_image_format`:
/// a destination matching only the HTML extension set (`svg`) is wrapped in
/// `\if{html}{…}`, only the PDF set (`pdf`) in `\if{pdf}{…}`, and one matching both
/// (raster: `jpg`/`jpeg`/`gif`/`png`) or neither stays a bare `\figure`. The title
/// is verbatim and omitted when empty.
fn figure_atom(url: &str, title: &str) -> String {
    let mut args = vec![format!("(VERB {})", encode_text(url))];
    if !title.is_empty() {
        args.push(format!("(VERB {})", encode_text(title)));
    }
    let figure = format!("(\\figure {})", args.join(" "));
    match image_format(url) {
        ImageFormat::Html => format!("(\\if (TEXT {}) {figure})", encode_text("html")),
        ImageFormat::Pdf => format!("(\\if (TEXT {}) {figure})", encode_text("pdf")),
        ImageFormat::All => figure,
    }
}

/// The conditional an image destination renders under, per roxygen2's
/// `get_image_format`/`default_image_formats` (`markdown.R`).
enum ImageFormat {
    Html,
    Pdf,
    All,
}

/// Classify an image destination by extension, mirroring roxygen2's
/// `default_image_formats` regexes (`[.](jpg|jpeg|gif|png|svg)$` for HTML,
/// `[.](jpg|jpeg|gif|png|pdf)$` for PDF). Matching both sets (or neither) is
/// `All` (a bare `\figure`); matching one only carves the `\if` wrapper.
fn image_format(url: &str) -> ImageFormat {
    let lower = url.to_ascii_lowercase();
    let has_dot_ext = |exts: &[&str]| {
        exts.iter()
            .any(|e| lower.strip_suffix(e).is_some_and(|p| p.ends_with('.')))
    };
    match (
        has_dot_ext(&["jpg", "jpeg", "gif", "png", "svg"]),
        has_dot_ext(&["jpg", "jpeg", "gif", "png", "pdf"]),
    ) {
        (true, false) => ImageFormat::Html,
        (false, true) => ImageFormat::Pdf,
        _ => ImageFormat::All,
    }
}

/// Wrap an atom in `(\code …)` when `is_code`, else return it unchanged.
fn code_wrap(inner: String, is_code: bool) -> String {
    if is_code {
        format!("(\\code {inner})")
    } else {
        inner
    }
}

/// If `s` is a single-backtick code span (`` `x` ``), return its inner text and
/// `true`; otherwise return `s` unchanged and `false`.
fn unwrap_code_span(s: &str) -> (&str, bool) {
    let b = s.as_bytes();
    if b.len() >= 2 && b[0] == b'`' && b[b.len() - 1] == b'`' {
        (&s[1..s.len() - 1], true)
    } else {
        (s, false)
    }
}

/// Index just past the balanced `close` byte matching the `open` at `start`, or
/// `None` if `start` is not `open` or the group never closes. Brackets are ASCII,
/// so a byte scan is sufficient.
fn scan_delimited(bytes: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
    if bytes.get(start) != Some(&open) {
        return None;
    }
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
    }
    None
}

/// Project a `ROXYGEN_MD_LIST` node into `(\itemize …)` or `(\enumerate …)`: each
/// `ROXYGEN_MD_LIST_ITEM` contributes a name-only `(\item)` followed by its
/// content atoms (the same inline serialization as prose), mirroring roxygen2's
/// translation of a markdown list into an Rd `\itemize`/`\enumerate`. The list is
/// ordered iff its first item's marker is a number.
fn serialize_md_list(node: &SyntaxNode) -> String {
    let head = if md_list_is_ordered(node) {
        "\\enumerate"
    } else {
        "\\itemize"
    };
    let mut atoms: Vec<String> = Vec::new();
    for item in node
        .children()
        .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
    {
        atoms.push("(\\item)".to_string());
        // A markdown list exists only under `@md`, so its item content is markdown.
        atoms.extend(serialize_inlines(&md_list_item_inlines(&item), true));
    }
    if atoms.is_empty() {
        format!("({head})")
    } else {
        format!("({head} {})", atoms.join(" "))
    }
}

/// Serialize a markdown list whose item contents have already been rewritten by the
/// whole-field link-reference pipeline ([`apply_user_linkrefs`]) — the resolved-items
/// analog of [`serialize_md_list`]. Each item renders a name-only `(\item)` followed
/// by its resolved inline run (markdown stays active inside a list item). An item the
/// pipeline emptied (it held only a consumed definition) renders a bare `(\item)`.
fn serialize_md_list_resolved(ordered: bool, items: &[Vec<Inline>]) -> String {
    let head = if ordered { "\\enumerate" } else { "\\itemize" };
    let mut atoms: Vec<String> = Vec::new();
    for item in items {
        atoms.push("(\\item)".to_string());
        atoms.extend(serialize_inlines(item, true));
    }
    if atoms.is_empty() {
        format!("({head})")
    } else {
        format!("({head} {})", atoms.join(" "))
    }
}

/// Project a `ROXYGEN_MD_CODE_BLOCK` node into roxygen2's three-atom fenced-code
/// rendering (`mdxml_code_block`, `R/markdown.R`): an opening
/// `\if{html}{\out{<div class="sourceCode[ <info>]">}}`, a `\preformatted{<code>}`,
/// and a closing `\if{html}{\out{</div>}}`. The `<div>` class carries the fence's
/// info string (empty → bare `sourceCode`); the code is the verbatim block content
/// with a trailing newline (commonmark's `xml_text`). The body's `%`/`{`/`}` are
/// `escape_verb`-escaped by roxygen2 but `parse_Rd` decodes them, so the pins (and
/// thus the projector) carry the raw characters.
fn serialize_md_code_block(node: &SyntaxNode) -> Vec<String> {
    let (info, code) = md_code_block_parts(node);
    let class = if info.is_empty() {
        "sourceCode".to_string()
    } else {
        format!("sourceCode {info}")
    };
    let html = encode_text("html");
    vec![
        format!(
            "(\\if (TEXT {html}) (\\out (VERB {})))",
            encode_text(&format!("<div class=\"{class}\">"))
        ),
        format!("(\\preformatted (VERB {}))", encode_text(&code)),
        format!(
            "(\\if (TEXT {html}) (\\out (VERB {})))",
            encode_text("</div>")
        ),
    ]
}

/// Project a `ROXYGEN_MD_INDENTED_CODE` node into the same three-atom rendering as
/// a fenced code block (`mdxml_code_block`), but with a bare `sourceCode` class (an
/// indented code block has no info string) and each line's indentation stripped: a
/// CommonMark indented code block drops four columns of indentation, on top of the
/// `#'` marker and the one space roxygen2 strips first. Each line therefore has its
/// marker, one following space, then up to four further leading spaces removed; the
/// result is joined with newlines (a trailing newline per line, commonmark's
/// `xml_text`) and split into one `VERB` per line by `parse_Rd`.
fn serialize_md_indented_code(node: &SyntaxNode) -> Vec<String> {
    let text = node.text().to_string();
    // A block opening as a tag's same-line value (`@details      x`) has a
    // marker-less first line (the `#'` belongs to the enclosing tag): roxygen2
    // strips only the single separator space there — `strip_marker` would trim
    // the whole run, eating the code's semantic indent.
    let from_value = node
        .first_token()
        .is_some_and(|t| t.kind() != SyntaxKind::ROXYGEN_MARKER);
    let mut code = String::new();
    for (idx, line) in text.split('\n').enumerate() {
        // `strip_marker` removes the `#'` marker and the single conventional space;
        // the indented code block then consumes up to four further leading columns.
        let after_marker = if idx == 0 && from_value {
            line.strip_prefix([' ', '\t']).unwrap_or(line)
        } else {
            strip_marker(line)
        };
        let content = after_marker
            .char_indices()
            .take(4)
            .take_while(|&(_, c)| c == ' ')
            .count();
        code.push_str(&after_marker[content..]);
        code.push('\n');
    }
    let html = encode_text("html");
    vec![
        format!(
            "(\\if (TEXT {html}) (\\out (VERB {})))",
            encode_text("<div class=\"sourceCode\">")
        ),
        format!("(\\preformatted {})", verb_atoms(&code).join(" ")),
        format!(
            "(\\if (TEXT {html}) (\\out (VERB {})))",
            encode_text("</div>")
        ),
    ]
}

/// Project a `ROXYGEN_MD_HTML_BLOCK` node into roxygen2's `\if{html}{\out{…}}`
/// (`mdxml_html_block`, `R/markdown.R`): the block's verbatim text — a leading
/// newline then each line with a trailing newline — goes into a single `\out`
/// inside an `\if{html}{…}`. parse_Rd splits the verbatim `\out` body at newlines
/// into one `VERB` per line (the leading `\n` becomes a bare `(VERB "\n")`). The
/// block lines are reconstructed from the node text with each `#'` marker and the
/// single following space stripped (like the fenced code block).
fn serialize_md_html_block(node: &SyntaxNode) -> String {
    let text = node.text().to_string();
    // A block opening as a tag's same-line value has a marker-less first line
    // (the `#'` belongs to the enclosing tag): roxygen2 strips only the single
    // separator space after the tag head, so drop exactly one leading whitespace
    // char and keep any further indent (it is part of the rendered line —
    // `strip_marker` would trim the whole run).
    let mid_value = node
        .first_token()
        .is_some_and(|t| t.kind() != SyntaxKind::ROXYGEN_MARKER);
    let mut body = String::from("\n");
    for (idx, line) in text.split('\n').enumerate() {
        if idx == 0 && mid_value {
            body.push_str(line.strip_prefix([' ', '\t']).unwrap_or(line));
        } else {
            body.push_str(strip_marker(line));
        }
        body.push('\n');
    }
    format!(
        "(\\if (TEXT {}) (\\out {}))",
        encode_text("html"),
        verb_atoms(&body).join(" ")
    )
}

/// Project a `ROXYGEN_MD_BLOCK_QUOTE` node into its **flattened plain text**.
/// roxygen2 does not support block quotes (`mdxml_unsupported`, `R/markdown.R`): it
/// warns, then renders `escape_comment(xml_text(node))` — the concatenation of every
/// descendant text node, with the `>` markers and all inner markup (emphasis, code
/// spans, links) dropped and **no separator** between the lines (softbreaks and
/// paragraph breaks contribute nothing). Each quote line has its `#'` marker, its
/// `>` marker (after up to three spaces), and one optional following space stripped;
/// its remaining markdown resolves to inlines whose plain text ([`inline_plain_text`],
/// softbreaks removed) is concatenated directly. The result is one whitespace-
/// normalized `(TEXT …)` atom, or `None` when the flattened text is blank.
///
/// Scoped to fully-marked, self-contained quotes: an inner Rd macro (roxygen2 keeps
/// its source), cross-line emphasis, and gluing the flattened text onto an adjacent
/// prose paragraph (roxygen2 emits no `\n\n` before a quote, so `before` + `> q`
/// renders `beforeq`) are deferred backlog and not pinned.
/// The **un-normalized** flattened plain text of a `ROXYGEN_MD_BLOCK_QUOTE`.
/// roxygen2 has no block-quote support (`mdxml_unsupported`, `R/markdown.R`): it
/// warns, then renders `escape_comment(xml_text(node))` — the concatenation of
/// every descendant text node, with the `>` markers and all inner markup
/// (emphasis, code spans, links) dropped and **no separator** between the lines
/// (softbreaks and paragraph breaks contribute nothing). Each quote line has its
/// `#'` marker, its `>` marker (after up to three spaces), and one optional
/// following space stripped; its remaining markdown resolves to inlines whose plain
/// text ([`inline_plain_text`], softbreaks removed) is concatenated directly.
///
/// Returned raw (not wrapped in a `(TEXT …)` atom, not whitespace-normalized) so
/// [`serialize_inlines`] can push it as a `Final` segment that glues onto adjacent
/// prose (roxygen2 emits no `\n\n` before a quote, so `before` + `> q` renders
/// `beforeq`), deferring the single `norm_ws` to the coalesced atom.
///
/// Scoped to fully-marked, self-contained quotes: an inner Rd macro (roxygen2 keeps
/// its source) and CommonMark lazy continuation (a non-`>` line folded into the
/// quote) are deferred backlog and not pinned.
fn block_quote_flat_text(node: &SyntaxNode) -> String {
    let text = node.text().to_string();
    let mut flat = String::new();
    for line in text.split('\n') {
        let content = strip_marker(line);
        let inner = strip_block_quote_marker(content);
        let inlines = resolve_macro_arg_inlines(inner);
        for ch in inline_plain_text(&inlines).chars() {
            if ch != SOFT_BREAK {
                flat.push(ch);
            }
        }
    }
    flat
}

/// Strip a block-quote line's `>` marker: up to three leading spaces, the `>`, then
/// one optional space. Mirrors [`crate::parser::roxygen`]'s `is_block_quote_marker`.
fn strip_block_quote_marker(content: &str) -> &str {
    let trimmed = content.trim_start_matches(' ');
    let after = trimmed.strip_prefix('>').unwrap_or(trimmed);
    after.strip_prefix(' ').unwrap_or(after)
}

/// Per-column alignment of a GFM table, from the delimiter row's colon markers.
#[derive(Clone, Copy)]
enum TableAlign {
    Left,
    Center,
    Right,
}

impl TableAlign {
    /// The `\tabular` format letter (`l`/`c`/`r`).
    fn code(self) -> char {
        match self {
            TableAlign::Left => 'l',
            TableAlign::Center => 'c',
            TableAlign::Right => 'r',
        }
    }
}

/// Project a `ROXYGEN_MD_TABLE` node into roxygen2's `\tabular{<align>}{<cells>}`
/// (`mdxml_table`, `R/markdown-table.R` via cmark-gfm). The delimiter row (the
/// node's second line) supplies the per-column alignment; the header row and every
/// body row fill one `GRP` in source order, each cell's content resolved as a
/// markdown inline run, separated by `\tab` and terminated by `\cr`. A row is
/// padded with empty cells (a `\tab` with no atom) or truncated to the column
/// count, matching cmark-gfm's ragged-row handling.
fn serialize_md_table(node: &SyntaxNode) -> String {
    let text = node.text().to_string();
    // A table whose header row opened as a tag's same-line value has a
    // marker-less first line (the `#'` belongs to the enclosing tag): drop only
    // the single separator space there (`strip_marker` would eat a leading `#`
    // in the header's first cell).
    let from_value = node
        .first_token()
        .is_some_and(|t| t.kind() != SyntaxKind::ROXYGEN_MARKER);
    let mut lines: Vec<&str> = text.split('\n').map(strip_marker).collect();
    if from_value && let Some(first) = text.split('\n').next() {
        lines[0] = first.strip_prefix([' ', '\t']).unwrap_or(first);
    }
    // The header is the first line and the delimiter the second; body rows follow.
    // The gate guarantees both exist, but stay defensive against a malformed node.
    if lines.len() < 2 {
        return "(\\tabular)".to_string();
    }
    let aligns = parse_table_delim(lines[1]);
    let ncol = aligns.len();
    let align_str: String = aligns.iter().map(|a| a.code()).collect();

    let mut grp: Vec<String> = Vec::new();
    let rows = std::iter::once(lines[0]).chain(lines[2..].iter().copied());
    for row in rows {
        let cells = split_table_row_cells(row);
        for c in 0..ncol {
            if c > 0 {
                grp.push("(\\tab)".to_string());
            }
            if let Some(cell) = cells.get(c) {
                let content = unescape_table_pipes(cell.trim());
                grp.extend(serialize_inlines(
                    &resolve_macro_arg_inlines(&content),
                    true,
                ));
            }
        }
        grp.push("(\\cr)".to_string());
    }
    format!(
        "(\\tabular (TEXT {}) (GRP {}))",
        encode_text(&align_str),
        grp.join(" ")
    )
}

/// The per-column alignments of a GFM delimiter row: a leading colon means left,
/// a trailing colon means right, both means center, none means default (left).
fn parse_table_delim(line: &str) -> Vec<TableAlign> {
    split_table_row_cells(line)
        .iter()
        .map(|cell| {
            let t = cell.trim();
            match (t.starts_with(':'), t.ends_with(':')) {
                (true, true) => TableAlign::Center,
                (false, true) => TableAlign::Right,
                _ => TableAlign::Left,
            }
        })
        .collect()
}

/// Unescape a GFM table cell's `\|` to a literal `|` — the one escape the table
/// extension resolves during block parsing, before the cell's inline content is
/// parsed (`x \| y` renders as `x | y`).
fn unescape_table_pipes(cell: &str) -> String {
    cell.replace("\\|", "|")
}

/// The verbatim `(VERB …)` atoms for an `\out` body, splitting at newlines and
/// attaching each `\n` to the atom it ends (parse_Rd's verbatim splitting, the
/// `VERB` analog of [`rcode_atoms`]).
fn verb_atoms(body: &str) -> Vec<String> {
    let mut atoms = Vec::new();
    let mut rest = body;
    while let Some(idx) = rest.find('\n') {
        let (seg, tail) = rest.split_at(idx + 1);
        atoms.push(format!("(VERB {})", encode_text(seg)));
        rest = tail;
    }
    if !rest.is_empty() {
        atoms.push(format!("(VERB {})", encode_text(rest)));
    }
    atoms
}

/// Project a raw inline-HTML span into roxygen2's `\if{html}{\out{<tag>}}`
/// (`mdxml_html_inline`, `markdown.R`): the tag text goes verbatim into a `\out`
/// inside an `\if{html}{…}`. roxygen2 `escape_verb`-escapes `}` (→ `\}`) but
/// `parse_Rd` decodes it, so the pin (and thus the projector) carries the raw
/// tag. A **multi-line** span keeps its newlines: `parse_Rd` splits the `\out`
/// body into one VERB leaf per line, each carrying its trailing `\n`
/// (engine-probed: `<!-- x`⏎`y -->` → `(VERB "<!-- x\n") (VERB "y -->")`).
fn html_inline_atom(raw: &str) -> String {
    let verbs: Vec<String> = raw
        .split_inclusive('\n')
        .map(|seg| format!("(VERB {})", encode_text(seg)))
        .collect();
    format!(
        "(\\if (TEXT {}) (\\out {}))",
        encode_text("html"),
        verbs.join(" ")
    )
}

/// Extract a fenced code block's `(info, code)` from its node. The info string is
/// the opener `ROXYGEN_MD_FENCE` leaf with its leading backtick run stripped and
/// trimmed (matching commonmark's `info` attribute). The code is every line
/// between the opener and closer fence lines, each with its `#'` marker and the
/// single following space stripped, joined by newlines with a trailing newline
/// (commonmark's `xml_text` for a code block).
fn md_code_block_parts(node: &SyntaxNode) -> (String, String) {
    let text = node.text().to_string();
    let lines: Vec<&str> = text.split('\n').collect();
    // The opener is the first line, the closer the last; the code is in between.
    // The opener fence may itself be indented (up to three columns at the top
    // level, or to a list item's content column when the block is folded into an
    // item). CommonMark removes the opener fence's own indentation both before
    // the info string and from up to that many leading columns of each content
    // line, so the leading spaces surviving in the code are `max(0, body_col -
    // fence_col)` — computable from the node text alone (`content_col` cancels).
    let opener = lines.first().map(|l| strip_marker(l)).unwrap_or_default();
    let fence_indent = opener.bytes().take_while(|&b| b == b' ').count();
    let info = opener[fence_indent..]
        .trim_start_matches('`')
        .trim()
        .to_string();
    let body = if lines.len() > 2 {
        &lines[1..lines.len() - 1]
    } else {
        &[]
    };
    let mut code = String::new();
    for line in body {
        let after_marker = strip_marker(line);
        // Strip up to the opener fence's indentation, never past what the line has.
        let strip = after_marker
            .bytes()
            .take_while(|&b| b == b' ')
            .count()
            .min(fence_indent);
        code.push_str(&after_marker[strip..]);
        code.push('\n');
    }
    (info, code)
}

/// Strip a `#'` line's marker prefix and the single following space, returning the
/// line's content. Tolerates leading indentation before the marker (inter-line
/// trivia) and a multi-`#` marker.
fn strip_marker(line: &str) -> &str {
    let trimmed = line.trim_start();
    let after_hashes = trimmed.trim_start_matches('#');
    let body = after_hashes.strip_prefix('\'').unwrap_or(after_hashes);
    body.strip_prefix(' ').unwrap_or(body)
}

/// Whether a `ROXYGEN_MD_LIST` is ordered (`\enumerate`): its first item's
/// `ROXYGEN_MD_LIST_MARKER` begins with a digit (`1.`/`1)`), as opposed to a
/// bullet (`-`/`*`/`+`).
fn md_list_is_ordered(node: &SyntaxNode) -> bool {
    // Only this list's own items decide its kind — a nested sublist's markers
    // (its own `\itemize`/`\enumerate`) must not flip the parent, so look at the
    // first *direct* item's marker, not any descendant marker.
    node.children()
        .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
        .find_map(|item| {
            item.children_with_tokens()
                .filter_map(|el| el.into_token())
                .find(|t| t.kind() == SyntaxKind::ROXYGEN_MD_LIST_MARKER)
        })
        .is_some_and(|t| t.text().starts_with(|c: char| c.is_ascii_digit()))
}

/// The inline elements of a markdown list item: its content after the marker
/// leaf, with the threaded `#'` markers dropped and inter-line newlines turned
/// into joining spaces (the same treatment as a prose paragraph). The
/// `ROXYGEN_MD_LIST_MARKER` leaf itself is the item bullet, not content.
fn md_list_item_inlines(item: &SyntaxNode) -> Vec<Inline> {
    let mut out = Vec::new();
    for el in item.children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_MD_LIST_MARKER | SyntaxKind::ROXYGEN_MARKER => {}
            SyntaxKind::NEWLINE => out.push(Inline::Text(SOFT_BREAK.to_string())),
            _ => push_inline(&mut out, el),
        }
    }
    out
}

/// roxygen2 renders a markdown code span as `\code{…}` when its content parses as
/// a single R expression (or is one of a fixed set of operator/keyword tokens),
/// and `\verb{…}` otherwise (`R/markdown.R`'s `mdxml_code`/`can_parse`). The
/// projector replicates the decision with arity's own parser: parseable ⇒
/// `(\code (RCODE …))`, else `(\verb (VERB …))`. Both bodies are verbatim (no
/// whitespace collapse).
fn md_code_atom(content: &str) -> String {
    if code_span_is_r(content) {
        format!("(\\code (RCODE {}))", encode_text(content))
    } else {
        format!("(\\verb (VERB {}))", encode_text(content))
    }
}

/// Operator and keyword tokens roxygen2's `can_parse` treats as `\code` even
/// though they are not complete expressions on their own (`R/markdown.R`'s
/// `special`).
const SPECIAL_CODE: &[&str] = &[
    "-", ":", "::", ":::", "!", "!=", "(", "[", "[[", "@", "*", "/", "&", "&&", "%*%", "%/%", "%%",
    "%in%", "%o%", "%x%", "^", "+", "<", "<=", "=", "==", ">", ">=", "|", "||", "~", "$", "for",
    "function", "if", "repeat", "while",
];

/// Whether `code` parses as a single R expression, the way roxygen2's `can_parse`
/// (rlang's `parse_expr`) does: exactly one complete top-level expression with no
/// parse diagnostics, or a `special` token. arity's lenient recovery would accept
/// two adjacent symbols (`inline code`) as two expressions, so the one-expression
/// count is what discriminates `\code` from `\verb`.
fn code_span_is_r(code: &str) -> bool {
    if SPECIAL_CODE.contains(&code) {
        return true;
    }
    let out = crate::parser::parse(code);
    if !out.diagnostics.is_empty() {
        return false;
    }
    let one_expr = out
        .cst
        .children_with_tokens()
        .filter(|el| {
            !matches!(
                el.kind(),
                SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
            )
        })
        .count()
        == 1;
    one_expr && !has_invalid_underscore_name(&out.cst)
}

/// R's lexer rejects any name beginning with `_` (so rlang's `parse_expr`
/// errors), with one exception: a lone `_` used as the native-pipe placeholder,
/// valid only inside a `|>` pipeline. arity's lexer is more lenient and lexes a
/// `_`-leading run as an ordinary identifier, so screen these out here to mirror
/// roxygen2's `can_parse`. A `_`-leading name of length ≥ 2 is never valid; a
/// lone `_` is valid only when a `|>` is present in the same expression.
fn has_invalid_underscore_name(cst: &SyntaxNode) -> bool {
    let has_pipe = cst
        .descendants_with_tokens()
        .any(|el| el.kind() == SyntaxKind::PIPE);
    cst.descendants_with_tokens()
        .filter_map(|el| el.into_token())
        .filter(|t| t.kind() == SyntaxKind::IDENT)
        .any(|t| {
            let text = t.text();
            text.starts_with('_') && (text.len() > 1 || !has_pipe)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_plain_prose_sections() {
        let src = "#' Add two numbers\n\
                   #' @param x,y Numbers to add.\n\
                   #' @return Their sum.\n\
                   #' @export\n\
                   add <- function(x, y) x + y\n";
        // @param feeds the excluded \arguments; @export is a directive. Title and
        // description are derived from the single intro paragraph.
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Add two numbers\"))\n\
             (\\title (TEXT \"Add two numbers\"))\n\
             (\\value (TEXT \"Their sum.\"))"
        );
    }

    #[test]
    fn two_intro_paragraphs_split_title_and_description() {
        let src = "#' Example dataset\n\
                   #'\n\
                   #' A longer description.\n\
                   #' @name d\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"A longer description.\"))\n\
             (\\title (TEXT \"Example dataset\"))"
        );
    }

    #[test]
    fn md_heading_hoists_section_and_nests_subsection() {
        // A level-1 heading in `@details` hoists to a top-level `\section` (out of
        // `\details`); a deeper heading nests as a `\subsection`. With no prose
        // before the first heading, `\details` is omitted entirely.
        let src = "#' Title\n\
                   #'\n\
                   #' @md\n\
                   #' @details\n\
                   #' # First\n\
                   #' a\n\
                   #'\n\
                   #' ## Nested\n\
                   #' b\n\
                   #' @name x\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Title\"))\n\
             (\\section (TEXT \"First\") (GRP (TEXT \"a\") (\\subsection (TEXT \"Nested\") (TEXT \"b\"))))\n\
             (\\title (TEXT \"Title\"))"
        );
    }

    #[test]
    fn md_subsection_without_level_one_stays_in_details() {
        // A level->=2 heading with no enclosing level-1 heading nests as a
        // `\subsection` inside the enclosing `\details`, which keeps its leading
        // prose (the section is not hoisted out).
        let src = "#' Title\n\
                   #'\n\
                   #' @md\n\
                   #' @details\n\
                   #' Lead.\n\
                   #'\n\
                   #' ## Sub\n\
                   #' body\n\
                   #' @name x\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Title\"))\n\
             (\\details (TEXT \"Lead.\") (\\subsection (TEXT \"Sub\") (TEXT \"body\")))\n\
             (\\title (TEXT \"Title\"))"
        );
    }

    #[test]
    fn md_setext_heading_hoists_section() {
        // A setext `===` underline promotes its preceding paragraph into a level-1
        // heading, hoisted to a top-level `\section` out of `\details` (same as an
        // ATX `#`). The prose after the underline is the section body.
        let src = "#' Title\n\
                   #'\n\
                   #' @md\n\
                   #' @details\n\
                   #' Big\n\
                   #' ===\n\
                   #'\n\
                   #' body\n\
                   #' @name x\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Title\"))\n\
             (\\section (TEXT \"Big\") (TEXT \"body\"))\n\
             (\\title (TEXT \"Title\"))"
        );
    }

    #[test]
    fn md_setext_multiline_title_and_nesting() {
        // The underline promotes the *whole* preceding paragraph: `intro`+`Sub` are
        // one paragraph (no blank between), so `---` makes both the H2 title
        // ("intro Sub"). A `-` underline is level 2, nested under the `===` H1.
        let src = "#' Title\n\
                   #'\n\
                   #' @md\n\
                   #' @details\n\
                   #' Top\n\
                   #' ===\n\
                   #' intro\n\
                   #' Sub\n\
                   #' ---\n\
                   #' deep\n\
                   #' @name x\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Title\"))\n\
             (\\section (TEXT \"Top\") (\\subsection (TEXT \"intro Sub\") (TEXT \"deep\")))\n\
             (\\title (TEXT \"Title\"))"
        );
    }

    #[test]
    fn md_setext_single_dash_underline_hoists_subsection() {
        // CommonMark resolves a lone `-` line after a paragraph as a level-2 setext
        // underline (an empty list item cannot interrupt a paragraph), so `Foo`
        // becomes an H2 `\subsection` and `bar` its body. A `- item` line with
        // content would instead interrupt as a list (not exercised here).
        let src = "#' Title\n\
                   #'\n\
                   #' @md\n\
                   #' @details\n\
                   #' Foo\n\
                   #' -\n\
                   #' bar\n\
                   #' @name x\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Title\"))\n\
             (\\details (\\subsection (TEXT \"Foo\") (TEXT \"bar\")))\n\
             (\\title (TEXT \"Title\"))"
        );
    }

    #[test]
    fn three_intro_paragraphs_split_title_description_details() {
        // roxygen2's `parse_description` (R/block.R): the 1st intro paragraph is
        // the title, the 2nd the description, and every remaining paragraph the
        // details — not all-the-rest folded into the description.
        let src = "#' title\n\
                   #'\n\
                   #' description\n\
                   #'\n\
                   #' details\n\
                   #' @name a\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"description\"))\n\
             (\\details (TEXT \"details\"))\n\
             (\\title (TEXT \"title\"))"
        );
    }

    #[test]
    fn section_body_serializes_inline_macros_with_grp_wrap() {
        // `@section Title: body` → \section{Title}{body}; parse_Rd models \section
        // as a two-arg structural macro, so the body sub-parses inline macros and
        // GRP-wraps its multi-atom argument while the single-atom title stays bare.
        let src = "#' Title\n\
                   #'\n\
                   #' Description.\n\
                   #' @section Foobar:\n\
                   #' With some \\strong{bold text}.\n\
                   #' @name x\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Description.\"))\n\
             (\\section (TEXT \"Foobar\") (GRP (TEXT \"With some\") (\\strong (TEXT \"bold text\")) (TEXT \".\")))\n\
             (\\title (TEXT \"Title\"))"
        );
    }

    #[test]
    fn non_md_percent_is_an_rd_line_comment() {
        // In non-markdown prose (literal Rd), an unescaped `%` begins a comment to
        // end of line, so `@format %` projects to an empty `\format` and a mid-line
        // `%` keeps only the prose before it (roxygen2 passes the value as raw Rd).
        let src = "#' Title here\n\
                   #'\n\
                   #' Desc with a %% comment to end of line\n\
                   #' @format %\n\
                   x <- list(a = 1, b = 2)\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Desc with a\"))\n\
             (\\format)\n\
             (\\title (TEXT \"Title here\"))"
        );
    }

    #[test]
    fn non_md_percent_comment_is_scoped_per_line() {
        // The `%` comment runs only to the end of *its* physical line: a multi-line
        // tag value drops the commented tail of the first line but keeps the next
        // line, then both coalesce under `norm_ws`.
        let src = "#' Title\n\
                   #' @details First detail line %% trailing comment\n\
                   #'   second detail line stays\n\
                   #' @name x\n\
                   NULL\n";
        // Sections sort in byte order: `\description` < `\details` < `\title`.
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Title\"))\n\
             (\\details (TEXT \"First detail line second detail line stays\"))\n\
             (\\title (TEXT \"Title\"))"
        );
    }

    #[test]
    fn md_mode_percent_survives() {
        // Under `@md` roxygen2 escapes `%` (`\%`), which `parse_Rd` decodes back to a
        // literal `%`, so the character survives in the projected text — the
        // projector must *not* treat it as a comment in markdown mode.
        let src = "#' Title\n\
                   #' @md\n\
                   #' @format % and more\n\
                   x <- list(a = 1)\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Title\"))\n\
             (\\format (TEXT \"% and more\"))\n\
             (\\title (TEXT \"Title\"))"
        );
    }

    #[test]
    fn strip_rd_line_comment_honors_backslash_escape() {
        // A `\%` is an escaped percent, not a comment opener.
        assert_eq!(strip_rd_line_comment("a %% b"), "a ");
        assert_eq!(strip_rd_line_comment("%"), "");
        assert_eq!(strip_rd_line_comment("no comment here"), "no comment here");
        assert_eq!(
            strip_rd_line_comment("keep \\% literal"),
            "keep \\% literal"
        );
        assert_eq!(
            strip_rd_line_comment("keep \\% then % cut"),
            "keep \\% then "
        );
    }

    #[test]
    fn block_macro_joins_its_paragraph_then_splits_at_blank_line() {
        // A block macro that directly follows a prose line (no blank `#'` line)
        // belongs to that paragraph; a blank line starts the next paragraph. So
        // here the first `\itemize` rides with the description and the second with
        // the details — roxygen2 splits the intro on `\n\n`, not per CST node.
        let src = "#' Title\n\
                   #'\n\
                   #' Description with some\n\
                   #' \\itemize{\n\
                   #' \\item itemized\n\
                   #' \\item list\n\
                   #' }\n\
                   #'\n\
                   #' And then another one:\n\
                   #' \\itemize{\n\
                   #' \\item item 1\n\
                   #' \\item item 2\n\
                   #' }\n\
                   foo <- function() {}\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Description with some\") \
             (\\itemize (\\item) (TEXT \"itemized\") (\\item) (TEXT \"list\")))\n\
             (\\details (TEXT \"And then another one:\") \
             (\\itemize (\\item) (TEXT \"item 1\") (\\item) (TEXT \"item 2\")))\n\
             (\\title (TEXT \"Title\"))"
        );
    }

    #[test]
    fn trailing_intro_details_merge_with_explicit_details_tag() {
        // When the intro has leftover paragraphs *and* there is an explicit
        // @details tag, roxygen2 folds them into a single \details (intro
        // paragraphs first, then the tag body), rather than two sections.
        let src = "#' Title\n\
                   #'\n\
                   #' Description\n\
                   #'\n\
                   #' Details1\n\
                   #'\n\
                   #' Details2\n\
                   #'\n\
                   #' @details Details3\n\
                   #'\n\
                   #' Details4\n\
                   foo <- function(x) {}\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Description\"))\n\
             (\\details (TEXT \"Details1 Details2 Details3 Details4\"))\n\
             (\\title (TEXT \"Title\"))"
        );
    }

    #[test]
    fn explicit_title_without_description_duplicates_into_description() {
        // roxygen2's title-as-description fallback: an explicit `@title` with no
        // intro prose and no `@description` reuses the title as the description.
        let src = "#' @title a\n#' @name a\nNULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"a\"))\n(\\title (TEXT \"a\"))"
        );
    }

    #[test]
    fn null_tag_value_suppresses_section() {
        // roxygen2's `rd_section()` treats a value of the literal string "NULL" as
        // a sentinel that suppresses the section (`R/field.R`). `@format NULL` and
        // `@details NULL` emit no section at all; `@description NULL` suppresses the
        // explicit description, which re-triggers the title-as-description fallback.
        let src = "#' Title\n\
                   #' @description NULL\n\
                   #' @details NULL\n\
                   #' @format NULL\n\
                   #' @name d\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Title\"))\n(\\title (TEXT \"Title\"))"
        );
    }

    #[test]
    fn sameline_tag_value_folds_plain_continuation() {
        // A tag with a same-line prose value folds its contiguous plain-prose
        // continuation into the `ROXYGEN_TAG` node (see `emit_tag_line`), so the
        // whole field value projects as one run. This exercises `tag_inlines`'
        // handling of the folded threaded `#'` markers (dropped) and inter-line
        // newlines (a soft break `norm_ws` collapses) — no markdown span involved.
        let src = "#' Title\n\
                   #' @details First line\n\
                   #' second line.\n\
                   #' @name d\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Title\"))\n\
             (\\details (TEXT \"First line second line.\"))\n\
             (\\title (TEXT \"Title\"))"
        );
    }

    #[test]
    fn md_table_projects_to_tabular() {
        // A GFM table under `@md` projects to `\tabular`: the delimiter row supplies
        // the per-column alignment (`l`/`c`/`r`), the header and body rows fill one
        // `GRP` with `\tab` between cells and `\cr` ending each row, a short row is
        // padded (an empty trailing cell) and a long row truncated to the column
        // count, and each cell's content resolves as markdown.
        let src = "#' T\n\
                   #' @md\n\
                   #' @details\n\
                   #' | a | b |\n\
                   #' | :-- | --: |\n\
                   #' | *x* | y |\n\
                   #' | solo |\n\
                   #' @name d\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"T\"))\n\
             (\\details (\\tabular (TEXT \"lr\") (GRP \
             (TEXT \"a\") (\\tab) (TEXT \"b\") (\\cr) \
             (\\emph (TEXT \"x\")) (\\tab) (TEXT \"y\") (\\cr) \
             (TEXT \"solo\") (\\tab) (\\cr))))\n\
             (\\title (TEXT \"T\"))"
        );
    }

    #[test]
    fn md_block_quote_flattens_to_plain_text() {
        // roxygen2 does not support block quotes: it warns and renders the node's
        // *flattened plain text* (`escape_comment(xml_text)`) — the `>` markers and
        // inner markdown (emphasis, code, link) dropped, and the two lines concatenated
        // with **no separator** (`code` + `and` glue to `codeand`).
        let src = "#' T\n\
                   #' @md\n\
                   #' @details\n\
                   #' > a *quote* with `code`\n\
                   #' > and [text](https://x.org)\n\
                   #' @name d\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"T\"))\n\
             (\\details (TEXT \"a quote with codeand text\"))\n\
             (\\title (TEXT \"T\"))"
        );
    }

    #[test]
    fn md_block_quote_glues_onto_adjacent_prose() {
        // A block quote emits no paragraph separator, so its flattened text glues
        // onto the surrounding prose with no space: a preceding paragraph on the
        // *same* line (`before` + `> q` → `beforeq`), a preceding paragraph across a
        // *blank* line (still glued), and a following paragraph that keeps its own
        // separating space (`beforeq after`). Two adjacent quotes also glue (`q1q2`).
        let same_part = "#' T\n\
                         #' @md\n\
                         #' @details\n\
                         #' before\n\
                         #' > quoted line\n\
                         #' @name d\n\
                         NULL\n";
        assert_eq!(
            project_to_rd(same_part),
            "(\\description (TEXT \"T\"))\n\
             (\\details (TEXT \"beforequoted line\"))\n\
             (\\title (TEXT \"T\"))"
        );

        let around = "#' T\n\
                      #' @md\n\
                      #' @details\n\
                      #' before\n\
                      #'\n\
                      #' > quoted\n\
                      #'\n\
                      #' after\n\
                      #' @name d\n\
                      NULL\n";
        assert_eq!(
            project_to_rd(around),
            "(\\description (TEXT \"T\"))\n\
             (\\details (TEXT \"beforequoted after\"))\n\
             (\\title (TEXT \"T\"))"
        );

        let two_quotes = "#' T\n\
                          #' @md\n\
                          #' @details\n\
                          #' > q1\n\
                          #'\n\
                          #' > q2\n\
                          #' @name d\n\
                          NULL\n";
        assert_eq!(
            project_to_rd(two_quotes),
            "(\\description (TEXT \"T\"))\n\
             (\\details (TEXT \"q1q2\"))\n\
             (\\title (TEXT \"T\"))"
        );
    }

    #[test]
    fn md_block_quote_lazy_continuation_folds_into_quote() {
        // CommonMark lazy continuation: a non-`>` paragraph line immediately after a
        // quote line (no blank) belongs to the quote's open paragraph, so it flattens
        // into the quote with **no** separator (`quoted line one` + `lazy continuation`
        // → `quoted line onelazy continuation`). A blank line ends the quote; the
        // following paragraph is separate and keeps its own separating space.
        let src = "#' T\n\
                   #' @md\n\
                   #' @details\n\
                   #' > quoted line one\n\
                   #' lazy continuation\n\
                   #'\n\
                   #' Separate paragraph.\n\
                   #' @name d\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"T\"))\n\
             (\\details (TEXT \"quoted line onelazy continuation Separate paragraph.\"))\n\
             (\\title (TEXT \"T\"))"
        );
    }

    #[test]
    fn md_thematic_break_renders_empty_and_coalesces() {
        // roxygen2 has no thematic-break support: it warns and renders the empty
        // `escape_comment(xml_text)` (a break has no text), so the break contributes
        // nothing and the surrounding paragraphs coalesce into one `\details` atom.
        // A `---` after a blank (setext heads nothing), a `***` interrupting a
        // paragraph, and an `___` all render identically.
        let src = "#' T\n\
                   #' @md\n\
                   #' @details\n\
                   #' Before.\n\
                   #'\n\
                   #' ---\n\
                   #'\n\
                   #' Foo\n\
                   #' ***\n\
                   #' bar\n\
                   #'\n\
                   #' ___\n\
                   #'\n\
                   #' After.\n\
                   #' @name d\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"T\"))\n\
             (\\details (TEXT \"Before. Foo bar After.\"))\n\
             (\\title (TEXT \"T\"))"
        );
    }

    #[test]
    fn examples_body_is_a_placeholder() {
        let src = "#' T\n#' @examples\n#' f(1)\n#' @name d\nNULL\n";
        assert!(project_to_rd(src).contains("(\\examples ...)"));
    }

    #[test]
    fn multiple_examples_tags_merge_into_one_section() {
        // roxygen2's `@examples`/`@examplesIf` is an aggregating field: every
        // examples tag of a topic concatenates into a *single* `\examples`
        // section, so the projector emits exactly one `(\examples ...)` no matter
        // how many tags appear.
        let src = "#' @name a\n\
                   #' @title a\n\
                   #' @examples\n\
                   #' TRUE\n\
                   #' @examples\n\
                   #' FALSE\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"a\"))\n\
             (\\examples ...)\n\
             (\\title (TEXT \"a\"))"
        );
    }

    #[test]
    fn md_non_fragile_macro_arg_is_markdown_processed() {
        // Under `@md`, a non-fragile inline text macro (`\emph`) has its argument
        // markdown-processed, so `*x*` becomes a nested `\emph` — matching
        // roxygen2's `\emph{\emph{x}}` (`escaped_for_md` protects only the fragile
        // set). A fragile macro (`\code`) keeps its argument literal.
        let emph = "#' @md\n#' @title T\n#' @details A \\emph{*x*} b.\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(emph)
                .contains("(\\details (TEXT \"A\") (\\emph (\\emph (TEXT \"x\"))) (TEXT \"b.\"))"),
            "{}",
            project_to_rd(emph)
        );

        let multi = "#' @md\n#' @title T\n#' @details A \\emph{a *b* c} d.\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(multi).contains(
                "(\\details (TEXT \"A\") (\\emph (TEXT \"a\") (\\emph (TEXT \"b\")) (TEXT \"c\")) (TEXT \"d.\"))"
            ),
            "{}",
            project_to_rd(multi)
        );

        let strong = "#' @md\n#' @title T\n#' @details A \\strong{*x*} b.\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(strong).contains("(\\strong (\\emph (TEXT \"x\")))"),
            "{}",
            project_to_rd(strong)
        );

        // `\code` is fragile — its body stays literal `*x*` (RCODE), not `\emph`.
        let code = "#' @md\n#' @title T\n#' @details A \\code{*x*} b.\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(code).contains("(\\code (RCODE \"*x*\"))"),
            "{}",
            project_to_rd(code)
        );
    }

    #[test]
    fn md_structural_macro_args_are_markdown_processed() {
        // Under `@md`, a structural two-arg macro (`\item`, `\tabular`, `\href`)
        // has *each* of its non-verbatim arguments markdown-processed, then a
        // multi-atom argument GRP-wraps (parse_Rd models it as a list). roxygen2
        // protects only its `escaped_for_md` set, so `\item`/`\tabular`/`\href`'s
        // text args are markdown while a nested fragile macro (`\code`) stays
        // literal and a verbatim argument (the `\href` URL) is untouched.
        let item = "#' @md\n#' @title T\n#' @details\n#' \\describe{\n\
                    #'   \\item{*term*}{a \\strong{bold} def}\n#' }\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(item).contains(
                "(\\describe (\\item (\\emph (TEXT \"term\")) \
                 (GRP (TEXT \"a\") (\\strong (TEXT \"bold\")) (TEXT \"def\"))))"
            ),
            "{}",
            project_to_rd(item)
        );

        // Both arguments single-atom markdown unwrap (no GRP).
        let two = "#' @md\n#' @title T\n#' @details\n#' \\describe{\n\
                   #'   \\item{*term*}{*def*}\n#' }\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(two)
                .contains("(\\describe (\\item (\\emph (TEXT \"term\")) (\\emph (TEXT \"def\"))))"),
            "{}",
            project_to_rd(two)
        );

        // A nested fragile macro keeps its argument literal even inside an md arg.
        let frag = "#' @md\n#' @title T\n#' @details\n#' \\describe{\n\
                    #'   \\item{x}{a \\code{*y*} b}\n#' }\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(frag).contains(
                "(\\item (TEXT \"x\") (GRP (TEXT \"a\") (\\code (RCODE \"*y*\")) (TEXT \"b\")))"
            ),
            "{}",
            project_to_rd(frag)
        );

        // `\href`: verbatim URL untouched, display markdown-processed and wrapped.
        let href = "#' @md\n#' @title T\n#' @details See \\href{http://x.org}{*the* site}.\n\
                    #' @name x\nNULL\n";
        assert!(
            project_to_rd(href).contains(
                "(\\href (VERB \"http://x.org\") (GRP (\\emph (TEXT \"the\")) (TEXT \"site\")))"
            ),
            "{}",
            project_to_rd(href)
        );

        // `\tabular`: the format string and each cell are markdown, `\tab`/`\cr`
        // preserved; the multi-atom body wraps in `(GRP …)`.
        let tab = "#' @md\n#' @title T\n#' @details\n#' \\tabular{ll}{\n\
                   #'   *a* \\tab **b** \\cr\n#' }\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(tab).contains(
                "(\\tabular (TEXT \"ll\") (GRP (\\emph (TEXT \"a\")) (\\tab) (\\strong (TEXT \"b\")) (\\cr)))"
            ),
            "{}",
            project_to_rd(tab)
        );
    }

    #[test]
    fn md_structural_macro_arg_emphasis_spans_nested_macro() {
        // roxygen2 resolves a structural argument as **one** cmark run, so an
        // emphasis span crosses a nested Rd macro (the macro is opaque text to
        // cmark, reconstituted afterward). arity must do the same rather than
        // splitting the run at the macro and leaving the `*` delimiters literal.
        //
        // `\item{x}{*a \strong{y} b*}` → the `\emph` wraps the whole second
        // argument *including* the `\strong`, so the argument is a single atom
        // (no `(GRP …)`).
        let item = "#' @md\n#' @title T\n#' @details\n#' \\describe{\n\
                    #'   \\item{x}{*a \\strong{y} b*}\n#' }\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(item).contains(
                "(\\item (TEXT \"x\") (\\emph (TEXT \"a\") (\\strong (TEXT \"y\")) (TEXT \"b\")))"
            ),
            "{}",
            project_to_rd(item)
        );

        // `\tabular`: an emphasis span even crosses a brace-less `\tab` separator
        // (cmark treats `\tab` as literal text). The `\emph` owns `a \tab b`, so
        // the body is `(GRP (\emph a \tab b) \cr)`.
        let tab = "#' @md\n#' @title T\n#' @details\n#' \\tabular{ll}{\n\
                   #'   *a \\tab b* \\cr\n#' }\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(tab).contains(
                "(\\tabular (TEXT \"ll\") (GRP (\\emph (TEXT \"a\") (\\tab) (TEXT \"b\")) (\\cr)))"
            ),
            "{}",
            project_to_rd(tab)
        );
    }

    #[test]
    fn md_emphasis_span_abuts_an_inline_macro() {
        // roxygen2 protects a fragile Rd tag as an alphanumeric placeholder before
        // cmark (`escape_rd_for_md`), so the macro flanks like a letter at its
        // leading edge — a `*` opener abutting the macro can open and the span
        // crosses it. `a*\code{x} y*` → `a` then `\emph{\code{x} y}`.
        let opens = "#' @md\n#' @title T\n#' @details a*\\code{x} y*\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(opens)
                .contains("(\\details (TEXT \"a\") (\\emph (\\code (RCODE \"x\")) (TEXT \"y\")))"),
            "{}",
            project_to_rd(opens)
        );

        // The placeholder ends in `-` (the `-<i>-` suffix), so a `*` closer abutting
        // the macro's trailing edge stays blocked — `a*\code{z}*b` keeps both `*`
        // literal (no emphasis), exactly as roxygen2 leaves it.
        let blocked = "#' @md\n#' @title T\n#' @details a*\\code{z}*b\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(blocked)
                .contains("(\\details (TEXT \"a*\") (\\code (RCODE \"z\")) (TEXT \"*b\"))"),
            "{}",
            project_to_rd(blocked)
        );
    }

    #[test]
    fn md_macro_arg_resolution_is_off_without_md() {
        // Without `@md`, `*x*` is literal Rd prose inside the macro (no emphasis).
        let src = "#' @title T\n#' @details A \\emph{*x*} b.\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(src).contains("(\\emph (TEXT \"*x*\"))"),
            "{}",
            project_to_rd(src)
        );
    }

    #[test]
    fn md_link_display_with_active_markdown_macro_drops() {
        // A shortcut link whose display carries a macro with cmark-active markdown
        // (`\emph{*x*}`) is dropped ("markdown links must contain plain text"); the
        // surrounding prose coalesces. A macro with a literal arg (`\emph{x}`) keeps
        // the link, and a fragile `\code{*x*}` keeps it too (its body is protected).
        let drop = "#' @md\n#' @title T\n#' @details See [a\\emph{*x*}] here.\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(drop).contains("(\\details (TEXT \"See here.\"))"),
            "{}",
            project_to_rd(drop)
        );

        let keep_plain =
            "#' @md\n#' @title T\n#' @details See [a\\emph{x}] here.\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(keep_plain).contains("(\\link (TEXT \"a\") (\\emph (TEXT \"x\")))"),
            "{}",
            project_to_rd(keep_plain)
        );

        let keep_code =
            "#' @md\n#' @title T\n#' @details See [a\\code{*x*}] here.\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(keep_code).contains("(\\link (TEXT \"a\") (\\code (RCODE \"*x*\")))"),
            "{}",
            project_to_rd(keep_code)
        );

        // Recursive: a nested non-fragile `\strong{*y*}` makes the display active.
        // (The display carries leading text `x`, so its truncated link-reference
        // label stays self-consistent — a *macro-only* display like `[\emph{…}]`
        // hits the empty-label demotion edge and is deferred to backlog.)
        let drop_nested = "#' @md\n#' @title T\n#' @details See [x \\emph{a \\strong{*y*}}] here.\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(drop_nested).contains("(\\details (TEXT \"See here.\"))"),
            "{}",
            project_to_rd(drop_nested)
        );
    }

    #[test]
    fn md_nested_fragile_macro_stays_literal() {
        // A fragile `\code` nested inside a non-fragile `\emph`: the outer arg is
        // markdown-processed, but `\code`'s own body stays literal (recursive
        // fragility check) — `(\emph (TEXT "a") (\code (RCODE "*x*")) (TEXT "b"))`.
        let src =
            "#' @md\n#' @title T\n#' @details A \\emph{a \\code{*x*} b} c.\n#' @name x\nNULL\n";
        assert!(
            project_to_rd(src)
                .contains("(\\emph (TEXT \"a\") (\\code (RCODE \"*x*\")) (TEXT \"b\"))"),
            "{}",
            project_to_rd(src)
        );
    }

    #[test]
    fn projects_inline_rd_macros() {
        // Nested latexlike macros, a dropped `[pkg]` option, and a verbatim
        // `\url` (VERB, not coalesced TEXT) — the faithful translation of the
        // CST macro nodes into roxygen2's Rd section shape.
        let src = "#' T\n\
                   #'\n\
                   #' See \\code{\\link{add}} and \\emph{e}, plus \\url{http://x}\n\
                   #' and \\link[stats]{lm} end.\n\
                   #' @name d\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\description (TEXT \"See\") (\\code (\\link (TEXT \"add\"))) \
                 (TEXT \"and\") (\\emph (TEXT \"e\")) (TEXT \", plus\") \
                 (\\url (VERB \"http://x\")) (TEXT \"and\") (\\link (TEXT \"lm\")) \
                 (TEXT \"end.\"))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn code_macro_body_projects_as_rcode() {
        // parse_Rd tags a `\code` body as verbatim R code: its plain text becomes
        // `(RCODE …)`, not the whitespace-normalized `(TEXT …)` every other
        // latexlike macro produces (`\verb` stays VERB; a nested macro recurses).
        let src = "#' T\n\
                   #'\n\
                   #' Some \\code{code} and \\verb{More code.}\n\
                   #' @name d\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\description (TEXT \"Some\") (\\code (RCODE \"code\")) (TEXT \"and\") \
                 (\\verb (VERB \"More code.\")))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn href_projects_verbatim_url_and_latexlike_text() {
        // `\href{url}{text}` is a two-arg *structural* macro with a per-argument
        // encoding: parse_Rd tags the first argument (the URL) as verbatim `VERB`
        // and sub-parses the second (the link text) like any latexlike body, so a
        // multi-atom link text wraps in `(GRP …)` and nested macros recurse.
        let src = "#' T\n\
                   #'\n\
                   #' See \\href{http://a.com/x y}{click \\emph{here} now}.\n\
                   #' @name d\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\description (TEXT \"See\") (\\href (VERB \"http://a.com/x y\") \
                 (GRP (TEXT \"click\") (\\emph (TEXT \"here\")) (TEXT \"now\"))) (TEXT \".\"))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn inline_link_code_span_text_subrenders() {
        // roxygen2 renders the markdown *children* of a link, so a code-span link
        // text becomes `\verb`/`\code` (via `mdxml_code`) rather than literal
        // prose. An **inline** `[text](url)` carries that rendered span as its
        // `\href` text argument; a **reference** `[text][ref]` keeps the always-
        // `\code` wrap around the whole `\link` (the has-link-text branch).
        let src = "#' Title\n\
                   #'\n\
                   #' Description, see [`code link text`][func].\n\
                   #' And also [`code as well`](https://external.com).\n\
                   #' @md\n\
                   foo <- function() {}\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Description, see\") \
             (\\code (\\link (TEXT \"code link text\"))) (TEXT \". And also\") \
             (\\href (VERB \"https://external.com\") (\\verb (VERB \"code as well\"))) \
             (TEXT \".\"))\n\
             (\\title (TEXT \"Title\"))"
        );
    }

    #[test]
    fn non_plain_shortcut_links_are_dropped() {
        // roxygen2's `parse_link` rejects a shortcut/reference link whose display is
        // not plain text ("markdown links must contain plain text") and renders it as
        // empty, leaving the surrounding prose contiguous: `[*foo*]` (emphasis) and
        // `` [`x` `y`] `` (two code spans) drop, while `[a_b]` (intraword `_` is not
        // emphasis) and `` [`code`] `` (a sole code span) survive.
        let src = "#' @details\n\
                   #' A shortcut [*foo*] is dropped, but [a_b] and [`code`] survive \
                   while [`x` `y`] drops too.\n\
                   #' @md\n\
                   #' @name x\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\details (TEXT \"A shortcut is dropped, but\") (\\link (TEXT \"a_b\")) \
             (TEXT \"and\") (\\code (\\link (TEXT \"code\"))) (TEXT \"survive while drops too.\"))"
        );
    }

    #[test]
    fn non_plain_reference_links_are_dropped() {
        // The reference (`[text][ref]`) analog of the shortcut drop: a reference
        // whose synthesized `R:` destination links as `\link` requires plain-text
        // display, so `[*foo*][r1]` (emphasis) and `` [`x` `y`][r4] `` (two code
        // spans) drop, while `[plain][r2]` (plain) and `` [`code`][r3] `` (a sole
        // code span) survive. All reference displays are now carved onto the arena
        // (`same_line_bracket_opener`) as `ROXYGEN_MD_LINK` nodes, reaching the same
        // projection the opaque leaf used to.
        let src = "#' @details\n\
                   #' A reference [*foo*][r1] is dropped, but [plain][r2] and \
                   [`code`][r3] survive while [`x` `y`][r4] drops too.\n\
                   #' @md\n\
                   #' @name x\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\details (TEXT \"A reference is dropped, but\") (\\link (TEXT \"plain\")) \
             (TEXT \"and\") (\\code (\\link (TEXT \"code\"))) (TEXT \"survive while drops too.\"))"
        );
    }

    #[test]
    fn link_display_droppable_boundary() {
        // A sole code span is unwrapped and allowed; pure text is allowed; anything
        // richer (emphasis, a second code span, an autolink) drops the link.
        assert!(!link_display_is_droppable(&[Inline::MdCode("x".into())]));
        assert!(!link_display_is_droppable(&[Inline::Text("a_b".into())]));
        assert!(link_display_is_droppable(&[Inline::MdEmphasis {
            strong: false,
            children: vec![Inline::Text("foo".into())],
        }]));
        assert!(link_display_is_droppable(&[
            Inline::MdCode("x".into()),
            Inline::Text(" ".into()),
            Inline::MdCode("y".into()),
        ]));
        assert!(link_display_is_droppable(&[Inline::MdLink(
            "<https://e.org>".into()
        )]));
    }

    #[test]
    fn multiline_itemize_projects_nested() {
        // A multi-line `\itemize` block macro: each `\item` is a name-only nested
        // macro, its trailing prose a sibling `(TEXT …)` --- the pinned shape, from
        // the kind-based `serialize_macro` walking the block-macro node.
        let src = "#' @details\n\
                   #' \\itemize{\n\
                   #'   \\item one\n\
                   #'   \\item two\n\
                   #' }\n\
                   #' @name x\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\details (\\itemize (\\item) (TEXT \"one\") (\\item) (TEXT \"two\")))"
        );
    }

    #[test]
    fn multiline_describe_item_projects_two_args() {
        // A multi-line `\describe` whose `\item{term}{def}` takes *two* brace
        // groups (Stage 3): the lexer pulls both groups into one macro token, the
        // tree builder emits both as `\item` children, and the projector flushes
        // at each closing `}` so they stay separate atoms ---
        // `(\item (TEXT "a") (TEXT "first"))`, byte-identical to roxygen2.
        let src = "#' T\n\
                   #' @format A frame:\n\
                   #' \\describe{\n\
                   #'   \\item{a}{first}\n\
                   #'   \\item{b}{second}\n\
                   #' }\n\
                   #' @name d\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\describe (\\item (TEXT \"a\") (TEXT \"first\")) \
                 (\\item (TEXT \"b\") (TEXT \"second\")))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn multiline_tabular_projects_format_and_grp_body() {
        // A multi-line `\tabular{format}{content}`: the format arg projects to a
        // single `(TEXT …)`, the multi-row body to a `(GRP …)` (parse_Rd models
        // each `\tabular` argument as a list, so a multi-atom one wraps), with
        // `\tab`/`\cr` as name-only macros --- byte-identical to roxygen2.
        let src = "#' T\n\
                   #' @details\n\
                   #' \\tabular{rl}{\n\
                   #'   a \\tab the first row \\cr\n\
                   #'   b \\tab the second row \\cr\n\
                   #' }\n\
                   #' @name d\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\details (\\tabular (TEXT \"rl\") \
                 (GRP (TEXT \"a\") (\\tab) (TEXT \"the first row\") (\\cr) \
                 (TEXT \"b\") (\\tab) (TEXT \"the second row\") (\\cr))))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn md_inline_projects_emph_strong_and_code_vs_verb() {
        // Under a resolved `@md` mode the inline grammar gains emphasis/strong and
        // markdown code spans. A code span renders as `\code` when its content
        // parses as a single R expression (`a + b`) and `\verb` otherwise (`inline
        // code` is two symbols) --- roxygen2's `can_parse` rule, replicated with
        // arity's own parser.
        let src = "#' T\n\
                   #' @details\n\
                   #' Text with *emphasis*, **strong** words, `inline code`, and `a + b` code.\n\
                   #' @md\n\
                   #' @name d\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\details (TEXT \"Text with\") (\\emph (TEXT \"emphasis\")) (TEXT \",\") \
                 (\\strong (TEXT \"strong\")) (TEXT \"words,\") (\\verb (VERB \"inline code\")) \
                 (TEXT \", and\") (\\code (RCODE \"a + b\")) (TEXT \"code.\"))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn underscore_leading_code_span_is_verb_not_code() {
        // R's lexer rejects any name beginning with `_` (rlang's `parse_expr`
        // errors), so roxygen2's `can_parse` is false and a `` `_` `` code span
        // renders `\verb`. arity's lexer is more lenient (it lexes `_` as an
        // ordinary identifier), so `code_span_is_r` must screen these out.
        assert!(!code_span_is_r("_"));
        assert!(!code_span_is_r("_x"));
        assert!(!code_span_is_r("_foo_"));
        // A lone `_` stays valid as the native-pipe placeholder.
        assert!(code_span_is_r("x |> _$col"));
        // Ordinary names with a non-leading underscore are unaffected.
        assert!(code_span_is_r("a_b"));
    }

    #[test]
    fn md_block_lists_project_itemize_and_enumerate() {
        // Under a resolved `@md` mode, a `-`/`*`/`+` list projects to `\itemize`
        // and a `1.`/`1)` list to `\enumerate`, each item a name-only `\item`
        // ahead of its content --- roxygen2's translation of a markdown list into
        // Rd, replicated from the `ROXYGEN_MD_LIST` node.
        let src = "#' T\n\
                   #' @details\n\
                   #' Bullets:\n\
                   #'\n\
                   #' - first\n\
                   #' - second\n\
                   #'\n\
                   #' Numbered:\n\
                   #'\n\
                   #' 1. one\n\
                   #' 2. two\n\
                   #' @md\n\
                   #' @name d\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\details (TEXT \"Bullets:\") \
                 (\\itemize (\\item) (TEXT \"first\") (\\item) (TEXT \"second\")) \
                 (TEXT \"Numbered:\") \
                 (\\enumerate (\\item) (TEXT \"one\") (\\item) (TEXT \"two\")))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn slot_tags_aggregate_into_slots_section() {
        // roxygen2 collects every `@slot` of an S4 class into a single
        // `\section{Slots}{\describe{…}}`, each slot a `\describe` item whose term
        // is the verbatim `\code{name}` and whose definition is the tag's prose.
        let src = "#' Important class.\n\
                   #'\n\
                   #' @slot a slot a\n\
                   #' @slot b slot b\n\
                   setClass('test')\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\section (TEXT \"Slots\") (\\describe \
                 (\\item (\\code (RCODE \"a\")) (TEXT \"slot a\")) \
                 (\\item (\\code (RCODE \"b\")) (TEXT \"slot b\"))))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn field_tags_aggregate_into_fields_section() {
        // The reference-class analog of `@slot`: every `@field` aggregates into a
        // single `\section{Fields}{\describe{…}}` with the same item shape.
        let src = "#' Important class.\n\
                   #'\n\
                   #' @field a field a\n\
                   #' @field b field b\n\
                   setRefClass('test')\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\section (TEXT \"Fields\") (\\describe \
                 (\\item (\\code (RCODE \"a\")) (TEXT \"field a\")) \
                 (\\item (\\code (RCODE \"b\")) (TEXT \"field b\"))))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn slot_with_unbalanced_brace_is_dropped() {
        // roxygen2 parses `@slot` with `tag_two_part`, which runs
        // `rdComplete(x$raw, is_code = FALSE)` on the *raw* tag value and drops the
        // whole tag on a brace imbalance (mode-independent). Only the balanced slot
        // survives the aggregated Slots section.
        let src = "#' Important class.\n\
                   #'\n\
                   #' @slot a sl{ot a\n\
                   #' @slot b slot b\n\
                   setClass('test')\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\section (TEXT \"Slots\") (\\describe \
                 (\\item (\\code (RCODE \"b\")) (TEXT \"slot b\"))))"
            ),
            "got: {out}"
        );
        assert!(!out.contains("slot a"), "dropped slot leaked: {out}");
    }

    #[test]
    fn all_fields_unbalanced_drops_fields_section() {
        // When every `@field` is brace-incomplete, all drop and roxygen2 emits no
        // Fields section at all (the aggregating field is empty).
        let src = "#' Important class.\n\
                   #'\n\
                   #' @field a fi{eld a\n\
                   setRefClass('test')\n";
        let out = project_to_rd(src);
        assert!(
            !out.contains("Fields"),
            "Fields section should be absent: {out}"
        );
    }

    #[test]
    fn slot_with_percent_commented_brace_survives() {
        // `rdComplete` runs on the *raw* value where `%` is a line comment, so an
        // unbalanced `{` after a `%` is commented out and the slot survives.
        let src = "#' Important class.\n\
                   #'\n\
                   #' @slot a desc %{\n\
                   setClass('test')\n";
        let out = project_to_rd(src);
        assert!(out.contains("Slots"), "Slots section should survive: {out}");
    }

    #[test]
    fn section_with_unbalanced_brace_drops_to_na_md_off() {
        // markdown-OFF: `markdown_if_active`'s else-branch runs `rdComplete(x$raw)`
        // unconditionally on the whole `@section` value; a brace imbalance replaces
        // it with "". `roxy_tag_rd` then splits "" on ":" → title="", content=NA →
        // `\section{}{NA}` → `(\section (TEXT "NA"))`.
        let src = "#' @title T\n\
                   #' @section Heading:\n\
                   #'   body with brace {\n\
                   #' @name x\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(out.contains("(\\section (TEXT \"NA\"))"), "got: {out}");
        assert!(!out.contains("Heading"), "dropped title leaked: {out}");
    }

    #[test]
    fn section_with_percent_commented_brace_survives_md_off() {
        // The raw `rdComplete` treats `%` as a line comment, so a `{` after a `%` is
        // commented out and the section renders normally (not dropped to NA).
        let src = "#' @title T\n\
                   #' @section Heading:\n\
                   #'   body %{\n\
                   #' @name x\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains("(\\section (TEXT \"Heading\") (TEXT \"body\"))"),
            "got: {out}"
        );
    }

    #[test]
    fn section_unbalanced_brace_not_dropped_md_on() {
        // markdown-ON: `@section` uses `tag_markdown` with `sections = FALSE`, so the
        // per-section `rdComplete` drop never fires — the body is not replaced by NA
        // (roxygen2 emits the imbalanced content as-is).
        let src = "#' @md\n\
                   #' @title T\n\
                   #' @section Heading:\n\
                   #'   body with brace {\n\
                   #' @name x\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(
            !out.contains("(\\section (TEXT \"NA\"))"),
            "md-on @section must not drop to NA: {out}"
        );
        assert!(out.contains("Heading"), "title should survive: {out}");
    }

    #[test]
    fn md_block_list_is_off_without_md_tag() {
        // No `@md`: the `-` lines stay literal Rd prose (no `\itemize`), one
        // coalesced `(TEXT …)` --- the CST, and thus the projection, is mode-keyed.
        let src = "#' T\n\
                   #' @details\n\
                   #' - first\n\
                   #' - second\n\
                   #' @name d\n\
                   NULL\n";
        assert!(
            project_to_rd(src).contains("(\\details (TEXT \"- first - second\"))"),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn md_inline_is_off_without_md_tag() {
        // No `@md`: markdown is not resolved, so `*emphasis*` and `` `code` `` stay
        // literal Rd prose (one coalesced `(TEXT …)`, delimiters included) --- the
        // CST, and thus the projection, is mode-keyed.
        let src = "#' T\n\
                   #' @details\n\
                   #' Text with *emphasis* and `code` here.\n\
                   #' @name d\n\
                   NULL\n";
        assert!(
            project_to_rd(src)
                .contains("(\\details (TEXT \"Text with *emphasis* and `code` here.\"))"),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn norm_ws_collapses_ascii_but_preserves_unicode_whitespace() {
        // ASCII whitespace runs collapse to a single space and the ends trim.
        assert_eq!(norm_ws("  a \t\n b  "), "a b");
        // Non-ASCII Unicode whitespace (NBSP, NEL) is preserved verbatim --- the
        // R driver's `[[:space:]]` is ASCII-only even in a UTF-8 locale.
        assert_eq!(norm_ws("*\u{a0}a\u{a0}*"), "*\u{a0}a\u{a0}*");
        assert_eq!(norm_ws("x\u{85}y"), "x\u{85}y");
    }

    #[test]
    fn nbsp_cannot_flank_emphasis_stays_literal() {
        // A NBSP is Unicode whitespace, so the `*`s around `\u{a0}a\u{a0}` cannot
        // flank --- no `\emph`, the literal text (NBSP intact) survives. (cm-355)
        let src = "#' @md\n\
                   #' @title T\n\
                   #' @details\n\
                   #' *\u{a0}a\u{a0}*\n\
                   #' @name spec\n\
                   NULL\n";
        assert!(
            project_to_rd(src).contains("(\\details (TEXT \"*\u{a0}a\u{a0}*\"))"),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn unescape_md_brackets_consumes_one_backslash_before_a_bracket() {
        // `\[`/`\]` lose exactly one backslash; a deeper run keeps the rest.
        assert_eq!(unescape_md_brackets(r"\[x\]"), "[x]");
        assert_eq!(unescape_md_brackets(r"\\[x"), r"\[x");
        // Other escapes are untouched (only brackets are special in roxygen2).
        assert_eq!(
            unescape_md_brackets(r"foo \* \` \% bar"),
            r"foo \* \` \% bar"
        );
        // A backslash not adjacent to a bracket (e.g. at a line break) is kept.
        assert_eq!(unescape_md_brackets("a\\\n[b"), "a\\\n[b");
    }

    #[test]
    fn collapse_md_backslash_runs_halves_a_run() {
        // A run of `k` source backslashes renders as `ceil(k/2)` (double_escape
        // doubles, cmark and parse_Rd each collapse pairs): `\\` → `\`,
        // `\\\\` → `\\`, but a lone `\` (`\*`, `\_`, …) is unchanged.
        assert_eq!(collapse_md_backslash_runs(r"a \ b"), r"a \ b");
        assert_eq!(collapse_md_backslash_runs(r"a \\ b"), r"a \ b");
        assert_eq!(collapse_md_backslash_runs(r"a \\\\ b"), r"a \\ b");
        assert_eq!(collapse_md_backslash_runs(r"a \\\\\\ b"), r"a \\\ b");
        assert_eq!(collapse_md_backslash_runs(r"\* \_ \%"), r"\* \_ \%");
        // A run abutting a bracket is left verbatim for `unescape_md_brackets`.
        assert_eq!(collapse_md_backslash_runs(r"\\[x"), r"\\[x");
        assert_eq!(collapse_md_backslash_runs(r"a\\]b"), r"a\\]b");
    }

    #[test]
    fn braceless_item_projects_as_unknown_node() {
        // A brace-less `\item` outside a list is parse_Rd's out-of-list recovery:
        // an `(UNKNOWN "\item")` node splitting the surrounding text (mode-
        // independent). It can start, sit mid-prose, or end a line; an escaped
        // `\\item` stays literal; two items on a line split twice.
        let src = "#' T\n\
                   #' @details a \\item b. c \\item. d \\\\item e. f \\item g \\item h.\n\
                   #' @name x\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"T\"))\n\
             (\\details (TEXT \"a\") (UNKNOWN \"\\\\item\") (TEXT \"b. c\") (UNKNOWN \"\\\\item\") \
             (TEXT \". d \\\\item e. f\") (UNKNOWN \"\\\\item\") (TEXT \"g\") (UNKNOWN \"\\\\item\") \
             (TEXT \"h.\"))\n\
             (\\title (TEXT \"T\"))"
        );
    }

    #[test]
    fn braceless_sticky_swallows_tail_per_line() {
        // A brace-less `\code`/`\verb` leaves parse_Rd in R-code/verbatim mode: the
        // dropped `\name` and everything after it, to section end, becomes one
        // `RCODE`/`VERB` atom per physical source line. The prose *before* the
        // trigger stays ordinary text. Non-`@md`: a continuation line keeps all but
        // the one `#'`-marker space.
        let src = "#' T\n\
                   #' @details a \\code z here\n\
                   #'   cont line.\n\
                   #' @seealso b \\verb c d.\n\
                   #' @name x\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"T\"))\n\
             (\\details (TEXT \"a\") (RCODE \" z here\\n\") (RCODE \"  cont line.\\n\"))\n\
             (\\seealso (TEXT \"b\") (VERB \" c d.\\n\"))\n\
             (\\title (TEXT \"T\"))"
        );
    }

    #[test]
    fn braceless_sticky_md_strips_continuation_indent() {
        // Under `@md`, cmark strips a continuation line's remaining leading
        // whitespace before the swallow captures it (`#'   cont` → flush `cont`),
        // unlike non-`@md` (two spaces survive above).
        let src = "#' T\n\
                   #' @md\n\
                   #' @details a \\code z here\n\
                   #'   cont line.\n\
                   #' @name x\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"T\"))\n\
             (\\details (TEXT \"a\") (RCODE \" z here\\n\") (RCODE \"cont line.\\n\"))\n\
             (\\title (TEXT \"T\"))"
        );
    }

    #[test]
    fn braceless_sticky_withholds_impure_tail() {
        // A tail carrying a real macro node still parses inside the swallow (it
        // splits the RCODE), and a bare `{`/`}` or `%` breaks the section or acts
        // as a comment — none are modeled at section granularity, so the swallow is
        // withheld and the `\code` stays literal prose (its prior projection).
        for tail in [
            "\\emph{x} after", // a macro node in the tail
            "{grp} after",     // a bare brace group
            "50% done",        // an Rd `%` comment
        ] {
            let src = format!("#' T\n#' @details a \\code z {tail}\n#' @name x\nNULL\n",);
            let out = project_to_rd(&src);
            assert!(
                !out.contains("(RCODE"),
                "expected withhold (no swallow) for tail {tail:?}, got: {out}"
            );
        }
    }

    #[test]
    fn braceless_drop_macro_vanishes_from_text() {
        // parse_Rd's drop-recovery: an unpaired `\` re-forming a brace-required
        // known macro without its `{` drops the `\name`; the text continues.
        assert_eq!(
            resolve_rd_text_escapes(r"before \emph z after"),
            "before  z after"
        );
        // The drop applies at end of input (the `{` never arrives) …
        assert_eq!(
            resolve_rd_text_escapes(r"end of line \strong"),
            "end of line "
        );
        // … and to section-header names misused mid-prose.
        assert_eq!(resolve_rd_text_escapes(r"a \title z"), "a  z");
        // An even run is a literal backslash, not a macro: no drop.
        assert_eq!(resolve_rd_text_escapes(r"a \\emph z"), r"a \emph z");
        // Sticky names (code/verbatim mode-flip, `\item`) stay literal (backlog).
        assert_eq!(resolve_rd_text_escapes(r"a \code z"), r"a \code z");
        assert_eq!(resolve_rd_text_escapes(r"a \item z"), r"a \item z");
        // The `@md` collapse mirrors the drop, keyed on the original run parity:
        // odd runs drop the name and keep the paired `k/2` backslashes.
        assert_eq!(collapse_md_backslash_runs(r"a \emph z"), "a  z");
        assert_eq!(collapse_md_backslash_runs(r"a \\\link q"), r"a \ q");
        assert_eq!(collapse_md_backslash_runs(r"a \\emph z"), r"a \emph z");
        assert_eq!(collapse_md_backslash_runs(r"a \code z"), r"a \code z");
    }

    #[test]
    fn md_percent_swallow_is_parity_keyed() {
        // A bare `%` (even run, k=0) stays literal.
        assert_eq!(md_percent_swallow("a % b"), "a % b");
        // A lone `\%` (odd) comments to end of line; the escaping `\` is kept
        // (later halved to `ceil(1/2) == 1` by collapse_md_backslash_runs).
        assert_eq!(md_percent_swallow(r"a \% b"), "a \\");
        // `\\%` (even) survives literal; `\\\%` (odd) swallows, keeping 3 `\`.
        assert_eq!(md_percent_swallow(r"a \\% b"), r"a \\% b");
        assert_eq!(md_percent_swallow(r"a \\\% b"), "a \\\\\\");
        // The first odd `%` wins even when a bare `%` precedes it on the line.
        assert_eq!(md_percent_swallow(r"a % b \% c"), "a % b \\");
        // Line-scoped: a continuation on the next physical line survives.
        assert_eq!(md_percent_swallow("a \\% b\nc"), "a \\\nc");
        // The physical line ends at a soft-wrap (SOFT_BREAK) too, not just a
        // paragraph break: the continuation on the next `#'` line survives.
        assert_eq!(
            md_percent_swallow(&format!("a \\% b{SOFT_BREAK}c")),
            "a \\\nc"
        );
    }

    #[test]
    fn strip_rd_comments_stops_at_soft_wrap() {
        // A non-`@md` `%` comment ends at the physical source line. Both a
        // paragraph break (`\n`) and a soft-wrap (SOFT_BREAK) end the line, so a
        // continuation on the next `#'` line survives the comment either way.
        assert_eq!(strip_rd_comments("a % swallowed\nc"), "a \nc");
        assert_eq!(
            strip_rd_comments(&format!("a % swallowed{SOFT_BREAK}c")),
            "a \nc"
        );
    }

    #[test]
    fn md_escaped_bracket_is_literal_with_the_backslash_consumed() {
        // Under `@md`, an escaped `\[` neither opens a link nor keeps its
        // backslash: roxygen2 renders `\[text](url)` as the literal `[text](url)`
        // (the `double_escape_md` bracket revert + cmark escape). The lexer
        // suppresses the link; the projector drops the backslash.
        let src = "#' @md\n\
                   #' @title T\n\
                   #' @details\n\
                   #' A \\[bracket](x) and \\[shortcut] stay literal.\n\
                   #' @name spec\n\
                   NULL\n";
        assert!(
            project_to_rd(src)
                .contains("(\\details (TEXT \"A [bracket](x) and [shortcut] stay literal.\"))"),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn shortcut_link_node_atom_resolves_text_and_code() {
        // A plain-text display is the destination: `\link{text}` (text coalesced).
        assert_eq!(
            shortcut_link_node_atom(&[Inline::Text("cross-line shortcut".to_string())]),
            "(\\link (TEXT \"cross-line shortcut\"))"
        );
        // A single code-span display is `\code`-wrapped, mirroring `shortcut_link_atom`.
        assert_eq!(
            shortcut_link_node_atom(&[Inline::MdCode("f".to_string())]),
            "(\\code (\\link (TEXT \"f\")))"
        );
    }

    #[test]
    fn md_cross_line_shortcut_link_joins_into_one_link() {
        // Under `@md`, a shortcut link `[text]` whose `[` opens on an earlier `#'`
        // line resolves into one `\link{text}` over the coalesced text; a stray `]`
        // with no opener stays literal (matching roxygen2).
        let src = "#' @md\n\
                   #' @title T\n\
                   #' @details\n\
                   #' A [broken\n\
                   #' across lines] joins, but a stray a] stays.\n\
                   #' @name spec\n\
                   NULL\n";
        assert!(
            project_to_rd(src).contains(
                "(\\details (TEXT \"A\") (\\link (TEXT \"broken across lines\")) \
                 (TEXT \"joins, but a stray a] stays.\"))"
            ),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn double_escape_md_reverts_only_bracket_escapes() {
        // Every backslash is doubled, then the two bracket escapes are reverted —
        // so a bracket escape survives unchanged, every other escape is neutralized.
        assert_eq!(double_escape_md("[text\\]"), "[text\\]");
        assert_eq!(double_escape_md("a\\*b"), "a\\\\*b");
        assert_eq!(double_escape_md("\\[x\\]"), "\\[x\\]");
        // Two source backslashes before `]` become three (2*2 then revert one pair).
        assert_eq!(double_escape_md("[text\\\\]"), "[text\\\\\\]");
    }

    #[test]
    fn url_encode_matches_r_urlencode() {
        // Alphanumerics and the unreserved/sub-delim set pass through; everything
        // else is percent-encoded uppercase (`\`→%5C, space→%20, `%`→%25).
        assert_eq!(url_encode("text\\"), "text%5C");
        assert_eq!(url_encode("a b"), "a%20b");
        assert_eq!(url_encode("a/b:c"), "a/b:c");
        assert_eq!(url_encode("100%"), "100%25");
    }

    #[test]
    fn cmark_unescape_drops_backslash_before_punctuation() {
        assert_eq!(cmark_unescape("[text\\]: R:text%5C"), "[text]: R:text%5C");
        // An escaped backslash collapses, then the escaped bracket (the multi-
        // backslash leaked-definition shape).
        assert_eq!(cmark_unescape("[text\\\\\\]"), "[text\\]");
        // A backslash before a non-punctuation char is kept.
        assert_eq!(cmark_unescape("a\\b"), "a\\b");
    }

    #[test]
    fn md_linkref_labels_ports_get_md_linkrefs() {
        // A bare shortcut; the second `[ref]` group wins as the label.
        assert_eq!(md_linkref_labels("see [foo] now"), vec!["foo".to_string()]);
        assert_eq!(md_linkref_labels("[text][ref]"), vec!["ref".to_string()]);
        // Lookbehind: a `[` preceded by `\` (an escaped-open bracket) is no match.
        assert!(md_linkref_labels("\\[foo]").is_empty());
        // Lookahead: a `[…]` immediately followed by `[` or `{` is no match.
        assert!(md_linkref_labels("[a]{x}").is_empty());
        // The escaped-close shortcut still matches (its `]` closes the content).
        assert_eq!(md_linkref_labels("[text\\]"), vec!["text\\".to_string()]);
    }

    #[test]
    fn linkref_label_closes_on_even_trailing_backslashes() {
        assert!(linkref_label_closes("text")); // 0 trailing — valid definition
        assert!(!linkref_label_closes("text\\")); // 1 trailing — `]` escaped, leaks
        assert!(!linkref_label_closes("text\\\\\\")); // 3 trailing — leaks
        assert!(linkref_label_closes("text\\\\")); // 2 trailing — `]` not escaped
    }

    #[test]
    fn leaked_linkref_text_leaks_from_first_invalid_definition() {
        // An escaped-close shortcut leaks its synthesized definition; a valid
        // shortcut before any invalid one does not (roxygen2 links it).
        assert_eq!(
            leaked_linkref_text("see [text\\] here"),
            vec!["[text]: R:text%5C".to_string()]
        );
        assert!(leaked_linkref_text("see [foo] here").is_empty());
        // Multiple escaped-close candidates each leak (all-invalid block).
        assert_eq!(
            leaked_linkref_text("a [one\\] b [two\\] c"),
            vec!["[one]: R:one%5C".to_string(), "[two]: R:two%5C".to_string()]
        );
        // An escaped-open `\[…]` is excluded by the lookbehind — no leak.
        assert!(leaked_linkref_text("an escaped \\[x\\] stays").is_empty());
        // Poisoning: the first invalid definition swallows the rest of the block, so
        // a *valid* candidate after it leaks too (and is de-linked elsewhere).
        assert_eq!(
            leaked_linkref_text("a [one] b [two\\] c [three] d"),
            vec![
                "[two]: R:two%5C".to_string(),
                "[three]: R:three".to_string()
            ]
        );
    }

    #[test]
    fn first_invalid_linkref_offset_finds_the_poison_bracket() {
        // The opening `[` of the first escaped-close candidate (`[two\]` at index 10).
        assert_eq!(
            first_invalid_linkref_offset("a [one] b [two\\] c"),
            Some(10)
        );
        // All candidates close → no poisoning.
        assert_eq!(first_invalid_linkref_offset("[foo] [bar]"), None);
        // A leading escaped-close candidate poisons from the start.
        assert_eq!(first_invalid_linkref_offset("[bad\\] tail"), Some(0));
    }

    #[test]
    fn demoted_link_source_targets_only_definition_backed_links() {
        // Shortcut/reference links lose their (now-leaked) definition → literal text.
        assert_eq!(
            demoted_link_source(&Inline::MdShortcutLink {
                display: vec![Inline::Text("foo".to_string())]
            }),
            Some("[foo]".to_string())
        );
        assert_eq!(
            demoted_link_source(&Inline::MdRefLink {
                dest: "ref".to_string(),
                display: vec![Inline::Text("disp".to_string())]
            }),
            Some("[disp][ref]".to_string())
        );
        assert_eq!(
            demoted_link_source(&Inline::MdLink("[foo]".to_string())),
            Some("[foo]".to_string())
        );
        assert_eq!(
            demoted_link_source(&Inline::MdLink("[t][r]".to_string())),
            Some("[t][r]".to_string())
        );
        // Inline links and autolinks carry their own destination → survive.
        assert_eq!(
            demoted_link_source(&Inline::MdLink("[t](u)".to_string())),
            None
        );
        assert_eq!(
            demoted_link_source(&Inline::MdLink("<http://x>".to_string())),
            None
        );
        assert_eq!(
            demoted_link_source(&Inline::Text("plain".to_string())),
            None
        );
    }

    #[test]
    fn skeleton_exposes_inline_link_brackets_for_leaked_defs() {
        // roxygen2's `get_md_linkrefs` synthesizes a `[text]: R:text` definition for
        // an inline `[text](url)` link too, so the skeleton must surface its `[text]`
        // as a candidate (a single space would hide it). The link itself survives.
        let link = Inline::MdInlineLink {
            url: "https://example.org".to_string(),
            display: vec![Inline::Text("after".to_string())],
        };
        assert_eq!(inline_skeleton_fragment(&link), "[after] ");
        // `skeleton_len` must agree with the fragment, or the boundary offset mapping
        // in `demote_poisoned_links` drifts.
        assert_eq!(skeleton_len(&link), "[after] ".len());
        // An escaped-close candidate poisons the tail; the surviving inline link's
        // definition leaks alongside it.
        let body = vec![Inline::Text("see [stop\\] then ".to_string()), link];
        assert_eq!(
            leaked_linkref_text(&inline_source_skeleton(&body)),
            vec![
                "[stop]: R:stop%5C".to_string(),
                "[after]: R:after".to_string(),
            ]
        );
        // Without a poison boundary nothing leaks (the def is consumed, not leaked).
        let clean = vec![Inline::MdInlineLink {
            url: "u".to_string(),
            display: vec![Inline::Text("x".to_string())],
        }];
        assert!(leaked_linkref_text(&inline_source_skeleton(&clean)).is_empty());
    }

    #[test]
    fn skeleton_exposes_image_alt_for_leaked_defs() {
        // An image `![alt](url)`'s `[alt]` is a bracket-free candidate too, so the
        // skeleton must surface it (a single space would hide it). The `\figure`
        // survives; only its synthesized `[alt]: R:alt` definition leaks.
        let image = Inline::MdImage("![alt](https://example.org/x.png)".to_string());
        assert_eq!(image_alt_text("![alt](u)"), Some("alt"));
        assert_eq!(inline_skeleton_fragment(&image), "[alt] ");
        assert_eq!(skeleton_len(&image), "[alt] ".len());
        // The image survives poisoning (carries its own destination), never demoted.
        assert_eq!(demoted_link_source(&image), None);
        // An escaped-close candidate poisons the tail; the surviving image's
        // definition leaks alongside it.
        let body = vec![Inline::Text("see [stop\\] then ".to_string()), image];
        assert_eq!(
            leaked_linkref_text(&inline_source_skeleton(&body)),
            vec!["[stop]: R:stop%5C".to_string(), "[alt]: R:alt".to_string()]
        );
    }

    #[test]
    fn skeleton_exposes_opaque_inline_link_inner_bracket_for_leaked_defs() {
        // A nested-bracket display keeps the inline link opaque (the lexer only
        // nodes a bracket-free display), yet `get_md_linkrefs` still finds the
        // *inner* `[b]` candidate (the outer `[a [b] c]` is not a candidate — its
        // content has brackets). The skeleton must surface the display verbatim.
        let link = Inline::MdLink("[a [b] c](https://example.org)".to_string());
        assert_eq!(
            opaque_inline_link_display("[a [b] c](https://example.org)"),
            Some("a [b] c")
        );
        // A shortcut/reference leaf has no `(` after the display; an autolink opens
        // with `<` — neither is an inline-link display.
        assert_eq!(opaque_inline_link_display("[shortcut]"), None);
        assert_eq!(opaque_inline_link_display("[text][ref]"), None);
        assert_eq!(opaque_inline_link_display("<https://example.org>"), None);
        assert_eq!(inline_skeleton_fragment(&link), "[a [b] c] ");
        assert_eq!(skeleton_len(&link), "[a [b] c] ".len());
        // The inline link survives poisoning (carries its own destination).
        assert_eq!(demoted_link_source(&link), None);
        // An escaped-close candidate poisons the tail; the surviving link's inner
        // `[b]` definition leaks alongside it.
        let body = vec![Inline::Text("see [stop\\] then ".to_string()), link];
        assert_eq!(
            leaked_linkref_text(&inline_source_skeleton(&body)),
            vec!["[stop]: R:stop%5C".to_string(), "[b]: R:b".to_string()]
        );
    }

    #[test]
    fn projects_mixed_linkref_poisoning() {
        // The end-to-end mixed case: a valid shortcut before the escaped-close
        // candidate links; the escaped-close poisons the appended definition block,
        // so a later shortcut is de-linked into literal text and *both* trailing
        // definitions leak.
        let src = "#' @md\n\
                   #' @title T\n\
                   #' @details\n\
                   #' See [before] then [stop\\] and [after].\n\
                   #' @name spec\n\
                   NULL\n";
        assert!(
            project_to_rd(src).contains(
                "(\\details (TEXT \"See\") (\\link (TEXT \"before\")) \
                 (TEXT \"then [stop] and [after]. [stop]: R:stop%5C [after]: R:after\"))"
            ),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn linkref_keys_skips_a_label_after_a_closing_bracket() {
        // The `(?<!\])` lookbehind: a `[` right after `]` defines nothing, so a
        // standalone `a][b]` produces an empty link-reference map; a label defined
        // elsewhere (a normal shortcut, or a second `[ref]` group) is present.
        let keys = |s: &str| linkref_keys(&[Inline::Text(s.to_string())]);
        assert!(keys("a][b]").is_empty());
        assert!(keys("a][b] and [b] here").contains("b"));
        assert!(keys("[text][ref]").contains("ref"));
        // Lookahead: a `[…]` followed by `{` defines nothing.
        assert!(keys("[a]{x}").is_empty());
    }

    #[test]
    fn projects_undefined_shortcut_after_bracket_as_literal() {
        // `a][b]` standalone: `b` is never a link-reference candidate (the `[` is
        // preceded by `]`), so roxygen2 leaves it literal — arity must demote its
        // optimistically-resolved `\link{b}` back to text.
        let src = "#' @md\n#' @details\n#' A stray a][b] here.\n#' @name x\nNULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\details (TEXT \"A stray a][b] here.\"))"
        );
    }

    #[test]
    fn projects_undefined_ref_links_only_the_defined_inner_shortcut() {
        // `[a [b] c][ref]`: the inner `[b]` is a defined candidate (links), the
        // outer `[ref]` after a `]` is not (stays literal with its brackets).
        let src = "#' @md\n#' @details\n#' A [a [b] c][ref] link.\n#' @name x\nNULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\details (TEXT \"A [a\") (\\link (TEXT \"b\")) (TEXT \"c][ref] link.\"))"
        );
    }

    #[test]
    fn undefined_shortcut_links_when_defined_elsewhere() {
        // The same `a][b]` resolves when a later standalone `[b]` defines `b` —
        // the full-field refmap, not a position rule (cf. md_ref_link_multiline).
        let src = "#' @md\n#' @details\n#' A stray a][b], later [b].\n#' @name x\nNULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\details (TEXT \"A stray a]\") (\\link (TEXT \"b\")) \
             (TEXT \", later\") (\\link (TEXT \"b\")) (TEXT \".\"))"
        );
    }

    #[test]
    fn projects_undefined_shortcut_inside_a_list_item_as_literal() {
        // The whole-field refmap + undefined-label demotion descend into list
        // items: an `a][b]` inside a list item is undefined (the `[` is preceded
        // by `]`), so roxygen2 keeps it literal — arity must demote its
        // optimistic `\link{b}` inside the `\itemize`.
        let src = "#' @md\n#' @details\n#' Top.\n#'\n\
                   #' - a stray a][b] keeps it\n#' @name x\nNULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\details (TEXT \"Top.\") \
             (\\itemize (\\item) (TEXT \"a stray a][b] keeps it\")))"
        );
    }

    #[test]
    fn projects_self_defined_shortcut_inside_a_list_item_as_link() {
        // A plain `[foo]` shortcut inside a list item self-defines (roxygen2
        // synthesizes `[foo]: R:foo`), so the whole-field refmap keeps it in
        // `keys` and it stays a `\link` — the refmap recursion must not demote a
        // self-defined in-list shortcut.
        let src = "#' @md\n#' @details\n#' Top.\n#'\n\
                   #' - see [foo] here\n#' @name x\nNULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\details (TEXT \"Top.\") \
             (\\itemize (\\item) (TEXT \"see\") (\\link (TEXT \"foo\")) (TEXT \"here\")))"
        );
    }

    #[test]
    fn projects_in_list_poisoning_demotes_a_later_in_list_shortcut() {
        // Whole-field poisoning descends into list items: an escaped-close
        // candidate inside a list item poisons the appended definition block, so a
        // *later* in-list shortcut is de-linked into literal text and both leaked
        // definitions surface as trailing prose.
        let src = "#' @md\n#' @details\n#' Pre [before] links.\n#'\n\
                   #' - an escaped close [stop\\] here\n\
                   #' - a shortcut [foo] after\n#' @name x\nNULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\details (TEXT \"Pre\") (\\link (TEXT \"before\")) (TEXT \"links.\") \
             (\\itemize (\\item) (TEXT \"an escaped close [stop] here\") \
             (\\item) (TEXT \"a shortcut [foo] after\")) \
             (TEXT \"[stop]: R:stop%5C [foo]: R:foo\"))"
        );
    }

    #[test]
    fn projects_in_list_candidate_before_the_boundary_survives() {
        // The boundary maps back through the list's per-item space-guard offsets:
        // a shortcut in an *earlier* item (before the escaped-close candidate)
        // still resolves, while one in a later item is demoted.
        let src = "#' @md\n#' @details\n#' Top.\n#'\n\
                   #' - early [foo] survives\n\
                   #' - an escaped close [stop\\] here\n\
                   #' - [bar] dead\n#' @name x\nNULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\details (TEXT \"Top.\") \
             (\\itemize (\\item) (TEXT \"early\") (\\link (TEXT \"foo\")) (TEXT \"survives\") \
             (\\item) (TEXT \"an escaped close [stop] here\") \
             (\\item) (TEXT \"[bar] dead\")) \
             (TEXT \"[stop]: R:stop%5C [bar]: R:bar\"))"
        );
    }

    #[test]
    fn decode_html_entities_resolves_named_and_numeric_refs() {
        assert_eq!(decode_html_entities("a&amp;b"), "a&b");
        assert_eq!(decode_html_entities("&lt;&gt;&quot;&apos;"), "<>\"'");
        assert_eq!(decode_html_entities("&#65;&#x42;"), "AB");
        // No `&`: byte-identical fast path. Unrecognized name or a bare `&`: verbatim.
        assert_eq!(decode_html_entities("plain"), "plain");
        assert_eq!(decode_html_entities("a&b=1"), "a&b=1");
        assert_eq!(decode_html_entities("&unknown;"), "&unknown;");
    }

    #[test]
    fn parses_a_multiline_linkref_definition() {
        // `[ref]:` then a continuation line carrying the URL resolve to one
        // `\href`; the definition lines are consumed.
        let src = "#' @md\n#' @details\n#' See [ref].\n#'\n\
                   #' [ref]:\n#'   https://example.com\n#' @name x\nNULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\details (TEXT \"See\") \
             (\\href (VERB \"https://example.com\") (TEXT \"ref\")) (TEXT \".\"))"
        );
    }

    #[test]
    fn append_rendered_text_coalesces_into_trailing_text() {
        // Merges into a trailing `(TEXT …)`, round-tripping the escape encoding.
        let mut atoms = vec!["(TEXT \"prose.\")".to_string()];
        append_rendered_text(&mut atoms, "[t]: R:t%5C");
        assert_eq!(atoms, vec!["(TEXT \"prose. [t]: R:t%5C\")".to_string()]);
        // With no trailing prose atom, a fresh `(TEXT …)` is pushed.
        let mut atoms = vec!["(\\link (TEXT \"x\"))".to_string()];
        append_rendered_text(&mut atoms, "[t]: R:t%5C");
        assert_eq!(
            atoms,
            vec![
                "(\\link (TEXT \"x\"))".to_string(),
                "(TEXT \"[t]: R:t%5C\")".to_string()
            ]
        );
    }

    #[test]
    fn projects_escaped_close_bracket_leaked_linkref() {
        // The end-to-end case: a `@md` shortcut whose closing bracket is escaped is
        // not a link, but roxygen2 leaks its synthesized reference definition into
        // the rendered prose (coalesced with the section text).
        let src = "#' @md\n\
                   #' @title T\n\
                   #' @details\n\
                   #' A link like [text\\] leaks.\n\
                   #' @name spec\n\
                   NULL\n";
        assert!(
            project_to_rd(src)
                .contains("(\\details (TEXT \"A link like [text] leaks. [text]: R:text%5C\"))"),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn rd_complete_ports_the_brace_balance_check() {
        // Balanced braces, escaped braces, and `%` line comments are complete.
        assert!(rd_complete("a{b}"));
        assert!(rd_complete("a\\{b")); // escaped `{` not counted
        assert!(rd_complete("\\emph{x}"));
        assert!(rd_complete("a%{")); // `%` comments the unmatched `{`
        assert!(rd_complete("{%}\n}")); // comment ends at newline; `}` then closes
        // Unbalanced or escaped-away closers are incomplete.
        assert!(!rd_complete("a{b"));
        assert!(!rd_complete("a}b"));
        assert!(!rd_complete("\\emph{\\}")); // trailing `\` escapes the closing `}`
        assert!(!rd_complete("a\\")); // a dangling escape is incomplete
        assert!(!rd_complete("{%}")); // comment swallows the `}`; `{` stays open
    }

    #[test]
    fn section_atoms_rd_complete_reconstructs_braces() {
        // A balanced inline macro projects complete; a `%` in prose is re-escaped
        // (no comment), so a following structural `}` still closes.
        assert!(section_atoms_rd_complete(
            &["(TEXT \"foo\")".into(), "(\\emph (TEXT \"x\"))".into()],
            true,
        ));
        assert!(section_atoms_rd_complete(
            &["(\\emph (TEXT \"a % b\"))".into()],
            true,
        ));
        // A `%` inside a verbatim URL is escaped too (roxygen2 renders `\%`), so an
        // `\href{…%20…}{…}` stays complete rather than the URL commenting out the
        // closing braces.
        assert!(section_atoms_rd_complete(
            &["(\\href (VERB \"https://x/a%20b\") (TEXT \"link % text\"))".into()],
            true,
        ));
        // An emphasis whose content is a lone backslash renders `\emph{\}`, whose
        // trailing `\` escapes the closing brace --- exactly roxygen2's `*\**` bug.
        assert!(!section_atoms_rd_complete(
            &["(TEXT \"foo\")".into(), "(\\emph (TEXT \"\\\\\"))".into()],
            true,
        ));
    }

    #[test]
    fn trailing_percent_swallow_does_not_false_drop() {
        // An odd-run `\%` swallow at a section's end keeps a dangling `\` in the
        // output atom (`y \% {z} end.` renders `y \`), but roxygen2 scans
        // `markdown(text)` = `y \\% {z} end.` (even run pairs, bare `%` comments) and
        // keeps the section. The drop scan must strip the whole region so no trailing
        // escape survives. A soft-wrap continuation already resolved the escape; a
        // physical line end did not (the bug).
        assert!(section_rd_complete(
            &[Inline::Text("y \\% {z} end.".into())],
            true,
        ));
        // A longer odd run behaves the same (comments to end of line).
        assert!(section_rd_complete(
            &[Inline::Text("a \\\\\\% b {c} d.".into())],
            true,
        ));
        // An even-run `%` is a genuine literal percent (escaped, not a comment): a
        // following unbalanced `{` still drops.
        assert!(!section_rd_complete(
            &[Inline::Text("a % b {c".into())],
            true
        ));
        // A real brace imbalance before the `%` still drops (`{` opens, the comment
        // eats the closer).
        assert!(!section_rd_complete(
            &[Inline::Text("{a \\% b}".into())],
            true,
        ));
        // The stripper drops the run + `%` + line tail, but keeps later physical
        // lines and an even-run `%`.
        assert_eq!(strip_scan_percent_comment("y \\% {z} end."), "y ");
        assert_eq!(
            strip_scan_percent_comment("y \\% gone\nkept % here"),
            "y \nkept % here"
        );
    }

    #[test]
    fn fragile_macro_arg_never_unbalances_the_scan() {
        // A fragile macro's argument is raw + brace-balanced in markdown() output,
        // so its interior braces (resolved to bare `{` in the atom, or kept as `\{`
        // in a code span) must not count against `rd_complete`. Both the resolved
        // and escaped forms stay complete; only the enclosing `\name{…}` pair counts.
        assert!(section_atoms_rd_complete(
            &["(\\verb (VERB \"d { e\"))".into()], // literal `\verb{d \{ e}`, resolved
            true,
        ));
        assert!(section_atoms_rd_complete(
            &["(\\verb (VERB \"x \\\\{ y\"))".into()], // code span, kept as `\{`
            true,
        ));
        assert!(section_atoms_rd_complete(
            &["(\\code (RCODE \"a { b\"))".into()],
            true,
        ));
        // The fragile-macro neutralization is confined to fragile heads: a
        // cmark-derived `\emph{\}` still counts its escaping backslash and drops.
        assert!(!section_atoms_rd_complete(
            &["(\\emph (TEXT \"\\\\\"))".into()],
            true,
        ));
    }

    #[test]
    fn projects_rdcomplete_failure_drops_the_section() {
        // roxygen2 runs `rdComplete` on the rendered Rd of an `@description`/
        // `@details` section (`markdown_if_active`, `sections = TRUE`); when the
        // braces are unbalanced it warns and drops the body to empty. An escaped
        // emphasis delimiter `*\**` renders `\emph{\}*`, which is incomplete, so the
        // section projects empty --- matching the `cm-439`/`442`/`451`/`454` pins.
        for delim in ["*\\**", "**\\***", "_\\__", "__\\___"] {
            let src =
                format!("#' @md\n#' @title T\n#' @details\n#' foo {delim}\n#' @name spec\nNULL\n");
            let out = project_to_rd(&src);
            assert!(
                out.contains("(\\details)") && !out.contains("(\\details "),
                "delim {delim:?} got: {out}"
            );
        }
    }

    #[test]
    fn rdcomplete_drop_is_scoped_to_with_sections_tags() {
        // `@return` (`tag_markdown`, `sections = FALSE`) is *not* dropped on an
        // imbalance --- only `@description`/`@details` carry the per-section check.
        let src = "#' @md\n#' @title T\n#' @return foo *\\**\n#' @name spec\nNULL\n";
        let out = project_to_rd(src);
        assert!(out.contains("(\\value"), "got: {out}");
    }

    #[test]
    fn markdown_off_escaped_brace_does_not_drop_the_section() {
        // Markdown-off, roxygen2 runs `rdComplete` on the *raw* tag value, where a
        // `\{`/`\}` is still escaped and therefore not counted --- so an unbalanced
        // *escaped* brace keeps the section (`\{` renders a bare `{`). The projector
        // must scan the pre-resolution raw text, not the escape-resolved atoms
        // (where `\{` has collapsed to a bare `{` that would false-drop the section).
        let src = "#' @title A title with an escaped a \\{ brace\n\
                   #' @description Desc with an escaped a \\} brace\n\
                   #' @name x\nNULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains("(\\title (TEXT \"A title with an escaped a { brace\"))"),
            "title got: {out}"
        );
        assert!(
            out.contains("(\\description (TEXT \"Desc with an escaped a } brace\"))"),
            "description got: {out}"
        );
        // A genuinely unbalanced *bare* brace still drops (no escape to protect it).
        let bare = "#' @title T\n#' @description bare a { brace\n#' @name x\nNULL\n";
        let out = project_to_rd(bare);
        assert!(
            out.contains("(\\description)") && !out.contains("(\\description "),
            "bare got: {out}"
        );
    }

    #[test]
    fn md_bare_brace_groups_project_as_lists() {
        // Under `@md` a balanced bare `{…}` is an Rd `LIST` too. The brace parity
        // is shared with non-md (odd run escapes, even run opens), but the
        // `%`-comment trigger is inverted (`group_brace_lists` mirrors
        // `md_percent_swallow`): a bare/even-preceded `%` stays literal and does
        // *not* hide a following group, while an odd-preceded `\%` renders bare and
        // swallows to the physical line end.
        let case = |body: &str| {
            let src = format!("#' @md\n#' @title T\n#' @details {body}\n#' @name x\nNULL\n");
            let out = project_to_rd(&src);
            out.lines()
                .find(|l| l.starts_with("(\\details"))
                .unwrap_or("")
                .to_string()
        };
        // Simple / nested / empty groups.
        assert_eq!(
            case("a {b c} d"),
            "(\\details (TEXT \"a\") (LIST (TEXT \"b c\")) (TEXT \"d\"))"
        );
        assert_eq!(
            case("a {b {c} d} e"),
            "(\\details (TEXT \"a\") (LIST (TEXT \"b\") (LIST (TEXT \"c\")) (TEXT \"d\")) (TEXT \"e\"))"
        );
        assert_eq!(
            case("a {} b"),
            "(\\details (TEXT \"a\") (LIST) (TEXT \"b\"))"
        );
        // An even backslash run opens the group (one literal backslash kept).
        assert_eq!(
            case(r"s \\{t} u"),
            "(\\details (TEXT \"s \\\\\") (LIST (TEXT \"t\")) (TEXT \"u\"))"
        );
        // An odd run escapes the braces: literal, no group.
        assert_eq!(case(r"p \{q\} r"), "(\\details (TEXT \"p {q} r\"))");
        // A bare/even `%` is literal (roxygen2 escapes it) and does not hide braces.
        assert_eq!(
            case("v % {w} x"),
            "(\\details (TEXT \"v %\") (LIST (TEXT \"w\")) (TEXT \"x\"))"
        );
        assert_eq!(
            case(r"v \\% {w} x"),
            "(\\details (TEXT \"v \\\\%\") (LIST (TEXT \"w\")) (TEXT \"x\"))"
        );
    }

    #[test]
    fn macro_arg_bare_groups_project_as_lists() {
        // A bare `{…}` inside a *prose* macro argument is an Rd `LIST` too
        // (parse_Rd lexes the argument with the same bare-group rule). Verbatim
        // arguments (`\code`) never group; structural macros (`\href`) GRP-wrap a
        // multi-atom display with the group counted as one atom.
        let case = |md: bool, body: &str| {
            let md_line = if md { "#' @md\n" } else { "" };
            let src = format!("{md_line}#' @title T\n#' @details {body}\n#' @name x\nNULL\n");
            project_to_rd(&src)
                .lines()
                .find(|l| l.starts_with("(\\details"))
                .unwrap_or("")
                .to_string()
        };
        for md in [false, true] {
            // A latexlike single-arg macro folds a bare group.
            assert_eq!(
                case(md, r"\emph{a {b} c}"),
                "(\\details (\\emph (TEXT \"a\") (LIST (TEXT \"b\")) (TEXT \"c\")))"
            );
            // Groups nest and span a nested macro.
            assert_eq!(
                case(md, r"\emph{i {j \strong{k} l} m}"),
                "(\\details (\\emph (TEXT \"i\") (LIST (TEXT \"j\") (\\strong (TEXT \"k\")) (TEXT \"l\")) (TEXT \"m\")))"
            );
            // An empty group is a bare `(LIST)`.
            assert_eq!(
                case(md, r"\emph{n {} o}"),
                "(\\details (\\emph (TEXT \"n\") (LIST) (TEXT \"o\")))"
            );
            // A structural display GRP-wraps; the group counts as one atom.
            assert_eq!(
                case(md, r"\href{http://x.org}{s {t} u}"),
                "(\\details (\\href (VERB \"http://x.org\") (GRP (TEXT \"s\") (LIST (TEXT \"t\")) (TEXT \"u\"))))"
            );
            // A verbatim `\code` argument is R code: braces stay literal, no group.
            assert_eq!(
                case(md, r"\code{v {w} x}"),
                "(\\details (\\code (RCODE \"v {w} x\")))"
            );
        }
        // Non-md only: escaped braces stay literal (an odd backslash run escapes).
        assert_eq!(
            case(false, r"\emph{p \{q\} r}"),
            "(\\details (\\emph (TEXT \"p {q} r\")))"
        );
    }

    #[test]
    fn even_run_braced_macro_projects_as_literal_plus_list() {
        // An *even* backslash run before a brace-required macro name defeats the
        // macro carve: parse_Rd pairs the backslashes to a literal `\`, leaves the
        // name as plain text (`\\emph` -> `\emph`), and the following `{…}` is an
        // ordinary bare-brace `LIST` -- never the macro's argument. This is the
        // even-run twin of `md_bare_brace_groups_project_as_lists`; it falls out of
        // the backslash-parity gate (no macro carve) plus `group_brace_lists`, and
        // holds identically in both modes.
        let case = |md: bool, body: &str| {
            let md_line = if md { "#' @md\n" } else { "" };
            let src = format!("{md_line}#' @title T\n#' @details {body}\n#' @name x\nNULL\n");
            project_to_rd(&src)
                .lines()
                .find(|l| l.starts_with("(\\details"))
                .unwrap_or("")
                .to_string()
        };
        for md in [false, true] {
            // Even run (k=2): one literal backslash kept, `emph` literal, `{x}` a LIST.
            assert_eq!(
                case(md, r"a \\emph{x} b"),
                "(\\details (TEXT \"a \\\\emph\") (LIST (TEXT \"x\")) (TEXT \"b\"))"
            );
            // A longer even run (k=4) halves to two literal backslashes.
            assert_eq!(
                case(md, r"c \\\\emph{y} d"),
                "(\\details (TEXT \"c \\\\\\\\emph\") (LIST (TEXT \"y\")) (TEXT \"d\"))"
            );
            // Even runs also spare a brace-required section macro from its
            // brace-less drop (`\link` normally drops when brace-less).
            assert_eq!(
                case(md, r"e \\link{z} f"),
                "(\\details (TEXT \"e \\\\link\") (LIST (TEXT \"z\")) (TEXT \"f\"))"
            );
            // The trailing group still nests.
            assert_eq!(
                case(md, r"g \\emph{h {i} j} k"),
                "(\\details (TEXT \"g \\\\emph\") (LIST (TEXT \"h\") (LIST (TEXT \"i\")) (TEXT \"j\")) (TEXT \"k\"))"
            );
        }
    }

    #[test]
    fn heading_title_bare_groups_project_as_lists() {
        // A bare `{…}` in a markdown heading title is an Rd `LIST`, exactly as in
        // prose and macro args; the multi-atom title GRP-wraps. ATX and setext
        // share the path (both flow through `render_heading_frame`); groups nest
        // and span emphasis.
        let section = |body: &str| {
            let src = format!(
                "#' @md\n#' @title T\n#' @details Intro.\n#'\n#' {body}\n#' body prose.\n#' @name x\nNULL\n"
            );
            project_to_rd(&src)
                .lines()
                .find(|l| l.starts_with("(\\section"))
                .unwrap_or("")
                .to_string()
        };
        // A bare group in an ATX title.
        assert_eq!(
            section("# H {a b}"),
            "(\\section (GRP (TEXT \"H\") (LIST (TEXT \"a b\"))) (TEXT \"body prose.\"))"
        );
        // Groups nest.
        assert_eq!(
            section("# H {a {b} c}"),
            "(\\section (GRP (TEXT \"H\") (LIST (TEXT \"a\") (LIST (TEXT \"b\")) (TEXT \"c\"))) (TEXT \"body prose.\"))"
        );
        // A group spans emphasis.
        assert_eq!(
            section("# H {k *x* l}"),
            "(\\section (GRP (TEXT \"H\") (LIST (TEXT \"k\") (\\emph (TEXT \"x\")) (TEXT \"l\"))) (TEXT \"body prose.\"))"
        );
    }

    #[test]
    fn resolve_md_brace_runs_pairs_a_backslash_run_before_a_brace() {
        // parse_Rd pairs the cmark-stage run into `floor(k/2)` backslashes; an odd
        // trailing `\` escapes the brace bare. Matches roxygen2 for odd `k`.
        assert_eq!(resolve_md_brace_runs(r"a \{ b \} c"), "a { b } c"); // k=1
        assert_eq!(resolve_md_brace_runs(r"a \{ b"), "a { b");
        assert_eq!(resolve_md_brace_runs(r"a \\\{ b \\\} c"), r"a \{ b \} c"); // k=3
        assert_eq!(resolve_md_brace_runs(r"a \\\\\{ c"), r"a \\{ c"); // k=5
        // An even run halves and leaves the brace bare (the `(LIST …)` backlog).
        assert_eq!(resolve_md_brace_runs(r"a \\{ b"), r"a \{ b"); // k=2
        // A bare brace and non-brace escapes are left alone.
        assert_eq!(resolve_md_brace_runs("a { b } c"), "a { b } c");
        assert_eq!(resolve_md_brace_runs(r"a \* \b c"), r"a \* \b c");
    }

    #[test]
    fn resolve_md_text_braces_only_touches_text_leaves() {
        // A `TEXT` leaf resolves its escaped braces; a `VERB` leaf (a verbatim code
        // span / URL) keeps them, and structure is copied verbatim.
        let sexpr = "(\\details (TEXT \"a \\\\{ b\") (\\verb (VERB \"c \\\\{ d\")))";
        assert_eq!(
            resolve_md_text_braces(sexpr),
            "(\\details (TEXT \"a { b\") (\\verb (VERB \"c \\\\{ d\")))"
        );
        // A literal `(TEXT "` inside a non-`TEXT` string is data, not a leaf opener.
        let tricky = "(\\verb (VERB \"see (TEXT \\\"x \\\\{ y\\\")\"))";
        assert_eq!(resolve_md_text_braces(tricky), tricky);
    }

    #[test]
    fn markdown_on_escaped_brace_renders_bare_but_does_not_drop() {
        // Under `@md` the source escape survives the double->cmark round trip, so an
        // unbalanced *escaped* brace is kept (not dropped) and the rendered TEXT is
        // bare; a `\code`-fragile arg and a verbatim code span are unaffected here.
        let src = "#' @title T\n#' @md\n#' @details a \\{ b\n#' @name x\nNULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains("(\\details (TEXT \"a { b\"))"),
            "details got: {out}"
        );
        // A genuinely unbalanced *bare* brace still drops under `@md` too.
        let bare = "#' @title T\n#' @md\n#' @details a { b\n#' @name x\nNULL\n";
        let out = project_to_rd(bare);
        assert!(
            out.contains("(\\details)") && !out.contains("(\\details "),
            "bare got: {out}"
        );
    }

    #[test]
    fn resolve_rd_arg_escapes_resolves_the_rd_metacharacter_escapes() {
        // The four braced-argument escapes render bare; backslashes pair
        // left-to-right; any other lone backslash stays literal.
        assert_eq!(resolve_rd_arg_escapes(r"a \{ b \} c"), "a { b } c");
        assert_eq!(resolve_rd_arg_escapes(r"a \% b"), "a % b");
        assert_eq!(resolve_rd_arg_escapes(r"a \\ b"), r"a \ b");
        assert_eq!(resolve_rd_arg_escapes(r"a \* \b c"), r"a \* \b c");
        // A paired `\\` before a metacharacter is a literal backslash, not an escape.
        assert_eq!(resolve_rd_arg_escapes(r"a \\{ b"), r"a \{ b");
    }

    #[test]
    fn literal_macro_args_resolve_rd_escapes_both_modes() {
        // A literal `\code`/`\emph`/`\url` argument resolves parse_Rd's Rd-string
        // escapes (`\{`/`\}`/`\%`/`\\`), verbatim `RCODE`/`VERB` and prose `TEXT`
        // alike, with markdown off.
        let src = "#' @title T\n\
                   #' @details c \\code{a \\{ b \\% d} e \\emph{f \\} g} u \\url{h/\\%20}\n\
                   #' @name x\nNULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains("(\\code (RCODE \"a { b % d\"))")
                && out.contains("(\\emph (TEXT \"f } g\"))")
                && out.contains("(\\url (VERB \"h/%20\"))"),
            "non-md got: {out}"
        );
        // A fragile macro's argument resolves the same braces under `@md` (the
        // TEXT-only post-pass never reaches its verbatim `RCODE`/`VERB`).
        let md = "#' @title T\n#' @md\n\
                  #' @details span \\code{a \\{ b \\} c} verb \\verb{d \\{ e \\} f}\n\
                  #' @name x\nNULL\n";
        let out = project_to_rd(md);
        assert!(
            out.contains("(\\code (RCODE \"a { b } c\"))")
                && out.contains("(\\verb (VERB \"d { e } f\"))"),
            "md got: {out}"
        );
    }

    #[test]
    fn markdown_code_span_keeps_its_backslash_brace() {
        // A markdown code span projects through a *different* path than a literal
        // `\verb`, so it must NOT resolve `\{` (roxygen2 renders the span verbatim).
        let src = "#' @title T\n#' @md\n#' @details a `x \\{ y` z\n#' @name w\nNULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains("(\\verb (VERB \"x \\\\{ y\"))"),
            "code span got: {out}"
        );
    }

    #[test]
    fn url_defined_reference_links_render_href() {
        // A user link-reference definition `[ref]: url` defines a destination, so
        // roxygen2 renders the referencing link as `\href{url}{display}` with the
        // display *kept* (the "must contain plain text" drop is `\link`-only). The
        // definition lines themselves are consumed (cmark removes them).
        let src = "#' @md\n#' @title T\n#' @details\n\
                   #' See [*foo*][r1] and [plain][r2] and [`code`][r3].\n\
                   #'\n\
                   #' [r1]: https://example.com\n\
                   #' [r2]: https://example.org\n\
                   #' [r3]: https://example.net\n\
                   #' @name spec\nNULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"T\"))\n\
             (\\details (TEXT \"See\") \
             (\\href (VERB \"https://example.com\") (\\emph (TEXT \"foo\"))) (TEXT \"and\") \
             (\\href (VERB \"https://example.org\") (TEXT \"plain\")) (TEXT \"and\") \
             (\\href (VERB \"https://example.net\") (\\code (RCODE \"code\"))) (TEXT \".\"))\n\
             (\\title (TEXT \"T\"))"
        );
    }

    #[test]
    fn url_defined_shortcut_link_renders_href() {
        // A bare shortcut `[r1]` whose label has a user URL definition → `\href`;
        // the definition line is consumed.
        let src = "#' @md\n#' @title T\n#' @details\n\
                   #' See [r1] here.\n#'\n#' [r1]: https://example.com\n\
                   #' @name spec\nNULL\n";
        assert!(
            project_to_rd(src).contains(
                "(\\details (TEXT \"See\") (\\href (VERB \"https://example.com\") (TEXT \"r1\")) (TEXT \"here.\"))"
            ),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn linkref_definition_cannot_interrupt_a_paragraph() {
        // A `[r1]: url` line *without* a preceding blank line is part of the
        // paragraph, not a definition (CommonMark): the label stays an R-topic
        // `\link` and the line renders literally. (Regression guard: the user-def
        // transform must only fire at a real block start.)
        let src = "#' @md\n#' @title T\n#' @details\n\
                   #' Some prose with [r1] here.\n#' [r1]: https://example.com\n\
                   #' @name spec\nNULL\n";
        assert!(
            project_to_rd(src).contains(
                "(\\details (TEXT \"Some prose with\") (\\link (TEXT \"r1\")) (TEXT \"here.\") (\\link (TEXT \"r1\")) (TEXT \": https://example.com\"))"
            ),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn linkref_definition_with_trailing_macro_is_not_a_definition() {
        // A `[foo]: url \emph{bar}` line has trailing inline content after the
        // destination (the `\emph{bar}` macro), which CommonMark forbids in a link
        // reference definition, so it is *not* a definition: the label stays an
        // R-topic `\link` (synthesized `R:foo`) and the line renders literally, with
        // the macro surfacing as its own subtree. (Regression guard: the user-def
        // scan only sees the trailing `Text` run, so it must also reject a trailing
        // non-`Text` inline.)
        let src = "#' @md\n#' @title T\n#' @details\n\
                   #' See [foo].\n#'\n#' [foo]: https://x.org \\emph{bar}\n\
                   #' @name spec\nNULL\n";
        assert!(
            project_to_rd(src).contains(
                "(\\details (TEXT \"See\") (\\link (TEXT \"foo\")) (TEXT \".\") (\\link (TEXT \"foo\")) (TEXT \": https://x.org\") (\\emph (TEXT \"bar\")))"
            ),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn backslash_word_in_link_display_renders_as_rd_macro() {
        // A markdown link display carrying a backslash word (`\b`, an Rd macro to
        // parse_Rd) keeps the link — at the markdown level the backslash is literal,
        // so roxygen2 does not drop it — and the macro surfaces as a nested subtree
        // inside the `\link` rather than collapsing into the topic text. The
        // reference form `[a\b][lbl]` drops its topic and renders identically.
        let src = "#' @md\n#' @title T\n#' @details See [a\\b] and [a\\b][lbl] now.\n\
                   #' @name spec\nNULL\n";
        assert!(
            project_to_rd(src).contains(
                "(\\details (TEXT \"See\") (\\link (TEXT \"a\") (UNKNOWN \"\\\\b\")) (TEXT \"and\") (\\link (TEXT \"a\") (UNKNOWN \"\\\\b\")) (TEXT \"now.\"))"
            ),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn escaped_emphasis_in_link_display_drops_the_link() {
        // `[a\*b\*]` resolves an emphasis node in its display (a non-text child), so
        // roxygen2's `parse_link` drops the whole link ("must contain plain text")
        // and the surrounding prose coalesces — unlike a backslash *word*, which is
        // markdown-level plain text and is kept.
        let src = "#' @md\n#' @title T\n#' @details A [a\\*b\\*] gap.\n\
                   #' @name spec\nNULL\n";
        assert!(
            project_to_rd(src).contains("(\\details (TEXT \"A gap.\"))"),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn pure_macro_active_link_display_drops() {
        // A shortcut whose display is a *pure* macro (no surrounding text) carrying
        // cmark-active markdown (`[\emph{*x*}]`) drops to empty like any non-plain
        // display — the link must reach the drop site, not be spuriously demoted to a
        // literal `[]` by an empty link-reference label. Regression guard for the
        // pure-macro label fix (`link_label_text` includes the macro source).
        let src = "#' @md\n#' @title T\n#' @details A [\\emph{*x*}] gap.\n\
                   #' @name spec\nNULL\n";
        assert!(
            project_to_rd(src).contains("(\\details (TEXT \"A gap.\"))"),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn pure_macro_inert_link_display_keeps() {
        // A pure-macro display with an *inert* argument (`[\emph{y}]`) or a *fragile*
        // macro (`[\code{f}]`) keeps the link, rendering `\link` over the macro
        // subtree — not a literal `[]`. The self-consistent macro-source label lets
        // the link survive the undefined-label demotion and reach the keep path.
        let src = "#' @md\n#' @title T\n#' @details Keep [\\emph{y}] and [\\code{f}].\n\
                   #' @name spec\nNULL\n";
        assert!(
            project_to_rd(src).contains(
                "(\\details (TEXT \"Keep\") (\\link (\\emph (TEXT \"y\"))) (TEXT \"and\") (\\link (\\code (RCODE \"f\"))) (TEXT \".\"))"
            ),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn blank_separated_content_column_prose_folds_into_item() {
        // A blank line closes a list item's paragraph, but a following prose line
        // indented to the item's content column opens a new paragraph *inside the
        // same item* (a loose item), which Rd rendering flattens into the item
        // text. Both blank-separated paragraphs fold — `- a` / blank / `  more` /
        // blank / `  even more` → one item, `a more even more`.
        let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#'\n#'   more\n#'\n#'   even more\n\
                   #' @name spec\nNULL\n";
        assert!(
            project_to_rd(src)
                .contains("(\\details (\\itemize (\\item) (TEXT \"a more even more\")))"),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn blank_then_underindented_prose_ends_the_list() {
        // A blank-separated continuation below the content column does *not* fold:
        // it ends the list and becomes sibling section prose (`- a` / blank /
        // `more` at column 1 → item `a`, then a separate `more`).
        let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#'\n#' more\n#' @name spec\nNULL\n";
        assert!(
            project_to_rd(src)
                .contains("(\\details (\\itemize (\\item) (TEXT \"a\")) (TEXT \"more\"))"),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn fence_at_content_column_folds_into_item() {
        // A fenced code block indented to the item's content column folds into the
        // item as a child block (the three-atom `\if…\preformatted…\if` sequence
        // inside the `\itemize`), with a below content-column marker after it a
        // sibling item — a fenced block interrupts the item's paragraph.
        let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#'   ```\n#'   code\n#'   ```\n\
                   #' - b\n#' @name spec\nNULL\n";
        assert!(
            project_to_rd(src).contains(
                "(\\details (\\itemize (\\item) (TEXT \"a\") \
                 (\\if (TEXT \"html\") (\\out (VERB \"<div class=\\\"sourceCode\\\">\"))) \
                 (\\preformatted (VERB \"code\\n\")) \
                 (\\if (TEXT \"html\") (\\out (VERB \"</div>\"))) (\\item) (TEXT \"b\")))"
            ),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn fence_below_content_column_ends_the_list() {
        // A fenced code block *below* the item's content column is a section-level
        // block, not part of the item: the list ends at the `\item` and the code
        // block is a sibling of the `\itemize`.
        let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#' ```\n#' code\n#' ```\n\
                   #' @name spec\nNULL\n";
        assert!(
            project_to_rd(src).contains(
                "(\\details (\\itemize (\\item) (TEXT \"a\")) \
                 (\\if (TEXT \"html\") (\\out (VERB \"<div class=\\\"sourceCode\\\">\")))"
            ),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn folded_fence_strips_only_its_own_indentation() {
        // CommonMark removes up to the opener fence's indentation from each content
        // line, so a body line indented *past* the fence keeps the surplus: fence at
        // column 3, body at column 5 → `  code` (two leading spaces survive).
        let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#'    ```\n#'      code\n#'    ```\n\
                   #' @name spec\nNULL\n";
        assert!(
            project_to_rd(src).contains("(\\preformatted (VERB \"  code\\n\"))"),
            "got: {}",
            project_to_rd(src)
        );
    }
}
