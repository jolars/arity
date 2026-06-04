use rowan::{NodeOrToken, SyntaxElement};

use super::super::context::FormatContext;
use super::super::core::{
    FormatError, format_expr_segment, format_expr_with_optional_comment, format_line,
    ir_expr_segment, ir_expr_with_optional_comment,
};
use super::super::ir::Ir;
use super::super::trivia::split_lines;
use crate::syntax::{RLanguage, SyntaxKind, SyntaxNode};

/// IR builder for unary expressions: operator directly prefixed to the operand.
pub(crate) fn ir_unary_expr(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    let elements: Vec<_> = node.children_with_tokens().collect();
    let op_idx = elements
        .iter()
        .position(|el| {
            matches!(
                el,
                NodeOrToken::Token(tok)
                    if matches!(
                        tok.kind(),
                        SyntaxKind::PLUS
                            | SyntaxKind::MINUS
                            | SyntaxKind::BANG
                            | SyntaxKind::TILDE
                            | SyntaxKind::QUESTION
                    )
            )
        })
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "unary operator not found",
            snippet: node.text().to_string(),
        })?;
    let op = match &elements[op_idx] {
        NodeOrToken::Token(tok) => tok.text().to_string(),
        NodeOrToken::Node(_) => unreachable!(),
    };
    let rhs = ir_expr_segment(&elements[op_idx + 1..], "unary operand", indent, ctx)?;
    Ok(Ir::concat([Ir::text(op), rhs]))
}

/// IR builder for assignment: the operands are space-separated around the
/// operator with no width-driven wrapping of its own (any wrapping comes from
/// the operands' own IR).
pub(crate) fn ir_assignment_expr(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    let elements: Vec<_> = node.children_with_tokens().collect();
    let op_idx = elements
        .iter()
        .position(|el| {
            matches!(
                el,
                NodeOrToken::Token(tok)
                    if matches!(
                        tok.kind(),
                        SyntaxKind::ASSIGN_LEFT
                            | SyntaxKind::SUPER_ASSIGN
                            | SyntaxKind::ASSIGN_RIGHT
                            | SyntaxKind::SUPER_ASSIGN_RIGHT
                            | SyntaxKind::ASSIGN_EQ
                    )
            )
        })
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "assignment operator not found",
            snippet: node.text().to_string(),
        })?;

    let op = match &elements[op_idx] {
        NodeOrToken::Token(tok) => tok.text().to_string(),
        NodeOrToken::Node(_) => unreachable!(),
    };
    let lhs = ir_expr_segment(&elements[..op_idx], "assignment lhs", indent, ctx)?;
    let rhs = ir_expr_segment(&elements[op_idx + 1..], "assignment rhs", indent, ctx)?;
    Ok(Ir::concat([lhs, Ir::text(format!(" {op} ")), rhs]))
}

