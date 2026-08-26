use rowan::{NodeOrToken, SyntaxElement};

use super::core::FormatError;
use super::ir::Ir;
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

        // A blank-line gap is preserved as a single empty line, whatever sits on
        // either side of it. A comment is not implicitly attached to whatever
        // follows: a license header, a section divider, and a `#'` block that
        // opens a new documentation unit all need the author's gap to survive.
        if break_count >= 2 {
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

/// Whether `text` is a Quarto code annotation such as `# <1>`.
///
/// Quarto attaches the annotation to its physical source line, so formatter
/// rules that normally relocate a dangling comment must leave this narrow,
/// documented form on the line where it appeared.
pub(super) fn is_quarto_code_annotation(text: &str) -> bool {
    let Some(annotation) = text.trim_end().strip_prefix("# <") else {
        return false;
    };
    let Some(number) = annotation.strip_suffix('>') else {
        return false;
    };
    !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
}

/// Render a structured trailing comment, retaining whether it is a Quarto
/// annotation so the printer can apply the annotation-specific gutter rule.
pub(super) fn ir_inline_trailing_comment(text: &str) -> Ir {
    let suffix = format!(" {text}");
    if is_quarto_code_annotation(text) {
        Ir::quarto_annotation_suffix(suffix)
    } else {
        Ir::line_suffix(suffix)
    }
}
