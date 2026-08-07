//! R's **system Rd user macros** and parse_Rd's expansion of them.
//!
//! R loads `share/Rd/macros/system.Rd` before every Rd file it parses, so
//! `tools::parse_Rd` sees `\doi`, `\CRANpkg`, … already defined. Encountering
//! one it emits **two** siblings into the enclosing node:
//!
//! 1. a `USERMACRO` leaf whose text is the macro's **raw definition body**
//!    followed by each argument's raw text, concatenated
//!    (`\doi{10.1/2}` → `USERMACRO "\Sexpr[results=rd]{tools:::Rd_expr_doi(\"#1\")}10.1/2"`);
//! 2. the **expansion** — the definition body with each `#N` replaced by
//!    argument `N`, re-parsed as Rd and spliced in as ordinary siblings.
//!
//! Because the expansion is spliced, a definition that expands to plain text
//! (`\I` is literally `#1`) coalesces with the prose around it — which is why
//! [`super::Inline::UserMacro`] carries the leaf and the expansion re-enters the
//! inline run rather than arriving as finished atoms.
//!
//! Expansion is *textual and pre-markdown*: roxygen2's cmark pass has already
//! run by the time parse_Rd expands, so the argument is never markdown (a `*b*`
//! inside `\CRANpkg{…}` stays literal in both modes) and the expansion is parsed
//! as plain Rd.

use super::*;

/// One system Rd macro: its name (no leading `\`) and its raw definition body,
/// verbatim from `share/Rd/macros/system.Rd`.
struct SystemRdMacro {
    name: &'static str,
    definition: &'static str,
}

/// The single-argument system Rd macros, transcribed from R 4.6.1's
/// `share/Rd/macros/system.Rd` (the definitions are stable across R releases;
/// they are data, not behavior, so a drift shows up as a pin mismatch).
///
/// **Deliberately omitted**, because arity cannot yet render their expansions
/// faithfully — each is recorded backlog, not a silent gap:
///
/// * `\sspace`, `\LaTeX`, `\proglang` expand through `\ifelse{fmt}{yes}{no}`, a
///   **three**-argument Rd macro. Arity's macro arity is the two-valued
///   [`is_two_arg_rd_macro`], so a third `{…}` group projects as a sibling
///   `(LIST …)` instead of an argument.
/// * `\manual` (two arguments) and `\bibinfo` (three) need the same
///   generalization on the *invocation* side.
///
/// A macro written **brace-less** (`\doi b`) is likewise out of scope: parse_Rd
/// treats it as sticky and swallows the rest of the section verbatim, which is
/// the brace-less machinery's concern, not this table's.
const SYSTEM_RD_MACROS: &[SystemRdMacro] = &[
    SystemRdMacro {
        name: "CRANpkg",
        definition: r"\href{https://CRAN.R-project.org/package=#1}{\pkg{#1}}",
    },
    SystemRdMacro {
        name: "PR",
        definition: r"\Sexpr[results=rd]{tools:::Rd_expr_PR(#1)}",
    },
    SystemRdMacro {
        name: "doi",
        definition: r##"\Sexpr[results=rd]{tools:::Rd_expr_doi("#1")}"##,
    },
    SystemRdMacro {
        name: "I",
        definition: "#1",
    },
    SystemRdMacro {
        name: "packageTitle",
        definition: r##"\Sexpr[results=rd,stage=build]{tools:::Rd_package_title("#1")}"##,
    },
    SystemRdMacro {
        name: "packageDescription",
        definition: r##"\Sexpr[results=rd,stage=build]{tools:::Rd_package_description("#1")}"##,
    },
    SystemRdMacro {
        name: "packageAuthor",
        definition: r##"\Sexpr[results=rd,stage=build]{tools:::Rd_package_author("#1")}"##,
    },
    SystemRdMacro {
        name: "packageMaintainer",
        definition: r##"\Sexpr[results=rd,stage=build]{tools:::Rd_package_maintainer("#1")}"##,
    },
    SystemRdMacro {
        name: "packageDESCRIPTION",
        definition: r##"\Sexpr[results=rd,stage=build]{tools:::Rd_package_DESCRIPTION("#1")}"##,
    },
    SystemRdMacro {
        name: "packageIndices",
        definition: r##"\Sexpr[results=rd,stage=build]{tools:::Rd_package_indices("#1")}"##,
    },
    SystemRdMacro {
        name: "bibcitep",
        definition: r##"\Sexpr[results=rd,stage=build]{tools:::Rd_expr_bibcite(r"(#1)", FALSE)}"##,
    },
    SystemRdMacro {
        name: "bibcitet",
        definition: r##"\Sexpr[results=rd,stage=build]{tools:::Rd_expr_bibcite(r"(#1)", TRUE)}"##,
    },
    SystemRdMacro {
        name: "bibshow",
        definition: r##"\Sexpr[results=rd,stage=build]{tools:::Rd_expr_bibshow("#1")}"##,
    },
];

