use crate::parser::context::{ParserCtx, push_token_diagnostic_ctx as push_token_diagnostic};
use crate::parser::cursor::find_function_body_recovery;
use crate::parser::diagnostics::ParseDiagnostic;
use crate::parser::events::{Event, ExprParse, push_range};
use crate::parser::expr::{ident_is_special_constant, parse_expr, parse_expr_in_brackets};
use crate::parser::lexer::{TokKind, Token};
use crate::parser::recovery::push_empty_error_node;
use crate::syntax::SyntaxKind;

fn skip_clause_trivia(tokens: &[Token], mut i: usize) -> usize {
    while tokens.get(i).is_some_and(|t| {
        matches!(t.kind, TokKind::Whitespace | TokKind::Newline) || t.kind.is_comment_like()
    }) {
        i += 1;
    }
    i
}

pub(crate) fn parse_if_expr(
    tokens: &[Token],
    start: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
    md_default: bool,
) -> Option<ExprParse> {
    let ctx = ParserCtx::new(tokens, md_default);
    let if_tok = tokens.get(start)?;
    let mut events = vec![Event::Start(SyntaxKind::IF_EXPR), Event::Tok(start)];
    let mut cursor = start + 1;
    let mut cond_start = skip_clause_trivia(tokens, cursor);
    let mut saw_lparen = false;

    if matches!(
        tokens.get(cond_start).map(|t| &t.kind),
        Some(TokKind::LParen)
    ) {
        push_range(&mut events, cursor, cond_start);
        events.push(Event::Tok(cond_start));
        cursor = cond_start + 1;
        cond_start = skip_clause_trivia(tokens, cursor);
        saw_lparen = true;
    } else {
        push_token_diagnostic(diagnostics, "expected '(' after 'if'", if_tok);
        push_range(&mut events, cursor, cond_start);
        cursor = cond_start;
    }

    let cond_parse = if saw_lparen {
        parse_expr_in_brackets(tokens, cond_start, 0, diagnostics, md_default)
    } else {
        parse_expr(tokens, cond_start, 0, diagnostics, md_default)
    };
    if let Some(cond) = cond_parse {
        push_range(&mut events, cursor, cond.start);
        events.extend(cond.events);
        cursor = cond.end;
    } else {
        push_token_diagnostic(
            diagnostics,
            "expected condition expression after 'if'",
            if_tok,
        );
        push_empty_error_node(&mut events);
        cursor = cond_start;
    }

    if saw_lparen {
        let cond_rparen = skip_clause_trivia(tokens, cursor);
        if matches!(
            tokens.get(cond_rparen).map(|t| &t.kind),
            Some(TokKind::RParen)
        ) {
            push_range(&mut events, cursor, cond_rparen);
            events.push(Event::Tok(cond_rparen));
            cursor = cond_rparen + 1;
        } else {
            push_token_diagnostic(diagnostics, "expected ')' after if condition", if_tok);
        }
    }

    let then_start = skip_clause_trivia(tokens, cursor);
    if let Some(then_expr) = parse_expr(tokens, then_start, 0, diagnostics, md_default) {
        push_range(&mut events, cursor, then_expr.start);
        events.extend(then_expr.events);
        cursor = then_expr.end;
    } else {
        push_token_diagnostic(
            diagnostics,
            "expected expression after if condition",
            if_tok,
        );
        let recovery = ctx.skip_ws_and_newlines(cursor);
        push_range(&mut events, cursor, recovery);
        push_empty_error_node(&mut events);
        cursor = recovery;
    }

    let else_idx = skip_clause_trivia(tokens, cursor);
    if matches!(tokens.get(else_idx).map(|t| &t.kind), Some(TokKind::ElseKw)) {
        push_range(&mut events, cursor, else_idx);
        events.push(Event::Tok(else_idx));
        cursor = else_idx + 1;
        let else_start = skip_clause_trivia(tokens, cursor);

        if let Some(parsed_else) = parse_expr(tokens, else_start, 0, diagnostics, md_default) {
            push_range(&mut events, cursor, parsed_else.start);
            events.extend(parsed_else.events);
            cursor = parsed_else.end;
        } else {
            push_token_diagnostic(
                diagnostics,
                "expected expression after 'else'",
                &tokens[else_idx],
            );
            let mut recovery = cursor;
            while matches!(
                tokens.get(recovery).map(|t| &t.kind),
                Some(TokKind::Whitespace | TokKind::Comment)
            ) {
                recovery += 1;
            }
            push_range(&mut events, cursor, recovery);
            push_empty_error_node(&mut events);
            cursor = recovery;
        }
    }

    events.push(Event::Finish);
    Some(ExprParse {
        start,
        end: cursor,
        events,
    })
}

