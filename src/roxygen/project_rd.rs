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

use rowan::NodeOrToken;

use crate::ast::{AstNode, RoxygenBlock, RoxygenParagraph, RoxygenSection, RoxygenTag};
use crate::parser::parse;
use crate::parser::roxygen::is_two_arg_rd_macro;
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
enum Inline {
    Text(String),
    Macro(SyntaxNode),
    /// A markdown inline leaf resolved under `@md` mode — emphasis, strong, or a
    /// code span — carrying its delimiter-stripped inner content. Emphasis/strong
    /// project to `\emph`/`\strong` over `(TEXT …)`; a code span projects to
    /// `\code` or `\verb` per roxygen2's R-parseability rule (see [`md_code_atom`]).
    Md(MdInline, String),
}

/// The kind of a resolved markdown inline leaf.
#[derive(Clone, Copy)]
enum MdInline {
    Emph,
    Strong,
    Code,
}

/// One topic's worth of sections from a single roxygen block.
///
/// The block already owns logical structure: its children are `ROXYGEN_SECTION`s
/// (the intro, then one per `@tag`), each holding a `ROXYGEN_TAG` heading and/or
/// `ROXYGEN_PARAGRAPH`s. So the projector is a direct walk — the line-reassembly
/// state machine the line-flat CST forced is gone. A tag section's body is the
/// tag's own inline prose followed by its paragraphs (continuation and
/// paragraph-break both collapse to a single space under `norm_ws`).
fn project_block(block: &RoxygenBlock, out: &mut Vec<String>) {
    let mut intro_paras: Vec<Vec<Inline>> = Vec::new();
    let mut tag_sections: Vec<(String, Vec<Inline>)> = Vec::new();

    for section in block.sections() {
        if let Some(tag) = section.tag() {
            let name = tag.name().map(|n| n.to_string()).unwrap_or_default();
            let mut body = tag_inlines(&tag);
            for part in section_body_parts(&section) {
                if !body.is_empty() {
                    body.push(Inline::Text(" ".to_string()));
                }
                body.extend(part);
            }
            tag_sections.push((name, body));
        } else {
            intro_paras.extend(section_body_parts(&section));
        }
    }

    let has_explicit_title = tag_sections.iter().any(|(n, _)| n == "title");
    let has_explicit_desc = tag_sections.iter().any(|(n, _)| n == "description");

    // Intro-derived \title and \description (roxygen2's rule: first paragraph is
    // the title; the rest is the description, or the title duplicated when the
    // intro is a single paragraph). An explicit @title/@description tag wins.
    if !has_explicit_title && let Some(first) = intro_paras.first() {
        push_section(out, "title", first);
    }
    if !has_explicit_desc && !intro_paras.is_empty() {
        let desc: Vec<Inline> = if intro_paras.len() >= 2 {
            join_paras(&intro_paras[1..])
        } else {
            join_paras(&intro_paras[0..1])
        };
        push_section(out, "description", &desc);
    }

    for (name, body) in &tag_sections {
        project_tag_section(name, body, out);
    }
}

/// Flatten paragraphs into a single inline run, with a space between each (the
/// canonical serializer collapses the paragraph break to one space anyway).
fn join_paras(paras: &[Vec<Inline>]) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    for (i, p) in paras.iter().enumerate() {
        if i > 0 {
            out.push(Inline::Text(" ".to_string()));
        }
        for inl in p {
            out.push(match inl {
                Inline::Text(s) => Inline::Text(s.clone()),
                Inline::Macro(n) => Inline::Macro(n.clone()),
                Inline::Md(k, s) => Inline::Md(*k, s.clone()),
            });
        }
    }
    out
}

