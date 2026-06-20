use super::*;

/// The foldable regions of `text`: brace blocks (function/`if`/`for`/`while`
/// bodies and bare `{}`), multi-line argument/parameter and subscript lists,
/// parenthesized expressions, and runs of standalone comment lines. A pure CST
/// walk with no semantic model — single-file, never consults the workspace.
///
/// Each bracketed construct folds from the line of its opening delimiter to the
/// line of its matching closing delimiter; only multi-line spans are emitted, so
/// single-line constructs never fold. Comment runs (two or more standalone
/// comment lines on consecutive lines) fold under [`FoldingRangeKind::Comment`].
pub fn compute_folding_ranges(text: &str) -> Vec<FoldingRange> {
    let root = parse(text).cst;
    let line_index = LineIndex::new(text);
    let line_of = |offset: TextSize| line_index.byte_to_position(u32::from(offset) as usize).line;
    let mut ranges = Vec::new();

    // Bracketed constructs: for each node carrying a delimiter pair, fold from the
    // opening token to the matching closing token. The pair is unique to the node
    // (rowan nesting), so this never double-counts.
    for node in root.descendants() {
        let Some((open, close)) = delimiter_pair(node.kind()) else {
            continue;
        };
        let tokens = || node.children_with_tokens().filter_map(|el| el.into_token());
        let Some(open_tok) = tokens().find(|t| t.kind() == open) else {
            continue;
        };
        let Some(close_tok) = tokens().filter(|t| t.kind() == close).last() else {
            continue;
        };
        let start_line = line_of(open_tok.text_range().start());
        let end_line = line_of(close_tok.text_range().end());
        if end_line > start_line {
            ranges.push(FoldingRange {
                start_line,
                end_line,
                ..Default::default()
            });
        }
    }

    // Comment runs: standalone comment lines (a comment that is the only non-trivia
    // token on its line; trailing comments are excluded) grouped into maximal runs
    // of consecutive lines. A run of two or more lines folds.
    let mut standalone: Vec<u32> = Vec::new();
    let mut code_on_line = false;
    for token in root
        .descendants_with_tokens()
        .filter_map(|el| el.into_token())
    {
        match token.kind() {
            SyntaxKind::NEWLINE => code_on_line = false,
            SyntaxKind::WHITESPACE => {}
            // A roxygen line's leading `#'` marker stands in for a comment here,
            // so runs of `#'` lines fold as comment runs like they did when
            // roxygen lines were plain `COMMENT` tokens. Other roxygen
            // sub-tokens on the line are immaterial: the marker already fired.
            SyntaxKind::COMMENT | SyntaxKind::ROXYGEN_MARKER => {
                if !code_on_line {
                    standalone.push(line_of(token.text_range().start()));
                }
            }
            kind if kind.is_roxygen_token() => {}
            _ => code_on_line = true,
        }
    }
    let mut i = 0;
    while i < standalone.len() {
        let mut j = i + 1;
        while j < standalone.len() && standalone[j] == standalone[j - 1] + 1 {
            j += 1;
        }
        if j - i >= 2 {
            ranges.push(FoldingRange {
                start_line: standalone[i],
                end_line: standalone[j - 1],
                kind: Some(FoldingRangeKind::Comment),
                ..Default::default()
            });
        }
        i = j;
    }

    ranges
}

/// The opening/closing delimiter token kinds for a foldable bracketed node, or
/// `None` if the node is not delimiter-bounded. `FUNCTION_EXPR` folds on its
/// parameter list `()`; the body block folds separately via its `BLOCK_EXPR`.
pub(crate) fn delimiter_pair(kind: SyntaxKind) -> Option<(SyntaxKind, SyntaxKind)> {
    Some(match kind {
        SyntaxKind::BLOCK_EXPR => (SyntaxKind::LBRACE, SyntaxKind::RBRACE),
        SyntaxKind::PAREN_EXPR | SyntaxKind::FUNCTION_EXPR | SyntaxKind::CALL_EXPR => {
            (SyntaxKind::LPAREN, SyntaxKind::RPAREN)
        }
        SyntaxKind::SUBSET_EXPR => (SyntaxKind::LBRACK, SyntaxKind::RBRACK),
        SyntaxKind::SUBSET2_EXPR => (SyntaxKind::LBRACK2, SyntaxKind::RBRACK2),
        _ => return None,
    })
}