pub(crate) fn parse_while_expr(
    tokens: &[Token],
    start: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
    md_default: bool,
) -> Option<ExprParse> {
    let ctx = ParserCtx::new(tokens, md_default);
    let while_tok = tokens.get(start)?;
    let mut events = vec![Event::Start(SyntaxKind::WHILE_EXPR), Event::Tok(start)];
    let mut cursor = start + 1;
    let mut cond_start = skip_clause_trivia(tokens, cursor);
    let mut saw_lparen = false;

    if matches!(
        tokens.get(cond_start).map(|t| &t.kind),
        Some(TokKind::LParen)
    ) {
        push_range(&mut events, cursor, cond_start);
        events.push(Event::Tok(cond_start));
        cursor = cond_start + 1;
        cond_start = skip_clause_trivia(tokens, cursor);
        saw_lparen = true;
    } else {
        push_token_diagnostic(diagnostics, "expected '(' after 'while'", while_tok);
        push_range(&mut events, cursor, cond_start);
        cursor = cond_start;
    }

    let cond_parse = if saw_lparen {
        parse_expr_in_brackets(tokens, cond_start, 0, diagnostics, md_default)
    } else {
        parse_expr(tokens, cond_start, 0, diagnostics, md_default)
    };
    if let Some(cond) = cond_parse {
        push_range(&mut events, cursor, cond.start);
        events.extend(cond.events);
        cursor = cond.end;
    } else {
        push_token_diagnostic(
            diagnostics,
            "expected condition expression after 'while'",
            while_tok,
        );
        push_empty_error_node(&mut events);
        cursor = cond_start;
    }

    if saw_lparen {
        let cond_rparen = skip_clause_trivia(tokens, cursor);
        if matches!(
            tokens.get(cond_rparen).map(|t| &t.kind),
            Some(TokKind::RParen)
        ) {
            push_range(&mut events, cursor, cond_rparen);
            events.push(Event::Tok(cond_rparen));
            cursor = cond_rparen + 1;
        } else {
            push_token_diagnostic(diagnostics, "expected ')' after while condition", while_tok);
            push_empty_error_node(&mut events);
        }
    }

    let body_start = ctx.skip_ws_and_newlines(cursor);
    if let Some(body_expr) = parse_expr(tokens, body_start, 0, diagnostics, md_default) {
        push_range(&mut events, cursor, body_expr.start);
        events.extend(body_expr.events);
        cursor = body_expr.end;
    } else {
        push_token_diagnostic(
            diagnostics,
            "expected expression after while condition",
            while_tok,
        );
        let recovery = ctx.skip_ws_and_newlines(cursor);
        push_range(&mut events, cursor, recovery);
        push_empty_error_node(&mut events);
        cursor = recovery;
    }

    events.push(Event::Finish);
    Some(ExprParse {
        start,
        end: cursor,
        events,
    })
}

pub(crate) fn parse_repeat_expr(
    tokens: &[Token],
    start: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
    md_default: bool,
) -> Option<ExprParse> {
    let ctx = ParserCtx::new(tokens, md_default);
    let repeat_tok = tokens.get(start)?;
    let mut events = vec![Event::Start(SyntaxKind::REPEAT_EXPR), Event::Tok(start)];
    let mut cursor = start + 1;

    let body_start = ctx.skip_ws_and_newlines(cursor);
    if let Some(body_expr) = parse_expr(tokens, body_start, 0, diagnostics, md_default) {
        push_range(&mut events, cursor, body_expr.start);
        events.extend(body_expr.events);
        cursor = body_expr.end;
    } else {
        push_token_diagnostic(
            diagnostics,
            "expected expression after 'repeat'",
            repeat_tok,
        );
        let recovery = ctx.skip_ws_and_newlines(cursor);
        push_range(&mut events, cursor, recovery);
        push_empty_error_node(&mut events);
        cursor = recovery;
    }

    events.push(Event::Finish);
    Some(ExprParse {
        start,
        end: cursor,
        events,
    })
}

