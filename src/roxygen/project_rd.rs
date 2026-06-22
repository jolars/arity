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
//! ## Current reach (the skeleton)
//!
//! This is intentionally minimal: a section body is projected as a single
//! coalesced `TEXT` atom of its CST content. So a **plain-prose** section
//! matches its pin, while a section the CST does not yet model structurally ---
//! a multi-line `\describe`/`\itemize`/`\tabular`, or any inline Rd macro /
//! markdown that roxygen2 translates into nested nodes (`*x*` → `\emph{x}`) ---
//! projects as flat text and therefore **diverges**. Those divergences are the
//! backlog: each is closed by teaching the *parser* the structure, then the
//! projector grows a faithful arm for the new nodes. Never patch the projector
//! to make a case pass.

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

/// One topic's worth of sections from a single roxygen block.
fn project_block(block: &RoxygenBlock, out: &mut Vec<String>) {
    // Intro paragraphs (prose before the first tag) and the tag sections.
    let mut intro_paras: Vec<String> = Vec::new();
    let mut cur_para = String::new();
    let mut tag_sections: Vec<(String, String)> = Vec::new();
    let mut in_intro = true;

    let flush_para = |cur: &mut String, paras: &mut Vec<String>| {
        if !cur.trim().is_empty() {
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
            tag_sections.push((name, tag_body(&tag)));
        } else if line.is_blank() {
            if in_intro {
                flush_para(&mut cur_para, &mut intro_paras);
            } else if let Some((_, body)) = tag_sections.last_mut() {
                body.push(' '); // paragraph break inside a section body
            }
        } else {
            // A prose continuation line.
            let content = line_content(&line);
            if in_intro {
                cur_para.push(' ');
                cur_para.push_str(&content);
            } else if let Some((_, body)) = tag_sections.last_mut() {
                body.push(' ');
                body.push_str(&content);
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
        let desc = if intro_paras.len() >= 2 {
            intro_paras[1..].join(" ")
        } else {
            intro_paras[0].clone()
        };
        push_section(out, "description", &desc);
    }

    for (name, body) in &tag_sections {
        project_tag_section(name, body, out);
    }
}

/// Map a tag to its Rd section macro and push the projected subtree. Tags that
/// roxygen2 does not turn into a parser-owned section (`@param`/`@field` feed the
/// excluded `\arguments`; `@export`/`@md`/`@name`/… are directives) are skipped.
fn project_tag_section(name: &str, body: &str, out: &mut Vec<String>) {
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
        // `@section Title: body` → \section{Title}{body}.
        "section" => {
            let (heading, rest) = body.split_once(':').unwrap_or((body, ""));
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

/// Push `(\<macro> <TEXT atom>)` for a prose section, or `(\<macro>)` when the
/// body is empty.
fn push_section(out: &mut Vec<String>, macro_name: &str, body: &str) {
    match text_atom(body) {
        Some(atom) => out.push(format!("(\\{macro_name} {atom})")),
        None => out.push(format!("(\\{macro_name})")),
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

/// The content of a prose line: every element after the `#'` marker and the
/// single marker→content whitespace.
fn line_content(line: &RoxygenLine) -> String {
    let mut s = String::new();
    let mut seen = false;
    for el in line.syntax().children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_MARKER => continue,
            SyntaxKind::WHITESPACE if !seen => continue,
            _ => seen = true,
        }
        append_element_text(&mut s, &el);
    }
    s
}

/// The prose body of a tag line: everything after the `@`, the tag name, and an
/// arg-bearing tag's argument (and the leading whitespace before the prose).
fn tag_body(tag: &RoxygenTag) -> String {
    let mut s = String::new();
    let mut seen_prose = false;
    for el in tag.syntax().children_with_tokens() {
        let NodeOrToken::Token(t) = &el else {
            continue;
        };
        match t.kind() {
            SyntaxKind::ROXYGEN_AT | SyntaxKind::ROXYGEN_TAG_NAME | SyntaxKind::ROXYGEN_TAG_ARG => {
                continue;
            }
            SyntaxKind::WHITESPACE => {
                if seen_prose {
                    s.push_str(t.text());
                }
            }
            k if is_prose_kind(k) => {
                seen_prose = true;
                s.push_str(t.text());
            }
            _ => {}
        }
    }
    s
}

fn append_element_text(s: &mut String, el: &NodeOrToken<SyntaxNode, crate::syntax::SyntaxToken>) {
    match el {
        NodeOrToken::Token(t) => s.push_str(t.text()),
        NodeOrToken::Node(n) => s.push_str(&n.text().to_string()),
    }
}

/// Whether `kind` is a roxygen prose leaf (plain text or a protected span).
fn is_prose_kind(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::ROXYGEN_TEXT
            | SyntaxKind::ROXYGEN_CODE
            | SyntaxKind::ROXYGEN_RD_MACRO
            | SyntaxKind::ROXYGEN_MD_LINK
    )
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
