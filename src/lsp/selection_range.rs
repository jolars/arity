use super::*;
use crate::syntax::SyntaxToken;

/// The "smart selection" chains (`textDocument/selectionRange`) for each cursor
/// `position`: from the token under the cursor outward through its enclosing CST
/// nodes to the whole file. A pure single-file CST walk with no semantic model —
/// never consults the workspace.
///
/// The response is index-aligned with `positions`: one [`SelectionRange`] chain
/// per position, its innermost range at the top level and each `parent` pointing
/// one step further out.
pub fn compute_selection_ranges(
    text: &str,
    positions: &[Position],
    encoding: PositionEncoding,
) -> Vec<SelectionRange> {
    compute_selection_ranges_in(&TextBuffer::from(text), positions, encoding)
}

/// [`compute_selection_ranges`] against a live buffer, reusing its maintained
/// line index instead of rebuilding one per request.
pub(crate) fn compute_selection_ranges_in(
    buffer: &TextBuffer,
    positions: &[Position],
    encoding: PositionEncoding,
) -> Vec<SelectionRange> {
    let text = buffer.text();
    let root = parse(text).cst;
    let line_index = buffer.line_index();
    positions
        .iter()
        .map(|&position| {
            let offset = TextSize::new(
                line_index
                    .position_to_byte(position, encoding)
                    .min(text.len()) as u32,
            );
            selection_range_at(&root, line_index, offset, encoding)
        })
        .collect()
}

/// Build one selection-range chain for `offset`: seed with the token under the
/// cursor (unless it is pure layout trivia) and expand through its ancestors,
/// collapsing equal-range wrapper nodes so no level equals its parent.
fn selection_range_at(
    root: &SyntaxNode,
    line_index: &LineIndex,
    offset: TextSize,
    encoding: PositionEncoding,
) -> SelectionRange {
    let mut ranges: Vec<TextRange> = Vec::new();

    if let Some(token) = token_at(root, offset) {
        // A cursor sitting on whitespace/newline expands from the enclosing node
        // rather than selecting the (invisible) layout token itself. Comments are
        // real content, so they stay as the innermost selection.
        if !matches!(token.kind(), SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE) {
            ranges.push(token.text_range());
        }
        for ancestor in token.parent_ancestors() {
            let range = ancestor.text_range();
            if ranges.last() != Some(&range) {
                ranges.push(range);
            }
        }
    }

    // Empty document, or a position with no token (e.g. past the last line): fall
    // back to a single whole-file range so every position still yields a chain.
    if ranges.is_empty() {
        ranges.push(root.text_range());
    }

    // Fold outermost -> innermost so each level's `parent` is one step further out.
    let mut selection: Option<SelectionRange> = None;
    for range in ranges.iter().rev() {
        selection = Some(SelectionRange {
            range: text_range_to_lsp_range(line_index, *range, encoding),
            parent: selection.map(Box::new),
        });
    }
    selection.expect("ranges is non-empty")
}

/// The token at `offset`, preferring a non-trivia side when the offset falls on a
/// token boundary so the cursor between a name and surrounding whitespace still
/// selects the name.
fn token_at(root: &SyntaxNode, offset: TextSize) -> Option<SyntaxToken> {
    let is_trivia = |kind| matches!(kind, SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE);
    match root.token_at_offset(offset) {
        TokenAtOffset::None => None,
        TokenAtOffset::Single(token) => Some(token),
        TokenAtOffset::Between(left, right) => {
            if !is_trivia(right.kind()) {
                Some(right)
            } else if !is_trivia(left.kind()) {
                Some(left)
            } else {
                Some(right)
            }
        }
    }
}
