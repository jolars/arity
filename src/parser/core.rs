use crate::parser::bracket_balancer::rebalance_brackets;
use crate::parser::events::Event;
use crate::parser::expr::parse_expr;
use crate::parser::lexer::{TokKind, lex};
use crate::parser::tree_builder::build_tree;
use crate::syntax::SyntaxNode;

pub use crate::parser::diagnostics::ParseDiagnostic;

#[derive(Debug, Clone)]
pub struct ParseOutput {
    pub cst: SyntaxNode,
    pub diagnostics: Vec<ParseDiagnostic>,
}

pub fn parse(text: &str) -> ParseOutput {
    let tokens = rebalance_brackets(lex(text));
    let mut diagnostics = Vec::new();
    let mut root_events = Vec::new();

    let mut i = 0usize;
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
            i = crate::parser::roxygen::emit_roxygen_block(&tokens, i, &mut root_events);
            continue;
        }

        if let Some(expr) = parse_expr(&tokens, i, 0, &mut diagnostics) {
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
