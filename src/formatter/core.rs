use rowan::{NodeOrToken, SyntaxElement};

use super::context::FormatContext;
use super::ir::Ir;
use super::printer::Printer;
use super::render::{format_atom_token, format_block_expr_with_prefixed_comments as render_block};
use super::rules::control_flow::{
    ir_for_expr, ir_if_expr, ir_repeat_expr, ir_while_expr, should_insert_comment_for_gap,
    try_format_for_with_external_body, try_format_if_with_external_body,
    try_format_repeat_with_external_body, try_format_while_with_external_body,
};
use super::rules::expressions::{
    ir_assignment_expr, ir_binary_expr, ir_paren_expr, ir_subset_expr, ir_unary_expr,
};
use super::rules::functions::{ir_call_expr, ir_function_expr};
use super::style::FormatStyle;
use super::trivia::{is_trivia as is_trivia_kind, split_lines};
use crate::ast::{
    AssignmentExpr, AstNode, BinaryExpr, BlockExpr, CallExpr, ForExpr, FunctionExpr, IfExpr,
    ParenExpr, UnaryExpr, WhileExpr,
};
use crate::parser::parse;
use crate::syntax::{RLanguage, SyntaxKind, SyntaxNode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    ParseErrors {
        count: usize,
    },
    UnsupportedConstruct {
        kind: SyntaxKind,
        snippet: String,
    },
    AmbiguousConstruct {
        context: &'static str,
        snippet: String,
    },
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ParseErrors { count } => write!(
                f,
                "input contains {count} parser diagnostic(s); formatter only supports parseable input"
            ),
            Self::UnsupportedConstruct { kind, snippet } => {
                write!(
                    f,
                    "unsupported construct for formatter: {kind:?} near {snippet:?}"
                )
            }
            Self::AmbiguousConstruct { context, snippet } => {
                write!(
                    f,
                    "ambiguous construct for formatter ({context}): {snippet:?}"
                )
            }
        }
    }
}

impl std::error::Error for FormatError {}

pub fn format(input: &str) -> Result<String, FormatError> {
    format_with_style(input, FormatStyle::default())
}

pub fn format_with_style(input: &str, style: FormatStyle) -> Result<String, FormatError> {
    let parse_output = parse(input);
    if !parse_output.diagnostics.is_empty() {
        return Err(FormatError::ParseErrors {
            count: parse_output.diagnostics.len(),
        });
    }

    format_node(&parse_output.cst, style, input.ends_with('\n'))
}

/// Format an already-parsed CST. The caller is responsible for rejecting input
/// that failed to parse (the diagnostics live next to the green tree in the
/// salsa cache, not on the node); this entry only guards against stray `ERROR`
/// tokens. `trailing_newline` preserves a final newline the source had.
///
/// Used by the language server's read path, which formats off the cached parse
/// tree in its salsa database rather than re-parsing the buffer.
pub fn format_node(
    root: &SyntaxNode,
    style: FormatStyle,
    trailing_newline: bool,
) -> Result<String, FormatError> {
    validate_supported_tokens(root)?;
    let ctx = FormatContext::new(style);
    let mut formatted = format_root(root, ctx)?;
    if trailing_newline && !formatted.ends_with('\n') {
        formatted.push('\n');
    }
    Ok(formatted)
}

fn validate_supported_tokens(root: &SyntaxNode) -> Result<(), FormatError> {
    for element in root.descendants_with_tokens() {
        let Some(token) = element.into_token() else {
            continue;
        };
        let kind = token.kind();
        if matches!(kind, SyntaxKind::ERROR) {
            return Err(FormatError::UnsupportedConstruct {
                kind,
                snippet: token.text().to_string(),
            });
        }
    }
    Ok(())
}

fn format_root(root: &SyntaxNode, ctx: FormatContext) -> Result<String, FormatError> {
    let ir = ir_root(root, ctx)?;
    Ok(Printer::new(ctx.style()).print(&ir))
}

/// IR builder for the whole document. Mirrors [`legacy_format_root`]: statements
/// are separated by hard breaks (a blank line where a gap should be preserved),
/// control-flow forms whose body is on a following line are rendered via the
/// (bridged) external-body handlers, and everything else is an [`ir_line`].
fn ir_root(root: &SyntaxNode, ctx: FormatContext) -> Result<Ir, FormatError> {
    let lines = split_lines(root.children_with_tokens().collect(), "root")?;
    if lines.is_empty() {
        return Ok(Ir::nil());
    }

    let mut items: Vec<Ir> = Vec::new();
    let mut idx = 0usize;
    let mut first = true;
    while idx < lines.len() {
        if !first {
            if should_insert_comment_for_gap(&lines, idx, 0, ctx)? {
                items.push(Ir::empty_line());
            } else {
                items.push(Ir::hard_line());
            }
        }
        first = false;

        let consumed = if let Some((formatted, consumed)) =
            try_format_for_with_external_body(&lines, idx, 0, ctx)?
        {
            items.push(Ir::verbatim(formatted));
            consumed
        } else if let Some((formatted, consumed)) =
            try_format_while_with_external_body(&lines, idx, 0, ctx)?
        {
            items.push(Ir::verbatim(formatted));
            consumed
        } else if let Some((formatted, consumed)) =
            try_format_if_with_external_body(&lines, idx, 0, ctx)?
        {
            items.push(Ir::verbatim(formatted));
            consumed
        } else if let Some((formatted, consumed)) =
            try_format_repeat_with_external_body(&lines, idx, 0, ctx)?
        {
            items.push(Ir::verbatim(formatted));
            consumed
        } else {
            items.push(ir_line(&lines[idx], 0, ctx)?);
            0
        };
        idx += consumed + 1;
    }
    Ok(Ir::concat(items))
}