/// IR builder for binary expressions. Mirrors [`format_binary_expr`]:
/// - `::` / `:::` / `^` / `:` / `$` / `@` are sticky and never wrap;
/// - `|>` and `%>%` always break after the operator;
/// - everything else gets a space-separated group whose broken form keeps the
///   operator on the prior line (R-valid continuation) and indents the right
///   operand one level.
pub(crate) fn ir_binary_expr(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    let elements: Vec<_> = node.children_with_tokens().collect();
    let op_idx = elements
        .iter()
        .position(|el| {
            matches!(
                el,
                NodeOrToken::Token(tok)
                    if matches!(
                        tok.kind(),
                        SyntaxKind::PLUS
                            | SyntaxKind::MINUS
                            | SyntaxKind::STAR
                            | SyntaxKind::SLASH
                            | SyntaxKind::CARET
                            | SyntaxKind::PIPE
                            | SyntaxKind::COLON
                            | SyntaxKind::OR
                            | SyntaxKind::OR2
                            | SyntaxKind::AND
                            | SyntaxKind::AND2
                            | SyntaxKind::EQUAL2
                            | SyntaxKind::NOT_EQUAL
                            | SyntaxKind::LESS_THAN
                            | SyntaxKind::LESS_THAN_OR_EQUAL
                            | SyntaxKind::GREATER_THAN
                            | SyntaxKind::GREATER_THAN_OR_EQUAL
                            | SyntaxKind::TILDE
                            | SyntaxKind::QUESTION
                            | SyntaxKind::USER_OP
                            | SyntaxKind::COLON2
                            | SyntaxKind::COLON3
                            | SyntaxKind::DOLLAR
                            | SyntaxKind::AT
                    )
            )
        })
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "binary operator not found",
            snippet: node.text().to_string(),
        })?;

    let (op_kind, op_text) = match &elements[op_idx] {
        NodeOrToken::Token(tok) => (tok.kind(), tok.text().to_string()),
        NodeOrToken::Node(_) => unreachable!(),
    };
    let lhs = ir_binary_side(&elements[..op_idx], "binary lhs", indent, ctx)?;
    let rhs = ir_binary_side(&elements[op_idx + 1..], "binary rhs", indent, ctx)?;

    // Sticky operators never wrap.
    if matches!(
        op_kind,
        SyntaxKind::COLON2
            | SyntaxKind::COLON3
            | SyntaxKind::CARET
            | SyntaxKind::COLON
            | SyntaxKind::DOLLAR
            | SyntaxKind::AT
    ) {
        return Ok(Ir::concat([lhs, Ir::text(op_text), rhs]));
    }

    // Pipes always break after the operator, indenting the continuation. The
    // continuation *and the right operand itself* live one level in, so when the
    // RHS call breaks its own arg list those args nest relative to the pipe stage
    // (close paren aligned with the call head) rather than dangling at the base
    // indent. The chain is left-associative, so only the final stage's RHS is a
    // leaf call here; deeper stages sit in the LHS and keep their own indent.
    if op_kind == SyntaxKind::PIPE || (op_kind == SyntaxKind::USER_OP && op_text == "%>%") {
        return Ok(Ir::concat([
            lhs,
            Ir::text(format!(" {op_text}")),
            Ir::indent(Ir::concat([Ir::hard_line(), rhs])),
        ]));
    }

    // Non-sticky binary operators: when broken, the operator stays on the
    // prior line so the continuation is R-valid (a leading operator on the
    // next line would be parsed as a separate unary statement outside
    // brackets). The right operand is indented one level.
    let flat_op = format!(" {op_text} ");
    let broken_lead = format!(" {op_text}");
    Ok(Ir::group(Ir::concat([
        lhs,
        Ir::if_break(
            Ir::text(flat_op),
            Ir::concat([Ir::text(broken_lead), Ir::indent(Ir::hard_line())]),
        ),
        rhs,
    ])))
}

fn ir_binary_side(
    elements: &[SyntaxElement<RLanguage>],
    context: &'static str,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    // Curly-curly is migrated later; bridge it through the legacy renderer.
    if let Some(curly_curly) = try_format_curly_curly(elements, indent, ctx)? {
        return Ok(Ir::verbatim(curly_curly));
    }
    ir_expr_segment(elements, context, indent, ctx)
}

fn try_format_curly_curly(
    elements: &[SyntaxElement<RLanguage>],
    indent: usize,
    ctx: FormatContext,
) -> Result<Option<String>, FormatError> {
    let significant: Vec<_> = elements
        .iter()
        .filter(|el| !super::super::core::is_trivia(el.kind()))
        .cloned()
        .collect();
    let [NodeOrToken::Node(outer)] = significant.as_slice() else {
        return Ok(None);
    };
    if outer.kind() != SyntaxKind::BLOCK_EXPR {
        return Ok(None);
    }

    let outer_significant: Vec<_> = outer
        .children_with_tokens()
        .filter(|el| !super::super::core::is_trivia(el.kind()))
        .collect();
    if outer_significant.len() != 3 {
        return Ok(None);
    }
    let [
        NodeOrToken::Token(outer_l),
        NodeOrToken::Node(inner),
        NodeOrToken::Token(outer_r),
    ] = outer_significant.as_slice()
    else {
        return Ok(None);
    };
    if outer_l.kind() != SyntaxKind::LBRACE || outer_r.kind() != SyntaxKind::RBRACE {
        return Ok(None);
    }
    if inner.kind() != SyntaxKind::BLOCK_EXPR {
        return Ok(None);
    }

    let inner_significant: Vec<_> = inner
        .children_with_tokens()
        .filter(|el| !super::super::core::is_trivia(el.kind()))
        .collect();
    if inner_significant.len() < 2 {
        return Ok(None);
    }
    let Some(NodeOrToken::Token(inner_l)) = inner_significant.first() else {
        return Ok(None);
    };
    let Some(NodeOrToken::Token(inner_r)) = inner_significant.last() else {
        return Ok(None);
    };
    if inner_l.kind() != SyntaxKind::LBRACE || inner_r.kind() != SyntaxKind::RBRACE {
        return Ok(None);
    }

    let inner_body = &inner_significant[1..inner_significant.len() - 1];
    if inner_body.is_empty() {
        return Ok(None);
    }
    let body = format_expr_segment(inner_body, "curly-curly inner body", indent, ctx)?;
    if body.contains('\n') || body.trim_start().starts_with('#') {
        return Ok(None);
    }
    Ok(Some(format!("{{{{ {body} }}}}")))
}

