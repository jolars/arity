//! `roxygen-examples`: `@examples` code that does not parse.
//!
//! `R CMD check` runs example code, so a syntax error that slipped into an
//! `@examples` section (or an `@examplesIf` condition) fails the package at
//! check time. The rule extracts the embedded code token-by-token (exact
//! byte-span bookkeeping, robust to `@md` markdown fragmentation), reparses
//! it with arity's own parser, and maps the first parse diagnostic of each
//! snippet back to its source range. Rd example macros (`\dontrun{}`,
//! `\donttest{}`, …) are not R: their macro name is blanked out—same
//! length, so spans stay valid—leaving a plain `{ ... }` block for the
//! parser.
//!
//! Only the first diagnostic per snippet is reported: R parse errors cascade,
//! and the first one is the actionable one.

use rowan::ast::AstNode as _;
use rowan::{TextRange, TextSize};

use crate::ast::RoxygenBlock;
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::roxygen::{ExtractedCode, extract_examples};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::parser::parse;
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct RoxygenExamples;

const EXAMPLES: &[Example] = &[Example {
    caption: "An unclosed call in the example:",
    source: "#' Add one\n#' @examples\n#' add_one(1\n#' @export\nadd_one <- function(x) x + 1\n",
}];

/// Rd macros that wrap example code but are not R themselves. The macro name
/// is replaced with spaces (same byte length, offsets survive), leaving its
/// braced argument as a parseable `{ ... }` block.
const RD_EXAMPLE_MACROS: &[&str] = &["\\dontrun", "\\donttest", "\\dontshow", "\\testonly"];

fn neutralize_rd_example_macros(code: &mut String) {
    for name in RD_EXAMPLE_MACROS {
        let mut from = 0;
        while let Some(pos) = code[from..].find(name) {
            let start = from + pos;
            let end = start + name.len();
            if code[end..].starts_with('{') {
                code.replace_range(start..end, &" ".repeat(name.len()));
            }
            from = end;
        }
    }
}

fn check_snippet(snippet: &ExtractedCode, what: &str, sink: &mut Vec<Diagnostic>) {
    let mut code = snippet.code.clone();
    neutralize_rd_example_macros(&mut code);
    let Some(diag) = parse(&code).diagnostics.into_iter().next() else {
        return;
    };
    let extracted = TextRange::new(
        TextSize::from(diag.start as u32),
        TextSize::from(diag.end as u32),
    );
    sink.push(Diagnostic {
        rule: "roxygen-examples",
        severity: Default::default(),
        path: Default::default(),
        range: snippet.map_range(extracted),
        message: ViolationData::new(
            "roxygen-examples",
            format!("{what} does not parse: {}", diag.message),
        )
        .with_suggestion("`R CMD check` runs example code; fix the syntax error."),
        fix: None,
    });
}

impl Rule for RoxygenExamples {
    fn id(&self) -> &'static str {
        "roxygen-examples"
    }

    fn description(&self) -> &'static str {
        "Flag `@examples` code that does not parse.\
         \n\n`R CMD check` runs example code, so a syntax error in an \
         `@examples` section (or an `@examplesIf` condition) fails the package \
         at check time. The embedded code is reparsed with arity's own parser \
         and the first syntax error of each snippet is reported at its exact \
         location in the comment. Rd wrappers like `\\dontrun{}` are \
         understood and their contents still checked."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::ROXYGEN_BLOCK]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(block) = el.as_node().cloned().and_then(RoxygenBlock::cast) else {
            return;
        };
        for section in block.sections() {
            let Some(code) = extract_examples(&section) else {
                continue;
            };
            if let Some(condition) = &code.condition {
                check_snippet(condition, "`@examplesIf` condition", sink);
            }
            if let Some(body) = &code.body {
                check_snippet(body, "example code", sink);
            }
        }
    }
}
