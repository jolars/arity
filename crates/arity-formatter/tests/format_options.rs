//! `format_with_options` — formatting under a caller-supplied roxygen
//! markdown default (`ParseOptions`), for packages that enable markdown
//! package-wide (`Roxygen: list(markdown = TRUE)`) instead of per-block `@md`.

use arity_formatter::FormatStyle;
use arity_formatter::formatter::{format_with_options, format_with_style};
use arity_formatter::parser::ParseOptions;

fn md_on() -> ParseOptions {
    ParseOptions::default().with_roxygen_markdown_default(true)
}

/// A directive-less block holding a markdown indented code block: Rd-first
/// formatting reflows the indent away (the line is prose there), while a
/// markdown-on default preserves it as an atomic code block — the same output
/// an explicit `@md` produces.
#[test]
fn markdown_default_preserves_indented_code() {
    let input = "#' Title\n#'\n#' @details\n#' Some prose before the code.\n#'\n#'     code_looking <- \"indented\"\n#' @name x\nNULL\n";

    let rd_first = format_with_style(input, FormatStyle::default()).expect("format");
    assert!(
        rd_first.contains("#' code_looking"),
        "Rd-first mode reflows the indent away:\n{rd_first}"
    );

    let md = format_with_options(input, FormatStyle::default(), &md_on()).expect("format");
    assert!(
        md.contains("#'     code_looking"),
        "markdown mode preserves the code block indent:\n{md}"
    );

    // And it matches what an explicit `@md` directive produces.
    let with_directive = format!("#' Title\n#'\n#' @md\n{}", &input["#' Title\n#'\n".len()..]);
    let explicit = format_with_style(&with_directive, FormatStyle::default()).expect("format");
    assert_eq!(explicit.replace("#' @md\n", ""), md);
}

/// Default options reproduce `format_with_style` exactly.
#[test]
fn default_options_match_format_with_style() {
    let input = "#' Title\n#'\n#' @details\n#' prose *stays* prose\n#' @name x\nNULL\nf <- function(x) {\n  x + 1\n}\n";
    assert_eq!(
        format_with_options(input, FormatStyle::default(), &ParseOptions::default())
            .expect("format"),
        format_with_style(input, FormatStyle::default()).expect("format"),
    );
}

/// Idempotence holds under a markdown-on default.
#[test]
fn markdown_default_formatting_is_idempotent() {
    let input = "#' Title\n#'\n#' @details\n#' A list:\n#'\n#' - one\n#' - two\n#'\n#'     indented <- \"code\"\n#' @param x an argument\nNULL\n";
    let options = md_on();
    let once = format_with_options(input, FormatStyle::default(), &options).expect("format");
    let twice = format_with_options(&once, FormatStyle::default(), &options).expect("format");
    assert_eq!(once, twice);
}
