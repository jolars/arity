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

use crate::ast::{AstNode, RoxygenBlock, RoxygenLine, RoxygenTag};
use crate::parser::parse;
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
}

/// One topic's worth of sections from a single roxygen block.
fn project_block(block: &RoxygenBlock, out: &mut Vec<String>) {
    // Intro paragraphs (prose before the first tag) and the tag sections.
    let mut intro_paras: Vec<Vec<Inline>> = Vec::new();
    let mut cur_para: Vec<Inline> = Vec::new();
    let mut tag_sections: Vec<(String, Vec<Inline>)> = Vec::new();
    let mut in_intro = true;

    let flush_para = |cur: &mut Vec<Inline>, paras: &mut Vec<Vec<Inline>>| {
        if !inlines_blank(cur) {
            paras.push(std::mem::take(cur));
        } else {
            cur.clear();
        }
    };

    for line in block.lines() {
        if let Some(tag) = line.tag() {
            in_intro = false;
            flush_para(&mut cur_para, &mut intro_paras);
            let name = tag.name().map(|n| n.to_string()).unwrap_or_default();
            tag_sections.push((name, tag_inlines(&tag)));
        } else if line.is_blank() {
            if in_intro {
                flush_para(&mut cur_para, &mut intro_paras);
            } else if let Some((_, body)) = tag_sections.last_mut() {
                body.push(Inline::Text(" ".to_string())); // paragraph break
            }
        } else {
            // A prose continuation line: a space joins it to the prior line.
            let inl = line_inlines(&line);
            if in_intro {
                cur_para.push(Inline::Text(" ".to_string()));
                cur_para.extend(inl);
            } else if let Some((_, body)) = tag_sections.last_mut() {
                body.push(Inline::Text(" ".to_string()));
                body.extend(inl);
            }
        }
    }
    if in_intro {
        flush_para(&mut cur_para, &mut intro_paras);
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
fn serialize_macro(node: &SyntaxNode) -> String {
    let mut head = String::new();
    let mut atoms: Vec<String> = Vec::new();
    let mut run = String::new();
    let flush = |run: &mut String, atoms: &mut Vec<String>| {
        if let Some(atom) = text_atom(run) {
            atoms.push(atom);
        }
        run.clear();
    };
    for el in node.children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_RD_MACRO_NAME => {
                head = el
                    .as_token()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();
            }
            SyntaxKind::ROXYGEN_RD_MACRO_VERB => {
                flush(&mut run, &mut atoms);
                let raw = el
                    .as_token()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();
                atoms.push(format!("(VERB {})", encode_text(&raw)));
            }
            SyntaxKind::ROXYGEN_RD_MACRO => {
                flush(&mut run, &mut atoms);
                if let Some(n) = el.as_node() {
                    atoms.push(serialize_macro(n));
                }
            }
            // Delimiters and the dropped option carry no projected content; any
            // other leaf (text) is prose.
            SyntaxKind::ROXYGEN_RD_MACRO_DELIM | SyntaxKind::ROXYGEN_RD_MACRO_OPT => {}
            _ => {
                if let Some(t) = el.as_token() {
                    run.push_str(t.text());
                }
            }
        }
    }
    flush(&mut run, &mut atoms);
    if atoms.is_empty() {
        format!("({head})")
    } else {
        format!("({head} {})", atoms.join(" "))
    }
}

/// Whether an inline run holds no projectable content (only whitespace text).
fn inlines_blank(body: &[Inline]) -> bool {
    body.iter().all(|inl| match inl {
        Inline::Text(s) => s.trim().is_empty(),
        Inline::Macro(_) => false,
    })
}

/// The raw source text of an inline run (text verbatim, macros as their CST
/// text), used for the textual `@section` heading split.
fn inlines_raw_text(body: &[Inline]) -> String {
    let mut s = String::new();
    for inl in body {
        match inl {
            Inline::Text(t) => s.push_str(t),
            Inline::Macro(n) => s.push_str(&n.text().to_string()),
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

/// The inline elements of a prose line: everything after the `#'` marker and the
/// single marker→content whitespace. An Rd macro becomes an `Inline::Macro`; all
/// other content (plain text and — in the absence of resolved markdown — inline
/// code and link spans, which are literal Rd prose) becomes `Inline::Text`.
fn line_inlines(line: &RoxygenLine) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut seen = false;
    for el in line.syntax().children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_MARKER => continue,
            SyntaxKind::WHITESPACE if !seen => continue,
            _ => seen = true,
        }
        push_inline(&mut out, el);
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
        NodeOrToken::Token(t) => out.push(Inline::Text(t.text().to_string())),
    }
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
    fn multiline_describe_projects_flat_so_it_diverges() {
        // The CST does not yet model a multi-line \describe as a block, so the
        // projector emits flat text --- which will NOT match roxygen2's nested
        // pin. This is the backlog signal, asserted so it can't silently change.
        let src = "#' T\n\
                   #' @format A frame:\n\
                   #' \\describe{\n\
                   #'   \\item{a}{first}\n\
                   #' }\n\
                   #' @name d\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(out.contains("\\format"));
        assert!(
            !out.contains("(\\describe"),
            "skeleton must not fake block structure: {out}"
        );
    }
}