/// IR builder for parenthesized expressions. Mirrors [`format_paren_expr`]:
/// a single inner expression (optionally with a trailing comment) is wrapped
/// inline in `( )` and lets the inner expression handle its own wrapping; the
/// rarer multi-statement form is bridged through the legacy renderer.
pub(crate) fn ir_paren_expr(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    let elements: Vec<_> = node.children_with_tokens().collect();
    let open_idx = elements
        .iter()
        .position(|el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::LPAREN))
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing '(' in parenthesized expression",
            snippet: node.text().to_string(),
        })?;
    let close_idx = elements
        .iter()
        .rposition(|el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::RPAREN))
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing ')' in parenthesized expression",
            snippet: node.text().to_string(),
        })?;

    if close_idx <= open_idx {
        return Err(FormatError::AmbiguousConstruct {
            context: "invalid parenthesized expression bounds",
            snippet: node.text().to_string(),
        });
    }

    let inner_elements = &elements[open_idx + 1..close_idx];
    if let Ok(inner) =
        ir_expr_with_optional_comment(inner_elements, "parenthesized expression", indent, ctx)
    {
        return Ok(Ir::concat([Ir::text("("), inner, Ir::text(")")]));
    }

    // Multi-statement / empty parens: bridge through the legacy renderer.
    Ok(Ir::verbatim(format_paren_expr(node, indent, ctx)?))
}

pub(crate) fn format_paren_expr(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<String, FormatError> {
    let elements: Vec<_> = node.children_with_tokens().collect();
    let open_idx = elements
        .iter()
        .position(|el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::LPAREN))
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing '(' in parenthesized expression",
            snippet: node.text().to_string(),
        })?;
    let close_idx = elements
        .iter()
        .rposition(|el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::RPAREN))
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing ')' in parenthesized expression",
            snippet: node.text().to_string(),
        })?;

    if close_idx <= open_idx {
        return Err(FormatError::AmbiguousConstruct {
            context: "invalid parenthesized expression bounds",
            snippet: node.text().to_string(),
        });
    }

    let inner_elements = &elements[open_idx + 1..close_idx];
    if let Ok(inner) =
        format_expr_with_optional_comment(inner_elements, "parenthesized expression", indent, ctx)
    {
        return Ok(format!("({inner})"));
    }

    let lines = split_lines(inner_elements.to_vec(), "parenthesized expression")?;
    if lines.is_empty() {
        return Ok("()".to_string());
    }

    let mut out = String::from("(\n");
    for (idx, line) in lines.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        out.push_str(&format_line(line, indent + 1, ctx)?);
    }
    out.push('\n');
    out.push_str(&ctx.indent_text(indent));
    out.push(')');
    Ok(out)
}

fn bracket_open_text(kind: SyntaxKind) -> &'static str {
    match kind {
        SyntaxKind::LBRACK => "[",
        SyntaxKind::LBRACK2 => "[[",
        _ => "",
    }
}

fn bracket_close_text(kind: SyntaxKind) -> &'static str {
    match kind {
        SyntaxKind::RBRACK => "]",
        SyntaxKind::RBRACK2 => "]]",
        _ => "",
    }
}