/// Map a tag to its Rd section macro and push the projected subtree. Tags that
/// roxygen2 does not turn into a parser-owned section (`@param`/`@field` feed the
/// excluded `\arguments`; `@export`/`@md`/`@name`/… are directives) are skipped.
fn project_tag_section(name: &str, body: &[Inline], out: &mut Vec<String>) {
    match name {
        // Direct prose → section-macro mappings.
        "description" => push_section(out, "description", body),
        "details" => push_section(out, "details", body),
        "return" => push_section(out, "value", body),
        "seealso" => push_section(out, "seealso", body),
        "source" => push_section(out, "source", body),
        "format" => push_section(out, "format", body),
        "references" => push_section(out, "references", body),
        "note" => push_section(out, "note", body),
        "author" => push_section(out, "author", body),
        "title" => push_section(out, "title", body),
        // `@section Title: body` → \section{Title}{body}. The split is textual;
        // macros in the heading are an out-of-scope edge (it would diverge, which
        // is the right backlog signal).
        "section" => {
            let raw = inlines_raw_text(body);
            let (heading, rest) = raw.split_once(':').unwrap_or((&raw, ""));
            let mut inner = String::new();
            if let Some(a) = text_atom(heading) {
                inner.push_str(&a);
            }
            if let Some(a) = text_atom(rest) {
                if !inner.is_empty() {
                    inner.push(' ');
                }
                inner.push_str(&a);
            }
            out.push(format!("(\\section{})", prefix_space(&inner)));
        }
        // The body is reformatted R; the oracle compares only its presence.
        "examples" | "examplesIf" => out.push("(\\examples ...)".to_string()),
        // Everything else is roclet scaffolding or an excluded section.
        _ => {}
    }
}

/// Push `(\<macro> <atoms…>)` for a prose section, or `(\<macro>)` when the body
/// has no content (after coalescing).
fn push_section(out: &mut Vec<String>, macro_name: &str, body: &[Inline]) {
    let atoms = serialize_inlines(body);
    if atoms.is_empty() {
        out.push(format!("(\\{macro_name})"));
    } else {
        out.push(format!("(\\{macro_name} {})", atoms.join(" ")));
    }
}

