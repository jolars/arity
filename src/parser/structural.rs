use crate::parser::context::{ParserCtx, push_token_diagnostic_ctx as push_token_diagnostic};
use crate::parser::cursor::find_function_body_recovery;
use crate::parser::diagnostics::ParseDiagnostic;
use crate::parser::events::{Event, ExprParse, push_range};
use crate::parser::expr::{parse_expr, parse_expr_in_brackets};
use crate::parser::lexer::{TokKind, Token};
use crate::parser::recovery::push_empty_error_node;
use crate::syntax::SyntaxKind;

fn skip_clause_trivia(tokens: &[Token], mut i: usize) -> usize {
    let ctx = ParserCtx::new(tokens);
    while matches!(
        ctx.token(i).map(|t| &t.kind),
        Some(TokKind::Whitespace | TokKind::Newline | TokKind::Comment)
    ) {
        i += 1;
    }
    i
}

pub(crate) fn parse_if_expr(
    tokens: &[Token],
    start: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Option<ExprParse> {
    let ctx = ParserCtx::new(tokens);
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
        parse_expr_in_brackets(tokens, cond_start, 0, diagnostics)
    } else {
        parse_expr(tokens, cond_start, 0, diagnostics)
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
    if let Some(then_expr) = parse_expr(tokens, then_start, 0, diagnostics) {
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

        if let Some(parsed_else) = parse_expr(tokens, else_start, 0, diagnostics) {
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
) -> Option<ExprParse> {
    let ctx = ParserCtx::new(tokens);
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
        parse_expr_in_brackets(tokens, cond_start, 0, diagnostics)
    } else {
        parse_expr(tokens, cond_start, 0, diagnostics)
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
    if let Some(body_expr) = parse_expr(tokens, body_start, 0, diagnostics) {
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
) -> Option<ExprParse> {
    let ctx = ParserCtx::new(tokens);
    let repeat_tok = tokens.get(start)?;
    let mut events = vec![Event::Start(SyntaxKind::REPEAT_EXPR), Event::Tok(start)];
    let mut cursor = start + 1;

    let body_start = ctx.skip_ws_and_newlines(cursor);
    if let Some(body_expr) = parse_expr(tokens, body_start, 0, diagnostics) {
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
) -> Option<ExprParse> {
    let ctx = ParserCtx::new(tokens);
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
        parse_expr_in_brackets(tokens, seq_start, 0, diagnostics)
    } else {
        parse_expr(tokens, seq_start, 0, diagnostics)
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
    if let Some(body_expr) = parse_expr(tokens, body_start, 0, diagnostics) {
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

pub(crate) fn parse_function_expr(
    tokens: &[Token],
    start: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Option<ExprParse> {
    let ctx = ParserCtx::new(tokens);
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
            let mut i = cursor;
            while i < close {
                if matches!(tokens[i].kind, TokKind::AssignEq) {
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
                        || matches!(tokens.get(value_idx).map(|t| &t.kind), Some(TokKind::Comma))
                    {
                        // No default expression (a malformed `a =,` / `a =)`); keep
                        // the trivia so the round-trip stays lossless.
                        for idx in val_start..value_idx {
                            events.push(Event::Tok(idx));
                        }
                        i = value_idx;
                    } else if let Some(val) =
                        parse_expr_in_brackets(tokens, value_idx, 0, diagnostics)
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
                } else {
                    events.push(Event::Tok(i));
                    i += 1;
                }
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
    if let Some(body_expr) = parse_expr(tokens, body_start, 0, diagnostics) {
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
