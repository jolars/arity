//! Argument shapes from R 4.6.1's Rd grammar and bundled system.Rd.
//!
//! These are Rd grammar facts, not roxygen's block/inline layout classes.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Prose,
    RLike,
    Verbatim,
    Equation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Items {
    None,
    Bare,
    Pairs,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Spec {
    pub modes: &'static [Mode],
    pub required: usize,
    pub option: bool,
    pub items: Items,
    pub section: bool,
    pub top_level: bool,
}

pub(super) fn lookup(name: &str, items: Items) -> Option<Spec> {
    use Mode::{Equation as E, Prose as P, RLike as R, Verbatim as V};
    let mut spec = Spec {
        modes: &[P],
        required: 1,
        option: false,
        items: Items::None,
        section: false,
        top_level: false,
    };
    match name {
        "author" | "concept" | "description" | "details" | "docType" | "encoding" | "format"
        | "keyword" | "note" | "references" | "seealso" | "source" | "title" => {
            spec.section = true;
        }
        "name" | "alias" | "synopsis" | "Rdversion" | "RdOpts" => {
            spec.modes = &[V];
            spec.section = true;
        }
        "usage" | "examples" => {
            spec.modes = &[R];
            spec.section = true;
        }
        "arguments" | "value" => {
            spec.items = Items::Pairs;
            spec.section = true;
        }
        "section" => {
            spec.modes = &[P, P];
            spec.required = 2;
            spec.section = true;
        }
        "abbr" | "acronym" | "bold" | "cite" | "command" | "dfn" | "dQuote" | "email" | "emph"
        | "file" | "pkg" | "sQuote" | "strong" | "var" => {}
        "describe" => spec.items = Items::Pairs,
        "itemize" | "enumerate" => spec.items = Items::Bare,
        "item" => match items {
            Items::Bare => {
                spec.modes = &[];
                spec.required = 0;
            }
            Items::Pairs => {
                spec.modes = &[P, P];
                spec.required = 2;
            }
            Items::None => return None,
        },
        "enc" | "if" | "method" | "S3method" | "S4method" | "tabular" | "subsection" => {
            spec.modes = &[P, P];
            spec.required = 2;
        }
        "ifelse" => {
            spec.modes = &[P, P, P];
            spec.required = 3;
        }
        "href" => {
            spec.modes = &[V, P];
            spec.required = 2;
        }
        "link" | "linkS4class" | "linkS4methods" => spec.option = true,
        "code" | "dontshow" | "donttest" | "dontdiff" | "testonly" | "special" => spec.modes = &[R],
        "Sexpr" => {
            spec.modes = &[R];
            spec.option = true;
            spec.top_level = true;
        }
        "dontrun" | "env" | "kbd" | "option" | "out" | "preformatted" | "samp" | "url" | "verb" => {
            spec.modes = &[V]
        }
        "eqn" | "deqn" | "figure" => spec.modes = &[E, V],
        "newcommand" | "renewcommand" => {
            spec.modes = &[E, V];
            spec.required = 2;
            spec.top_level = true;
        }
        "cr" | "dots" | "ldots" | "R" | "tab" => {
            spec.modes = &[];
            spec.required = 0;
        }
        "sspace" | "LaTeX" => {
            spec.modes = &[];
            spec.required = 0;
            spec.top_level = true;
        }
        "CRANpkg" | "PR" | "doi" | "I" | "packageTitle" | "packageDescription"
        | "packageAuthor" | "packageMaintainer" | "packageDESCRIPTION" | "packageIndices"
        | "bibcitep" | "bibcitet" | "bibshow" | "proglang" => {
            spec.modes = &[V];
            spec.top_level = true;
        }
        "manual" => {
            spec.modes = &[V, V];
            spec.required = 2;
            spec.top_level = true;
        }
        "bibinfo" => {
            spec.modes = &[V, V, V];
            spec.required = 3;
            spec.top_level = true;
        }
        _ => return None,
    }
    spec.top_level |= spec.section;
    Some(spec)
}
