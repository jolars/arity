use crate::parser::context::{ParserCtx, push_token_diagnostic_ctx as push_token_diagnostic};
use crate::parser::diagnostics::ParseDiagnostic;
use crate::parser::events::{Event, ExprParse};
use crate::parser::lexer::{TokKind, Token};
use crate::parser::recovery::error_expr_to_line_end;
use crate::parser::structural::{
    parse_for_expr, parse_function_expr, parse_if_expr, parse_repeat_expr, parse_while_expr,
};
use crate::syntax::SyntaxKind;

fn is_assignment_operator(kind: &TokKind) -> bool {
    matches!(
        kind,
        TokKind::AssignLeft
            | TokKind::SuperAssign
            | TokKind::AssignRight
            | TokKind::SuperAssignRight
            | TokKind::AssignEq
            | TokKind::Walrus
    )
}

fn is_infix_operator(kind: &TokKind) -> bool {
    infix_binding_power(kind).is_some()
}

fn next_operator<'a>(
    ctx: &ParserCtx<'a>,
    start: usize,
    inside_brackets: bool,
) -> Option<(usize, &'a Token)> {
    let op_idx = ctx.skip_ws(start);
    let op = ctx.token(op_idx)?;
    if op.kind == TokKind::Newline {
        // Outside brackets, a newline after a complete operand terminates the
        // expression — R only continues across the newline if the prior line is
        // incomplete (i.e. ended in an operator). Inside `(`, `[`, `[[` (and
        // the conditions/sequences of `if`/`while`/`for`), newlines are not
        // statement separators, so peek past them for a continuation operator.
        if !inside_brackets {
            return None;
        }
        let next_idx = ctx.skip_ws_and_newlines(start);
        let next = ctx.token(next_idx)?;
        if is_assignment_operator(&next.kind) || is_infix_operator(&next.kind) {
            return Some((next_idx, next));
        }
        return None;
    }
    if is_assignment_operator(&op.kind) || is_infix_operator(&op.kind) {
        return Some((op_idx, op));
    }
    None
}

