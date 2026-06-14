use crate::parser::lexer::{TokKind, Token};

pub(crate) fn skip_ws(tokens: &[Token], mut i: usize) -> usize {
    while matches!(tokens.get(i).map(|t| &t.kind), Some(TokKind::Whitespace)) {
        i += 1;
    }
    i
}

pub(crate) fn skip_ws_and_newlines(tokens: &[Token], mut i: usize) -> usize {
    while matches!(
        tokens.get(i).map(|t| &t.kind),
        Some(TokKind::Whitespace | TokKind::Newline)
    ) {
        i += 1;
    }
    i
}

/// Like [`skip_ws_and_newlines`] but also skips comments. Used when an operand
/// is pending (after an infix operator), where an intervening comment is trivia
/// before the operand rather than an operand in its own right.
pub(crate) fn skip_ws_newlines_comments(tokens: &[Token], mut i: usize) -> usize {
    while matches!(
        tokens.get(i).map(|t| &t.kind),
        Some(TokKind::Whitespace | TokKind::Newline | TokKind::Comment)
    ) {
        i += 1;
    }
    i
}

pub(crate) fn consume_to_line_end(tokens: &[Token], mut i: usize) -> usize {
    while i < tokens.len() && !matches!(tokens[i].kind, TokKind::Newline) {
        i += 1;
    }
    if i < tokens.len() && matches!(tokens[i].kind, TokKind::Newline) {
        i += 1;
    }
    i
}

pub(crate) fn find_function_body_recovery(tokens: &[Token], start: usize) -> usize {
    for (i, tok) in tokens.iter().enumerate().skip(start) {
        if matches!(tok.kind, TokKind::Newline) {
            return i;
        }
    }
    tokens.len()
}
