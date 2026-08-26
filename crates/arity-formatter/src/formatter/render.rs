use rowan::{NodeOrToken, SyntaxElement, SyntaxToken};

use super::context::FormatContext;
use super::core::FormatError;
use super::ir::Ir;
use super::trivia::{
    ir_inline_trailing_comment, is_quarto_code_annotation, is_trivia, split_lines,
};

use crate::syntax::{RLanguage, SyntaxKind, SyntaxNode};

type FormatExprElementFn =
    fn(&SyntaxElement<RLanguage>, usize, FormatContext) -> Result<String, FormatError>;
type IrLineFn = fn(&[SyntaxElement<RLanguage>], usize, FormatContext) -> Result<Ir, FormatError>;

/// Extract the elements of a block's body, i.e. everything between the first
/// `{` and the last `}`. Shared by the block formatters and by range formatting,
/// which formats a window of a block's statements without the braces.
pub(super) fn block_statement_elements(
    node: &SyntaxNode,
) -> Result<Vec<SyntaxElement<RLanguage>>, FormatError> {
    let elements: Vec<_> = node.children_with_tokens().collect();
    let open_idx = elements
        .iter()
        .position(|el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::LBRACE))
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing '{' in block",
            snippet: node.text().to_string(),
        })?;
    let close_idx = elements
        .iter()
        .rposition(|el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::RBRACE))
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing '}' in block",
            snippet: node.text().to_string(),
        })?;
    if close_idx <= open_idx {
        return Err(FormatError::AmbiguousConstruct {
            context: "invalid block bounds",
            snippet: node.text().to_string(),
        });
    }

    Ok(elements[open_idx + 1..close_idx].to_vec())
}

/// Build a block expression as IR, optionally prefixing leading comments inside
/// the braces. The body is always multi-line: each statement (and any leading
/// prefixed comment) sits on
/// its own indented line via hard breaks, with the closing brace dedented to the
/// block's own indent. An empty block with no prefixed comments collapses to
/// `{}`.
pub(super) fn ir_block_expr_with_prefixed_comments(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
    prefixed_comments: &[String],
    ir_line: IrLineFn,
) -> Result<Ir, FormatError> {
    let mut body_elements = block_statement_elements(node)?;
    let opening_annotation_idx = body_elements
        .iter()
        .take_while(|element| element.kind() != SyntaxKind::NEWLINE)
        .position(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::COMMENT => {
                is_quarto_code_annotation(token.text())
            }
            _ => false,
        })
        .filter(|idx| {
            body_elements[..*idx]
                .iter()
                .all(|element| element.kind() == SyntaxKind::WHITESPACE)
        });
    let opening_annotation =
        opening_annotation_idx.and_then(|idx| match body_elements.remove(idx) {
            NodeOrToken::Token(token) => Some(token.text().to_string()),
            NodeOrToken::Node(_) => None,
        });
    let lines = split_lines(body_elements, "block body")?;

    let plan = super::directive::plan(&lines, ctx.ignored_directive());
    let mut items: Vec<Ir> = Vec::new();
    for comment in prefixed_comments {
        items.push(Ir::text(comment.clone()));
    }
    let mut idx = 0usize;
    while idx < lines.len() {
        // A line the author marked `# arity-format skip`/`off` comes back
        // exactly as written, indent included.
        match super::directive::skipped_at(&lines, &plan, idx) {
            Some((skipped, last)) => {
                items.push(skipped);
                idx = last + 1;
            }
            None => {
                items.push(ir_line(&lines[idx], indent + 1, ctx)?);
                idx += 1;
            }
        }
    }
    if items.is_empty() && opening_annotation.is_none() {
        return Ok(Ir::text("{}"));
    }

    let open = match opening_annotation {
        Some(annotation) => Ir::concat([Ir::text("{"), ir_inline_trailing_comment(&annotation)]),
        None => Ir::text("{"),
    };

    let body = Ir::concat(
        items
            .into_iter()
            .map(|it| Ir::concat([Ir::hard_line(), it])),
    );
    Ok(Ir::concat([
        open,
        Ir::indent(Ir::break_body(body)),
        Ir::hard_line(),
        Ir::text("}"),
    ]))
}

pub(super) fn format_expr_segment(
    elements: &[SyntaxElement<RLanguage>],
    context: &'static str,
    indent: usize,
    ctx: FormatContext,
    format_expr_element: FormatExprElementFn,
) -> Result<String, FormatError> {
    let significant: Vec<_> = elements
        .iter()
        .filter(|el| !is_trivia(el.kind()))
        .cloned()
        .collect();
    if significant.len() != 1 {
        return Err(FormatError::AmbiguousConstruct {
            context,
            snippet: snippet_from_elements(elements),
        });
    }
    format_expr_element(&significant[0], indent, ctx)
}

pub(super) fn format_atom_token(token: &SyntaxToken<RLanguage>) -> Result<String, FormatError> {
    match token.kind() {
        SyntaxKind::IDENT
        | SyntaxKind::INT
        | SyntaxKind::FLOAT
        | SyntaxKind::COMPLEX
        | SyntaxKind::STRING
        | SyntaxKind::BANG => Ok(token.text().to_string()),
        kind => Err(FormatError::UnsupportedConstruct {
            kind,
            snippet: token.text().to_string(),
        }),
    }
}

pub(super) fn snippet_from_elements(elements: &[SyntaxElement<RLanguage>]) -> String {
    elements
        .iter()
        .map(|el| match el {
            NodeOrToken::Node(node) => node.text().to_string(),
            NodeOrToken::Token(tok) => tok.text().to_string(),
        })
        .collect::<String>()
}

/// Reconstruct a snippet for *reparsing* a flat token run, separating elements
/// with single spaces so adjacent tokens never merge. Plain concatenation (see
/// [`snippet_from_elements`]) is wrong here because callers pass trivia-stripped
/// runs: `1 else 2` would collapse to `1else2`, which re-lexes as `1` followed
/// by the identifier `else2`. Spaces can only ever separate tokens, never fuse
/// them, and the reparsed node is consumed structurally (the formatter re-emits
/// its own spacing), so the inserted spaces have no effect on the output.
pub(super) fn reparse_snippet_from_elements(elements: &[SyntaxElement<RLanguage>]) -> String {
    elements
        .iter()
        .map(|el| match el {
            NodeOrToken::Node(node) => node.text().to_string(),
            NodeOrToken::Token(tok) => tok.text().to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}
