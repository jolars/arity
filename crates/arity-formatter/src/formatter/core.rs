use rowan::{NodeOrToken, SyntaxElement, TextRange};

use super::context::FormatContext;
use super::ir::Ir;
use super::printer::Printer;
use super::render::format_atom_token;
use super::rules::control_flow::{
    ir_for_expr, ir_if_expr, ir_repeat_expr, ir_while_expr, should_insert_comment_for_gap,
    try_format_for_with_external_body, try_format_if_with_external_body,
    try_format_repeat_with_external_body, try_format_while_with_external_body,
};
use super::rules::expressions::{
    ir_assignment_expr, ir_binary_expr, ir_paren_expr, ir_subset_expr, ir_unary_expr,
};
use super::rules::functions::{ir_call_expr, ir_function_expr};
use super::style::{FormatStyle, apply_line_ending};
use super::trivia::{is_trivia as is_trivia_kind, split_lines};
use crate::ast::{
    AssignmentExpr, AstNode, BinaryExpr, BlockExpr, CallExpr, ForExpr, FunctionExpr, IfExpr,
    ParenExpr, UnaryExpr, WhileExpr,
};
use crate::parser::{ParseOptions, parse_with_options};
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
    format_with_options(input, style, &ParseOptions::default())
}

/// [`format_with_style`] with caller-supplied [`ParseOptions`] for the parse.
/// The one option that matters to the formatter is the roxygen markdown
/// default: a package that enables markdown package-wide
/// (`Roxygen: list(markdown = TRUE)` in `DESCRIPTION`) writes no per-block
/// `@md`, and its doc comments only format correctly — markdown lists, code
/// blocks, and tables preserved as atomic units instead of reflowed as prose —
/// when the parse resolves them in markdown mode. The mode lives entirely in
/// the parsed tree; the formatting rules themselves key off node kinds and
/// need no flag.
pub fn format_with_options(
    input: &str,
    style: FormatStyle,
    options: &ParseOptions,
) -> Result<String, FormatError> {
    let parse_output = parse_with_options(input, options);
    if !parse_output.diagnostics.is_empty() {
        return Err(FormatError::ParseErrors {
            count: parse_output.diagnostics.len(),
        });
    }

    format_node(&parse_output.cst, style, input)
}

/// Format an already-parsed CST. The caller is responsible for rejecting input
/// that failed to parse (the diagnostics live next to the green tree in the
/// salsa cache, not on the node); this entry only guards against stray `ERROR`
/// tokens. `source` is the original text the CST was parsed from: its trailing
/// newline is preserved, and (for `line-ending = "auto"`) its first line ending
/// selects the output's newline style.
///
/// Used by the language server's read path, which formats off the cached parse
/// tree in its salsa database rather than re-parsing the buffer.
pub fn format_node(
    root: &SyntaxNode,
    style: FormatStyle,
    source: &str,
) -> Result<String, FormatError> {
    validate_supported_tokens(root)?;
    let ctx = FormatContext::new(style);
    let mut formatted = format_root(root, ctx)?;
    if source.ends_with('\n') && !formatted.ends_with('\n') {
        formatted.push('\n');
    }
    Ok(apply_line_ending(
        &formatted,
        style.line_ending.resolve(source),
    ))
}

/// A region reformatted by [`format_range`]: the byte range to replace and the
/// text to replace it with. The range is the union of the selected statements'
/// non-whitespace spans (so the first line's existing indentation is left
/// untouched), and `text` carries no leading indent on its first line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeFormatted {
    pub range: TextRange,
    pub text: String,
}