pub(super) fn format_line(
    line: &[SyntaxElement<RLanguage>],
    indent: usize,
    ctx: FormatContext,
) -> Result<String, FormatError> {
    let significant: Vec<_> = line
        .iter()
        .filter(|el| !is_trivia_kind(el.kind()))
        .cloned()
        .collect();
    if significant.is_empty() {
        return Ok(String::new());
    }

    if let [NodeOrToken::Token(token)] = significant.as_slice()
        && token.kind() == SyntaxKind::COMMENT
    {
        return Ok(format!("{}{}", ctx.indent_text(indent), token.text()));
    }

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
        return Ok(format!("{}{} {}", ctx.indent_text(indent), expr, comment));
    }

    let expr = format_expr_segment(&significant, "line expression", indent, ctx)?;
    Ok(format!("{}{}", ctx.indent_text(indent), expr))
}

pub(super) fn format_expr_segment(
    elements: &[SyntaxElement<RLanguage>],
    context: &'static str,
    indent: usize,
    ctx: FormatContext,
) -> Result<String, FormatError> {
    super::render::format_expr_segment(elements, context, indent, ctx, format_expr_element)
}

/// IR counterpart of [`format_line`]: a single statement line as IR, without the
/// leading indentation (the caller supplies that structurally via [`Ir::Indent`]
/// and line breaks). An empty (blank) line yields [`Ir::Nil`].
pub(super) fn ir_line(
    line: &[SyntaxElement<RLanguage>],
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    let significant: Vec<_> = line
        .iter()
        .filter(|el| !is_trivia_kind(el.kind()))
        .cloned()
        .collect();
    if significant.is_empty() {
        return Ok(Ir::nil());
    }

    if let [NodeOrToken::Token(token)] = significant.as_slice()
        && token.kind() == SyntaxKind::COMMENT
    {
        return Ok(Ir::text(token.text().to_string()));
    }

    if significant.len() == 2
        && matches!(
            significant.last(),
            Some(NodeOrToken::Token(token)) if token.kind() == SyntaxKind::COMMENT
        )
    {
        let expr = ir_expr_element(&significant[0], indent, ctx)?;
        let comment = match &significant[1] {
            NodeOrToken::Token(token) => token.text().to_string(),
            NodeOrToken::Node(_) => unreachable!(),
        };
        return Ok(Ir::concat([expr, Ir::text(" "), Ir::text(comment)]));
    }

    ir_expr_segment(&significant, "line expression", indent, ctx)
}

pub(super) fn format_expr_element(
    element: &SyntaxElement<RLanguage>,
    indent: usize,
    ctx: FormatContext,
) -> Result<String, FormatError> {
    let ir = ir_expr_element(element, indent, ctx)?;
    Ok(Printer::new(ctx.style()).print_at(&ir, indent))
}

/// IR dispatch for an element. Migrated constructs build real IR; the rest fall
/// back to the legacy string formatter wrapped as a `Verbatim` node (Bridge A).
pub(super) fn ir_expr_element(
    element: &SyntaxElement<RLanguage>,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    match element {
        NodeOrToken::Node(node) => ir_expr_node(node, indent, ctx),
        NodeOrToken::Token(token) => ir_atom_token(token),
    }
}

fn ir_expr_node(node: &SyntaxNode, indent: usize, ctx: FormatContext) -> Result<Ir, FormatError> {
    if let Some(expr) = AssignmentExpr::cast(node.clone()) {
        return ir_assignment_expr(expr.syntax(), indent, ctx);
    }
    if let Some(expr) = UnaryExpr::cast(node.clone()) {
        return ir_unary_expr(expr.syntax(), indent, ctx);
    }
    if let Some(expr) = BinaryExpr::cast(node.clone()) {
        return ir_binary_expr(expr.syntax(), indent, ctx);
    }
    if let Some(expr) = ParenExpr::cast(node.clone()) {
        return ir_paren_expr(expr.syntax(), indent, ctx);
    }
    if let Some(expr) = BlockExpr::cast(node.clone()) {
        return ir_block_expr(expr.syntax(), indent, ctx);
    }
    if let Some(expr) = ForExpr::cast(node.clone()) {
        return ir_for_expr(expr.syntax(), indent, ctx);
    }
    if let Some(expr) = WhileExpr::cast(node.clone()) {
        return ir_while_expr(expr.syntax(), indent, ctx);
    }
    if node.kind() == SyntaxKind::REPEAT_EXPR {
        return ir_repeat_expr(node, indent, ctx);
    }
    if let Some(expr) = IfExpr::cast(node.clone()) {
        return ir_if_expr(expr.syntax(), indent, ctx);
    }
    // Subset, call, and function arg lists are rendered natively on the IR.
    // Calls and function definitions fall back to the legacy string renderer
    // (inside `ir_call_expr` / `ir_function_expr`) for the cases that involve
    // comment relocation not yet ported to the IR.
    if matches!(
        node.kind(),
        SyntaxKind::SUBSET_EXPR | SyntaxKind::SUBSET2_EXPR
    ) {
        return ir_subset_expr(node, indent, ctx);
    }
    if let Some(expr) = CallExpr::cast(node.clone()) {
        return ir_call_expr(expr.syntax(), indent, ctx);
    }
    if let Some(expr) = FunctionExpr::cast(node.clone()) {
        return ir_function_expr(expr.syntax(), indent, ctx);
    }

    Err(FormatError::UnsupportedConstruct {
        kind: node.kind(),
        snippet: node.text().to_string(),
    })
}

