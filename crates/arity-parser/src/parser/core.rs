use crate::parser::bracket_balancer::rebalance_brackets;
use crate::parser::context::StatementTracker;
use crate::parser::events::Event;
use crate::parser::expr::parse_expr;
use crate::parser::lexer::{TokKind, lex_with_md};
use crate::parser::tree_builder::build_tree;
use crate::syntax::SyntaxNode;

pub use crate::parser::diagnostics::ParseDiagnostic;

#[derive(Debug, Clone)]
pub struct ParseOutput {
    pub cst: SyntaxNode,
    pub diagnostics: Vec<ParseDiagnostic>,
}

/// Options controlling a parse. Construct via [`Default`] and set the fields
/// you need — the struct is `#[non_exhaustive]`, so new options can be added
/// without a breaking change.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ParseOptions {
    /// The markdown mode of a roxygen block carrying no `@md`/`@noMd`
    /// directive of its own (a block's directive always wins, last one in the
    /// block winning — roxygen2's block-level toggle).
    ///
    /// Off means Rd-first, matching [`parse`] and roxygen2's loose-file
    /// default. Since roxygen2 8.0.0 packages normally enable markdown
    /// package-wide (`Config/roxygen2/markdown` in `DESCRIPTION`, or the older
    /// `Roxygen: list(markdown = TRUE)` field) rather than writing per-block
    /// `@md`, so a caller processing such a package should set this to `true`.
    /// Discovering the setting from `DESCRIPTION` is the caller's job — this
    /// crate never does file I/O beyond the text it is handed.
    pub roxygen_markdown_default: bool,
}

impl ParseOptions {
    /// Builder-style setter for [`roxygen_markdown_default`]: the struct is
    /// `#[non_exhaustive]`, so outside this crate
    /// `ParseOptions::default().with_roxygen_markdown_default(true)` is the way
    /// to construct an options value with markdown on.
    ///
    /// [`roxygen_markdown_default`]: ParseOptions::roxygen_markdown_default
    #[must_use]
    pub fn with_roxygen_markdown_default(mut self, on: bool) -> Self {
        self.roxygen_markdown_default = on;
        self
    }
}

pub fn parse(text: &str) -> ParseOutput {
    parse_with_options(text, &ParseOptions::default())
}

/// [`parse`] with caller-supplied [`ParseOptions`]. `parse` itself is
/// `parse_with_options` under default options.
pub fn parse_with_options(text: &str, options: &ParseOptions) -> ParseOutput {
    let md_default = options.roxygen_markdown_default;
    let tokens = rebalance_brackets(lex_with_md(text, md_default));
    let mut diagnostics = Vec::new();
    let mut root_events = Vec::new();

    let mut i = 0usize;
    let mut statements = StatementTracker::default();
    while i < tokens.len() {
        if matches!(
            tokens[i].kind,
            TokKind::Whitespace | TokKind::Newline | TokKind::Semicolon
        ) {
            root_events.push(Event::Tok(i));
            i += 1;
            continue;
        }

        if tokens[i].kind == TokKind::RoxygenMarker {
            i = crate::parser::roxygen::emit_roxygen_block(
                &tokens,
                i,
                &mut root_events,
                md_default,
            );
            continue;
        }

        let before = diagnostics.len();
        if let Some(expr) = parse_expr(&tokens, i, 0, &mut diagnostics, md_default) {
            // A comment is parsed as an atom but is not a statement: it needs no
            // separator from the code it trails, and it must not become the
            // "previous statement" for the next one's check.
            if !tokens[expr.start].kind.is_comment_like() {
                let recovered = diagnostics.len() > before;
                statements.record(&tokens, expr.start, expr.end, recovered, &mut diagnostics);
            }
            root_events.extend(expr.events);
            i = expr.end;
        } else {
            // A token that can't start an expression at the top level (a stray
            // closing delimiter like the second `)` in `f(1))`, or lexer junk).
            // R rejects these outright; flag one so the leniency isn't silent,
            // but keep the token in the tree to preserve losslessness (Tenet 4).
            crate::parser::diagnostics::push_token_diagnostic(
                &mut diagnostics,
                &format!("unexpected '{}'", tokens[i].text),
                &tokens[i],
            );
            root_events.push(Event::Tok(i));
            i += 1;
            statements.record_recovery(i);
        }
    }

    let cst = build_tree(&tokens, &root_events);
    ParseOutput { cst, diagnostics }
}

pub fn reconstruct(text: &str) -> String {
    parse(text)
        .cst
        .descendants_with_tokens()
        .filter_map(|el| el.into_token())
        .map(|tok| tok.text().to_string())
        .collect::<String>()
}