// ============================ Native IR subset =============================
//
// `ir_subset_expr` renders `target[args]` / `target[[args]]` directly onto the
// document IR. The arg list is one `Group`: flat when it fits, otherwise broken
// one-per-line with the closing bracket on its own line. A trailing block hugs
// the bracket via `group_hug` (e.g. `dt[, {`…`}]`). Holes, comment slots, and
// the leading-hole hug mirror the tidyverse layout the legacy string renderer
// produced.

/// One comma-delimited position in an arg list (subset `[ ]` or call `( )`).
pub(crate) enum ArgSlot {
    /// An empty hole, e.g. the gaps in `x[, 2]` / `f(, a)`.
    Empty,
    /// A comment-only slot (no expression), e.g. `x[\n  # note\n]`.
    Comment(String),
    /// A formatted argument expression.
    Expr {
        ir: Ir,
        /// The argument's significant expression node, when it is a node (used
        /// to detect a trailing block to hug); `None` for a bare token.
        expr_node: Option<SyntaxNode>,
        /// A value-less named argument (`name =`): its rendered IR ends in `=`,
        /// so a following comma or the closing bracket keeps a separating space
        /// (`fn(NULL = )`, `fn(NULL = , )`), matching air's hole spacing.
        ends_with_eq: bool,
    },
}

impl ArgSlot {
    pub(crate) fn is_empty_hole(&self) -> bool {
        matches!(self, ArgSlot::Empty)
    }

    /// Whether the slot will unconditionally force its arg list to break (a
    /// comment, or an expression containing a block / other hard break).
    pub(crate) fn has_forced_break(&self) -> bool {
        match self {
            ArgSlot::Empty => false,
            ArgSlot::Comment(_) => true,
            ArgSlot::Expr { ir, .. } => ir.contains_forced_break(),
        }
    }

    pub(crate) fn content(&self) -> Ir {
        match self {
            ArgSlot::Empty => Ir::nil(),
            ArgSlot::Comment(text) => Ir::verbatim_forced(text.clone()),
            ArgSlot::Expr { ir, .. } => ir.clone(),
        }
    }

    /// Whether this slot is a value-less named argument (`name =`); see
    /// [`ArgSlot::Expr`]'s `ends_with_eq`.
    pub(crate) fn ends_with_eq(&self) -> bool {
        matches!(
            self,
            ArgSlot::Expr {
                ends_with_eq: true,
                ..
            }
        )
    }
}

struct ArgSlots {
    slots: Vec<ArgSlot>,
    has_comment_only: bool,
    has_comment_prefixed: bool,
}

pub(crate) fn ir_subset_expr(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    let elements: Vec<_> = node.children_with_tokens().collect();
    let (open_kind, close_kind) = match node.kind() {
        SyntaxKind::SUBSET_EXPR => (SyntaxKind::LBRACK, SyntaxKind::RBRACK),
        SyntaxKind::SUBSET2_EXPR => (SyntaxKind::LBRACK2, SyntaxKind::RBRACK2),
        _ => {
            return Err(FormatError::AmbiguousConstruct {
                context: "subset formatter called on non-subset node",
                snippet: node.text().to_string(),
            });
        }
    };
    let open_idx = elements
        .iter()
        .position(|el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == open_kind))
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing opening bracket in subset expression",
            snippet: node.text().to_string(),
        })?;
    let target = ir_expr_segment(&elements[..open_idx], "subset target", indent, ctx)?;
    let arg_list = elements
        .iter()
        .find_map(|el| match el {
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::ARG_LIST => Some(n.clone()),
            _ => None,
        })
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing arg list in subset expression",
            snippet: node.text().to_string(),
        })?;

    let data = collect_subset_ir_slots(&arg_list, indent, ctx)?;
    let open = bracket_open_text(open_kind);
    let close = bracket_close_text(close_kind);
    Ok(Ir::concat([
        target,
        build_subset_args_ir(&data, open, close),
    ]))
}