/// Serialize an inline run into the canonical atom sequence: maximal prose runs
/// coalesce into one whitespace-normalized `(TEXT …)`, and each macro becomes a
/// nested subtree — mirroring the R driver's `serialize_children`.
fn serialize_inlines(body: &[Inline]) -> Vec<String> {
    let mut atoms: Vec<String> = Vec::new();
    let mut run = String::new();
    for inl in body {
        match inl {
            Inline::Text(s) => run.push_str(s),
            Inline::Macro(node) => {
                if let Some(atom) = text_atom(&run) {
                    atoms.push(atom);
                }
                run.clear();
                atoms.push(serialize_macro(node));
            }
            Inline::Md(kind, content) => {
                if let Some(atom) = text_atom(&run) {
                    atoms.push(atom);
                }
                run.clear();
                atoms.push(serialize_md_inline(*kind, content));
            }
        }
    }
    if let Some(atom) = text_atom(&run) {
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
    let mut head = String::new();
    let mut structural = false;
    let mut out_atoms: Vec<String> = Vec::new();
    let mut group: Vec<String> = Vec::new();
    let mut run = String::new();
    // Flush the pending prose run into the current argument group as one (TEXT …).
    let flush = |run: &mut String, group: &mut Vec<String>| {
        if let Some(atom) = text_atom(run) {
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
                flush(&mut run, &mut group);
                let raw = el
                    .as_token()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();
                group.push(format!("(VERB {})", encode_text(&raw)));
            }
            SyntaxKind::ROXYGEN_RD_MACRO => {
                flush(&mut run, &mut group);
                if let Some(n) = el.as_node() {
                    group.push(serialize_macro(n));
                }
            }
            // A closing `}` ends an argument group: flush the run, then finalize
            // the group (GRP-wrapping a structural macro's multi-atom argument).
            // The opening `{` carries no content.
            SyntaxKind::ROXYGEN_RD_MACRO_DELIM => {
                if el.as_token().is_some_and(|t| t.text() == "}") {
                    flush(&mut run, &mut group);
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
    flush(&mut run, &mut group);
    finalize(&mut group, &mut out_atoms, structural);
    if out_atoms.is_empty() {
        format!("({head})")
    } else {
        format!("({head} {})", out_atoms.join(" "))
    }
}

/// The raw source text of an inline run (text verbatim, macros as their CST
/// text), used for the textual `@section` heading split.
fn inlines_raw_text(body: &[Inline]) -> String {
    let mut s = String::new();
    for inl in body {
        match inl {
            Inline::Text(t) => s.push_str(t),
            Inline::Macro(n) => s.push_str(&n.text().to_string()),
            // The `@section` heading split is textual; a markdown leaf's inner
            // content stands in for its source (delimiters are immaterial to the
            // `:` split, and markup in a heading is already an out-of-scope edge).
            Inline::Md(_, content) => s.push_str(content),
        }
    }
    s
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

/// Collapse every whitespace run to a single space and trim (the R `norm_ws`).
fn norm_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
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

/// The body parts of a section in document order, excluding its `@tag` heading:
/// each prose `ROXYGEN_PARAGRAPH` becomes its inline run, and each block
/// `ROXYGEN_RD_MACRO` child (a multi-line `\itemize`/`\enumerate`/…) becomes a
/// single-element `Inline::Macro` group. A block macro is a *direct* section
/// child (a sibling of the paragraphs), so it is picked up here rather than via
/// `paragraphs()`.
fn section_body_parts(section: &RoxygenSection) -> Vec<Vec<Inline>> {
    section
        .syntax()
        .children()
        .filter_map(|n| match n.kind() {
            SyntaxKind::ROXYGEN_PARAGRAPH => {
                RoxygenParagraph::cast(n).map(|p| paragraph_inlines(&p))
            }
            SyntaxKind::ROXYGEN_RD_MACRO => Some(vec![Inline::Macro(n)]),
            _ => None,
        })
        .collect()
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
        NodeOrToken::Node(n) => out.push(Inline::Text(n.text().to_string())),
        // Markdown inline leaves (emitted only under `@md`): carve off their
        // delimiters and carry the inner content; the kind chooses the Rd macro.
        NodeOrToken::Token(t) if t.kind() == SyntaxKind::ROXYGEN_MD_EMPH => {
            out.push(Inline::Md(MdInline::Emph, strip_delim(t.text(), 1)));
        }
        NodeOrToken::Token(t) if t.kind() == SyntaxKind::ROXYGEN_MD_STRONG => {
            out.push(Inline::Md(MdInline::Strong, strip_delim(t.text(), 2)));
        }
        NodeOrToken::Token(t) if t.kind() == SyntaxKind::ROXYGEN_MD_CODE => {
            out.push(Inline::Md(MdInline::Code, strip_code_span(t.text())));
        }
        NodeOrToken::Token(t) => out.push(Inline::Text(t.text().to_string())),
    }
}

/// Strip `n` leading and `n` trailing delimiter bytes from an emphasis/strong
/// span (`*x*` → `x`, `**x**` → `x`). The span always has at least `2*n`
/// delimiter bytes around non-empty content (the lexer guarantees it).
fn strip_delim(text: &str, n: usize) -> String {
    text.get(n..text.len() - n).unwrap_or("").to_string()
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

/// Project a resolved markdown inline leaf into its Rd atom: `\emph`/`\strong`
/// wrap whitespace-normalized `(TEXT …)`, while a code span becomes `\code` or
/// `\verb` per [`md_code_atom`].
fn serialize_md_inline(kind: MdInline, content: &str) -> String {
    match kind {
        MdInline::Emph => format!("(\\emph {})", text_atom(content).unwrap_or_default()),
        MdInline::Strong => format!("(\\strong {})", text_atom(content).unwrap_or_default()),
        MdInline::Code => md_code_atom(content),
    }
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
    out.cst
        .children_with_tokens()
        .filter(|el| {
            !matches!(
                el.kind(),
                SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
            )
        })
        .count()
        == 1
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
    fn examples_body_is_a_placeholder() {
        let src = "#' T\n#' @examples\n#' f(1)\n#' @name d\nNULL\n";
        assert!(project_to_rd(src).contains("(\\examples ...)"));
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
}