/// Atom tokens (identifiers, literals, `!`) become plain text. Reuses the legacy
/// token validation so unsupported tokens keep raising `UnsupportedConstruct`.
fn ir_atom_token(token: &rowan::SyntaxToken<RLanguage>) -> Result<Ir, FormatError> {
    Ok(Ir::text(format_atom_token(token)?))
}

/// IR counterpart of [`format_expr_segment`]: a run of elements that must reduce
/// to exactly one significant expression.
pub(super) fn ir_expr_segment(
    elements: &[SyntaxElement<RLanguage>],
    context: &'static str,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    let significant: Vec<_> = elements
        .iter()
        .filter(|el| !is_trivia_kind(el.kind()))
        .cloned()
        .collect();
    if significant.len() != 1 {
        return Err(FormatError::AmbiguousConstruct {
            context,
            snippet: snippet_from_elements(elements),
        });
    }
    ir_expr_element(&significant[0], indent, ctx)
}

/// IR counterpart of [`format_expr_with_optional_comment`]: a single expression
/// optionally followed by a trailing comment on the same line.
pub(super) fn ir_expr_with_optional_comment(
    elements: &[SyntaxElement<RLanguage>],
    context: &'static str,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    let significant: Vec<_> = elements
        .iter()
        .filter(|el| !is_trivia_kind(el.kind()))
        .cloned()
        .collect();

    if significant.len() == 2
        && matches!(
            significant.last(),
            Some(NodeOrToken::Token(token)) if token.kind() == SyntaxKind::COMMENT
        )
    {
        let expr = ir_expr_element(&significant[0], indent, ctx)?;
        let comment = match &significant[1] {
            NodeOrToken::Token(token) => token.text().to_string(),
            NodeOrToken::Node(_) => unreachable!(),
        };
        return Ok(Ir::concat([expr, Ir::text(" "), Ir::text(comment)]));
    }

    ir_expr_segment(elements, context, indent, ctx)
}

pub(super) fn format_block_expr_with_prefixed_comments(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
    prefixed_comments: &[String],
) -> Result<String, FormatError> {
    render_block(node, indent, ctx, prefixed_comments, format_line)
}

fn ir_block_expr(node: &SyntaxNode, indent: usize, ctx: FormatContext) -> Result<Ir, FormatError> {
    ir_block_expr_with_prefixed_comments(node, indent, ctx, &[])
}

pub(super) fn ir_block_expr_with_prefixed_comments(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
    prefixed_comments: &[String],
) -> Result<Ir, FormatError> {
    super::render::ir_block_expr_with_prefixed_comments(
        node,
        indent,
        ctx,
        prefixed_comments,
        ir_line,
    )
}

pub(super) fn snippet_from_elements(elements: &[SyntaxElement<RLanguage>]) -> String {
    super::render::snippet_from_elements(elements)
}

pub(super) fn format_expr_with_optional_comment(
    elements: &[SyntaxElement<RLanguage>],
    context: &'static str,
    indent: usize,
    ctx: FormatContext,
) -> Result<String, FormatError> {
    super::render::format_expr_with_optional_comment(
        elements,
        context,
        indent,
        ctx,
        format_expr_element,
    )
}

pub(super) fn is_trivia(kind: SyntaxKind) -> bool {
    is_trivia_kind(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Formatting an already-parsed CST must match formatting the same text,
    /// so the LSP read path (which formats off the cached parse tree) stays
    /// byte-identical to the text entry point.
    #[test]
    fn format_node_matches_format_with_style() {
        let style = FormatStyle::default();
        for input in [
            "x<-1\n",
            "x <- 1\n",
            "f(a,b ,c)\n",
            "if(x){y}else{z}\n",
            "x<-1", // no trailing newline
            "",
        ] {
            let via_text = format_with_style(input, style);
            let parsed = parse(input);
            let via_node = format_node(&parsed.cst, style, input.ends_with('\n'));
            assert_eq!(via_text, via_node, "mismatch for {input:?}");
        }
    }
}
