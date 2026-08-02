use rowan::{NodeOrToken, SyntaxElement};

use super::core::FormatError;
use crate::syntax::{RLanguage, SyntaxKind};

pub(super) fn split_lines(
    elements: Vec<SyntaxElement<RLanguage>>,
    context: &'static str,
) -> Result<Vec<Vec<SyntaxElement<RLanguage>>>, FormatError> {
    let mut lines: Vec<Vec<SyntaxElement<RLanguage>>> = Vec::new();
    let mut current: Vec<SyntaxElement<RLanguage>> = Vec::new();
    let mut break_count = 0usize;

    for element in elements {
        if let NodeOrToken::Token(token) = &element {
            if token.kind() == SyntaxKind::WHITESPACE {
                continue;
            }
            if token.kind() == SyntaxKind::NEWLINE || token.kind() == SyntaxKind::SEMICOLON {
                if !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                    break_count = 1;
                } else if !lines.is_empty() {
                    break_count += 1;
                }
                continue;
            }
        }

        if break_count >= 2
            && (!matches!(lines.last(), Some(last) if is_comment_only_line(last))
                || matches!(element, NodeOrToken::Token(ref tok) if tok.kind() == SyntaxKind::COMMENT)
                || matches!(lines.last(), Some(last) if is_separator_comment_line(last))
                || should_preserve_section_heading_gap(&lines))
        {
            lines.push(Vec::new());
        }
        break_count = 0;

        if !current.is_empty() {
            if is_inline_trailing_comment(&element)
                && !current.iter().any(is_inline_trailing_comment)
            {
                current.push(element);
                continue;
            }
            return Err(FormatError::AmbiguousConstruct {
                context,
                snippet: super::render::snippet_from_elements(&[current[0].clone(), element]),
            });
        }
        current.push(element);
    }

    if !current.is_empty() {
        lines.push(current);
    }

    Ok(lines)
}

pub(super) fn is_trivia(kind: SyntaxKind) -> bool {
    matches!(kind, SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE)
}

/// An element that, when it trails a statement on the same line, is a trailing
/// comment: a `#` comment token, or a single-line `#'` `ROXYGEN_BLOCK` node.
///
/// A mid-line `#'` is only a roxygen marker to the lexer; roxygen2 treats `#'`
/// as documentation solely at line start, so trailing a statement it is a plain
/// comment (e.g. `object <- formula #'formula' because ...` in `survival`). The
/// block is attached to the statement's line here and rendered as a trailing
/// comment by [`super::core::ir_line`]. Multi-line roxygen blocks are excluded
/// so the single-line line-suffix rendering stays valid.
pub(super) fn is_inline_trailing_comment(element: &SyntaxElement<RLanguage>) -> bool {
    inline_trailing_comment_text(element).is_some()
}

/// The verbatim comment text of an [`is_inline_trailing_comment`] element, for
/// rendering as a trailing line suffix. `None` when the element is not a
/// trailing comment. A `#'` block is emitted as its own source text (not
/// reflowed): mid-line it is a comment, so it is preserved like any `#` comment.
pub(super) fn inline_trailing_comment_text(element: &SyntaxElement<RLanguage>) -> Option<String> {
    match element {
        NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT => {
            Some(tok.text().to_string())
        }
        NodeOrToken::Node(node) if node.kind() == SyntaxKind::ROXYGEN_BLOCK => {
            let text = node.text().to_string();
            (!text.contains('\n')).then_some(text)
        }
        _ => None,
    }
}

fn is_comment_only_line(line: &[SyntaxElement<RLanguage>]) -> bool {
    let significant: Vec<_> = line.iter().filter(|el| !is_trivia(el.kind())).collect();
    matches!(
        significant.as_slice(),
        [NodeOrToken::Token(tok)] if tok.kind() == SyntaxKind::COMMENT
    )
}

fn is_separator_comment_line(line: &[SyntaxElement<RLanguage>]) -> bool {
    let significant: Vec<_> = line.iter().filter(|el| !is_trivia(el.kind())).collect();
    matches!(
        significant.as_slice(),
        [NodeOrToken::Token(tok)]
            if tok.kind() == SyntaxKind::COMMENT
                && tok.text().trim() == "# ------------------------------------------------------------------------"
    )
}

fn should_preserve_section_heading_gap(lines: &[Vec<SyntaxElement<RLanguage>>]) -> bool {
    let Some(last) = lines.last() else {
        return false;
    };
    if !is_section_heading_comment_line(last) {
        return false;
    }
    lines
        .iter()
        .rev()
        .nth(1)
        .is_some_and(|line| is_separator_comment_line(line))
}

fn is_section_heading_comment_line(line: &[SyntaxElement<RLanguage>]) -> bool {
    let significant: Vec<_> = line.iter().filter(|el| !is_trivia(el.kind())).collect();
    matches!(
        significant.as_slice(),
        [NodeOrToken::Token(tok)]
            if tok.kind() == SyntaxKind::COMMENT
                && matches!(tok.text().trim(), "# Dots" | "# Dot dot i")
    )
}