pub(crate) fn parse_for_expr(
    tokens: &[Token],
    start: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
    md_default: bool,
) -> Option<ExprParse> {
    let ctx = ParserCtx::new(tokens, md_default);
    let for_tok = tokens.get(start)?;
    let mut events = vec![Event::Start(SyntaxKind::FOR_EXPR), Event::Tok(start)];
    let mut cursor = start + 1;
    let clause_start = skip_clause_trivia(tokens, cursor);
    let mut saw_lparen = false;

    if matches!(
        tokens.get(clause_start).map(|t| &t.kind),
        Some(TokKind::LParen)
    ) {
        push_range(&mut events, cursor, clause_start);
        events.push(Event::Tok(clause_start));
        cursor = clause_start + 1;
        saw_lparen = true;
    } else {
        push_token_diagnostic(diagnostics, "expected '(' after 'for'", for_tok);
        push_range(&mut events, cursor, clause_start);
        cursor = clause_start;
    }

    let var_start = skip_clause_trivia(tokens, cursor);
    if matches!(tokens.get(var_start).map(|t| &t.kind), Some(TokKind::Ident)) {
        push_range(&mut events, cursor, var_start);
        events.push(Event::Tok(var_start));
        cursor = var_start + 1;
    } else {
        push_token_diagnostic(
            diagnostics,
            "expected loop variable after '(' in 'for'",
            for_tok,
        );
        push_range(&mut events, cursor, var_start);
        push_empty_error_node(&mut events);
        cursor = var_start;
    }

    let in_idx = skip_clause_trivia(tokens, cursor);
    if matches!(tokens.get(in_idx).map(|t| &t.kind), Some(TokKind::InKw)) {
        push_range(&mut events, cursor, in_idx);
        events.push(Event::Tok(in_idx));
        cursor = in_idx + 1;
    } else {
        push_token_diagnostic(diagnostics, "expected 'in' after for variable", for_tok);
        push_range(&mut events, cursor, in_idx);
        push_empty_error_node(&mut events);
        cursor = in_idx;
    }

    let seq_start = skip_clause_trivia(tokens, cursor);
    let seq_parse = if saw_lparen {
        parse_expr_in_brackets(tokens, seq_start, 0, diagnostics, md_default)
    } else {
        parse_expr(tokens, seq_start, 0, diagnostics, md_default)
    };
    if let Some(seq_expr) = seq_parse {
        push_range(&mut events, cursor, seq_expr.start);
        events.extend(seq_expr.events);
        cursor = seq_expr.end;
    } else {
        push_token_diagnostic(
            diagnostics,
            "expected sequence expression after 'in'",
            for_tok,
        );
        push_range(&mut events, cursor, seq_start);
        push_empty_error_node(&mut events);
        cursor = seq_start;
    }

    if saw_lparen {
        let clause_rparen = skip_clause_trivia(tokens, cursor);
        if matches!(
            tokens.get(clause_rparen).map(|t| &t.kind),
            Some(TokKind::RParen)
        ) {
            push_range(&mut events, cursor, clause_rparen);
            events.push(Event::Tok(clause_rparen));
            cursor = clause_rparen + 1;
        } else {
            push_token_diagnostic(diagnostics, "expected ')' after for clause", for_tok);
            push_empty_error_node(&mut events);
        }
    }

    let body_start = ctx.skip_ws_and_newlines(cursor);
    if let Some(body_expr) = parse_expr(tokens, body_start, 0, diagnostics, md_default) {
        push_range(&mut events, cursor, body_expr.start);
        events.extend(body_expr.events);
        cursor = body_expr.end;
    } else {
        push_token_diagnostic(diagnostics, "expected expression after for clause", for_tok);
        let recovery = ctx.skip_ws_and_newlines(cursor);
        push_range(&mut events, cursor, recovery);
        push_empty_error_node(&mut events);
        cursor = recovery;
    }

    events.push(Event::Finish);
    Some(ExprParse {
        start,
        end: cursor,
        events,
    })
}

