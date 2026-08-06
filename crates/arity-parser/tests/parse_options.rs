//! `ParseOptions` — the caller-supplied roxygen markdown default.
//!
//! Since roxygen2 8.0.0 packages normally enable markdown package-wide via
//! `Config/roxygen2/markdown` in `DESCRIPTION`, so real blocks rarely carry a
//! per-block `@md`. `parse_with_options` lets an embedder honor that global
//! default; a block's own `@md`/`@noMd` directive still wins, and `parse`
//! itself keeps the Rd-first default.

use arity_parser::parser::{
    Edit, ParseOptions, parse, parse_with_options, reparse_edits_with_options, reparse_with_options,
};
use arity_parser::syntax::{SyntaxKind, SyntaxNode};

fn md_on() -> ParseOptions {
    ParseOptions::default().with_roxygen_markdown_default(true)
}

fn kinds(root: &SyntaxNode) -> Vec<SyntaxKind> {
    root.descendants_with_tokens().map(|el| el.kind()).collect()
}

fn reconstruct_from(root: &SyntaxNode) -> String {
    root.descendants_with_tokens()
        .filter_map(|el| el.into_token())
        .map(|tok| tok.text().to_string())
        .collect()
}

/// Default options reproduce `parse` exactly — tree and diagnostics.
#[test]
fn default_options_match_parse() {
    let text = "#' Title\n#'\n#' @md\n#' *emph* and `code`\n#' @name x\nNULL\n";
    let plain = parse(text);
    let with_options = parse_with_options(text, &ParseOptions::default());
    assert_eq!(
        format!("{:#?}", plain.cst),
        format!("{:#?}", with_options.cst)
    );
    assert_eq!(plain.diagnostics, with_options.diagnostics);
}

/// A directive-less block resolves markdown from the caller's default.
#[test]
fn markdown_default_enables_directiveless_block() {
    let text = "#' Title\n#'\n#' @details\n#' *emph* and `code`\n#' @name x\nNULL\n";

    let rd_first = parse(text);
    let rd_kinds = kinds(&rd_first.cst);
    assert!(!rd_kinds.contains(&SyntaxKind::ROXYGEN_MD_EMPH));
    assert!(!rd_kinds.contains(&SyntaxKind::ROXYGEN_MD_CODE));

    let md = parse_with_options(text, &md_on());
    let md_kinds = kinds(&md.cst);
    assert!(md_kinds.contains(&SyntaxKind::ROXYGEN_MD_EMPH));
    assert!(md_kinds.contains(&SyntaxKind::ROXYGEN_MD_CODE));
}

/// A block's own `@noMd` overrides a markdown-on default (last directive wins).
#[test]
fn no_md_directive_overrides_markdown_default() {
    let text = "#' Title\n#'\n#' @noMd\n#' @details\n#' *emph* stays literal\n#' @name x\nNULL\n";
    let md = parse_with_options(text, &md_on());
    assert!(!kinds(&md.cst).contains(&SyntaxKind::ROXYGEN_MD_EMPH));
}

/// The markdown default changes block *structure*, not just leaf kinds: at
/// column five an `@param` is indented-code text, not a tag.
#[test]
fn markdown_default_reclassifies_indented_tag_as_code() {
    let text = "#' Title\n#'\n#' @details\n#' Some prose before the code.\n#'\n#'     @param x not a tag\n#' @name x\nNULL\n";

    let count_tags = |root: &SyntaxNode| {
        root.descendants()
            .filter(|n| n.kind() == SyntaxKind::ROXYGEN_TAG)
            .count()
    };

    let rd_first = parse(text);
    assert!(!kinds(&rd_first.cst).contains(&SyntaxKind::ROXYGEN_MD_INDENTED_CODE));
    assert_eq!(count_tags(&rd_first.cst), 3); // @details, @param, @name

    let md = parse_with_options(text, &md_on());
    assert!(kinds(&md.cst).contains(&SyntaxKind::ROXYGEN_MD_INDENTED_CODE));
    assert_eq!(count_tags(&md.cst), 2); // @param swallowed by the code block
}

/// The default also reaches a roxygen block nested inside braces (the
/// expression-parser path, not just the top-level loop).
#[test]
fn markdown_default_reaches_nested_block() {
    let text = "f <- function() {\n  #' *emph* in a body\n  1\n}\n";
    assert!(!kinds(&parse(text).cst).contains(&SyntaxKind::ROXYGEN_MD_EMPH));
    assert!(kinds(&parse_with_options(text, &md_on()).cst).contains(&SyntaxKind::ROXYGEN_MD_EMPH));
}

/// Losslessness holds under a markdown-on default.
#[test]
fn losslessness_under_markdown_default() {
    let text = "#' Title\n#'\n#' @details\n#' *emph*, `code`, [a link](https://example.com)\n#'\n#'     indented <- \"code\"\n#' @name x\nNULL\nf <- function() {\n  #' nested *block*\n  1\n}\n";
    let md = parse_with_options(text, &md_on());
    assert_eq!(reconstruct_from(&md.cst), text);
}

/// An incremental reparse under options is byte- and tree-identical to a full
/// `parse_with_options` of the edited text (Tenets 2 and 4).
#[test]
fn reparse_with_options_matches_full_parse() {
    let options = md_on();
    let old_text = "f <- function() {\n  #' *emph* text\n  1\n}\n";
    let old = parse_with_options(old_text, &options);

    // Replace `1` with `12` — lands inside the `{ … }` block.
    let edit = Edit {
        range: old_text.find("  1\n").unwrap() + 2..old_text.find("  1\n").unwrap() + 3,
        insert: "12".to_string(),
    };
    let new_text = edit.apply(old_text);

    let reparsed = reparse_with_options(&old.cst, old_text, &old.diagnostics, &edit, &options)
        .expect("edit inside a block should reparse incrementally");
    let full = parse_with_options(&new_text, &options);

    let reparsed_root = SyntaxNode::new_root(reparsed.green);
    assert_eq!(format!("{reparsed_root:#?}"), format!("{:#?}", full.cst));
    assert_eq!(reparsed.diagnostics, full.diagnostics);
    // The nested roxygen block kept its markdown interpretation through the splice.
    assert!(kinds(&reparsed_root).contains(&SyntaxKind::ROXYGEN_MD_EMPH));
}

/// Same equivalence through the multi-edit entry point.
#[test]
fn reparse_edits_with_options_matches_full_parse() {
    let options = md_on();
    let old_text = "f <- function() {\n  #' *emph* text\n  1\n}\n";
    let old = parse_with_options(old_text, &options);

    let pos = old_text.find("  1\n").unwrap() + 2;
    let edits = vec![
        Edit {
            range: pos..pos + 1,
            insert: "2".to_string(),
        },
        Edit {
            range: pos..pos + 1,
            insert: "23".to_string(),
        },
    ];
    let mut new_text = old_text.to_string();
    for edit in &edits {
        new_text = edit.apply(&new_text);
    }

    let reparsed = reparse_edits_with_options(
        &old.cst,
        old_text,
        &old.diagnostics,
        &edits,
        &new_text,
        &options,
    )
    .expect("edits inside a block should reparse incrementally");
    let full = parse_with_options(&new_text, &options);

    let reparsed_root = SyntaxNode::new_root(reparsed.green);
    assert_eq!(format!("{reparsed_root:#?}"), format!("{:#?}", full.cst));
    assert_eq!(reparsed.diagnostics, full.diagnostics);
}
