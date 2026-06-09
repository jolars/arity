use rowan::{NodeOrToken, SyntaxElement, SyntaxToken};

use super::super::context::FormatContext;
use super::super::core::{
    FormatError, format_block_expr_with_prefixed_comments, format_expr_segment,
    format_expr_with_optional_comment, ir_block_expr_with_prefixed_comments, ir_expr_element,
    ir_expr_segment, ir_expr_with_optional_comment, is_trivia,
};
use super::super::ir::Ir;
use crate::ast::{AstNode, ForExpr, ForExprParts, IfExpr, WhileExpr, WhileExprParts};
use crate::syntax::{RLanguage, SyntaxKind, SyntaxNode};

/// Whether an `if` expression sits in statement (effect) position and so must
/// always be braced, mirroring air's `SyntaxPosition` (minus the
/// persistent-line-break forcing that Tenet 1 forbids ravel from copying).
/// Derived purely from the CST parent:
///
/// * a top-level program statement (`ROOT` child) --- always effect;
/// * a `{ }` block child --- effect, unless it is the block's last expression
///   (the block's return value, which is value position);
/// * anything else (assignment RHS, call argument, function body, an `else`
///   alternative, …) --- value position.
fn in_statement_position(node: &SyntaxNode) -> bool {
    match node.parent() {
        Some(parent) if parent.kind() == SyntaxKind::ROOT => true,
        Some(parent) if parent.kind() == SyntaxKind::BLOCK_EXPR => {
            !is_block_value_child(node, &parent)
        }
        _ => false,
    }
}

/// Whether `node` is the block's last expression (its return value). `children()`
/// yields only the block's statement nodes (braces, `;`, and trivia are tokens),
/// so the last one is the value position.
fn is_block_value_child(node: &SyntaxNode, block: &SyntaxNode) -> bool {
    block.children().last().is_some_and(|last| &last == node)
}

/// `force_braces` propagates the "this chain uses braces" decision down an
/// `else if` chain so the whole chain is braced (or not) as a unit, per the
/// tidyverse all-or-nothing rule.
fn format_if_expr_impl(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
    force_braces: bool,
) -> Result<String, FormatError> {
    let if_expr = IfExpr::cast(node.clone()).ok_or_else(|| FormatError::AmbiguousConstruct {
        context: "invalid if expression node",
        snippet: node.text().to_string(),
    })?;

    if_expr
        .if_keyword()
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing 'if' keyword",
            snippet: node.text().to_string(),
        })?;
    if_expr
        .lparen_index()
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing '(' after if",
            snippet: node.text().to_string(),
        })?;
    if_expr
        .rparen_index()
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing ')' after if condition",
            snippet: node.text().to_string(),
        })?;

    let condition_elements =
        if_expr
            .condition_elements()
            .ok_or_else(|| FormatError::AmbiguousConstruct {
                context: "missing '(' after if",
                snippet: node.text().to_string(),
            })?;

    let then_elements = if_expr
        .then_elements()
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing ')' after if condition",
            snippet: node.text().to_string(),
        })?;

    let has_else = if_expr.else_keyword().is_some();
    let condition = format_expr_segment(&condition_elements, "if condition", indent, ctx)?;
    let (mut then_expr, then_is_block, interstitial_comments, interstitial_attach_to_then) =
        format_if_then_branch_with_comments(&then_elements, indent, ctx, has_else)?;

    let else_elements = if has_else {
        Some(
            if_expr
                .else_elements()
                .ok_or_else(|| FormatError::AmbiguousConstruct {
                    context: "missing else branch",
                    snippet: node.text().to_string(),
                })?,
        )
    } else {
        None
    };

    // A chain uses braces iff any branch in it does (or a parent forced it):
    // braces are all-or-nothing across an `if`/`else if`/`else` chain. Beyond an
    // already-braced branch, air's always-brace rule also forces braces when the
    // consequence is itself an `if` (a nested-if consequence) or the alternative
    // is itself an `if` (an `else if` chain) --- the chain then braces even in
    // value position. Statement position is propagated via `force_braces`.
    let consequence_is_if = branch_starts_with_if(&then_elements);
    let alternative_is_if = else_elements
        .as_deref()
        .is_some_and(|els| sole_else_if_node(els).is_some());
    let needs_braces = force_braces
        || then_is_block
        || consequence_is_if
        || alternative_is_if
        || else_elements
            .as_deref()
            .is_some_and(else_branch_requires_braces);

    // Wrap a bare then-branch when the chain uses braces. This must happen even
    // with no `else` so a forced `else if` (which has no `else` of its own)
    // still braces its then-branch. A branch that already rendered as a block
    // (e.g. a comment-led empty `{}`) must not be re-wrapped.
    if needs_braces && !then_is_block && !rendered_is_block(&then_expr) {
        then_expr = wrap_branch_in_block(&then_expr, &[], indent, ctx);
    }

    let mut out = format!("if ({condition}) {then_expr}");
    if let Some(else_elements) = else_elements {
        // Render the else branch. An `else if` (the else branch is itself an
        // `if`) recurses so brace usage propagates across the whole chain
        // instead of nesting the `if` inside a synthetic block.
        let else_if_node = sole_else_if_node(&else_elements);
        let mut else_expr;
        let mut else_is_block;
        if let Some(if_node) = &else_if_node {
            else_expr = format_if_expr_impl(if_node, indent, ctx, needs_braces)?;
            else_is_block = true;
        } else {
            else_expr = format_if_branch(&else_elements, indent, ctx, true)?;
            // Treat a (comment-led) `else if` as block-like so it is never
            // wrapped into a synthetic block below.
            else_is_block =
                branch_starts_with_block(&else_elements) || branch_starts_with_if(&else_elements);
        }

        if !interstitial_comments.is_empty() {
            if then_is_block && interstitial_attach_to_then {
                then_expr =
                    prepend_comments_to_branch(&then_expr, &interstitial_comments, indent, ctx);
                out = format!("if ({condition}) {then_expr}");
            } else {
                else_expr =
                    prepend_comments_to_branch(&else_expr, &interstitial_comments, indent, ctx);
                else_is_block = true;
            }
        }

        if needs_braces && !else_is_block && !rendered_is_block(&else_expr) {
            else_expr = wrap_branch_in_block(&else_expr, &[], indent, ctx);
        }

        out.push_str(" else ");
        out.push_str(&else_expr);
    }
    Ok(out)
}

/// The sole significant element of an else branch when it is exactly one `if`
/// expression (an `else if` with no leading comments). Comment-led `else if`s
/// fall back to the generic branch renderer.
fn sole_else_if_node(elements: &[SyntaxElement<RLanguage>]) -> Option<SyntaxNode> {
    match significant_elements(elements).as_slice() {
        [NodeOrToken::Node(node)] if node.kind() == SyntaxKind::IF_EXPR => Some(node.clone()),
        _ => None,
    }
}