fn collect_subset_ir_slots(
    arg_list: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<ArgSlots, FormatError> {
    let mut slots: Vec<ArgSlot> = Vec::new();
    let mut comments: Vec<String> = Vec::new();
    let mut expr: Option<(Ir, Option<SyntaxNode>)> = None;
    let mut has_comment_only = false;
    let mut has_comment_prefixed = false;

    // Several `ARG` nodes can share one comma-delimited slot (e.g. a comment
    // `ARG` directly followed by an expression `ARG`); fold them together,
    // emitting one slot per comma.
    fn finalize(
        comments: &mut Vec<String>,
        expr: &mut Option<(Ir, Option<SyntaxNode>)>,
        has_comment_prefixed: &mut bool,
    ) -> ArgSlot {
        let lead = std::mem::take(comments);
        match expr.take() {
            Some((ir, node)) => {
                if !lead.is_empty() {
                    *has_comment_prefixed = true;
                }
                let ir = if lead.is_empty() {
                    ir
                } else {
                    let mut parts: Vec<Ir> = Vec::new();
                    for comment in &lead {
                        parts.push(Ir::verbatim_forced(comment.clone()));
                        parts.push(Ir::hard_line());
                    }
                    parts.push(ir);
                    Ir::concat(parts)
                };
                ArgSlot::Expr {
                    ir,
                    expr_node: node,
                    // Subset named args are `ASSIGNMENT_EXPR` nodes, never the
                    // raw value-less `name =` token shape this flag tracks.
                    ends_with_eq: false,
                }
            }
            None if lead.is_empty() => ArgSlot::Empty,
            None => ArgSlot::Comment(lead.join("\n")),
        }
    }

    for element in arg_list.children_with_tokens() {
        match element {
            NodeOrToken::Node(arg) if arg.kind() == SyntaxKind::ARG => {
                let arg_elements: Vec<_> = arg.children_with_tokens().collect();
                if arg_elements.is_empty() {
                    continue;
                }
                let has_comment = arg_elements.iter().any(
                    |el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT),
                );
                let has_non_comment = arg_elements.iter().any(|el| match el {
                    NodeOrToken::Node(_) => true,
                    NodeOrToken::Token(tok) => !matches!(
                        tok.kind(),
                        SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
                    ),
                });
                if has_comment && !has_non_comment {
                    if let Some(text) = arg_elements.iter().find_map(|el| match el {
                        NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT => {
                            Some(tok.text().to_string())
                        }
                        _ => None,
                    }) {
                        comments.push(text);
                    }
                    has_comment_only = true;
                } else {
                    let (ir, node, prefixed) = ir_subset_argument(&arg_elements, indent, ctx)?;
                    if prefixed {
                        has_comment_prefixed = true;
                    }
                    expr = Some((ir, node));
                }
            }
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMA => {
                slots.push(finalize(
                    &mut comments,
                    &mut expr,
                    &mut has_comment_prefixed,
                ));
            }
            _ => {}
        }
    }
    slots.push(finalize(
        &mut comments,
        &mut expr,
        &mut has_comment_prefixed,
    ));

    Ok(ArgSlots {
        slots,
        has_comment_only,
        has_comment_prefixed,
    })
}

/// IR counterpart of [`format_subset_argument`]: the argument expression, the
/// significant node (if any), and whether a leading comment prefixes it.
fn ir_subset_argument(
    elements: &[SyntaxElement<RLanguage>],
    indent: usize,
    ctx: FormatContext,
) -> Result<(Ir, Option<SyntaxNode>, bool), FormatError> {
    let expr_start = elements.iter().position(|el| {
        !matches!(el, NodeOrToken::Token(tok) if matches!(
            tok.kind(),
            SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
        ))
    });
    let Some(expr_start) = expr_start else {
        return Ok((
            ir_expr_segment(elements, "subset argument", indent, ctx)?,
            None,
            false,
        ));
    };
    let expr_node = match &elements[expr_start] {
        NodeOrToken::Node(n) => Some(n.clone()),
        NodeOrToken::Token(_) => None,
    };
    let leading_comments: Vec<String> = elements[..expr_start]
        .iter()
        .filter_map(|el| match el {
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT => {
                Some(tok.text().to_string())
            }
            _ => None,
        })
        .collect();
    if leading_comments.is_empty() {
        return Ok((
            ir_expr_segment(elements, "subset argument", indent, ctx)?,
            expr_node,
            false,
        ));
    }
    let expr_ir =
        ir_expr_with_optional_comment(&elements[expr_start..], "subset argument", indent, ctx)?;
    let mut parts: Vec<Ir> = Vec::new();
    for comment in &leading_comments {
        parts.push(Ir::verbatim_forced(comment.clone()));
        parts.push(Ir::hard_line());
    }
    parts.push(expr_ir);
    Ok((Ir::concat(parts), expr_node, true))
}