/// Format only the statements overlapping `range`, leaving the rest of the
/// document untouched. Mirrors air: the selection is widened to whole statements
/// of the deepest enclosing statement list (ROOT or a block body), those
/// statements are formatted at their structural indent, and the first line's
/// existing indentation is preserved (only continuation lines are reindented).
///
/// Returns `Ok(None)` when the selection covers no statement (empty, whitespace,
/// or comment-gap only). Like [`format_node`], the caller must reject input that
/// failed to parse; this only guards against stray `ERROR` tokens.
pub fn format_range(
    root: &SyntaxNode,
    range: TextRange,
    style: FormatStyle,
    source: &str,
) -> Result<Option<RangeFormatted>, FormatError> {
    validate_supported_tokens(root)?;
    let ctx = FormatContext::new(style);

    let container = statement_container(root, range);
    let in_block = container.kind() == SyntaxKind::BLOCK_EXPR;
    let elements: Vec<SyntaxElement<RLanguage>> = if in_block {
        super::render::block_statement_elements(&container)?
    } else {
        container.children_with_tokens().collect()
    };
    let lines = split_lines(elements, "range")?;
    if lines.is_empty() {
        return Ok(None);
    }

    // Statements inside `n` nested blocks sit at indent level `n`; ROOT is 0.
    let base_indent = container
        .ancestors()
        .filter(|n| n.kind() == SyntaxKind::BLOCK_EXPR)
        .count();

    // Widen the selection to every statement line whose span touches `range`.
    let mut window_start: Option<usize> = None;
    let mut window_end = 0usize;
    for (idx, line) in lines.iter().enumerate() {
        if let Some(span) = line_significant_span(line)
            && span.start() <= range.end()
            && range.start() <= span.end()
        {
            window_start.get_or_insert(idx);
            window_end = idx + 1;
        }
    }
    let Some(window_start) = window_start else {
        return Ok(None);
    };
    let window = window_start..window_end;

    let rendered = if in_block {
        ir_block_statements(&lines, window, base_indent, ctx)?
    } else {
        ir_statements(&lines, window, base_indent, ctx)?
    };
    let (Some(first_line), Some(last_line)) = (rendered.first_line, rendered.last_line) else {
        return Ok(None);
    };

    let start = line_significant_span(&lines[first_line])
        .expect("emitted line has a significant span")
        .start();
    let end = line_significant_span(&lines[last_line])
        .expect("consumed line has a significant span")
        .end();

    let mut text = Printer::new(style).print_at(&rendered.ir, base_indent);
    while text.ends_with('\n') {
        text.pop();
    }
    // The printer emits `\n`-only; convert internal breaks to the source's
    // ending so a range edit never splices LF into a CRLF buffer (Tenet 1).
    let text = apply_line_ending(&text, style.line_ending.resolve(source));

    Ok(Some(RangeFormatted {
        range: TextRange::new(start, end),
        text,
    }))
}

/// The deepest ROOT/BLOCK_EXPR node whose statement list fully contains `range`.
/// `covering_element` already yields the smallest element spanning the whole
/// range, so its nearest statement-list ancestor is the deepest common one.
fn statement_container(root: &SyntaxNode, range: TextRange) -> SyntaxNode {
    let is_container = |kind: SyntaxKind| matches!(kind, SyntaxKind::ROOT | SyntaxKind::BLOCK_EXPR);
    let found = match root.covering_element(range) {
        NodeOrToken::Node(node) => node.ancestors().find(|n| is_container(n.kind())),
        NodeOrToken::Token(token) => token.parent_ancestors().find(|n| is_container(n.kind())),
    };
    found.unwrap_or_else(|| root.clone())
}

/// The span of a line from its first to its last non-trivia element (comments
/// included). `None` for a blank line, which has no significant element.
fn line_significant_span(line: &[SyntaxElement<RLanguage>]) -> Option<TextRange> {
    let mut significant = line.iter().filter(|el| !is_trivia_kind(el.kind()));
    let first = significant.next()?;
    let last = significant.next_back().unwrap_or(first);
    Some(TextRange::new(
        first.text_range().start(),
        last.text_range().end(),
    ))
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
    Ok(ir_statements(&lines, 0..lines.len(), 0, ctx)?.ir)
}