/// Whether an else branch (a block, or an `else if` chain) contains any braced
/// branch, which forces the enclosing chain to use braces throughout.
fn else_branch_requires_braces(elements: &[SyntaxElement<RLanguage>]) -> bool {
    match significant_elements(elements).first() {
        Some(NodeOrToken::Node(node)) if node.kind() == SyntaxKind::BLOCK_EXPR => true,
        Some(NodeOrToken::Node(node)) if node.kind() == SyntaxKind::IF_EXPR => {
            if_node_requires_braces(node)
        }
        _ => false,
    }
}

/// Whether an `if` node has a braced branch anywhere in its own chain.
fn if_node_requires_braces(node: &SyntaxNode) -> bool {
    let Some(if_expr) = IfExpr::cast(node.clone()) else {
        return false;
    };
    if let Some(then_elements) = if_expr.then_elements()
        && branch_starts_with_block(&then_elements)
    {
        return true;
    }
    if if_expr.else_keyword().is_some()
        && let Some(else_elements) = if_expr.else_elements()
    {
        return else_branch_requires_braces(&else_elements);
    }
    false
}

fn format_if_then_branch_with_comments(
    elements: &[SyntaxElement<RLanguage>],
    indent: usize,
    ctx: FormatContext,
    has_else: bool,
) -> Result<(String, bool, Vec<String>, bool), FormatError> {
    let significant = significant_elements(elements);
    if significant.is_empty() {
        return Err(FormatError::AmbiguousConstruct {
            context: "if then branch is empty",
            snippet: String::new(),
        });
    }
    if let Some(first) = significant.first()
        && matches!(first, NodeOrToken::Node(node) if node.kind() == SyntaxKind::BLOCK_EXPR)
    {
        let block =
            format_expr_segment(std::slice::from_ref(first), "if then branch", indent, ctx)?;
        let comments = significant
            .iter()
            .skip(1)
            .filter_map(|el| match el {
                NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT => {
                    Some(tok.text().to_string())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let attach_to_then = comments_attach_to_then_block(elements);
        return Ok((block, true, comments, attach_to_then));
    }
    // For a bare body, any comment between the body and `else` would either
    // get swallowed by the comment (same-line trailing) or render as an
    // ambiguous sibling (own-line interstitial). Wrap the body in a synthetic
    // block so the `else` survives, and surface own-line comments as
    // interstitial for the caller to attach to the `else` branch.
    let interstitial = bare_branch_interstitial_comments(elements);
    if has_else && (!interstitial.is_empty() || bare_branch_trailing_comment(elements).is_some()) {
        let body_only = elements_without_trailing_own_line_content(elements);
        let body_rendered = format_if_branch(&body_only, indent + 1, ctx, false)?;
        let wrapped = wrap_branch_in_block(&body_rendered, &[], indent, ctx);
        return Ok((wrapped, true, interstitial, false));
    }
    Ok((
        format_if_branch(elements, indent, ctx, false)?,
        false,
        Vec::new(),
        false,
    ))
}

/// Comments that appear after the bare body on their own line. A trailing
/// comment on the same line as the body stays bundled with the body (and is
/// rendered by [`format_expr_with_optional_comment`]); only own-line comments
/// are surfaced as interstitial so the caller can move them onto the `else`
/// branch.
fn bare_branch_interstitial_comments(elements: &[SyntaxElement<RLanguage>]) -> Vec<String> {
    let Some(body_idx) = elements.iter().position(|el| !is_trivia(el.kind())) else {
        return Vec::new();
    };
    let post = &elements[body_idx + 1..];
    let Some(first_newline) = post.iter().position(|el| el.kind() == SyntaxKind::NEWLINE) else {
        return Vec::new();
    };
    post[first_newline + 1..]
        .iter()
        .filter_map(|el| match el {
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT => {
                Some(tok.text().to_string())
            }
            _ => None,
        })
        .collect()
}

/// A same-line trailing comment on the bare body, if any. Used to detect
/// branches that cannot be inlined safely (the comment would swallow `else`).
fn bare_branch_trailing_comment(elements: &[SyntaxElement<RLanguage>]) -> Option<String> {
    let body_idx = elements.iter().position(|el| !is_trivia(el.kind()))?;
    let post = &elements[body_idx + 1..];
    let scan_end = post
        .iter()
        .position(|el| el.kind() == SyntaxKind::NEWLINE)
        .unwrap_or(post.len());
    post[..scan_end].iter().find_map(|el| match el {
        NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT => {
            Some(tok.text().to_string())
        }
        _ => None,
    })
}

/// Trim everything after the bare body's line, so the remaining elements feed
/// `format_if_branch` like a single line: the body, plus any same-line trailing
/// comment, but no own-line trailing comments.
fn elements_without_trailing_own_line_content(
    elements: &[SyntaxElement<RLanguage>],
) -> Vec<SyntaxElement<RLanguage>> {
    let Some(body_idx) = elements.iter().position(|el| !is_trivia(el.kind())) else {
        return elements.to_vec();
    };
    let after_body = body_idx + 1;
    let Some(rel_nl) = elements[after_body..]
        .iter()
        .position(|el| el.kind() == SyntaxKind::NEWLINE)
    else {
        return elements.to_vec();
    };
    elements[..after_body + rel_nl].to_vec()
}

fn comments_attach_to_then_block(elements: &[SyntaxElement<RLanguage>]) -> bool {
    let mut first_block_idx = None;
    let mut first_comment_idx = None;
    for (idx, el) in elements.iter().enumerate() {
        match el {
            NodeOrToken::Node(node)
                if first_block_idx.is_none() && node.kind() == SyntaxKind::BLOCK_EXPR =>
            {
                first_block_idx = Some(idx);
            }
            NodeOrToken::Token(tok)
                if first_block_idx.is_some()
                    && first_comment_idx.is_none()
                    && tok.kind() == SyntaxKind::COMMENT =>
            {
                first_comment_idx = Some(idx);
                break;
            }
            _ => {}
        }
    }
    let (Some(block_idx), Some(comment_idx)) = (first_block_idx, first_comment_idx) else {
        return false;
    };
    !elements[block_idx + 1..comment_idx]
        .iter()
        .any(|el| el.kind() == SyntaxKind::NEWLINE)
}

fn branch_starts_with_block(elements: &[SyntaxElement<RLanguage>]) -> bool {
    significant_elements(elements).first().is_some_and(
        |el| matches!(el, NodeOrToken::Node(node) if node.kind() == SyntaxKind::BLOCK_EXPR),
    )
}

/// True when the (else) branch is itself an `if` expression, i.e. an `else if`
/// chain. Such a branch is rendered as-is, not wrapped in a synthetic block.
fn branch_starts_with_if(elements: &[SyntaxElement<RLanguage>]) -> bool {
    significant_elements(elements).first().is_some_and(
        |el| matches!(el, NodeOrToken::Node(node) if node.kind() == SyntaxKind::IF_EXPR),
    )
}

fn prepend_comments_to_branch(
    rendered: &str,
    comments: &[String],
    indent: usize,
    ctx: FormatContext,
) -> String {
    if comments.is_empty() {
        return rendered.to_string();
    }
    let close_suffix = format!("\n{}}}", ctx.indent_text(indent));
    if let Some(inner) = rendered
        .strip_prefix("{\n")
        .and_then(|rest| rest.strip_suffix(&close_suffix))
    {
        let mut out = String::from("{\n");
        for comment in comments {
            out.push_str(&ctx.indent_text(indent + 1));
            out.push_str(comment);
            out.push('\n');
        }
        if !inner.is_empty() {
            out.push_str(inner);
            out.push('\n');
        }
        out.push_str(&ctx.indent_text(indent));
        out.push('}');
        return out;
    }
    wrap_branch_in_block(rendered, comments, indent, ctx)
}

/// Whether a rendered branch is already a `{ }` block, so that
/// [`wrap_branch_in_block`] would double-wrap it. A bare expression rendering
/// never starts with `{` (only a block does).
fn rendered_is_block(rendered: &str) -> bool {
    rendered.trim_start().starts_with('{')
}

fn wrap_branch_in_block(
    rendered: &str,
    comments: &[String],
    indent: usize,
    ctx: FormatContext,
) -> String {
    if rendered == "{}" && comments.is_empty() {
        return "{}".to_string();
    }
    let mut out = String::from("{\n");
    for comment in comments {
        out.push_str(&ctx.indent_text(indent + 1));
        out.push_str(comment);
        out.push('\n');
    }
    if rendered != "{}" {
        for line in rendered.lines() {
            out.push_str(&ctx.indent_text(indent + 1));
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(&ctx.indent_text(indent));
    out.push('}');
    out
}

fn format_if_branch(
    elements: &[SyntaxElement<RLanguage>],
    indent: usize,
    ctx: FormatContext,
    keep_trailing_comment: bool,
) -> Result<String, FormatError> {
    let significant: Vec<_> = elements
        .iter()
        .filter(|el| !is_trivia(el.kind()))
        .cloned()
        .collect();
    if significant.is_empty() {
        return Err(FormatError::AmbiguousConstruct {
            context: "if branch is empty",
            snippet: String::new(),
        });
    }

    let mut start = 0usize;
    let mut leading_comments = Vec::new();
    while start < significant.len() {
        match &significant[start] {
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT => {
                leading_comments.push(tok.text().to_string());
                start += 1;
            }
            _ => break,
        }
    }

    let mut end = significant.len();
    let trailing_comment = if keep_trailing_comment
        && end > start
        && matches!(
            significant.last(),
            Some(NodeOrToken::Token(tok)) if tok.kind() == SyntaxKind::COMMENT
        ) {
        end -= 1;
        match &significant[end] {
            NodeOrToken::Token(tok) => Some(tok.text().to_string()),
            NodeOrToken::Node(_) => None,
        }
    } else {
        None
    };

    let core = &significant[start..end];
    if core.is_empty() {
        return Ok(match trailing_comment {
            Some(comment) => comment,
            None => "{}".to_string(),
        });
    }

    let mut rendered = if core.len() == 1 {
        match &core[0] {
            NodeOrToken::Node(node) if node.kind() == SyntaxKind::BLOCK_EXPR => {
                format_block_expr_with_prefixed_comments(node, indent, ctx, &leading_comments)?
            }
            _ => {
                if leading_comments.is_empty() {
                    format_expr_with_optional_comment(core, "if branch", indent, ctx)?
                } else {
                    let expr =
                        format_expr_with_optional_comment(core, "if branch", indent + 1, ctx)?;
                    let mut out = String::from("{\n");
                    for comment in &leading_comments {
                        out.push_str(&ctx.indent_text(indent + 1));
                        out.push_str(comment);
                        out.push('\n');
                    }
                    out.push_str(&ctx.indent_text(indent + 1));
                    out.push_str(&expr);
                    out.push('\n');
                    out.push_str(&ctx.indent_text(indent));
                    out.push('}');
                    out
                }
            }
        }
    } else if leading_comments.is_empty() {
        format_expr_with_optional_comment(core, "if branch", indent, ctx)?
    } else {
        let expr = format_expr_with_optional_comment(core, "if branch", indent + 1, ctx)?;
        let mut out = String::from("{\n");
        for comment in &leading_comments {
            out.push_str(&ctx.indent_text(indent + 1));
            out.push_str(comment);
            out.push('\n');
        }
        out.push_str(&ctx.indent_text(indent + 1));
        out.push_str(&expr);
        out.push('\n');
        out.push_str(&ctx.indent_text(indent));
        out.push('}');
        out
    };

    if let Some(comment) = trailing_comment {
        rendered.push(' ');
        rendered.push_str(&comment);
    }
    Ok(rendered)
}

/// IR builder for if/else.
///
/// In **statement position** the chain is always braced. In **value position** a
/// simple one-liner stays flat when it fits but braces when it does not: we offer
/// both the un-forced rendering and the braced rendering as an
/// [`Ir::conditional_group`], letting the printer pick the flat candidate only
/// while its first line fits at the current column (air's `if_group_breaks`). The
/// un-forced rendering is *already* braced (multi-line, so it carries a forced
/// break) for nested-if / `else if` / block-arm chains, in which case no flat
/// alternative exists.
///
/// Comment-free chains build native IR ([`ir_if_expr_impl`]); chains carrying any
/// comment fall back to the legacy string renderer
/// ([`ir_if_expr_legacy`]/[`format_if_expr_impl`]) whose comment relocation is not
/// yet ported to the IR.
pub(crate) fn ir_if_expr(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    // The eligibility gate guards only the comment-free path, whose bare branches
    // are spliced as single-line opaque units. Comment-bearing chains relocate
    // comments natively (wrapping wide bare branches in a block), so they bypass
    // the gate and build native IR directly.
    if !if_subtree_has_comment(node) && !if_chain_native_eligible(node, indent, ctx)? {
        return ir_if_expr_legacy(node, indent, ctx);
    }
    if in_statement_position(node) {
        return ir_if_expr_impl(node, indent, ctx, true);
    }
    // Structurally braced chains (a block arm, a nested-if consequence, or an
    // `else if`) have no flat alternative: the un-forced native build already
    // carries a forced break, so it *is* the answer.
    let unforced = ir_if_expr_impl(node, indent, ctx, false)?;
    if unforced.contains_forced_break() {
        return Ok(unforced);
    }
    // A simple bare-branch chain. The flat candidate must be an *unbreakable*
    // single-line unit: it either fits on one line as-is, or the chain braces.
    // The legacy renderer produces exactly that string (it renders each branch
    // standalone at the `if`'s own indent), so we reuse it for the flat candidate
    // while the braced candidate stays native (structural `Ir::indent`). A branch
    // wide enough to wrap even standalone keeps the legacy un-braced layout.
    let flat = format_if_expr_impl(node, indent, ctx, false)?;
    if flat.contains('\n') {
        return Ok(Ir::verbatim(flat));
    }
    let braced = ir_if_expr_impl(node, indent, ctx, true)?;
    Ok(Ir::conditional_group([Ir::verbatim(flat), braced]))
}

/// Whether the `if` subtree carries any comment token. Comment-bearing chains
/// bypass the comment-free eligibility gate and build native IR directly (see
/// [`ir_if_expr`]).
fn if_subtree_has_comment(node: &SyntaxNode) -> bool {
    node.descendants_with_tokens()
        .any(|el| el.kind() == SyntaxKind::COMMENT)
}

/// Legacy string-bridge for comment-bearing if/else (see [`ir_if_expr`]).
fn ir_if_expr_legacy(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    if in_statement_position(node) {
        return Ok(Ir::verbatim(format_if_expr_impl(node, indent, ctx, true)?));
    }
    let flat = format_if_expr_impl(node, indent, ctx, false)?;
    if flat.contains('\n') {
        return Ok(Ir::verbatim(flat));
    }
    let braced = format_if_expr_impl(node, indent, ctx, true)?;
    Ok(Ir::conditional_group([
        Ir::verbatim(flat),
        Ir::verbatim(braced),
    ]))
}

/// Native-IR mirror of [`format_if_expr_impl`] for comment-free chains. Builds
/// the branch bodies with structural [`Ir::indent`] (via [`synthetic_block`] and
/// the native block builder) rather than baked indentation, so a wrapping context
/// can re-indent the whole `if` by layering another `Ir::indent`. `force_braces`
/// propagates the chain-wide brace decision down an `else if` chain, exactly as in
/// the legacy renderer.
fn ir_if_expr_impl(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
    force_braces: bool,
) -> Result<Ir, FormatError> {
    let if_expr = IfExpr::cast(node.clone()).ok_or_else(|| FormatError::AmbiguousConstruct {
        context: "invalid if expression node",
        snippet: node.text().to_string(),
    })?;
    let condition_elements =
        if_expr
            .condition_elements()
            .ok_or_else(|| FormatError::AmbiguousConstruct {
                context: "missing '(' after if",
                snippet: node.text().to_string(),
            })?;
    let then_elements = if_expr
        .then_elements()
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing ')' after if condition",
            snippet: node.text().to_string(),
        })?;
    let has_else = if_expr.else_keyword().is_some();
    let else_elements = if has_else {
        Some(
            if_expr
                .else_elements()
                .ok_or_else(|| FormatError::AmbiguousConstruct {
                    context: "missing else branch",
                    snippet: node.text().to_string(),
                })?,
        )
    } else {
        None
    };

    // The condition is rendered standalone and spliced opaquely: an `if`
    // condition never wraps on the `if (…)` prefix's account (unlike `while`),
    // so it must not participate in the header's width measurement.
    let condition = Ir::verbatim(format_expr_segment(
        &condition_elements,
        "if condition",
        indent,
        ctx,
    )?);
    // Comment relocation (IR port of `format_if_then_branch_with_comments` +
    // `prepend_comments_to_branch`). A comment trailing the then-block's `}` on
    // the same line stays with the then block; any other comment between the
    // branches moves onto the `else` branch (or braces a bare then-branch).
    let then_significant = significant_elements(&then_elements);
    let then_starts_block = matches!(
        then_significant.first(),
        Some(NodeOrToken::Node(node)) if node.kind() == SyntaxKind::BLOCK_EXPR
    );

    let mut interstitial: Vec<String> = Vec::new();
    let mut attach_to_then = false;
    let (mut then_ir, then_is_block) = if then_starts_block {
        interstitial = then_significant
            .iter()
            .skip(1)
            .filter_map(|el| match el {
                NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT => {
                    Some(tok.text().to_string())
                }
                _ => None,
            })
            .collect();
        attach_to_then = comments_attach_to_then_block(&then_elements);
        let then_comments: &[String] = if attach_to_then {
            interstitial.as_slice()
        } else {
            &[]
        };
        let block = then_significant
            .first()
            .and_then(|el| el.as_node())
            .expect("then branch starts with a block");
        (
            ir_block_expr_with_prefixed_comments(block, indent, ctx, then_comments)?,
            true,
        )
    } else {
        // Bare then-branch. A comment between the body and `else` cannot stay
        // inline (it would swallow `else` or read as an ambiguous sibling), so
        // wrap the body in a block; own-line comments are surfaced as
        // interstitial for the `else` branch, while a same-line trailing comment
        // rides the body's line.
        let inter = bare_branch_interstitial_comments(&then_elements);
        if has_else && (!inter.is_empty() || bare_branch_trailing_comment(&then_elements).is_some())
        {
            interstitial = inter;
            let body_only = elements_without_trailing_own_line_content(&then_elements);
            let (body_ir, _) = ir_if_branch(&body_only, indent + 1, ctx, &[])?;
            (synthetic_block(vec![body_ir]), true)
        } else {
            ir_if_branch(&then_elements, indent, ctx, &[])?
        }
    };

    // Braces are all-or-nothing across a chain (tidyverse): forced by statement
    // position, a braced branch anywhere, or a nested-if / `else if` arm.
    let consequence_is_if = branch_starts_with_if(&then_elements);
    let alternative_is_if = else_elements
        .as_deref()
        .is_some_and(|els| sole_else_if_node(els).is_some());
    let needs_braces = force_braces
        || then_is_block
        || consequence_is_if
        || alternative_is_if
        || else_elements
            .as_deref()
            .is_some_and(else_branch_requires_braces);

    if needs_braces && !then_is_block {
        then_ir = synthetic_block(vec![then_ir]);
    }

    // Interstitial comments not bound to the then block attach to the `else`
    // branch (the IR port of `prepend_comments_to_branch`).
    let else_comments: &[String] = if attach_to_then {
        &[]
    } else {
        interstitial.as_slice()
    };

    let mut parts = vec![Ir::text("if ("), condition, Ir::text(") "), then_ir];
    if let Some(else_elements) = else_elements {
        // An `else if` recurses so the brace decision propagates across the whole
        // chain instead of nesting the `if` inside a synthetic block.
        let (mut else_ir, else_is_block) = if let Some(if_node) = sole_else_if_node(&else_elements)
        {
            let inner = ir_if_expr_impl(&if_node, indent, ctx, needs_braces)?;
            if else_comments.is_empty() {
                (inner, true)
            } else {
                (
                    synthetic_block_with_comments(vec![inner], else_comments),
                    true,
                )
            }
        } else {
            let (ir, is_block) = ir_if_branch(&else_elements, indent, ctx, else_comments)?;
            (ir, is_block || branch_starts_with_if(&else_elements))
        };
        if needs_braces && !else_is_block {
            else_ir = synthetic_block(vec![else_ir]);
        }
        parts.push(Ir::text(" else "));
        parts.push(else_ir);
    }
    Ok(Ir::concat(parts))
}

/// Render a comment-free if/else branch to IR, returning whether it is a `{ }`
/// block (so the caller knows not to wrap it again). Mirrors the comment-free
/// path of [`format_if_branch`]: a sole block renders natively (structural
/// indent); a bare expression is rendered standalone at the `if`'s indent and
/// spliced opaquely. A bare branch is rendered standalone — like the legacy
/// renderer — so it neither wraps on the surrounding column nor on the extra
/// indent level a brace would add; [`if_chain_native_eligible`] has already
/// guaranteed it fits on one line, so the opaque splice re-indents cleanly inside
/// a [`synthetic_block`].
fn ir_if_branch(
    elements: &[SyntaxElement<RLanguage>],
    indent: usize,
    ctx: FormatContext,
    prefixed_comments: &[String],
) -> Result<(Ir, bool), FormatError> {
    let significant = significant_elements(elements);

    // Peel leading own-line comments and a single same-line trailing comment,
    // mirroring `format_if_branch`. Leading comments join the relocated
    // `prefixed_comments`; the trailing comment rides the body's line.
    let mut start = 0usize;
    let mut leading = Vec::new();
    while let Some(NodeOrToken::Token(tok)) = significant.get(start) {
        if tok.kind() != SyntaxKind::COMMENT {
            break;
        }
        leading.push(tok.text().to_string());
        start += 1;
    }
    let mut end = significant.len();
    let trailing = if end > start
        && matches!(
            significant.last(),
            Some(NodeOrToken::Token(tok)) if tok.kind() == SyntaxKind::COMMENT
        ) {
        end -= 1;
        match &significant[end] {
            NodeOrToken::Token(tok) => Some(tok.text().to_string()),
            NodeOrToken::Node(_) => None,
        }
    } else {
        None
    };
    let core = &significant[start..end];

    let mut combined = prefixed_comments.to_vec();
    combined.extend(leading);

    if core.is_empty() {
        if combined.is_empty() {
            return match trailing {
                Some(comment) => Ok((Ir::verbatim_forced(comment), false)),
                None => Ok((Ir::text("{}"), true)),
            };
        }
        let items = combined
            .iter()
            .map(|c| Ir::verbatim_forced(c.clone()))
            .collect();
        return Ok((synthetic_block(items), true));
    }

    // A sole block renders natively (structural indent), with any relocated and
    // leading comments prepended inside it.
    if core.len() == 1
        && let NodeOrToken::Node(node) = &core[0]
        && node.kind() == SyntaxKind::BLOCK_EXPR
    {
        let mut ir = ir_block_expr_with_prefixed_comments(node, indent, ctx, &combined)?;
        if let Some(comment) = trailing {
            ir = Ir::concat([ir, Ir::text(" "), Ir::verbatim_forced(comment)]);
        }
        return Ok((ir, true));
    }

    // A bare expression. With no comments it stays a single-line opaque unit (the
    // comment-free native path); with comments it is wrapped in a block so the
    // comments survive (the IR port of the legacy relocation).
    if combined.is_empty() {
        let mut rendered = format_expr_with_optional_comment(core, "if branch", indent, ctx)?;
        if let Some(comment) = trailing {
            rendered.push(' ');
            rendered.push_str(&comment);
        }
        return Ok((Ir::verbatim(rendered), false));
    }
    let mut expr_ir = ir_expr_with_optional_comment(core, "if branch", indent + 1, ctx)?;
    if let Some(comment) = trailing {
        expr_ir = Ir::concat([expr_ir, Ir::text(" "), Ir::verbatim_forced(comment)]);
    }
    Ok((
        synthetic_block_with_comments(vec![expr_ir], &combined),
        true,
    ))
}

/// Whether a comment-free `if` chain can be rendered natively. The native builder
/// splices each bare (non-block) branch as a single-line opaque unit, so it is
/// only faithful when every bare branch in the chain renders on one line at the
/// `if`'s indent. A bare branch wide enough to wrap standalone hits the legacy
/// renderer's baked-indent line-shifting, which the structural IR does not
/// reproduce, so such a chain falls back to the legacy path.
fn if_chain_native_eligible(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<bool, FormatError> {
    let Some(if_expr) = IfExpr::cast(node.clone()) else {
        return Ok(false);
    };
    let Some(then_elements) = if_expr.then_elements() else {
        return Ok(false);
    };
    if !branch_native_eligible(&then_elements, indent, ctx)? {
        return Ok(false);
    }
    if if_expr.else_keyword().is_some() {
        let Some(else_elements) = if_expr.else_elements() else {
            return Ok(false);
        };
        if let Some(if_node) = sole_else_if_node(&else_elements) {
            return if_chain_native_eligible(&if_node, indent, ctx);
        }
        return branch_native_eligible(&else_elements, indent, ctx);
    }
    Ok(true)
}

/// A single branch is native-eligible when it is a block (rendered structurally)
/// or a bare expression that fits on one line standalone (see
/// [`if_chain_native_eligible`]).
fn branch_native_eligible(
    elements: &[SyntaxElement<RLanguage>],
    indent: usize,
    ctx: FormatContext,
) -> Result<bool, FormatError> {
    let significant = significant_elements(elements);
    match significant.first() {
        None => Ok(true),
        Some(NodeOrToken::Node(node)) if node.kind() == SyntaxKind::BLOCK_EXPR => Ok(true),
        _ => {
            let rendered =
                format_expr_with_optional_comment(&significant, "if branch", indent, ctx)?;
            Ok(!rendered.contains('\n'))
        }
    }
}

/// IR builder for `for`. Mirrors [`format_for_expr`].
pub(crate) fn ir_for_expr(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    let parts = parse_for_expr_parts(node, indent, ctx)?;
    let header = ir_for_header(&parts, indent, ctx)?;
    let prefixed = comment_texts(&parts.post_clause_comments);
    let body = ir_loop_body(parts.body.as_ref(), indent, ctx, &prefixed)?;
    Ok(ir_with_leading_comments(
        &parts.leading_comments,
        header,
        body,
    ))
}

/// The `for (var in seq)` header as IR, shared by [`ir_for_expr`] and the
/// external-body handler.
fn ir_for_header(
    parts: &ForExprParts,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    let variable = ir_expr_segment(&parts.variable_elements, "for loop variable", indent, ctx)?;
    let sequence = ir_expr_segment(&parts.sequence_elements, "for loop sequence", indent, ctx)?;
    Ok(Ir::concat([
        Ir::text("for ("),
        variable,
        Ir::text(" in "),
        sequence,
        Ir::text(")"),
    ]))
}

/// IR builder for `while`. Mirrors [`format_while_expr`]: the condition is
/// wrapped onto its own indented line when it cannot stay inline.
pub(crate) fn ir_while_expr(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    let parts = parse_while_expr_parts(node, indent, ctx)?;
    let header = ir_while_header(&parts, indent, ctx)?;
    let prefixed = comment_texts(&parts.post_clause_comments);
    let body = ir_loop_body(parts.body.as_ref(), indent, ctx, &prefixed)?;
    Ok(ir_with_leading_comments(
        &parts.leading_comments,
        header,
        body,
    ))
}

/// The `while (cond)` header as IR (the condition wraps onto its own indented
/// line when it cannot stay inline), shared by [`ir_while_expr`] and the
/// external-body handler.
fn ir_while_header(
    parts: &WhileExprParts,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    let condition = ir_expr_segment(
        &parts.condition_elements,
        "while loop condition",
        indent + 1,
        ctx,
    )?;
    Ok(Ir::group(Ir::concat([
        Ir::text("while ("),
        Ir::indent(Ir::concat([Ir::soft_line(), condition])),
        Ir::soft_line(),
        Ir::text(")"),
    ])))
}

/// IR builder for `repeat`. Mirrors [`format_repeat_expr`].
pub(crate) fn ir_repeat_expr(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    let parts = parse_repeat_expr_parts(node)?;
    let body = ir_loop_body(
        parts.body.as_ref(),
        indent,
        ctx,
        &parts.post_keyword_comments,
    )?;
    Ok(Ir::concat([Ir::text("repeat "), body]))
}

fn comment_texts(tokens: &[SyntaxToken<RLanguage>]) -> Vec<String> {
    tokens.iter().map(|tok| tok.text().to_string()).collect()
}

/// Emit any leading comments each on their own line, then `header`, a space, and
/// the `body`. Mirrors the assembly shared by the loop formatters.
fn ir_with_leading_comments(leading: &[SyntaxToken<RLanguage>], header: Ir, body: Ir) -> Ir {
    let mut parts = Vec::new();
    for comment in leading {
        parts.push(Ir::text(comment.text().to_string()));
        parts.push(Ir::hard_line());
    }
    parts.push(header);
    parts.push(Ir::text(" "));
    parts.push(body);
    Ir::concat(parts)
}

/// IR counterpart of the (identical) loop body formatters: a block body, an
/// auto-braced bare expression, or an empty/comment-only synthesized block.
fn ir_loop_body(
    body: Option<&SyntaxElement<RLanguage>>,
    indent: usize,
    ctx: FormatContext,
    prefixed_comments: &[String],
) -> Result<Ir, FormatError> {
    match body {
        Some(NodeOrToken::Node(node)) if node.kind() == SyntaxKind::BLOCK_EXPR => {
            ir_block_expr_with_prefixed_comments(node, indent, ctx, prefixed_comments)
        }
        Some(element) => {
            let expr = ir_expr_element(element, indent + 1, ctx)?;
            let mut items: Vec<Ir> = prefixed_comments
                .iter()
                .map(|c| Ir::text(c.clone()))
                .collect();
            items.push(expr);
            Ok(synthetic_block(items))
        }
        None => {
            if prefixed_comments.is_empty() {
                return Ok(Ir::text("{}"));
            }
            let items: Vec<Ir> = prefixed_comments
                .iter()
                .map(|c| Ir::text(c.clone()))
                .collect();
            Ok(synthetic_block(items))
        }
    }
}

/// Wrap `items` (one per line) in a hard-broken, indented `{ }` block.
fn synthetic_block(items: Vec<Ir>) -> Ir {
    synthetic_block_with_comments(items, &[])
}

/// Like [`synthetic_block`] but prefixes `comments` (each forced onto its own
/// line) before the items. Mirror of `brace_wrap_body_with_comments` in
/// `functions.rs`; a single-line comment carries no indentation, so it
/// re-indents structurally inside the block's [`Ir::indent`].
fn synthetic_block_with_comments(items: Vec<Ir>, comments: &[String]) -> Ir {
    let mut inner: Vec<Ir> = Vec::new();
    for comment in comments {
        inner.push(Ir::hard_line());
        inner.push(Ir::verbatim_forced(comment.clone()));
    }
    for it in items {
        inner.push(Ir::hard_line());
        inner.push(it);
    }
    Ir::concat([
        Ir::text("{"),
        Ir::indent(Ir::concat(inner)),
        Ir::hard_line(),
        Ir::text("}"),
    ])
}

pub(crate) fn try_format_for_with_external_body(
    lines: &[Vec<SyntaxElement<RLanguage>>],
    line_idx: usize,
    indent: usize,
    ctx: FormatContext,
) -> Result<Option<(Ir, usize)>, FormatError> {
    let significant = significant_elements(&lines[line_idx]);
    let (for_node, trailing_comment) = match significant.as_slice() {
        [NodeOrToken::Node(node)] if node.kind() == SyntaxKind::FOR_EXPR => (node.clone(), None),
        [NodeOrToken::Node(node), NodeOrToken::Token(tok)]
            if node.kind() == SyntaxKind::FOR_EXPR && tok.kind() == SyntaxKind::COMMENT =>
        {
            (node.clone(), Some(tok.text().to_string()))
        }
        _ => return Ok(None),
    };

    let parts = parse_for_expr_parts(&for_node, indent, ctx)?;
    if parts.body.is_some() {
        return Ok(None);
    }

    let mut extra_body_comments = Vec::new();
    let mut cursor = line_idx + 1;
    while cursor < lines.len() {
        if let Some(comment) = comment_only_line_text(&lines[cursor]) {
            extra_body_comments.push(comment);
            cursor += 1;
            continue;
        }
        break;
    }

    let body_element = if cursor < lines.len() {
        match significant_elements(&lines[cursor]).as_slice() {
            [element] => Some(element.clone()),
            _ => None,
        }
    } else {
        None
    };
    let Some(body_element) = body_element else {
        return Ok(None);
    };

    let mut merged_comment_texts: Vec<String> = parts
        .post_clause_comments
        .iter()
        .map(|tok| tok.text().to_string())
        .collect();
    merged_comment_texts.extend(extra_body_comments);
    let header = ir_for_header(&parts, indent, ctx)?;
    let body = ir_loop_body(Some(&body_element), indent, ctx, &merged_comment_texts)?;
    let ir = ir_external_body(&parts.leading_comments, header, body, trailing_comment);
    Ok(Some((ir, cursor - line_idx)))
}

/// Assemble an external-body control-flow construct: leading comments, the
/// header, the body, and an optional trailing comment. The IR counterpart of the
/// string assembly the external-body handlers shared.
fn ir_external_body(
    leading: &[SyntaxToken<RLanguage>],
    header: Ir,
    body: Ir,
    trailing_comment: Option<String>,
) -> Ir {
    let assembled = ir_with_leading_comments(leading, header, body);
    match trailing_comment {
        Some(comment) => Ir::concat([assembled, Ir::text(" "), Ir::text(comment)]),
        None => assembled,
    }
}

pub(crate) fn try_format_while_with_external_body(
    lines: &[Vec<SyntaxElement<RLanguage>>],
    line_idx: usize,
    indent: usize,
    ctx: FormatContext,
) -> Result<Option<(Ir, usize)>, FormatError> {
    let significant = significant_elements(&lines[line_idx]);
    let (while_node, trailing_comment) = match significant.as_slice() {
        [NodeOrToken::Node(node)] if node.kind() == SyntaxKind::WHILE_EXPR => (node.clone(), None),
        [NodeOrToken::Node(node), NodeOrToken::Token(tok)]
            if node.kind() == SyntaxKind::WHILE_EXPR && tok.kind() == SyntaxKind::COMMENT =>
        {
            (node.clone(), Some(tok.text().to_string()))
        }
        _ => return Ok(None),
    };

    let parts = parse_while_expr_parts(&while_node, indent, ctx)?;
    if parts.body.is_some() {
        return Ok(None);
    }

    let mut extra_body_comments = Vec::new();
    let mut cursor = line_idx + 1;
    while cursor < lines.len() {
        if let Some(comment) = comment_only_line_text(&lines[cursor]) {
            extra_body_comments.push(comment);
            cursor += 1;
            continue;
        }
        break;
    }

    let body_element = if cursor < lines.len() {
        match significant_elements(&lines[cursor]).as_slice() {
            [element] => Some(element.clone()),
            _ => None,
        }
    } else {
        None
    };
    let Some(body_element) = body_element else {
        return Ok(None);
    };

    let mut merged_comment_texts: Vec<String> = parts
        .post_clause_comments
        .iter()
        .map(|tok| tok.text().to_string())
        .collect();
    merged_comment_texts.extend(extra_body_comments);
    let header = ir_while_header(&parts, indent, ctx)?;
    let body = ir_loop_body(Some(&body_element), indent, ctx, &merged_comment_texts)?;
    let ir = ir_external_body(&parts.leading_comments, header, body, trailing_comment);
    Ok(Some((ir, cursor - line_idx)))
}

pub(crate) fn try_format_repeat_with_external_body(
    lines: &[Vec<SyntaxElement<RLanguage>>],
    line_idx: usize,
    indent: usize,
    ctx: FormatContext,
) -> Result<Option<(Ir, usize)>, FormatError> {
    let significant = significant_elements(&lines[line_idx]);
    let (repeat_node, trailing_comment) = match significant.as_slice() {
        [NodeOrToken::Node(node)] if node.kind() == SyntaxKind::REPEAT_EXPR => (node.clone(), None),
        [NodeOrToken::Node(node), NodeOrToken::Token(tok)]
            if node.kind() == SyntaxKind::REPEAT_EXPR && tok.kind() == SyntaxKind::COMMENT =>
        {
            (node.clone(), Some(tok.text().to_string()))
        }
        _ => return Ok(None),
    };

    let parts = parse_repeat_expr_parts(&repeat_node)?;
    if parts.body.is_some() {
        return Ok(None);
    }

    let mut extra_body_comments = Vec::new();
    let mut cursor = line_idx + 1;
    while cursor < lines.len() {
        if let Some(comment) = comment_only_line_text(&lines[cursor]) {
            extra_body_comments.push(comment);
            cursor += 1;
            continue;
        }
        break;
    }

    let body_element = if cursor < lines.len() {
        match significant_elements(&lines[cursor]).as_slice() {
            [element] => Some(element.clone()),
            _ => None,
        }
    } else {
        None
    };
    let Some(body_element) = body_element else {
        return Ok(None);
    };

    let mut merged_comments = parts.post_keyword_comments;
    merged_comments.extend(extra_body_comments);
    let body = ir_loop_body(Some(&body_element), indent, ctx, &merged_comments)?;
    let ir = ir_external_body(&[], Ir::text("repeat"), body, trailing_comment);
    Ok(Some((ir, cursor - line_idx)))
}

pub(crate) fn try_format_if_with_external_body(
    lines: &[Vec<SyntaxElement<RLanguage>>],
    line_idx: usize,
    indent: usize,
    ctx: FormatContext,
) -> Result<Option<(Ir, usize)>, FormatError> {
    let significant = significant_elements(&lines[line_idx]);
    let (if_node, trailing_comment) = match significant.as_slice() {
        [NodeOrToken::Node(node)] if node.kind() == SyntaxKind::IF_EXPR => (node.clone(), None),
        [NodeOrToken::Node(node), NodeOrToken::Token(tok)]
            if node.kind() == SyntaxKind::IF_EXPR && tok.kind() == SyntaxKind::COMMENT =>
        {
            (node.clone(), Some(tok.text().to_string()))
        }
        _ => return Ok(None),
    };
    let if_expr = IfExpr::cast(if_node).ok_or_else(|| FormatError::AmbiguousConstruct {
        context: "invalid if expression node",
        snippet: String::new(),
    })?;
    if if_expr.else_keyword().is_some() {
        return Ok(None);
    }

    let then_elements = if_expr
        .then_elements()
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing if branch",
            snippet: String::new(),
        })?;
    let then_significant = significant_elements(&then_elements);
    let then_comment = match then_significant.as_slice() {
        [NodeOrToken::Token(tok)] if tok.kind() == SyntaxKind::COMMENT => tok.text().to_string(),
        _ => return Ok(None),
    };

    let mut cursor = line_idx + 1;
    while cursor < lines.len() && significant_elements(&lines[cursor]).is_empty() {
        cursor += 1;
    }
    if cursor >= lines.len() {
        return Ok(None);
    }
    let body_element = match significant_elements(&lines[cursor]).as_slice() {
        [el] => el.clone(),
        _ => return Ok(None),
    };

    let condition_elements =
        if_expr
            .condition_elements()
            .ok_or_else(|| FormatError::AmbiguousConstruct {
                context: "missing if condition",
                snippet: String::new(),
            })?;
    // The then-branch is a comment; wrap the next-line body (bare or block) in a
    // synthetic block led by that comment, exactly as the legacy
    // `wrap_branch_in_block(body, &[then_comment], …)` did.
    let condition = Ir::verbatim(format_expr_segment(
        &condition_elements,
        "if condition",
        indent,
        ctx,
    )?);
    let body_expr = ir_expr_element(&body_element, indent + 1, ctx)?;
    let body = synthetic_block(vec![Ir::text(then_comment), body_expr]);
    let header = Ir::concat([Ir::text("if ("), condition, Ir::text(") "), body]);
    let ir = match trailing_comment {
        Some(comment) => Ir::concat([header, Ir::text(" "), Ir::text(comment)]),
        None => header,
    };
    Ok(Some((ir, cursor - line_idx)))
}

pub(crate) fn should_insert_comment_for_gap(
    lines: &[Vec<SyntaxElement<RLanguage>>],
    idx: usize,
    indent: usize,
    ctx: FormatContext,
) -> Result<bool, FormatError> {
    if idx < 2 || !is_comment_only_line(&lines[idx - 1]) || !is_comment_only_line(&lines[idx - 2]) {
        return Ok(false);
    }
    if idx >= 3 && is_comment_only_line(&lines[idx - 3]) {
        return Ok(false);
    }
    if !line_starts_with_control_flow_loop(&lines[idx]) {
        return Ok(false);
    }
    Ok(loop_leading_comment_count(&lines[idx], indent, ctx)?.unwrap_or(0) <= 1)
}

fn parse_for_expr_parts(
    node: &SyntaxNode,
    _indent: usize,
    _ctx: FormatContext,
) -> Result<ForExprParts, FormatError> {
    let for_expr = ForExpr::cast(node.clone()).ok_or_else(|| FormatError::AmbiguousConstruct {
        context: "invalid for expression node",
        snippet: node.text().to_string(),
    })?;
    for_expr
        .parts()
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "invalid for expression structure",
            snippet: node.text().to_string(),
        })
}