fn build_subset_args_ir(data: &ArgSlots, open: &str, close: &str) -> Ir {
    let slots = &data.slots;
    let last = slots.len() - 1;
    let first_non_empty = slots.iter().position(|s| !s.is_empty_hole());
    let no_non_empty = first_non_empty.is_none();

    // Trailing-block hug: the last slot ends in a non-empty block, leading slots
    // are single-line, and there are no comments.
    let trailing_block = !data.has_comment_only
        && !data.has_comment_prefixed
        && slots[..last].iter().all(|s| !s.has_forced_break())
        && matches!(&slots[last], ArgSlot::Expr { ir, expr_node: Some(node), .. }
            if expr_ends_in_block(node) && ir.contains_forced_break());

    if trailing_block {
        return build_arg_hug(slots, open, close, first_non_empty, no_non_empty);
    }

    let force = data.has_comment_only
        || data.has_comment_prefixed
        || should_force_leading_hole_expand(slots, first_non_empty);

    build_arg_group(slots, open, close, first_non_empty, no_non_empty, force)
}

/// Whether a subset arg's expression ends in a block (`{ … }`), so its arg list
/// can hug the opening brace: a bare block or a named arg `name = { … }`.
pub(crate) fn expr_ends_in_block(node: &SyntaxNode) -> bool {
    match node.kind() {
        SyntaxKind::BLOCK_EXPR => true,
        SyntaxKind::ASSIGNMENT_EXPR => node
            .children()
            .last()
            .is_some_and(|child| child.kind() == SyntaxKind::BLOCK_EXPR),
        _ => false,
    }
}

/// Mirrors the legacy `should_force_subset_multiline`: a leading hole followed
/// by a multi-line first argument and at least one more non-empty arg forces the
/// whole list open (so the block is not the trailing element and cannot hug).
pub(crate) fn should_force_leading_hole_expand(
    slots: &[ArgSlot],
    first_non_empty: Option<usize>,
) -> bool {
    let Some(first) = first_non_empty else {
        return false;
    };
    let leading_hole = slots[0].is_empty_hole();
    let non_empty_count = slots.iter().filter(|s| !s.is_empty_hole()).count();
    leading_hole && slots[first].has_forced_break() && non_empty_count > 1
}

/// The flat (inline) separator for the gap after slot `idx`. Adjacent holes
/// before the first real argument collapse to a bare `,`; everything else gets
/// `, `. Mirrors `format_subset_args_inline_from_parts`.
pub(crate) fn flat_arg_sep(
    slots: &[ArgSlot],
    idx: usize,
    first_non_empty: Option<usize>,
    no_non_empty: bool,
) -> &'static str {
    let left_empty = slots[idx].is_empty_hole();
    let right_empty = slots[idx + 1].is_empty_hole();
    let compact = left_empty
        && right_empty
        && (no_non_empty || first_non_empty.is_some_and(|first| idx + 1 < first));
    if compact { "," } else { ", " }
}

/// Number of consecutive empty holes at the start of the slot list. These pack
/// onto the open-bracket line (`fn(,,`) rather than each taking their own line
/// when the group breaks --- matching air's treatment of leading holes. An
/// interior hole (one preceded by any non-hole slot) still breaks onto its own
/// line.
pub(crate) fn leading_empty_run(slots: &[ArgSlot]) -> usize {
    slots.iter().take_while(|s| s.is_empty_hole()).count()
}