/// IR built for a sub-window of a statement list, plus the bounds of the lines it
/// actually emitted. `first_line`/`last_line` are line indices into `lines`:
/// `last_line` accounts for control-flow bodies pulled in from following lines,
/// so callers can compute the exact text span the IR replaces.
pub(super) struct StatementsIr {
    pub(super) ir: Ir,
    pub(super) first_line: Option<usize>,
    pub(super) last_line: Option<usize>,
}

/// Sequence the statements of a ROOT-style list (blank-line gaps preserved,
/// control-flow forms whose body sits on a following line rejoined) emitting only
/// the lines whose index falls in `window`. Iterating the full `lines` (not a
/// sub-slice) keeps the external-body lookahead and the blank-line gap context
/// (`should_insert_comment_for_gap`, which inspects preceding lines) correct at
/// the window's edges. `ir_root` passes `0..lines.len()`.
pub(super) fn ir_statements(
    lines: &[Vec<SyntaxElement<RLanguage>>],
    window: std::ops::Range<usize>,
    indent: usize,
    ctx: FormatContext,
) -> Result<StatementsIr, FormatError> {
    let mut items: Vec<Ir> = Vec::new();
    let mut first_line: Option<usize> = None;
    let mut last_line: Option<usize> = None;
    let mut idx = 0usize;
    while idx < lines.len() {
        if !window.contains(&idx) {
            idx += 1;
            continue;
        }

        if first_line.is_some() {
            if should_insert_comment_for_gap(lines, idx, indent, ctx)? {
                items.push(Ir::empty_line());
            } else {
                items.push(Ir::hard_line());
            }
        }

        let consumed = if let Some((body_ir, consumed)) =
            try_format_for_with_external_body(lines, idx, indent, ctx)?
        {
            items.push(body_ir);
            consumed
        } else if let Some((body_ir, consumed)) =
            try_format_while_with_external_body(lines, idx, indent, ctx)?
        {
            items.push(body_ir);
            consumed
        } else if let Some((body_ir, consumed)) =
            try_format_if_with_external_body(lines, idx, indent, ctx)?
        {
            items.push(body_ir);
            consumed
        } else if let Some((body_ir, consumed)) =
            try_format_repeat_with_external_body(lines, idx, indent, ctx)?
        {
            items.push(body_ir);
            consumed
        } else {
            items.push(ir_line(&lines[idx], indent, ctx)?);
            0
        };

        if first_line.is_none() {
            first_line = Some(idx);
        }
        last_line = Some(idx + consumed);
        idx += consumed + 1;
    }
    Ok(StatementsIr {
        ir: Ir::concat(items),
        first_line,
        last_line,
    })
}

/// Sequence the statements of a block body for a sub-window. Mirrors
/// [`super::render::ir_block_expr_with_prefixed_comments`]'s body rules (plain
/// hard breaks between statements, no blank-line preservation, no external-body
/// lookahead), but without the surrounding braces, so range formatting of a
/// block's interior matches whole-document block output.
pub(super) fn ir_block_statements(
    lines: &[Vec<SyntaxElement<RLanguage>>],
    window: std::ops::Range<usize>,
    indent: usize,
    ctx: FormatContext,
) -> Result<StatementsIr, FormatError> {
    let mut items: Vec<Ir> = Vec::new();
    let mut first_line: Option<usize> = None;
    let mut last_line: Option<usize> = None;
    for idx in window {
        if idx >= lines.len() {
            break;
        }
        if first_line.is_some() {
            items.push(Ir::hard_line());
        }
        items.push(ir_line(&lines[idx], indent, ctx)?);
        if first_line.is_none() {
            first_line = Some(idx);
        }
        last_line = Some(idx);
    }
    Ok(StatementsIr {
        ir: Ir::concat(items),
        first_line,
        last_line,
    })
}