fn parse_while_expr_parts(
    node: &SyntaxNode,
    _indent: usize,
    _ctx: FormatContext,
) -> Result<WhileExprParts, FormatError> {
    let while_expr =
        WhileExpr::cast(node.clone()).ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "invalid while expression node",
            snippet: node.text().to_string(),
        })?;
    while_expr
        .parts()
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "invalid while expression structure",
            snippet: node.text().to_string(),
        })
}

struct RepeatExprParts {
    post_keyword_comments: Vec<String>,
    body: Option<SyntaxElement<RLanguage>>,
}

fn parse_repeat_expr_parts(node: &SyntaxNode) -> Result<RepeatExprParts, FormatError> {
    let elements: Vec<_> = node.children_with_tokens().collect();
    let repeat_idx = elements
        .iter()
        .position(|el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::REPEAT_KW))
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing 'repeat' keyword",
            snippet: node.text().to_string(),
        })?;

    let mut post_keyword_comments = Vec::new();
    let mut body = None;
    for element in elements.iter().skip(repeat_idx + 1) {
        if is_trivia(element.kind()) {
            continue;
        }
        if let NodeOrToken::Token(tok) = element
            && tok.kind() == SyntaxKind::COMMENT
        {
            post_keyword_comments.push(tok.text().to_string());
            continue;
        }
        body = Some(element.clone());
        break;
    }

    Ok(RepeatExprParts {
        post_keyword_comments,
        body,
    })
}