/// The definition body of the system Rd macro `name` (without the leading `\`),
/// or `None` when `name` is not one arity models.
fn system_rd_macro(name: &str) -> Option<&'static str> {
    SYSTEM_RD_MACROS
        .iter()
        .find(|m| m.name == name)
        .map(|m| m.definition)
}

/// Expand a `ROXYGEN_RD_MACRO` node that names a system Rd macro into the
/// `USERMACRO` leaf text and the inline run its expansion contributes (see the
/// module doc). Returns `None` for any other macro, and for a system macro
/// written without its `{…}` argument (parse_Rd's brace-less handling is the
/// brace-less machinery's job, not an expansion).
pub(super) fn expand_user_macro(node: &SyntaxNode) -> Option<(String, Vec<Inline>)> {
    let name = macro_head(node);
    let definition = system_rd_macro(name.trim_start_matches('\\'))?;
    let argument = macro_single_arg_content(node)?;

    // The `USERMACRO` leaf is the definition followed by the raw argument text;
    // the expansion substitutes it for `#1`.
    let leaf = format!("{definition}{argument}");
    let expanded = definition.replace("#1", &argument);
    // parse_Rd expands *after* roxygen2's markdown pass, so the substituted body
    // is plain Rd — never re-run through the markdown lexer.
    let para = resolve_rd_inline(&expanded);
    Some((leaf, para_to_inlines(&para)))
}

/// Rewrite an inline run, replacing every system-Rd-macro node with its
/// `Inline::UserMacro` leaf followed by its spliced expansion. Returns `None`
/// when the run holds no user macro, so the overwhelmingly common path keeps its
/// borrowed slice and its byte-identical serialization.
pub(super) fn expand_user_macros(body: &[Inline]) -> Option<Vec<Inline>> {
    if !body
        .iter()
        .any(|inl| matches!(inl, Inline::Macro(n) if expand_user_macro(n).is_some()))
    {
        return None;
    }
    let mut out = Vec::with_capacity(body.len());
    for inl in body {
        match inl {
            Inline::Macro(n) => match expand_user_macro(n) {
                Some((leaf, expansion)) => {
                    out.push(Inline::UserMacro(leaf));
                    out.extend(expansion);
                }
                None => out.push(inl.clone()),
            },
            _ => out.push(inl.clone()),
        }
    }
    Some(out)
}

/// The atoms a system Rd macro contributes where an inline run is *not* in play
/// — a macro nested directly in another macro's argument, whose pieces are
/// already-serialized atoms. The expansion's atoms splice in as siblings; text
/// coalescing across the boundary is the inline-run path's concern.
pub(super) fn user_macro_atoms(node: &SyntaxNode, md: bool) -> Option<Vec<String>> {
    let (leaf, expansion) = expand_user_macro(node)?;
    let mut atoms = vec![format!("(USERMACRO {})", encode_text(&leaf))];
    atoms.extend(serialize_inlines(&expansion, md));
    Some(atoms)
}