pub(super) fn format_expr_segment(
    elements: &[SyntaxElement<RLanguage>],
    context: &'static str,
    indent: usize,
    ctx: FormatContext,
) -> Result<String, FormatError> {
    super::render::format_expr_segment(elements, context, indent, ctx, format_expr_element)
}

/// A single statement line as IR, without the leading indentation (the caller
/// supplies that structurally via [`Ir::Indent`] and line breaks). An empty
/// (blank) line yields [`Ir::Nil`].
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

    if let [NodeOrToken::Node(node)] = significant.as_slice()
        && node.kind() == SyntaxKind::ROXYGEN_BLOCK
    {
        return Ok(super::roxygen::ir_roxygen_block(node, indent, ctx));
    }

    if significant.len() == 2
        && let Some(comment) = super::trivia::inline_trailing_comment_text(&significant[1])
    {
        let expr = ir_expr_element(&significant[0], indent, ctx)?;
        // The trailing comment is a zero-width line suffix so it never forces the
        // expression's own groups to break (matching air); the statement
        // separator supplies the newline that ends its line.
        return Ok(Ir::concat([expr, Ir::line_suffix(format!(" {comment}"))]));
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
    // A `#'` line inside an argument list parses as a comment-only `ROXYGEN_BLOCK`
    // (roxygen2 does not treat it as documentation there, but the CST shape
    // matches the block/root parsers). Lay it out like any other roxygen block.
    if node.kind() == SyntaxKind::ROXYGEN_BLOCK {
        return Ok(super::roxygen::ir_roxygen_block(node, indent, ctx));
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

/// A single expression optionally followed by a trailing comment on the same
/// line.
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
        && let Some(comment) = super::trivia::inline_trailing_comment_text(&significant[1])
    {
        let expr = ir_expr_element(&significant[0], indent, ctx)?;
        // Zero-width line suffix: the trailing comment must not force the
        // expression to break (matching air). A break always follows it in
        // context (statement separator, or the enclosing list that a comment
        // forces open).
        return Ok(Ir::concat([expr, Ir::line_suffix(format!(" {comment}"))]));
    }

    ir_expr_segment(elements, context, indent, ctx)
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

pub(super) fn reparse_snippet_from_elements(elements: &[SyntaxElement<RLanguage>]) -> String {
    super::render::reparse_snippet_from_elements(elements)
}

pub(super) fn is_trivia(kind: SyntaxKind) -> bool {
    is_trivia_kind(kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

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
            let via_node = format_node(&parsed.cst, style, input);
            assert_eq!(via_text, via_node, "mismatch for {input:?}");
        }
    }

    #[test]
    fn line_ending_auto_mirrors_source() {
        use crate::formatter::LineEnding;
        let style = FormatStyle {
            line_ending: LineEnding::Auto,
            ..FormatStyle::default()
        };
        // A CRLF source round-trips to CRLF output; an LF source stays LF.
        assert_eq!(
            format_with_style("x<-1\r\ny<-2\r\n", style).unwrap(),
            "x <- 1\r\ny <- 2\r\n"
        );
        assert_eq!(
            format_with_style("x<-1\ny<-2\n", style).unwrap(),
            "x <- 1\ny <- 2\n"
        );
    }

    #[test]
    fn line_ending_explicit_overrides_source() {
        use crate::formatter::LineEnding;
        let crlf = FormatStyle {
            line_ending: LineEnding::Crlf,
            ..FormatStyle::default()
        };
        assert_eq!(
            format_with_style("x<-1\ny<-2\n", crlf).unwrap(),
            "x <- 1\r\ny <- 2\r\n"
        );
        let lf = FormatStyle {
            line_ending: LineEnding::Lf,
            ..FormatStyle::default()
        };
        assert_eq!(
            format_with_style("x<-1\r\ny<-2\r\n", lf).unwrap(),
            "x <- 1\ny <- 2\n"
        );
    }
}