pub(crate) fn build_arg_group(
    slots: &[ArgSlot],
    open: &str,
    close: &str,
    first_non_empty: Option<usize>,
    no_non_empty: bool,
    force: bool,
) -> Ir {
    let n = slots.len();
    // Leading holes pack onto the open line: suppress the break before each one
    // so their bare commas trail the opening bracket. In flat mode soft lines
    // emit nothing anyway, so this only changes the broken layout. The whole
    // list being empty is never reached forced, so the run never covers `n`.
    let lead_run = leading_empty_run(slots);

    let mut body: Vec<Ir> = Vec::new();
    for idx in 0..n {
        let is_last = idx + 1 == n;
        // A trailing empty slot is dropped: the preceding arg keeps its comma
        // but there is no final blank line (`fn[a, , b, , ]`, `fn(x, {}, )`).
        if is_last && idx > 0 && slots[idx].is_empty_hole() {
            continue;
        }
        if idx >= lead_run {
            body.push(Ir::soft_line());
        }
        body.push(slots[idx].content());
        if !is_last {
            // A value-less named arg (`name =`) keeps a space before the comma
            // (` , ` flat, ` ,` broken) so the empty value position stays visible.
            if slots[idx].ends_with_eq() {
                body.push(Ir::if_break(Ir::text(" , "), Ir::text(" ,")));
            } else {
                let sep = flat_arg_sep(slots, idx, first_non_empty, no_non_empty);
                body.push(Ir::if_break(Ir::text(sep), Ir::text(",")));
            }
        }
    }

    // When the final argument is a value-less named arg, the closing bracket
    // keeps a separating space on a single line (`fn(NULL = )`); broken, the
    // arg sits alone on its line with no trailing space.
    let close_space = slots[n - 1].ends_with_eq();
    let inner = Ir::concat([
        Ir::text(open),
        Ir::indent(Ir::concat(body)),
        if close_space {
            Ir::concat([Ir::if_break(Ir::text(" "), Ir::nil()), Ir::soft_line()])
        } else {
            Ir::soft_line()
        },
        Ir::text(close),
    ]);
    if force {
        Ir::group_expanded(inner)
    } else {
        Ir::group(inner)
    }
}

pub(crate) fn build_arg_hug(
    slots: &[ArgSlot],
    open: &str,
    close: &str,
    first_non_empty: Option<usize>,
    no_non_empty: bool,
) -> Ir {
    let inner = build_arg_hug_inner(slots, open, close, first_non_empty, no_non_empty);
    // Excuse a leading argument that overflows on its own line *only* when every
    // leading argument is a bare atom: a nested breakable group could be
    // rescued by breaking, so its overflow must still force the list open. With
    // only bare atoms, breaking buys no width — just lines — so the prefix hugs
    // (e.g. `test_that("<very long desc>", { … })`).
    let last = slots.len() - 1;
    let leading_all_atoms = slots[..last]
        .iter()
        .all(|slot| !slot.content().contains_group());
    if leading_all_atoms {
        Ir::group_hug_excused(inner)
    } else {
        Ir::group_hug(inner)
    }
}

fn build_arg_hug_inner(
    slots: &[ArgSlot],
    open: &str,
    close: &str,
    first_non_empty: Option<usize>,
    no_non_empty: bool,
) -> Ir {
    let last = slots.len() - 1;

    // Leading args (everything before the trailing block) render flat in the
    // prefix; each is followed by its comma.
    let mut leading: Vec<Ir> = vec![Ir::soft_line()];
    for idx in 0..last {
        leading.push(slots[idx].content());
        if slots[idx].ends_with_eq() {
            leading.push(Ir::if_break(Ir::text(" , "), Ir::text(" ,")));
        } else {
            leading.push(Ir::if_break(
                Ir::text(flat_arg_sep(slots, idx, first_non_empty, no_non_empty)),
                Ir::text(","),
            ));
        }
        if idx + 1 < last {
            leading.push(Ir::soft_line());
        }
    }

    let block_ir = slots[last].content();
    Ir::concat([
        Ir::text(open),
        Ir::indent(Ir::concat(leading)),
        // Flat: the block hugs the prefix. Broken: it drops to its own indented
        // line so the whole list expands.
        Ir::if_break(
            block_ir.clone(),
            Ir::indent(Ir::concat([Ir::soft_line(), block_ir])),
        ),
        Ir::soft_line(),
        Ir::text(close),
    ])
}