pub(crate) fn parse_expr(
    tokens: &[Token],
    start: usize,
    min_bp: u8,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Option<ExprParse> {
    parse_expr_with_mode(tokens, start, min_bp, diagnostics, false, false)
}

pub(crate) fn parse_expr_in_brackets(
    tokens: &[Token],
    start: usize,
    min_bp: u8,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Option<ExprParse> {
    parse_expr_with_mode(tokens, start, min_bp, diagnostics, true, true)
}

fn parse_expr_with_mode(
    tokens: &[Token],
    start: usize,
    min_bp: u8,
    diagnostics: &mut Vec<ParseDiagnostic>,
    allow_newline_prefix: bool,
    inside_brackets: bool,
) -> Option<ExprParse> {
    let ctx = ParserCtx::new(tokens);
    let start_non_ws = if allow_newline_prefix {
        ctx.skip_ws_and_newlines(start)
    } else {
        ctx.skip_ws(start)
    };
    if matches!(
        tokens.get(start_non_ws).map(|t| &t.kind),
        Some(TokKind::IfKw)
    ) {
        return parse_if_expr(tokens, start_non_ws, diagnostics);
    }
    if matches!(
        tokens.get(start_non_ws).map(|t| &t.kind),
        Some(TokKind::ForKw)
    ) {
        return parse_for_expr(tokens, start_non_ws, diagnostics);
    }
    if matches!(
        tokens.get(start_non_ws).map(|t| &t.kind),
        Some(TokKind::WhileKw)
    ) {
        return parse_while_expr(tokens, start_non_ws, diagnostics);
    }
    if matches!(
        tokens.get(start_non_ws).map(|t| &t.kind),
        Some(TokKind::RepeatKw)
    ) {
        return parse_repeat_expr(tokens, start_non_ws, diagnostics);
    }
    if matches!(
        tokens.get(start_non_ws).map(|t| &t.kind),
        Some(TokKind::FunctionKw)
    ) {
        return parse_function_expr(tokens, start_non_ws, diagnostics);
    }
    if matches!(
        tokens.get(start_non_ws).map(|t| &t.kind),
        Some(TokKind::LambdaFn)
    ) {
        return parse_function_expr(tokens, start_non_ws, diagnostics);
    }

    let mut lhs = parse_prefix(
        &ctx,
        start,
        diagnostics,
        allow_newline_prefix,
        inside_brackets,
    )?;

    loop {
        lhs = parse_postfix_chain(&ctx, lhs, diagnostics);

        let Some((op_idx, op)) = next_operator(&ctx, lhs.end, inside_brackets) else {
            break;
        };

        if is_assignment_operator(&op.kind) {
            let (l_bp, r_bp) = (1, 1);
            if l_bp < min_bp {
                break;
            }

            let rhs_start = op_idx + 1;
            // A pending operator means the line is incomplete, so newlines and
            // comments before the RHS operand are not statement terminators or
            // operands — skip past them to the operand (R does the same). The
            // skipped tokens stay in the tree as trivia between the operator and
            // the operand, emitted by the event builder.
            let rhs_operand = ctx.skip_ws_newlines_comments(rhs_start);
            let Some(rhs) = parse_expr_with_mode(
                tokens,
                rhs_operand,
                r_bp,
                diagnostics,
                true,
                inside_brackets,
            ) else {
                push_token_diagnostic(diagnostics, "expected assignment right-hand side", op);
                return Some(error_expr_to_line_end(tokens, lhs.start, rhs_start));
            };

            lhs = build_assignment_expr(lhs, op_idx, rhs);
            continue;
        }

        let Some((l_bp, r_bp)) = infix_binding_power(&op.kind) else {
            break;
        };
        if l_bp < min_bp {
            break;
        }

        let rhs_start = op_idx + 1;
        // A pending operator means the line is incomplete, so newlines and
        // comments before the RHS operand are not statement terminators or
        // operands — skip past them to the operand (the skipped tokens remain
        // in the tree as trivia between the operator and the operand).
        let rhs_operand = ctx.skip_ws_newlines_comments(rhs_start);
        let Some(rhs) = parse_expr_with_mode(
            tokens,
            rhs_operand,
            r_bp,
            diagnostics,
            true,
            inside_brackets,
        ) else {
            push_token_diagnostic(
                diagnostics,
                "expected right-hand side for binary operator",
                op,
            );
            return Some(error_expr_to_line_end(tokens, lhs.start, rhs_start));
        };

        if matches!(op.kind, TokKind::Colon2 | TokKind::Colon3)
            && lhs
                .events
                .iter()
                .any(|event| matches!(event, Event::Start(SyntaxKind::CALL_EXPR)))
        {
            push_token_diagnostic(
                diagnostics,
                "namespace operator requires a package name on the left-hand side",
                op,
            );
        }

        if matches!(op.kind, TokKind::Colon2 | TokKind::Colon3)
            && matches!(expr_root_kind(&rhs), Some(SyntaxKind::BLOCK_EXPR))
        {
            push_token_diagnostic(
                diagnostics,
                "namespace operator requires an identifier on the right-hand side",
                op,
            );
        }

        if matches!(op.kind, TokKind::Colon2 | TokKind::Colon3)
            && matches!(
                expr_root_kind(&rhs),
                Some(SyntaxKind::INT | SyntaxKind::FLOAT | SyntaxKind::COMPLEX)
            )
        {
            push_token_diagnostic(
                diagnostics,
                "namespace operator requires an identifier on the right-hand side",
                op,
            );
        }

        if matches!(op.kind, TokKind::Colon2 | TokKind::Colon3)
            && matches!(expr_root_kind(&rhs), Some(SyntaxKind::IDENT))
        {
            let rhs_tok_idx = ctx.skip_ws(rhs.start);
            if let Some(tok) = tokens.get(rhs_tok_idx)
                && ident_is_special_constant(tok.text.as_str())
            {
                push_token_diagnostic(
                    diagnostics,
                    "namespace operator requires an identifier on the right-hand side",
                    op,
                );
            }
        }

        if matches!(op.kind, TokKind::Colon2 | TokKind::Colon3)
            && matches!(expr_root_kind(&lhs), Some(SyntaxKind::BINARY_EXPR))
            && lhs
                .events
                .iter()
                .any(|event| matches!(event, Event::Tok(idx) if matches!(tokens[*idx].kind, TokKind::Colon2 | TokKind::Colon3)))
        {
            push_token_diagnostic(diagnostics, "namespace operator chaining is not allowed", op);
        }

        lhs = build_binary_expr(lhs, op_idx, rhs);
    }

    Some(lhs)
}

fn parse_prefix(
    ctx: &ParserCtx<'_>,
    start: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
    allow_newline_prefix: bool,
    inside_brackets: bool,
) -> Option<ExprParse> {
    let tokens = ctx.tokens();
    let i = if allow_newline_prefix {
        ctx.skip_ws_and_newlines(start)
    } else {
        ctx.skip_ws(start)
    };
    let tok = tokens.get(i)?;

    match tok.kind {
        TokKind::Plus | TokKind::Minus | TokKind::Bang | TokKind::Tilde | TokKind::Question => {
            // Unary `~` (R formula) and unary `?` (help) sit at the same low
            // precedence tier as their binary counterparts, so `~ x + y` must
            // parse as `~(x + y)` and `? x + y` as `?(x + y)`. Match the infix
            // right-binding power so every higher-precedence infix folds into
            // the operand.
            //
            // Unary `!` binds *looser* than the comparison and arithmetic
            // operators but tighter than `&&`/`||` (R's precedence table): `!a ==
            // b` is `!(a == b)` and `!a + b` is `!(a + b)`, while `!a & b` is
            // `(!a) & b`. Its right-binding power sits between the And tier
            // (60/61) and the Relational tier (80/81) so relational and tighter
            // infixes fold into the operand and the logical operators do not.
            // Unary `+`/`-`, by contrast, bind tightly (just below `^`), so they
            // keep the high right-binding power.
            let rbp = match tok.kind {
                TokKind::Tilde => 41,
                TokKind::Question => 1,
                TokKind::Bang => 70,
                _ => 130,
            };
            let operand_start = i + 1;
            let Some(operand) = parse_expr_with_mode(
                tokens,
                operand_start,
                rbp,
                diagnostics,
                true,
                inside_brackets,
            ) else {
                push_token_diagnostic(diagnostics, "expected operand for unary operator", tok);
                return Some(error_expr_to_line_end(tokens, i, operand_start));
            };
            let mut events = Vec::new();
            events.push(Event::Start(SyntaxKind::UNARY_EXPR));
            events.push(Event::Tok(i));
            for idx in (i + 1)..operand.start {
                events.push(Event::Tok(idx));
            }
            events.extend(operand.events);
            events.push(Event::Finish);
            Some(ExprParse {
                start: i,
                end: operand.end,
                events,
            })
        }
        TokKind::Ident
        | TokKind::Int
        | TokKind::Float
        | TokKind::Complex
        | TokKind::String
        | TokKind::Comment
        | TokKind::UserOp => Some(ExprParse {
            start: i,
            end: i + 1,
            events: vec![Event::Tok(i)],
        }),
        TokKind::LParen => {
            let inner_start = i + 1;
            let mut expr_start = inner_start;
            while matches!(
                tokens.get(expr_start).map(|t| &t.kind),
                Some(TokKind::Whitespace | TokKind::Newline | TokKind::Comment)
            ) {
                expr_start += 1;
            }
            let Some(inner) = parse_expr_with_mode(tokens, expr_start, 0, diagnostics, true, true)
            else {
                push_token_diagnostic(diagnostics, "expected expression after '('", tok);
                return Some(error_expr_to_line_end(tokens, i, inner_start));
            };
            let mut close_idx = inner.end;
            while matches!(
                tokens.get(close_idx).map(|t| &t.kind),
                Some(TokKind::Whitespace | TokKind::Newline | TokKind::Comment)
            ) {
                close_idx += 1;
            }
            if !matches!(
                tokens.get(close_idx).map(|t| &t.kind),
                Some(TokKind::RParen)
            ) {
                push_token_diagnostic(diagnostics, "expected ')'", tok);
                let mut events = Vec::new();
                events.push(Event::Start(SyntaxKind::PAREN_EXPR));
                events.push(Event::Tok(i));
                for idx in inner_start..inner.start {
                    events.push(Event::Tok(idx));
                }
                events.extend(inner.events);
                for idx in inner.end..close_idx {
                    events.push(Event::Tok(idx));
                }
                events.push(Event::Finish);
                return Some(ExprParse {
                    start: i,
                    end: close_idx,
                    events,
                });
            }

            let mut events = Vec::new();
            events.push(Event::Start(SyntaxKind::PAREN_EXPR));
            events.push(Event::Tok(i));
            for idx in inner_start..inner.start {
                events.push(Event::Tok(idx));
            }
            events.extend(inner.events);
            for idx in inner.end..close_idx {
                events.push(Event::Tok(idx));
            }
            events.push(Event::Tok(close_idx));
            events.push(Event::Finish);

            Some(ExprParse {
                start: i,
                end: close_idx + 1,
                events,
            })
        }
        TokKind::LBrace => parse_block_expr(ctx, i, diagnostics),
        TokKind::Star
        | TokKind::Slash
        | TokKind::Caret
        | TokKind::AssignLeft
        | TokKind::SuperAssign
        | TokKind::AssignRight
        | TokKind::SuperAssignRight
        | TokKind::AssignEq
        | TokKind::Walrus
        | TokKind::Colon
        | TokKind::Or
        | TokKind::Or2
        | TokKind::And
        | TokKind::And2
        | TokKind::Equal2
        | TokKind::NotEqual
        | TokKind::LessThan
        | TokKind::LessThanOrEqual
        | TokKind::GreaterThan
        | TokKind::GreaterThanOrEqual
        | TokKind::Colon2
        | TokKind::Colon3
        | TokKind::Dollar
        | TokKind::At
        | TokKind::Semicolon
        | TokKind::Comma
        | TokKind::Pipe => {
            push_token_diagnostic(diagnostics, "unexpected operator at expression start", tok);
            Some(error_expr_to_line_end(tokens, i, i + 1))
        }
        TokKind::Whitespace
        | TokKind::Newline
        | TokKind::RParen
        | TokKind::RBrack
        | TokKind::RBrack2
        | TokKind::RBrace
        | TokKind::IfKw
        | TokKind::ElseKw
        | TokKind::ForKw
        | TokKind::WhileKw
        | TokKind::RepeatKw
        | TokKind::FunctionKw
        | TokKind::LambdaFn
        | TokKind::InKw
        | TokKind::LBrack
        | TokKind::LBrack2
        | TokKind::RoxygenMarker
        | TokKind::RoxygenAt
        | TokKind::RoxygenTagName
        | TokKind::RoxygenTagArg
        | TokKind::RoxygenText
        | TokKind::RoxygenCode
        | TokKind::RoxygenRdMacro
        | TokKind::RoxygenMdLink
        | TokKind::RoxygenMdImage
        | TokKind::RoxygenMdEmph
        | TokKind::RoxygenMdStrong
        | TokKind::RoxygenMdCode
        | TokKind::RoxygenMdListMarker
        | TokKind::RoxygenMdFence
        | TokKind::RoxygenMdHtml
        | TokKind::Unknown => None,
    }
}

fn parse_block_expr(
    ctx: &ParserCtx<'_>,
    start: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Option<ExprParse> {
    let tokens = ctx.tokens();
    let mut i = start + 1;
    let mut events = vec![Event::Start(SyntaxKind::BLOCK_EXPR), Event::Tok(start)];

    loop {
        let next = ctx.skip_ws(i);
        let Some(tok) = tokens.get(next) else {
            push_token_diagnostic(diagnostics, "expected '}' to close block", &tokens[start]);
            for idx in i..tokens.len() {
                events.push(Event::Tok(idx));
            }
            events.push(Event::Finish);
            return Some(ExprParse {
                start,
                end: tokens.len(),
                events,
            });
        };

        if tok.kind == TokKind::RBrace {
            for idx in i..next {
                events.push(Event::Tok(idx));
            }
            events.push(Event::Tok(next));
            events.push(Event::Finish);
            return Some(ExprParse {
                start,
                end: next + 1,
                events,
            });
        }

        // Consume newlines and semicolons as statement separators.
        if matches!(tok.kind, TokKind::Newline | TokKind::Semicolon) {
            for idx in i..=next {
                events.push(Event::Tok(idx));
            }
            i = next + 1;
            continue;
        }

        if tok.kind == TokKind::RoxygenMarker {
            // Emit the leading whitespace gap, then the grouped block.
            for idx in i..next {
                events.push(Event::Tok(idx));
            }
            i = crate::parser::roxygen::emit_roxygen_block(tokens, next, &mut events);
            continue;
        }

        if let Some(expr) = parse_expr(tokens, i, 0, diagnostics) {
            for idx in i..expr.start {
                events.push(Event::Tok(idx));
            }
            events.extend(expr.events);
            i = expr.end;
        } else {
            events.push(Event::Tok(i));
            i += 1;
        }
    }
}

fn parse_postfix_chain(
    ctx: &ParserCtx<'_>,
    mut lhs: ExprParse,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> ExprParse {
    let tokens = ctx.tokens();
    loop {
        // No newline is allowed between the callee and `(` — `f\n(x)` is two statements in R.
        let after_lhs = ctx.skip_ws(lhs.end);
        if matches!(
            tokens.get(after_lhs).map(|t| &t.kind),
            Some(TokKind::LParen)
        ) {
            lhs = parse_call_expr(ctx, lhs, after_lhs, diagnostics);
            continue;
        }
        if matches!(
            tokens.get(after_lhs).map(|t| &t.kind),
            Some(TokKind::LBrack)
        ) {
            lhs = parse_subset_expr(ctx, lhs, after_lhs, diagnostics);
            continue;
        }
        if matches!(
            tokens.get(after_lhs).map(|t| &t.kind),
            Some(TokKind::LBrack2)
        ) {
            lhs = parse_subset2_expr(ctx, lhs, after_lhs, diagnostics);
            continue;
        }
        break;
    }
    lhs
}

fn build_binary_like_expr(
    kind: SyntaxKind,
    lhs: ExprParse,
    op_idx: usize,
    rhs: ExprParse,
) -> ExprParse {
    let mut events = Vec::new();
    events.push(Event::Start(kind));
    events.extend(lhs.events);
    for idx in lhs.end..op_idx {
        events.push(Event::Tok(idx));
    }
    events.push(Event::Tok(op_idx));
    for idx in (op_idx + 1)..rhs.start {
        events.push(Event::Tok(idx));
    }
    events.extend(rhs.events);
    events.push(Event::Finish);

    ExprParse {
        start: lhs.start,
        end: rhs.end,
        events,
    }
}

fn build_binary_expr(lhs: ExprParse, op_idx: usize, rhs: ExprParse) -> ExprParse {
    build_binary_like_expr(SyntaxKind::BINARY_EXPR, lhs, op_idx, rhs)
}

fn build_assignment_expr(lhs: ExprParse, op_idx: usize, rhs: ExprParse) -> ExprParse {
    build_binary_like_expr(SyntaxKind::ASSIGNMENT_EXPR, lhs, op_idx, rhs)
}

/// Returns true if tokens starting at `i` match the pattern `name =` (named
/// argument), where `=` is not `==`. The name may be an identifier or a string
/// literal: inside an argument list `=` is always named-argument syntax (never
/// assignment), and R accepts a string as the name (`c("SE" = 13)`).
fn is_named_arg(ctx: &ParserCtx<'_>, i: usize) -> bool {
    let tokens = ctx.tokens();
    if !matches!(
        tokens.get(i).map(|t| &t.kind),
        Some(TokKind::Ident | TokKind::String)
    ) {
        return false;
    }
    let mut next = i + 1;
    while matches!(
        tokens.get(next).map(|t| &t.kind),
        Some(TokKind::Whitespace | TokKind::Newline | TokKind::Comment)
    ) {
        next += 1;
    }
    matches!(tokens.get(next).map(|t| &t.kind), Some(TokKind::AssignEq))
}

fn has_newline(tokens: &[Token], start: usize, end: usize) -> bool {
    tokens[start..end]
        .iter()
        .any(|t| t.kind == TokKind::Newline)
}

fn expr_root_kind(expr: &ExprParse) -> Option<SyntaxKind> {
    expr.events.iter().find_map(|event| match event {
        Event::Start(kind) => Some(*kind),
        _ => None,
    })
}

pub(crate) fn ident_is_special_constant(text: &str) -> bool {
    matches!(
        text,
        "NA" | "NULL"
            | "TRUE"
            | "FALSE"
            | "Inf"
            | "NaN"
            | "NA_integer_"
            | "NA_real_"
            | "NA_complex_"
            | "NA_character_"
    )
}

fn parse_call_expr(
    ctx: &ParserCtx<'_>,
    callee: ExprParse,
    lparen_idx: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> ExprParse {
    let tokens = ctx.tokens();
    let mut events = vec![Event::Start(SyntaxKind::CALL_EXPR)];
    events.extend(callee.events);
    // Whitespace between callee end and `(`
    for idx in callee.end..lparen_idx {
        events.push(Event::Tok(idx));
    }
    events.push(Event::Tok(lparen_idx)); // (

    events.push(Event::Start(SyntaxKind::ARG_LIST));
    let mut i = lparen_idx + 1;

    let mut expect_delimiter = false;
    let mut last_arg_was_comment_only = false;
    loop {
        // Skip whitespace and newlines within the argument list.
        let next_i = ctx.skip_ws_and_newlines(i);
        let had_newline_gap = has_newline(tokens, i, next_i);
        for idx in i..next_i {
            events.push(Event::Tok(idx));
        }
        i = next_i;

        // End of argument list.
        if matches!(tokens.get(i).map(|t| &t.kind), Some(TokKind::RParen) | None) {
            break;
        }

        if expect_delimiter
            && !had_newline_gap
            && let Some(tok) = tokens.get(i)
        {
            let current_is_comment = tok.kind == TokKind::Comment;
            if !current_is_comment && !last_arg_was_comment_only {
                push_token_diagnostic(diagnostics, "expected ',' between arguments", tok);
            }
        }

        // Empty argument (leading or consecutive comma).
        if matches!(tokens.get(i).map(|t| &t.kind), Some(TokKind::Comma)) {
            events.push(Event::Start(SyntaxKind::ARG));
            events.push(Event::Finish);
            events.push(Event::Tok(i)); // ,
            i += 1;
            expect_delimiter = false;
            last_arg_was_comment_only = false;
            continue;
        }

        events.push(Event::Start(SyntaxKind::ARG));
        last_arg_was_comment_only = false;

        if is_named_arg(ctx, i) {
            // Named argument: ident = expr
            events.push(Event::Tok(i)); // ident
            let mut eq_idx = i + 1;
            while matches!(
                tokens.get(eq_idx).map(|t| &t.kind),
                Some(TokKind::Whitespace | TokKind::Newline | TokKind::Comment)
            ) {
                eq_idx += 1;
            }
            for idx in (i + 1)..eq_idx {
                events.push(Event::Tok(idx));
            }
            if !matches!(tokens.get(eq_idx).map(|t| &t.kind), Some(TokKind::AssignEq)) {
                events.push(Event::Finish); // ARG
                expect_delimiter = true;
                i = eq_idx;
                continue;
            }
            events.push(Event::Tok(eq_idx)); // =
            let val_start = eq_idx + 1;
            let mut value_idx = val_start;
            while matches!(
                tokens.get(value_idx).map(|t| &t.kind),
                Some(TokKind::Whitespace | TokKind::Newline | TokKind::Comment)
            ) {
                value_idx += 1;
            }
            if matches!(
                tokens.get(value_idx).map(|t| &t.kind),
                Some(TokKind::Comma | TokKind::RParen) | None
            ) {
                for idx in val_start..value_idx {
                    events.push(Event::Tok(idx));
                }
                i = value_idx;
            } else if let Some(val) = parse_expr_in_brackets(tokens, value_idx, 0, diagnostics) {
                for idx in val_start..val.start {
                    events.push(Event::Tok(idx));
                }
                events.extend(val.events);
                i = val.end;
            } else {
                i = val_start;
            }
        } else {
            // Positional argument.
            if let Some(arg) = parse_expr_in_brackets(tokens, i, 0, diagnostics) {
                last_arg_was_comment_only = arg.end == arg.start + 1
                    && matches!(
                        tokens.get(arg.start).map(|t| &t.kind),
                        Some(TokKind::Comment)
                    );
                for idx in i..arg.start {
                    events.push(Event::Tok(idx));
                }
                events.extend(arg.events);
                i = arg.end;
            } else {
                last_arg_was_comment_only = false;
                events.push(Event::Tok(i));
                i += 1;
            }
        }

        events.push(Event::Finish); // ARG
        expect_delimiter = true;

        // Skip whitespace/newlines after arg, then consume optional comma.
        let next_i = ctx.skip_ws_and_newlines(i);
        for idx in i..next_i {
            events.push(Event::Tok(idx));
        }
        i = next_i;

        if matches!(tokens.get(i).map(|t| &t.kind), Some(TokKind::Comma)) {
            events.push(Event::Tok(i)); // ,
            i += 1;
            expect_delimiter = false;
            last_arg_was_comment_only = false;
        }
    }

    events.push(Event::Finish); // ARG_LIST

    // Closing paren (may have trailing whitespace/newlines before it).
    let next_i = ctx.skip_ws_and_newlines(i);
    for idx in i..next_i {
        events.push(Event::Tok(idx));
    }
    i = next_i;

    if matches!(tokens.get(i).map(|t| &t.kind), Some(TokKind::RParen)) {
        events.push(Event::Tok(i));
        i += 1;
    } else if let Some(tok) = tokens.get(lparen_idx) {
        push_token_diagnostic(diagnostics, "expected ')' to close function call", tok);
    }

    events.push(Event::Finish); // CALL_EXPR

    ExprParse {
        start: callee.start,
        end: i,
        events,
    }
}

fn parse_subset_expr(
    ctx: &ParserCtx<'_>,
    target: ExprParse,
    lbrack_idx: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> ExprParse {
    parse_bracket_expr(
        ctx,
        target,
        lbrack_idx,
        SyntaxKind::SUBSET_EXPR,
        diagnostics,
    )
}

fn parse_subset2_expr(
    ctx: &ParserCtx<'_>,
    target: ExprParse,
    lbrack2_idx: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> ExprParse {
    parse_bracket_expr(
        ctx,
        target,
        lbrack2_idx,
        SyntaxKind::SUBSET2_EXPR,
        diagnostics,
    )
}

fn parse_bracket_expr(
    ctx: &ParserCtx<'_>,
    target: ExprParse,
    open_idx: usize,
    node_kind: SyntaxKind,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> ExprParse {
    let tokens = ctx.tokens();
    let mut events = vec![Event::Start(node_kind)];
    events.extend(target.events);
    for idx in target.end..open_idx {
        events.push(Event::Tok(idx));
    }
    events.push(Event::Tok(open_idx));

    events.push(Event::Start(SyntaxKind::ARG_LIST));
    let mut i = open_idx + 1;

    let mut expect_delimiter = false;
    let mut last_arg_was_comment_only = false;
    loop {
        let next_i = ctx.skip_ws_and_newlines(i);
        let had_newline_gap = has_newline(tokens, i, next_i);
        for idx in i..next_i {
            events.push(Event::Tok(idx));
        }
        i = next_i;

        if is_matching_subset_close(tokens, i, node_kind) || tokens.get(i).is_none() {
            break;
        }

        if expect_delimiter
            && !had_newline_gap
            && let Some(tok) = tokens.get(i)
        {
            let current_is_comment = tok.kind == TokKind::Comment;
            if !current_is_comment && !last_arg_was_comment_only {
                push_token_diagnostic(diagnostics, "expected ',' between subset arguments", tok);
            }
        }

        if matches!(tokens.get(i).map(|t| &t.kind), Some(TokKind::Comma)) {
            events.push(Event::Start(SyntaxKind::ARG));
            events.push(Event::Finish);
            events.push(Event::Tok(i));
            i += 1;
            expect_delimiter = false;
            last_arg_was_comment_only = false;
            continue;
        }

        events.push(Event::Start(SyntaxKind::ARG));
        last_arg_was_comment_only = false;

        if is_named_arg(ctx, i) {
            // Named subscript argument: `ident = expr` (e.g. `drop = FALSE`).
            // `=` inside `[`/`[[` is named-argument syntax, never an assignment,
            // so this parses flat (like a call argument), not as an
            // ASSIGNMENT_EXPR.
            events.push(Event::Tok(i)); // ident
            let mut eq_idx = i + 1;
            while matches!(
                tokens.get(eq_idx).map(|t| &t.kind),
                Some(TokKind::Whitespace | TokKind::Newline | TokKind::Comment)
            ) {
                eq_idx += 1;
            }
            for idx in (i + 1)..eq_idx {
                events.push(Event::Tok(idx));
            }
            // `is_named_arg` guarantees an `=` at `eq_idx`.
            events.push(Event::Tok(eq_idx)); // =
            let val_start = eq_idx + 1;
            let mut value_idx = val_start;
            while matches!(
                tokens.get(value_idx).map(|t| &t.kind),
                Some(TokKind::Whitespace | TokKind::Newline | TokKind::Comment)
            ) {
                value_idx += 1;
            }
            if matches!(
                tokens.get(value_idx).map(|t| &t.kind),
                Some(TokKind::Comma) | None
            ) || is_matching_subset_close(tokens, value_idx, node_kind)
            {
                // Empty value, e.g. `x[drop = ]`.
                for idx in val_start..value_idx {
                    events.push(Event::Tok(idx));
                }
                i = value_idx;
            } else if let Some(val) = parse_expr_in_brackets(tokens, value_idx, 0, diagnostics) {
                for idx in val_start..val.start {
                    events.push(Event::Tok(idx));
                }
                events.extend(val.events);
                i = val.end;
            } else {
                i = val_start;
            }
        } else if let Some(arg) = parse_expr_in_brackets(tokens, i, 0, diagnostics) {
            last_arg_was_comment_only = arg.end == arg.start + 1
                && matches!(
                    tokens.get(arg.start).map(|t| &t.kind),
                    Some(TokKind::Comment)
                );
            for idx in i..arg.start {
                events.push(Event::Tok(idx));
            }
            events.extend(arg.events);
            i = arg.end;
        } else {
            last_arg_was_comment_only = false;
            events.push(Event::Tok(i));
            i += 1;
        }
        events.push(Event::Finish);
        expect_delimiter = true;

        let next_i = ctx.skip_ws_and_newlines(i);
        for idx in i..next_i {
            events.push(Event::Tok(idx));
        }
        i = next_i;
        if matches!(tokens.get(i).map(|t| &t.kind), Some(TokKind::Comma)) {
            events.push(Event::Tok(i));
            i += 1;
            expect_delimiter = false;
            last_arg_was_comment_only = false;
        }
    }

    events.push(Event::Finish); // ARG_LIST
    let next_i = ctx.skip_ws_and_newlines(i);
    for idx in i..next_i {
        events.push(Event::Tok(idx));
    }
    i = next_i;

    if is_matching_subset_close(tokens, i, node_kind) {
        match node_kind {
            SyntaxKind::SUBSET_EXPR => {
                if matches!(tokens.get(i).map(|t| &t.kind), Some(TokKind::RBrack2)) {
                    events.push(Event::Tok(i)); // consume one ] out of ]]
                    i += 1;
                } else {
                    events.push(Event::Tok(i));
                    i += 1;
                }
            }
            SyntaxKind::SUBSET2_EXPR => {
                events.push(Event::Tok(i));
                i += 1;
            }
            _ => {}
        }
    } else if node_kind == SyntaxKind::SUBSET_EXPR
        && matches!(previous_non_trivia_kind(tokens, i), Some(TokKind::RBrack2))
    {
        // `]]` may have been consumed as a single token by an inner subset node.
        // In that case, treat the outer subset close as satisfied.
    } else if let Some(tok) = tokens.get(open_idx) {
        push_token_diagnostic(
            diagnostics,
            "expected closing bracket in subset expression",
            tok,
        );
    }

    events.push(Event::Finish);
    ExprParse {
        start: target.start,
        end: i,
        events,
    }
}

fn is_matching_subset_close(tokens: &[Token], idx: usize, node_kind: SyntaxKind) -> bool {
    match node_kind {
        SyntaxKind::SUBSET_EXPR => matches!(
            tokens.get(idx).map(|t| &t.kind),
            Some(TokKind::RBrack | TokKind::RBrack2)
        ),
        SyntaxKind::SUBSET2_EXPR => {
            matches!(tokens.get(idx).map(|t| &t.kind), Some(TokKind::RBrack2))
        }
        _ => false,
    }
}

fn previous_non_trivia_kind(tokens: &[Token], mut idx: usize) -> Option<TokKind> {
    if idx == 0 {
        return None;
    }
    idx -= 1;
    loop {
        let tok = tokens.get(idx)?;
        if !matches!(tok.kind, TokKind::Whitespace | TokKind::Newline) {
            return Some(tok.kind.clone());
        }
        if idx == 0 {
            return None;
        }
        idx -= 1;
    }
}

fn infix_binding_power(kind: &TokKind) -> Option<(u8, u8)> {
    // Binding powers are aligned to AIR's operator precedence tiers:
    // LogicalOr (5), LogicalAnd (6), Relational (8), Additive (9),
    // Multiplicative (10), Special (11), Colon (12), Tilde (4), Exponential (14).
    // Help (`?`) sits below assignment as in R, so `x <- 1 ? 2` parses as
    // `(x <- 1) ? 2`. Namespace/extract operators (`::`, `:::`, `$`, `@`) bind
    // tighter than exponentiation and are treated as left-associative.
    match kind {
        TokKind::Question => Some((0, 1)),
        TokKind::Or | TokKind::Or2 => Some((50, 51)),
        TokKind::And | TokKind::And2 => Some((60, 61)),
        TokKind::Equal2
        | TokKind::NotEqual
        | TokKind::LessThan
        | TokKind::LessThanOrEqual
        | TokKind::GreaterThan
        | TokKind::GreaterThanOrEqual => Some((80, 81)),
        TokKind::Plus | TokKind::Minus => Some((90, 91)),
        TokKind::Star | TokKind::Slash => Some((100, 101)),
        TokKind::Pipe | TokKind::UserOp => Some((110, 111)),
        TokKind::Colon => Some((120, 121)),
        TokKind::Tilde => Some((40, 41)),
        TokKind::Caret => Some((140, 140)),
        TokKind::Colon2 | TokKind::Colon3 | TokKind::Dollar | TokKind::At => Some((150, 151)),
        _ => None,
    }
}
