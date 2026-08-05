use crate::parser::lexer::{TokKind, Token};

/// Re-tokenize runs of `]` characters so each close matches the corresponding
/// open on the bracket stack.
///
/// The lexer greedily merges `]]` into a single `RBrack2` token, but `]]` can
/// be either one `]]` closing `[[` or two `]` closing two adjacent `[`s
/// (e.g. `df[df$col > 7, map[names(df)]]`). This pass walks the token stream,
/// tracks open `[` / `[[`, collects each maximal run of adjacent close-bracket
/// tokens, and re-emits them as `RBrack` / `RBrack2` according to what the
/// stack says should close next.
pub(crate) fn rebalance_brackets(tokens: Vec<Token>) -> Vec<Token> {
    if !needs_rebalance(&tokens) {
        return tokens;
    }
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut stack: Vec<TokKind> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].kind {
            TokKind::LBrack | TokKind::LBrack2 => {
                stack.push(tokens[i].kind.clone());
                out.push(tokens[i].clone());
                i += 1;
            }
            TokKind::RBrack | TokKind::RBrack2 => {
                let run_start = tokens[i].start;
                let mut run_end = tokens[i].end;
                let mut j = i + 1;
                while j < tokens.len()
                    && matches!(tokens[j].kind, TokKind::RBrack | TokKind::RBrack2)
                    && tokens[j].start == run_end
                {
                    run_end = tokens[j].end;
                    j += 1;
                }
                emit_close_run(run_start, run_end, &mut stack, &mut out);
                i = j;
            }
            _ => {
                out.push(tokens[i].clone());
                i += 1;
            }
        }
    }
    out
}

fn needs_rebalance(tokens: &[Token]) -> bool {
    // The only ambiguity comes from `]]` potentially closing two `[`s, so
    // rebalancing is a no-op unless both an `[` and a `]]` are present.
    let mut has_rb2 = false;
    let mut has_lb1 = false;
    for tok in tokens {
        match tok.kind {
            TokKind::LBrack => has_lb1 = true,
            TokKind::RBrack2 => has_rb2 = true,
            _ => {}
        }
        if has_rb2 && has_lb1 {
            return true;
        }
    }
    false
}

fn emit_close_run(
    run_start: usize,
    run_end: usize,
    stack: &mut Vec<TokKind>,
    out: &mut Vec<Token>,
) {
    let mut pos = run_start;
    while pos < run_end {
        match stack.last() {
            Some(TokKind::LBrack2) if pos + 1 < run_end => {
                out.push(Token {
                    kind: TokKind::RBrack2,
                    text: "]]".to_string(),
                    start: pos,
                    end: pos + 2,
                });
                stack.pop();
                pos += 2;
            }
            Some(TokKind::LBrack) => {
                out.push(Token {
                    kind: TokKind::RBrack,
                    text: "]".to_string(),
                    start: pos,
                    end: pos + 1,
                });
                stack.pop();
                pos += 1;
            }
            // Stack mismatch or `[[` with only one `]` left: emit a stray `]`
            // and let the parser surface a diagnostic.
            _ => {
                out.push(Token {
                    kind: TokKind::RBrack,
                    text: "]".to_string(),
                    start: pos,
                    end: pos + 1,
                });
                pos += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::lexer::lex;

    fn kinds(tokens: &[Token]) -> Vec<TokKind> {
        tokens.iter().map(|t| t.kind.clone()).collect()
    }

    fn text_total(tokens: &[Token]) -> String {
        tokens.iter().map(|t| t.text.clone()).collect()
    }

    #[test]
    fn splits_double_close_when_inner_is_single_bracket() {
        let input = "df[, map[x]]";
        let toks = rebalance_brackets(lex(input));
        assert_eq!(text_total(&toks), input);
        let closes: Vec<_> = toks
            .iter()
            .filter(|t| matches!(t.kind, TokKind::RBrack | TokKind::RBrack2))
            .map(|t| t.kind.clone())
            .collect();
        assert_eq!(closes, vec![TokKind::RBrack, TokKind::RBrack]);
    }

    #[test]
    fn preserves_double_close_when_inner_is_double_bracket() {
        let input = "x[[1]]";
        let toks = rebalance_brackets(lex(input));
        assert_eq!(text_total(&toks), input);
        assert!(toks.iter().any(|t| t.kind == TokKind::RBrack2));
        assert!(!toks.iter().any(|t| t.kind == TokKind::RBrack));
    }

    #[test]
    fn merges_then_splits_for_outer_double_inner_single() {
        // f[[g[h]]] => `]]` (close g[ then start closing f[[) needs `]` then `]]`
        let input = "f[[g[h]]]";
        let toks = rebalance_brackets(lex(input));
        assert_eq!(text_total(&toks), input);
        let closes: Vec<_> = toks
            .iter()
            .filter(|t| matches!(t.kind, TokKind::RBrack | TokKind::RBrack2))
            .map(|t| t.kind.clone())
            .collect();
        assert_eq!(closes, vec![TokKind::RBrack, TokKind::RBrack2]);
    }

    #[test]
    fn pass_is_idempotent() {
        let input = "df[df$col > 7, map[names(df)]]";
        let once = rebalance_brackets(lex(input));
        let twice = rebalance_brackets(once.clone());
        assert_eq!(kinds(&once), kinds(&twice));
    }

    #[test]
    fn leaves_input_untouched_without_double_close() {
        let input = "x[1] + y[2]";
        let original = lex(input);
        let rebalanced = rebalance_brackets(original.clone());
        assert_eq!(kinds(&original), kinds(&rebalanced));
    }
}
