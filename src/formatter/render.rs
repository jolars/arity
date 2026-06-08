use rowan::{NodeOrToken, SyntaxElement, SyntaxToken};

use super::context::FormatContext;
use super::core::FormatError;
use super::ir::Ir;
use super::trivia::{is_trivia, split_lines};

use crate::syntax::{RLanguage, SyntaxKind, SyntaxNode};

type FormatLineFn =
    fn(&[SyntaxElement<RLanguage>], usize, FormatContext) -> Result<String, FormatError>;
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

/// IR counterpart of [`format_block_expr_with_prefixed_comments`]. The body is
/// always multi-line: each statement (and any leading prefixed comment) sits on
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
    let lines = split_lines(block_statement_elements(node)?, "block body")?;

    let mut items: Vec<Ir> = Vec::new();
    for comment in prefixed_comments {
        items.push(Ir::text(comment.clone()));
    }
    for line in &lines {
        items.push(ir_line(line, indent + 1, ctx)?);
    }
    if items.is_empty() {
        return Ok(Ir::text("{}"));
    }

    let body = Ir::concat(
        items
            .into_iter()
            .map(|it| Ir::concat([Ir::hard_line(), it])),
    );
    Ok(Ir::concat([
        Ir::text("{"),
        Ir::indent(body),
        Ir::hard_line(),
        Ir::text("}"),
    ]))
}

pub(super) fn format_block_expr_with_prefixed_comments(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
    prefixed_comments: &[String],
    format_line: FormatLineFn,
) -> Result<String, FormatError> {
    let lines = split_lines(block_statement_elements(node)?, "block body")?;
    if lines.is_empty() && prefixed_comments.is_empty() {
        return Ok("{}".to_string());
    }

    let mut out = String::from("{\n");
    let mut emitted_any = false;
    for comment in prefixed_comments {
        if emitted_any {
            out.push('\n');
        }
        out.push_str(&ctx.indent_text(indent + 1));
        out.push_str(comment);
        emitted_any = true;
    }
    for line in &lines {
        if emitted_any {
            out.push('\n');
        }
        out.push_str(&format_line(line, indent + 1, ctx)?);
        emitted_any = true;
    }
    out.push('\n');
    out.push_str(&ctx.indent_text(indent));
    out.push('}');
    Ok(out)
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

pub(super) fn format_expr_with_optional_comment(
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

    if significant.len() == 2
        && matches!(
            significant.last(),
            Some(NodeOrToken::Token(token)) if token.kind() == SyntaxKind::COMMENT
        )
    {
        let expr = format_expr_element(&significant[0], indent, ctx)?;
        let comment = match &significant[1] {
            NodeOrToken::Token(token) => token.text(),
            NodeOrToken::Node(_) => unreachable!(),
        };
        return Ok(format!("{expr} {comment}"));
    }

    format_expr_segment(elements, context, indent, ctx, format_expr_element)
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