fn significant_elements(line: &[SyntaxElement<RLanguage>]) -> Vec<SyntaxElement<RLanguage>> {
    line.iter()
        .filter(|el| !is_trivia(el.kind()))
        .cloned()
        .collect()
}

fn comment_only_line_text(line: &[SyntaxElement<RLanguage>]) -> Option<String> {
    match significant_elements(line).as_slice() {
        [NodeOrToken::Token(tok)] if tok.kind() == SyntaxKind::COMMENT => {
            Some(tok.text().to_string())
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

fn line_starts_with_control_flow_loop(line: &[SyntaxElement<RLanguage>]) -> bool {
    let significant = significant_elements(line);
    match significant.as_slice() {
        [NodeOrToken::Node(node)] => {
            node.kind() == SyntaxKind::FOR_EXPR
                || node.kind() == SyntaxKind::WHILE_EXPR
                || node.kind() == SyntaxKind::REPEAT_EXPR
        }
        [NodeOrToken::Node(node), NodeOrToken::Token(tok)] => {
            (node.kind() == SyntaxKind::FOR_EXPR
                || node.kind() == SyntaxKind::WHILE_EXPR
                || node.kind() == SyntaxKind::REPEAT_EXPR)
                && tok.kind() == SyntaxKind::COMMENT
        }
        _ => false,
    }
}

fn loop_leading_comment_count(
    line: &[SyntaxElement<RLanguage>],
    indent: usize,
    ctx: FormatContext,
) -> Result<Option<usize>, FormatError> {
    let significant = significant_elements(line);
    let (node, kind) = match significant.as_slice() {
        [NodeOrToken::Node(node)] if node.kind() == SyntaxKind::FOR_EXPR => {
            (Some(node.clone()), SyntaxKind::FOR_EXPR)
        }
        [NodeOrToken::Node(node)] if node.kind() == SyntaxKind::WHILE_EXPR => {
            (Some(node.clone()), SyntaxKind::WHILE_EXPR)
        }
        [NodeOrToken::Node(node)] if node.kind() == SyntaxKind::REPEAT_EXPR => {
            (Some(node.clone()), SyntaxKind::REPEAT_EXPR)
        }
        [NodeOrToken::Node(node), NodeOrToken::Token(tok)]
            if node.kind() == SyntaxKind::FOR_EXPR && tok.kind() == SyntaxKind::COMMENT =>
        {
            (Some(node.clone()), SyntaxKind::FOR_EXPR)
        }
        [NodeOrToken::Node(node), NodeOrToken::Token(tok)]
            if node.kind() == SyntaxKind::WHILE_EXPR && tok.kind() == SyntaxKind::COMMENT =>
        {
            (Some(node.clone()), SyntaxKind::WHILE_EXPR)
        }
        [NodeOrToken::Node(node), NodeOrToken::Token(tok)]
            if node.kind() == SyntaxKind::REPEAT_EXPR && tok.kind() == SyntaxKind::COMMENT =>
        {
            (Some(node.clone()), SyntaxKind::REPEAT_EXPR)
        }
        _ => (None, SyntaxKind::ERROR),
    };
    let Some(node) = node else {
        return Ok(None);
    };

    if kind == SyntaxKind::FOR_EXPR {
        return Ok(Some(
            parse_for_expr_parts(&node, indent, ctx)?
                .leading_comments
                .len(),
        ));
    }

    if kind == SyntaxKind::WHILE_EXPR {
        return Ok(Some(
            parse_while_expr_parts(&node, indent, ctx)?
                .leading_comments
                .len(),
        ));
    }

    Ok(Some(
        parse_repeat_expr_parts(&node)?.post_keyword_comments.len(),
    ))
}
