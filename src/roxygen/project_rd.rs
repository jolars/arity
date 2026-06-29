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
use crate::parser::roxygen::{is_known_rd_macro, is_two_arg_rd_macro};
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
    /// A raw inline-HTML tag resolved under `@md` mode, carrying the verbatim tag
    /// text (`<img …>`, `</span>`). Projects to roxygen2's
    /// `\if{html}{\out{<tag>}}` (`mdxml_html_inline`; see [`html_inline_atom`]).
    MdHtml(String),
    /// A markdown HTML block resolved under `@md` mode (a `ROXYGEN_MD_HTML_BLOCK`
    /// node). Projects to roxygen2's `\if{html}{\out{…}}` with one verbatim line
    /// per `VERB` (`mdxml_html_block`; see [`serialize_md_html_block`]).
    MdHtmlBlock(SyntaxNode),
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
                if !body.is_empty() {
                    // A line break in the source (norm_ws collapses it to a space,
                    // but it bounds an Rd `%` comment in non-markdown prose).
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
        None => intro_title.clone().or(explicit_title_body),
    };
    if let Some(description) = description {
        push_section(out, "description", &description, md, true);
    }

    // The intro-derived details (and any folded-in @details).
    if merge_details {
        let mut body = join_paras(intro_details);
        for (_, ed) in tag_sections.iter().filter(|(n, _)| n == "details") {
            body.push(Inline::Text("\n".to_string()));
            body.extend(join_paras(std::slice::from_ref(ed)));
        }
        push_section(out, "details", &body, md, true);
    }

    for (name, body) in &tag_sections {
        // A folded-in @details was emitted above; skip the standalone section.
        if merge_details && name == "details" {
            continue;
        }
        project_tag_section(name, body, out, md);
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
        "description" => push_section(out, "description", body, md, true),
        "details" => push_section(out, "details", body, md, true),
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
            // (so `add_linkrefs_to_md` poisoning spans both halves), then splits the
            // rendered Rd on the first `:`. Demote first on the whole body, split the
            // demoted run, and append the leaked definitions to the content — they
            // render at the very end of the field, i.e. after the `:`.
            let demoted = md.then(|| demote_poisoned_links(body)).flatten();
            let body = demoted.as_deref().unwrap_or(body);
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
    if check_drop && !section_atoms_rd_complete(&atoms, md) {
        out.push(format!("(\\{macro_name})"));
        return;
    }
    if atoms.is_empty() {
        out.push(format!("(\\{macro_name})"));
    } else {
        out.push(format!("(\\{macro_name} {})", atoms.join(" ")));
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
    render_sexpr(bytes, &mut i, md, out);
}

fn render_sexpr(bytes: &[u8], i: &mut usize, md: bool, out: &mut String) {
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
            append_leaf_text(&text, escape_percent, out);
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
                    render_sexpr(bytes, i, md, out);
                } else {
                    out.push('{');
                    render_sexpr(bytes, i, md, out);
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
fn append_leaf_text(text: &str, escape_percent: bool, out: &mut String) {
    if escape_percent {
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
    let demoted = md.then(|| demote_poisoned_links(body)).flatten();
    let body = demoted.as_deref().unwrap_or(body);
    let mut atoms = serialize_inlines(body, md);
    if md {
        for leaked in leaked_linkref_text(&inline_source_skeleton(body)) {
            append_rendered_text(&mut atoms, &leaked);
        }
    }
    atoms
}

/// Serialize an inline run into the canonical atom sequence: maximal prose runs
/// coalesce into one whitespace-normalized `(TEXT …)`, and each macro becomes a
/// nested subtree — mirroring the R driver's `serialize_children`. `md` is the
/// block's resolved markdown mode: with markdown off a prose run is literal Rd, so
/// `prose_text_atom` strips its `%` line comments.
fn serialize_inlines(body: &[Inline], md: bool) -> Vec<String> {
    let mut atoms: Vec<String> = Vec::new();
    let mut run = String::new();
    for inl in body {
        match inl {
            Inline::Text(s) => run.push_str(s),
            Inline::Macro(node) => {
                if let Some(atom) = prose_text_atom(&run, md) {
                    atoms.push(atom);
                }
                run.clear();
                atoms.push(serialize_macro(node));
            }
            Inline::MdCode(content) => {
                if let Some(atom) = prose_text_atom(&run, md) {
                    atoms.push(atom);
                }
                run.clear();
                atoms.push(md_code_atom(content));
            }
            Inline::MdEmphasis { strong, children } => {
                if let Some(atom) = prose_text_atom(&run, md) {
                    atoms.push(atom);
                }
                run.clear();
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
                if let Some(atom) = prose_text_atom(&run, md) {
                    atoms.push(atom);
                }
                run.clear();
                atoms.push(serialize_md_list(node));
            }
            Inline::MdLink(raw) => {
                if let Some(atom) = prose_text_atom(&run, md) {
                    atoms.push(atom);
                }
                run.clear();
                atoms.push(resolve_md_link(raw).unwrap_or_default());
            }
            Inline::MdInlineLink { url, display } => {
                if let Some(atom) = prose_text_atom(&run, md) {
                    atoms.push(atom);
                }
                run.clear();
                atoms.push(inline_link_node_atom(url, display, md));
            }
            Inline::MdRefLink { dest, display } => {
                if let Some(atom) = prose_text_atom(&run, md) {
                    atoms.push(atom);
                }
                run.clear();
                atoms.push(ref_link_node_atom(display, dest));
            }
            Inline::MdShortcutLink { display } => {
                if let Some(atom) = prose_text_atom(&run, md) {
                    atoms.push(atom);
                }
                run.clear();
                atoms.push(shortcut_link_node_atom(display));
            }
            Inline::MdImage(raw) => {
                if let Some(atom) = prose_text_atom(&run, md) {
                    atoms.push(atom);
                }
                run.clear();
                if let Some(atom) = resolve_md_image(raw) {
                    atoms.push(atom);
                }
            }
            Inline::MdCodeBlock(node) => {
                if let Some(atom) = prose_text_atom(&run, md) {
                    atoms.push(atom);
                }
                run.clear();
                atoms.extend(serialize_md_code_block(node));
            }
            Inline::MdHtml(raw) => {
                if let Some(atom) = prose_text_atom(&run, md) {
                    atoms.push(atom);
                }
                run.clear();
                atoms.push(html_inline_atom(raw));
            }
            Inline::MdHtmlBlock(node) => {
                if let Some(atom) = prose_text_atom(&run, md) {
                    atoms.push(atom);
                }
                run.clear();
                atoms.push(serialize_md_html_block(node));
            }
        }
    }
    if let Some(atom) = prose_text_atom(&run, md) {
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
fn serialize_macro(node: &SyntaxNode) -> String {
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
    let mut head = String::new();
    let mut structural = false;
    let mut out_atoms: Vec<String> = Vec::new();
    let mut group: Vec<String> = Vec::new();
    let mut run = String::new();
    // Flush the pending text run into the current argument group. A `\code` macro
    // tags its textual content as verbatim `(RCODE …)` (parse_Rd treats `\code`
    // bodies as R code, preserving whitespace and splitting at newlines); every
    // other macro coalesces prose into one whitespace-normalized `(TEXT …)`.
    let flush = |run: &mut String, group: &mut Vec<String>, code: bool| {
        if code {
            group.extend(rcode_atoms(run));
        } else if let Some(atom) = text_atom(run) {
            group.push(atom);
        }
        run.clear();
    };
    // Finalize a `{…}` argument group at its closing `}`: a structural macro's
    // multi-atom argument becomes a `(GRP …)` (parse_Rd models it as a list);
    // everything else (a single-atom argument, or a latexlike macro's inlined
    // content) splices its atoms in directly.
    let finalize = |group: &mut Vec<String>, out: &mut Vec<String>, structural: bool| {
        if structural && group.len() > 1 {
            out.push(format!("(GRP {})", group.join(" ")));
            group.clear();
        } else {
            out.append(group);
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
                flush(&mut run, &mut group, head == "\\code");
                let raw = el
                    .as_token()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();
                group.push(format!("(VERB {})", encode_text(&raw)));
            }
            SyntaxKind::ROXYGEN_RD_MACRO => {
                flush(&mut run, &mut group, head == "\\code");
                if let Some(n) = el.as_node() {
                    group.push(serialize_macro(n));
                }
            }
            // A closing `}` ends an argument group: flush the run, then finalize
            // the group (GRP-wrapping a structural macro's multi-atom argument).
            // The opening `{` carries no content.
            SyntaxKind::ROXYGEN_RD_MACRO_DELIM => {
                if el.as_token().is_some_and(|t| t.text() == "}") {
                    flush(&mut run, &mut group, head == "\\code");
                    finalize(&mut group, &mut out_atoms, structural);
                }
            }
            // The dropped option and the `#'` markers threaded into a multi-line
            // block macro carry no projected content; any other leaf (text, and
            // the collapsed newline/whitespace trivia) is prose.
            SyntaxKind::ROXYGEN_RD_MACRO_OPT | SyntaxKind::ROXYGEN_MARKER => {}
            _ => {
                if let Some(t) = el.as_token() {
                    run.push_str(t.text());
                }
            }
        }
    }
    // Defensive: trailing content with no closing brace (a malformed macro).
    flush(&mut run, &mut group, head == "\\code");
    finalize(&mut group, &mut out_atoms, structural);
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
    verb_atoms(&body)
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

/// A prose run's `(TEXT …)` atom, accounting for markdown mode. With markdown off
/// the run is literal Rd, where an unescaped `%` begins a comment to end of line
/// (parse_Rd's rule), so the comment is stripped per physical line before the run
/// coalesces; with markdown on roxygen2 escapes `%` (`\%`), so it survives and the
/// run is taken verbatim. (The run carries source line breaks as `\n`, which
/// `norm_ws` later collapses, so the comment's end-of-line is honored either way.)
fn prose_text_atom(run: &str, md: bool) -> Option<String> {
    if md {
        text_atom(&unescape_md_brackets(run))
    } else {
        text_atom(&strip_rd_comments(run))
    }
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
    let skeleton = inline_source_skeleton(body);
    let boundary = first_invalid_linkref_offset(&skeleton)?;
    let mut out = Vec::with_capacity(body.len());
    let mut offset = 0;
    for inl in body {
        let start = offset;
        offset += skeleton_len(inl);
        if start > boundary
            && let Some(text) = demoted_link_source(inl)
        {
            out.push(Inline::Text(text));
        } else {
            out.push(inl.clone());
        }
    }
    Some(relink_demoted_inline_links(out))
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
/// inline (unescaping happens later in `prose_text_atom`), so it is skipped here and
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
        Inline::MdShortcutLink { display } => Some(format!("[{}]", inline_plain_text(display))),
        Inline::MdRefLink { dest, display } => {
            Some(format!("[{}][{}]", inline_plain_text(display), dest))
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
    s.split('\n')
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
            SyntaxKind::ROXYGEN_PARAGRAPH
            | SyntaxKind::ROXYGEN_RD_MACRO
            | SyntaxKind::ROXYGEN_MD_LIST
            | SyntaxKind::ROXYGEN_MD_CODE_BLOCK
            | SyntaxKind::ROXYGEN_MD_HTML_BLOCK => {
                let Some(node) = el.into_node() else { continue };
                let inlines = match node.kind() {
                    SyntaxKind::ROXYGEN_PARAGRAPH => RoxygenParagraph::cast(node)
                        .map(|p| paragraph_inlines(&p))
                        .unwrap_or_default(),
                    SyntaxKind::ROXYGEN_MD_LIST => vec![Inline::MdList(node)],
                    SyntaxKind::ROXYGEN_MD_CODE_BLOCK => vec![Inline::MdCodeBlock(node)],
                    SyntaxKind::ROXYGEN_MD_HTML_BLOCK => vec![Inline::MdHtmlBlock(node)],
                    _ => vec![Inline::Macro(node)],
                };
                if !cur.is_empty() {
                    cur.push(Inline::Text(" ".to_string()));
                }
                cur.extend(inlines);
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
            SyntaxKind::NEWLINE => out.push(Inline::Text(" ".to_string())), // line join
            _ => push_inline(&mut out, el),
        }
    }
    out
}

/// The inline elements of a tag line: everything after the `@`, the tag name, and
/// an arg-bearing tag's argument (and the leading whitespace before the prose).
fn tag_inlines(tag: &RoxygenTag) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut seen_prose = false;
    for el in tag.syntax().children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_AT | SyntaxKind::ROXYGEN_TAG_NAME | SyntaxKind::ROXYGEN_TAG_ARG => {
                continue;
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
                    SyntaxKind::NEWLINE => children.push(Inline::Text(" ".to_string())),
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
                    SyntaxKind::NEWLINE => display.push(Inline::Text(" ".to_string())),
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
    // A CommonMark autolink `<scheme:…>` whose destination equals its text →
    // `\url{…}` (roxygen2's `mdxml_link` `dest == xml_text(xml)` branch).
    if bytes.first() == Some(&b'<') {
        return Some(url_atom(raw.strip_prefix('<')?.strip_suffix('>')?));
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
    let (inner, is_code) = match display {
        [Inline::MdCode(content)] => (content.clone(), true),
        _ => (display_text, false),
    };
    code_wrap(
        format!("(\\link {})", text_atom(&inner).unwrap_or_default()),
        is_code,
    )
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
        _ => shortcut_link_atom(&inline_plain_text(display)),
    }
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

/// Project a `ROXYGEN_MD_HTML_BLOCK` node into roxygen2's `\if{html}{\out{…}}`
/// (`mdxml_html_block`, `R/markdown.R`): the block's verbatim text — a leading
/// newline then each line with a trailing newline — goes into a single `\out`
/// inside an `\if{html}{…}`. parse_Rd splits the verbatim `\out` body at newlines
/// into one `VERB` per line (the leading `\n` becomes a bare `(VERB "\n")`). The
/// block lines are reconstructed from the node text with each `#'` marker and the
/// single following space stripped (like the fenced code block).
fn serialize_md_html_block(node: &SyntaxNode) -> String {
    let text = node.text().to_string();
    let mut body = String::from("\n");
    for line in text.split('\n') {
        body.push_str(strip_marker(line));
        body.push('\n');
    }
    format!(
        "(\\if (TEXT {}) (\\out {}))",
        encode_text("html"),
        verb_atoms(&body).join(" ")
    )
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

/// Project a raw inline-HTML leaf into roxygen2's `\if{html}{\out{<tag>}}`
/// (`mdxml_html_inline`, `markdown.R`): the tag text goes verbatim into a `\out`
/// inside an `\if{html}{…}`. roxygen2 `escape_verb`-escapes `}` (→ `\}`) but
/// `parse_Rd` decodes it, so the pin (and thus the projector) carries the raw
/// tag.
fn html_inline_atom(raw: &str) -> String {
    format!(
        "(\\if (TEXT {}) (\\out (VERB {})))",
        encode_text("html"),
        encode_text(raw)
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
    let info = lines
        .first()
        .map(|l| strip_marker(l).trim_start_matches('`').trim().to_string())
        .unwrap_or_default();
    let body = if lines.len() > 2 {
        &lines[1..lines.len() - 1]
    } else {
        &[]
    };
    let mut code = String::new();
    for line in body {
        code.push_str(strip_marker(line));
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
            SyntaxKind::NEWLINE => out.push(Inline::Text(" ".to_string())),
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
}