/// Reported wherever a function parameter list has something other than a
/// symbol in a name position — an empty slot, a literal, a reserved word.
const MISSING_PARAMETER_NAME: &str = "expected a function parameter name";

/// Whether an identifier is a word R reserves, and so rejects as a parameter
/// name. The rest of R's reserved words (`if`, `for`, `function`, …) lex as
/// their own [`TokKind`] and are caught by not being a [`TokKind::Ident`] at
/// all; only the constants and `break`/`next` arrive as identifiers. A
/// backticked name is not one of these: its text carries the backticks.
fn is_reserved_formal_name(text: &str) -> bool {
    ident_is_special_constant(text) || matches!(text, "break" | "next")
}

pub(crate) fn parse_function_expr(
    tokens: &[Token],
    start: usize,
    inside_brackets: bool,
    diagnostics: &mut Vec<ParseDiagnostic>,
    md_default: bool,
) -> Option<ExprParse> {
    let ctx = ParserCtx::new(tokens, md_default);
    let function_tok = tokens.get(start)?;
    let mut events = vec![Event::Start(SyntaxKind::FUNCTION_EXPR), Event::Tok(start)];
    let mut cursor = start + 1;
    let mut params_lparen = ctx.skip_ws_and_newlines(cursor);
    while matches!(
        tokens.get(params_lparen).map(|t| &t.kind),
        Some(TokKind::Comment)
    ) {
        params_lparen += 1;
        params_lparen = ctx.skip_ws_and_newlines(params_lparen);
    }
    let function_like = matches!(function_tok.kind, TokKind::FunctionKw | TokKind::LambdaFn);

    if matches!(
        tokens.get(params_lparen).map(|t| &t.kind),
        Some(TokKind::LParen)
    ) {
        push_range(&mut events, cursor, params_lparen);
        events.push(Event::Tok(params_lparen));
        cursor = params_lparen + 1;

        // Locate the matching ')'. Default values contain balanced parens, so
        // depth counting still lands on the parameter list's own close.
        let mut close = cursor;
        let mut depth = 1usize;
        while close < tokens.len() {
            match tokens[close].kind {
                TokKind::LParen => depth += 1,
                TokKind::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            close += 1;
        }

        if close < tokens.len() && matches!(tokens[close].kind, TokKind::RParen) {
            // Walk the parameter list, parsing each default value after `=` into
            // a proper expression node (mirroring call-argument value parsing in
            // `parse_call_expr`). Names, commas, `=`, and trivia stay flat tokens;
            // turning the default into a node means a non-trivial default
            // (`if`/call/binary/block) is shaped — and so formats — like the same
            // expression anywhere else, instead of arriving as a loose token run.
            //
            // The walk doubles as the formal list's grammar check. R's
            // `formlist` admits only `SYMBOL` and `SYMBOL = expr` slots, so the
            // shapes a call's argument list tolerates — an empty slot, a string
            // name — are syntax errors here (issue #109). Diagnosing is all
            // that happens: the tokens still land in the tree exactly as
            // written, so the round-trip stays lossless.
            let mut i = cursor;
            // Whether the slot being walked has anything in its name position.
            // A `(` or a `,` clears it; R's grammar requires exactly one symbol
            // before the slot's `,` or the list's `)`.
            let mut slot_named = false;
            let mut seen_comma = false;
            while i < close {
                match tokens[i].kind {
                    TokKind::AssignEq => {
                        if !slot_named {
                            push_token_diagnostic(diagnostics, MISSING_PARAMETER_NAME, &tokens[i]);
                        }
                        slot_named = true;
                        events.push(Event::Tok(i)); // =
                        let val_start = i + 1;
                        let mut value_idx = val_start;
                        while value_idx < close
                            && matches!(
                                tokens.get(value_idx).map(|t| &t.kind),
                                Some(TokKind::Whitespace | TokKind::Newline | TokKind::Comment)
                            )
                        {
                            value_idx += 1;
                        }
                        if value_idx >= close
                            || matches!(
                                tokens.get(value_idx).map(|t| &t.kind),
                                Some(TokKind::Comma)
                            )
                        {
                            // No default expression (a malformed `a =,` / `a =)`); keep
                            // the trivia so the round-trip stays lossless.
                            push_token_diagnostic(
                                diagnostics,
                                "expected a default value after '='",
                                &tokens[i],
                            );
                            for idx in val_start..value_idx {
                                events.push(Event::Tok(idx));
                            }
                            i = value_idx;
                        } else if let Some(val) =
                            parse_expr_in_brackets(tokens, value_idx, 0, diagnostics, md_default)
                        {
                            for idx in val_start..val.start {
                                events.push(Event::Tok(idx));
                            }
                            events.extend(val.events);
                            i = val.end;
                        } else {
                            for idx in val_start..value_idx {
                                events.push(Event::Tok(idx));
                            }
                            i = value_idx;
                        }
                    }
                    TokKind::Comma => {
                        if !slot_named {
                            push_token_diagnostic(diagnostics, MISSING_PARAMETER_NAME, &tokens[i]);
                        }
                        slot_named = false;
                        seen_comma = true;
                        events.push(Event::Tok(i));
                        i += 1;
                    }
                    ref kind
                        if matches!(kind, TokKind::Whitespace | TokKind::Newline)
                            || kind.is_comment_like() =>
                    {
                        events.push(Event::Tok(i));
                        i += 1;
                    }
                    ref kind => {
                        let is_name = matches!(kind, TokKind::Ident)
                            && !is_reserved_formal_name(tokens[i].text);
                        if !is_name {
                            push_token_diagnostic(diagnostics, MISSING_PARAMETER_NAME, &tokens[i]);
                        } else if slot_named {
                            push_token_diagnostic(
                                diagnostics,
                                "expected ',' between function parameters",
                                &tokens[i],
                            );
                        }
                        // Whatever stands here fills the name position, so a
                        // following `=` is not separately reported as nameless.
                        slot_named = true;
                        events.push(Event::Tok(i));
                        i += 1;
                    }
                }
            }
            // A trailing `,` leaves a final empty slot, which R rejects at the
            // `)`. An empty list (`function()`) has no slot at all.
            if seen_comma && !slot_named {
                push_token_diagnostic(diagnostics, MISSING_PARAMETER_NAME, &tokens[close]);
            }
            events.push(Event::Tok(close)); // )
            cursor = close + 1;
        } else {
            push_token_diagnostic(
                diagnostics,
                "expected ')' after function parameters",
                function_tok,
            );
            let recovery = find_function_body_recovery(tokens, cursor);
            push_range(&mut events, cursor, recovery);
            push_empty_error_node(&mut events);
            cursor = recovery;
        }
    } else {
        let message = if function_like {
            "expected '(' after function"
        } else {
            "expected '(' after 'function'"
        };
        push_token_diagnostic(diagnostics, message, function_tok);
        push_range(&mut events, cursor, params_lparen);
        cursor = params_lparen;
    }

    let mut body_start = ctx.skip_ws_and_newlines(cursor);
    while matches!(
        tokens.get(body_start).map(|t| &t.kind),
        Some(TokKind::Comment)
    ) {
        body_start += 1;
        body_start = ctx.skip_ws_and_newlines(body_start);
    }
    // Inside brackets a newline does not terminate the body, so a body continued
    // by a binary operator on the next line (`vapply(p, function(x) x == 1\n ||
    // g(x), NA)`) keeps going. At top level the newline ends it, so `function(x)
    // x` followed by `+1` on the next line stays two statements.
    let body = if inside_brackets {
        parse_expr_in_brackets(tokens, body_start, 0, diagnostics, md_default)
    } else {
        parse_expr(tokens, body_start, 0, diagnostics, md_default)
    };
    if let Some(body_expr) = body {
        push_range(&mut events, cursor, body_expr.start);
        events.extend(body_expr.events);
        cursor = body_expr.end;
    } else {
        push_token_diagnostic(
            diagnostics,
            "expected expression after function parameters",
            function_tok,
        );
        let recovery = ctx.skip_ws_and_newlines(cursor);
        push_range(&mut events, cursor, recovery);
        push_empty_error_node(&mut events);
        cursor = recovery;
    }

    events.push(Event::Finish);
    Some(ExprParse {
        start,
        end: cursor,
        events,
    })
}
