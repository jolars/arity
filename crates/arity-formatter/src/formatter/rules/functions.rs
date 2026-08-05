use rowan::{NodeOrToken, SyntaxElement};

use super::super::context::FormatContext;
use super::super::core::{
    FormatError, ir_block_expr_with_prefixed_comments, ir_expr_element, ir_expr_segment,
    ir_expr_with_optional_comment, ir_line, reparse_snippet_from_elements, snippet_from_elements,
};
use super::super::ir::Ir;
use super::super::trivia::split_lines;
use super::expressions::{
    ArgSlot, build_arg_group, build_arg_hug, expr_ends_in_block, should_force_leading_hole_expand,
};
use crate::parser::parse;
use crate::syntax::{RLanguage, SyntaxKind, SyntaxNode};

pub(crate) fn ir_call_expr(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    let elements: Vec<_> = node.children_with_tokens().collect();
    let lparen_idx = elements
        .iter()
        .position(|el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::LPAREN))
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing '(' in call expression",
            snippet: node.text().to_string(),
        })?;
    let arg_list = elements
        .iter()
        .find_map(|el| match el {
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::ARG_LIST => Some(n.clone()),
            _ => None,
        })
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing arg list in call expression",
            snippet: node.text().to_string(),
        })?;

    let callee = ir_expr_segment(&elements[..lparen_idx], "call callee", indent, ctx)?;

    // Comments are relocated natively (own-line vs trailing) by an always-broken
    // item-stream layout; the flat/hug optimizations below never apply once a
    // comment is present.
    if arg_list_needs_comment_layout(&arg_list) {
        return Ok(Ir::concat([
            callee,
            ir_call_args_with_comments(&arg_list, indent, ctx)?,
        ]));
    }

    let (slots, comma_count) = collect_call_ir_slots(&arg_list, indent, ctx)?;

    // Empty call: no arguments and no holes.
    if comma_count == 0 && slots.iter().all(ArgSlot::is_empty_hole) {
        return Ok(Ir::concat([callee, Ir::text("()")]));
    }

    // Single-argument hug: a lone positional argument that owns a breakable
    // trailing arg list (a call/subset, possibly behind `::`/`$`) hugs the
    // parens with no extra indent, so nested wrapping falls out of the inner
    // construct's own group (`c(list(\n  ...\n))`, `abort(glue::glue(\n  ...\n))`).
    if comma_count == 0
        && let [ArgSlot::Expr { ir, .. }] = slots.as_slice()
        && single_arg_is_huggable(&arg_list)
    {
        return Ok(Ir::concat([
            callee,
            Ir::text("("),
            ir.clone(),
            Ir::text(")"),
        ]));
    }

    let force_named_functions = should_force_multiline_named_functions(&arg_list);
    Ok(Ir::concat([
        callee,
        build_call_args_ir(&slots, force_named_functions),
    ]))
}

/// The native port of [`should_force_multiline_for_named_function_args`]: a call
/// with more than one non-empty argument and at least two named arguments whose
/// value is a `function` definition always expands one argument per line, even
/// when it would otherwise fit (`list(a = function() {}, b = function() {})`).
fn should_force_multiline_named_functions(arg_list: &SyntaxNode) -> bool {
    let args: Vec<_> = arg_list
        .children()
        .filter(|n| n.kind() == SyntaxKind::ARG)
        .collect();
    let non_empty = args.iter().filter(|arg| arg_has_significant(arg)).count();
    if non_empty <= 1 {
        return false;
    }
    args.iter().filter(|arg| arg_is_named_function(arg)).count() >= 2
}

fn arg_has_significant(arg: &SyntaxNode) -> bool {
    arg.children_with_tokens()
        .any(|el| !super::super::core::is_trivia(el.kind()))
}

/// A named argument whose value is (or contains) a `function` definition,
/// matching the legacy `is_named && formatted.contains("function(")` test.
/// Lambda (`\(...)`) values do not count, since they never render `function(`.
fn arg_is_named_function(arg: &SyntaxNode) -> bool {
    let is_named = arg
        .children_with_tokens()
        .any(|el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::ASSIGN_EQ));
    is_named
        && arg.descendants().any(|n| {
            n.kind() == SyntaxKind::FUNCTION_EXPR
                && n.children_with_tokens().any(|el| {
                    matches!(el, NodeOrToken::Token(tok)
                        if tok.kind() == SyntaxKind::FUNCTION_KW && tok.text() == "function")
                })
        })
}

/// Whether this arg list owns comments that need relocating onto their own lines
/// or trailing the previous argument. That is the case for a comment sitting
/// directly in an `ARG` (a comment-only arg, or a leading/trailing/around-`=`
/// comment) or for a curly-curly `{{ … }}` argument whose lifted comments this
/// level prints. Comments buried in a nested call/subset/function/block are
/// relocated by that construct's own renderer, so they do not count here.
fn arg_list_needs_comment_layout(arg_list: &SyntaxNode) -> bool {
    arg_list
        .children()
        .filter(|n| n.kind() == SyntaxKind::ARG)
        .any(|arg| {
            arg.children_with_tokens()
                .any(|el| el.kind() == SyntaxKind::COMMENT)
                || arg
                    .children()
                    .any(|n| n.kind() == SyntaxKind::ROXYGEN_BLOCK)
                || arg_value_is_commented_curly_curly(&arg)
        })
}

/// Whether an argument's value is a curly-curly `{{ … }}` carrying a comment
/// somewhere inside (so its comments must be lifted out — see
/// [`ir_curly_curly_with_comments`]). Handles both bare (`{{ x }}`) and named
/// (`name = {{ x }}`) arguments.
fn arg_value_is_commented_curly_curly(arg: &SyntaxNode) -> bool {
    let elements: Vec<_> = arg.children_with_tokens().collect();
    let value_elements = match elements
        .iter()
        .position(|el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::ASSIGN_EQ))
    {
        Some(eq_idx) => &elements[eq_idx + 1..],
        None => &elements[..],
    };
    let value_significant: Vec<_> = value_elements
        .iter()
        .filter(|el| !super::super::core::is_trivia(el.kind()))
        .cloned()
        .collect();
    let [NodeOrToken::Node(outer)] = value_significant.as_slice() else {
        return false;
    };
    is_curly_curly_symbol_block(outer)
        && outer
            .descendants_with_tokens()
            .any(|el| el.kind() == SyntaxKind::COMMENT)
}

/// Whether a `BLOCK_EXPR` is a curly-curly `{{ symbol }}` wrapper that
/// [`ir_curly_curly_with_comments`] will accept: an outer block whose only
/// significant content (ignoring comments) is an inner block holding exactly one
/// `IDENT`. A `{{ … }}` with zero or a non-symbol inner expression is *not* a
/// curly-curly — it renders as ordinary nested blocks (and so must not be routed
/// to the comment layout, where it would lose the trailing-block hug).
fn is_curly_curly_symbol_block(outer: &SyntaxNode) -> bool {
    if outer.kind() != SyntaxKind::BLOCK_EXPR {
        return false;
    }
    let outer_sig: Vec<_> = outer
        .children_with_tokens()
        .filter(|el| !super::super::core::is_trivia(el.kind()))
        .collect();
    if outer_sig.len() < 2 {
        return false;
    }
    let (Some(NodeOrToken::Token(l)), Some(NodeOrToken::Token(r))) =
        (outer_sig.first(), outer_sig.last())
    else {
        return false;
    };
    if l.kind() != SyntaxKind::LBRACE || r.kind() != SyntaxKind::RBRACE {
        return false;
    }
    let mut inner = None::<SyntaxNode>;
    for el in &outer_sig[1..outer_sig.len() - 1] {
        match el {
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::BLOCK_EXPR => {
                if inner.is_some() {
                    return false;
                }
                inner = Some(n.clone());
            }
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT => {}
            _ => return false,
        }
    }
    let Some(inner) = inner else {
        return false;
    };
    let inner_sig: Vec<_> = inner
        .children_with_tokens()
        .filter(|el| !super::super::core::is_trivia(el.kind()))
        .collect();
    if inner_sig.len() < 3 {
        return false;
    }
    let (Some(NodeOrToken::Token(il)), Some(NodeOrToken::Token(ir))) =
        (inner_sig.first(), inner_sig.last())
    else {
        return false;
    };
    if il.kind() != SyntaxKind::LBRACE || ir.kind() != SyntaxKind::RBRACE {
        return false;
    }
    let exprs: Vec<_> = inner_sig[1..inner_sig.len() - 1]
        .iter()
        .filter(|el| !matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT))
        .collect();
    matches!(exprs.as_slice(), [NodeOrToken::Token(tok)] if tok.kind() == SyntaxKind::IDENT)
}

/// A *positional* function-definition argument (`function(...) ...` or
/// `\(...) ...`) used as the trailing arg of a call. Named function args
/// (`f = function(...) ...`) are excluded, mirroring the legacy renderer,
/// which only hugs trailing args that start with `function(`.
fn expr_is_positional_function(node: &SyntaxNode) -> bool {
    node.kind() == SyntaxKind::FUNCTION_EXPR && !value_node_is_named_arg(node)
}

/// Whether this argument-value node sits in a named call argument: its parent is
/// an `ARG` carrying an `=` (`name = <node>`).
fn value_node_is_named_arg(node: &SyntaxNode) -> bool {
    node.parent().is_some_and(|arg| {
        arg.kind() == SyntaxKind::ARG
            && arg.children_with_tokens().any(
                |el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::ASSIGN_EQ),
            )
    })
}

/// Native IR for a curly-curly `{{ x }}` call argument. Flat: `{{ x }}`; when the
/// symbol can't fit inline the group breaks to put it on its own indented line.
/// Returns `None` for any other shape (`{{ 1 }}`, `{{ (x) }}`, multi-statement,
/// ...), which then formats as ordinary nested blocks. Comment-bearing forms
/// never reach here — they route to the legacy renderer via [`call_needs_legacy`].
pub(crate) fn ir_curly_curly(
    significant: &[SyntaxElement<RLanguage>],
    indent: usize,
    ctx: FormatContext,
) -> Result<Option<Ir>, FormatError> {
    let Some(symbol) = curly_curly_inner_symbol(significant) else {
        return Ok(None);
    };
    let inner = ir_expr_element(&symbol, indent + 1, ctx)?;
    Ok(Some(Ir::group(Ir::concat([
        Ir::text("{{"),
        Ir::indent(Ir::concat([Ir::line(), inner])),
        Ir::line(),
        Ir::text("}}"),
    ]))))
}

/// The single symbol of a curly-curly `{{ x }}` argument: an outer `BLOCK_EXPR`
/// wrapping an inner `BLOCK_EXPR` whose only content is one identifier. Returns
/// `None` for any other shape, matching the legacy renderer's symbol-only
/// detection.
fn curly_curly_inner_symbol(
    significant: &[SyntaxElement<RLanguage>],
) -> Option<SyntaxElement<RLanguage>> {
    let [NodeOrToken::Node(outer)] = significant else {
        return None;
    };
    if outer.kind() != SyntaxKind::BLOCK_EXPR {
        return None;
    }
    let outer_significant: Vec<_> = outer
        .children_with_tokens()
        .filter(|el| !super::super::core::is_trivia(el.kind()))
        .collect();
    let [
        NodeOrToken::Token(outer_l),
        NodeOrToken::Node(inner),
        NodeOrToken::Token(outer_r),
    ] = outer_significant.as_slice()
    else {
        return None;
    };
    if outer_l.kind() != SyntaxKind::LBRACE
        || outer_r.kind() != SyntaxKind::RBRACE
        || inner.kind() != SyntaxKind::BLOCK_EXPR
    {
        return None;
    }
    let inner_significant: Vec<_> = inner
        .children_with_tokens()
        .filter(|el| !super::super::core::is_trivia(el.kind()))
        .collect();
    let [
        NodeOrToken::Token(inner_l),
        inner_expr @ NodeOrToken::Token(symbol),
        NodeOrToken::Token(inner_r),
    ] = inner_significant.as_slice()
    else {
        return None;
    };
    if inner_l.kind() == SyntaxKind::LBRACE
        && inner_r.kind() == SyntaxKind::RBRACE
        && symbol.kind() == SyntaxKind::IDENT
    {
        Some(inner_expr.clone())
    } else {
        None
    }
}

/// Whether a call's sole argument is a lone positional expression that owns a
/// non-empty trailing arg list, so the call can hug it with no extra indent.
fn single_arg_is_huggable(arg_list: &SyntaxNode) -> bool {
    let args: Vec<_> = arg_list
        .children()
        .filter(|n| n.kind() == SyntaxKind::ARG)
        .collect();
    let [arg] = args.as_slice() else {
        return false;
    };
    // Named arguments (`name = value`) never hug.
    if arg
        .children_with_tokens()
        .any(|el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::ASSIGN_EQ))
    {
        return false;
    }
    let significant: Vec<_> = arg
        .children_with_tokens()
        .filter(|el| !super::super::core::is_trivia(el.kind()))
        .collect();
    let [NodeOrToken::Node(n)] = significant.as_slice() else {
        return false;
    };
    is_huggable_node(n)
}

/// A call/subset with a non-empty arg list, or a `::`/`$`-style binary whose
/// right side is one — i.e. it ends in a bracketed list that can break.
fn is_huggable_node(node: &SyntaxNode) -> bool {
    match node.kind() {
        SyntaxKind::CALL_EXPR | SyntaxKind::SUBSET_EXPR | SyntaxKind::SUBSET2_EXPR => {
            call_or_subset_has_content(node)
        }
        // A sole function-definition argument hugs the parens with no extra
        // indent and breaks its own params / braces its own body
        // (`fn(function(<long params>) { ... })`).
        SyntaxKind::FUNCTION_EXPR => true,
        // Only a *tight access* binary (`pkg::fn(...)`, `obj$method(...)`) hugs:
        // its space-less operator makes the whole left side read as the callee.
        // A spaced operator binary (`x %in% c(...)`, `a + f(...)`) is a real
        // expression and breaks onto its own argument line instead of hugging.
        SyntaxKind::BINARY_EXPR => {
            binary_op_is_tight_access(node)
                && node
                    .children()
                    .last()
                    .is_some_and(|rhs| is_huggable_node(&rhs))
        }
        _ => false,
    }
}

/// Whether a `BINARY_EXPR`'s operator is a tight, space-less access operator
/// (`::`, `:::`, `$`, `@`) --- the only binaries that hug a trailing arg list.
fn binary_op_is_tight_access(node: &SyntaxNode) -> bool {
    node.children_with_tokens().any(|el| {
        matches!(el, NodeOrToken::Token(t)
        if matches!(
            t.kind(),
            SyntaxKind::COLON2 | SyntaxKind::COLON3 | SyntaxKind::DOLLAR | SyntaxKind::AT
        ))
    })
}

fn call_or_subset_has_content(node: &SyntaxNode) -> bool {
    node.children()
        .find(|c| c.kind() == SyntaxKind::ARG_LIST)
        .is_some_and(|arg_list| {
            arg_list.children_with_tokens().any(|el| match el {
                NodeOrToken::Token(tok) => tok.kind() == SyntaxKind::COMMA,
                NodeOrToken::Node(arg) => {
                    arg.kind() == SyntaxKind::ARG
                        && arg
                            .children_with_tokens()
                            .any(|e| !super::super::core::is_trivia(e.kind()))
                }
            })
        })
}

fn collect_call_ir_slots(
    arg_list: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<(Vec<ArgSlot>, usize), FormatError> {
    let mut slots: Vec<ArgSlot> = Vec::new();
    let mut comma_count = 0usize;
    let mut current: Option<ArgSlot> = None;
    for element in arg_list.children_with_tokens() {
        match element {
            NodeOrToken::Node(arg) if arg.kind() == SyntaxKind::ARG => {
                let arg_elements: Vec<_> = arg.children_with_tokens().collect();
                let significant: Vec<_> = arg_elements
                    .iter()
                    .filter(|el| !super::super::core::is_trivia(el.kind()))
                    .cloned()
                    .collect();
                if significant.is_empty() {
                    continue;
                }
                current = Some(ir_call_argument(&arg_elements, &significant, indent, ctx)?);
            }
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMA => {
                slots.push(current.take().unwrap_or(ArgSlot::Empty));
                comma_count += 1;
            }
            _ => {}
        }
    }
    slots.push(current.take().unwrap_or(ArgSlot::Empty));
    Ok((slots, comma_count))
}

fn ir_call_argument(
    elements: &[SyntaxElement<RLanguage>],
    significant: &[SyntaxElement<RLanguage>],
    indent: usize,
    ctx: FormatContext,
) -> Result<ArgSlot, FormatError> {
    // Named argument `name = value`: in calls these are raw tokens (not an
    // `ASSIGNMENT_EXPR` node as in subsets), so split on `=`. `expr_node` points
    // at the value so a `name = { ... }` arg is still seen as a trailing block.
    if let Some(eq_idx) = elements
        .iter()
        .position(|el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::ASSIGN_EQ))
    {
        let (name_ir, name_empty) =
            ir_arg_side(&elements[..eq_idx], "named arg name", indent, ctx)?;
        let value_elements = &elements[eq_idx + 1..];
        let value_significant: Vec<_> = value_elements
            .iter()
            .filter(|el| !super::super::core::is_trivia(el.kind()))
            .cloned()
            .collect();
        let value_node = single_node(&value_significant);
        let value_ir = if value_significant.is_empty() {
            None
        } else if let Some(curly) = ir_curly_curly(&value_significant, indent, ctx)? {
            Some(curly)
        } else {
            Some(ir_expr_segment(
                value_elements,
                "named arg value",
                indent,
                ctx,
            )?)
        };
        let ends_with_eq = value_significant.is_empty();
        let ir = build_named_arg_ir(name_ir, name_empty, value_ir);
        return Ok(ArgSlot::Expr {
            ir,
            expr_node: value_node,
            ends_with_eq,
        });
    }

    let expr_node = single_node(significant);
    // Curly-curly `{{ x }}` renders natively as its own group.
    if let Some(curly) = ir_curly_curly(significant, indent, ctx)? {
        return Ok(ArgSlot::Expr {
            ir: curly,
            expr_node,
            ends_with_eq: false,
        });
    }
    let ir = ir_expr_segment(elements, "call argument", indent, ctx)?;
    Ok(ArgSlot::Expr {
        ir,
        expr_node,
        ends_with_eq: false,
    })
}

pub(crate) fn single_node(significant: &[SyntaxElement<RLanguage>]) -> Option<SyntaxNode> {
    match significant {
        [NodeOrToken::Node(n)] => Some(n.clone()),
        _ => None,
    }
}

/// Format one side of a named argument; reports whether it was empty.
pub(crate) fn ir_arg_side(
    elements: &[SyntaxElement<RLanguage>],
    context: &'static str,
    indent: usize,
    ctx: FormatContext,
) -> Result<(Ir, bool), FormatError> {
    let has_significant = elements
        .iter()
        .any(|el| !super::super::core::is_trivia(el.kind()));
    if !has_significant {
        return Ok((Ir::nil(), true));
    }
    Ok((ir_expr_segment(elements, context, indent, ctx)?, false))
}

/// `name = value`, with the spacing for the value-less variants
/// (`name = `, `= value`, `=`). A value-less named arg keeps the trailing
/// space after `=` (`fn(NULL = )`), matching air.
pub(crate) fn build_named_arg_ir(name_ir: Ir, name_empty: bool, value_ir: Option<Ir>) -> Ir {
    match (name_empty, value_ir) {
        (false, Some(value)) => Ir::concat([name_ir, Ir::text(" = "), value]),
        (false, None) => Ir::concat([name_ir, Ir::text(" =")]),
        (true, Some(value)) => Ir::concat([Ir::text("= "), value]),
        (true, None) => Ir::text("="),
    }
}

fn build_call_args_ir(slots: &[ArgSlot], force_named_functions: bool) -> Ir {
    let last = slots.len() - 1;
    let first_non_empty = slots.iter().position(|s| !s.is_empty_hole());
    let no_non_empty = first_non_empty.is_none();

    // A positional trailing function-definition argument or block hugs its
    // call: the hug applies only when the whole prefix up to the hugged block's
    // opening `{` fits flat on the line (`callee(leading, function(params) {` or
    // `callee(leading, {`). When it does not fit, the outer call explodes one
    // argument per line rather than letting the prefix overflow or the
    // function's own params break to "rescue" the hug. This makes the hug rule
    // uniform across blocks and trailing functions: line width wins over the
    // hug, and the flat-only `group_hug` measurement enforces it. (Tenet 1: no
    // special-casing of particular callees such as `test_that` to keep an
    // over-width prefix on one line.)
    let leading_ok = !force_named_functions && slots[..last].iter().all(|s| !s.has_forced_break());
    let trailing_function = leading_ok
        && matches!(&slots[last], ArgSlot::Expr { expr_node: Some(node), .. }
            if expr_is_positional_function(node));
    let trailing_block = leading_ok
        && matches!(&slots[last], ArgSlot::Expr { ir, expr_node: Some(node), .. }
            if expr_ends_in_block(node) && ir.contains_forced_break());
    if trailing_function || trailing_block {
        return build_arg_hug(slots, "(", ")", first_non_empty, no_non_empty);
    }

    let force = force_named_functions || should_force_leading_hole_expand(slots, first_non_empty);
    build_arg_group(slots, "(", ")", first_non_empty, no_non_empty, force)
}

// ===================== Native IR call comment relocation =====================
//
// When an arg list owns comments, the layout is *always* broken: the flat and
// hug forms never apply. `ir_call_args_with_comments` walks a flat item stream
// (`Arg` / `Comma`, the IR port of `collect_call_items`) and decides, per
// comment, whether it trails the previous argument's last line, leads the next
// element on its own line, or stands alone — reproducing `format_arg_list_multiline`
// while emitting real IR for every argument expression.

/// One formatted argument in the comment-aware item stream.
struct IrCallArg {
    /// The argument's IR (for a comment-only arg, just the comment text).
    ir: Ir,
    /// An empty hole, e.g. the gaps in `f(, a)`.
    is_empty: bool,
    /// A comment-only arg (`# note` with no expression).
    is_comment_only: bool,
    /// The raw comment text, for a comment-only arg.
    comment_text: String,
    /// Whether a source newline precedes this arg (distinguishes a comment that
    /// trails the previous line from one that leads on its own line).
    leading_newline: bool,
    /// Whether the rendered argument ends in `=` (a value-less named arg), which
    /// changes the comma/comment separator (` ,` / ` , ` rather than `,` / `, `).
    ends_with_eq: bool,
}

enum IrCallItem {
    Arg(IrCallArg),
    Comma { newline_after: bool },
}

fn ir_call_args_with_comments(
    arg_list: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    let items = collect_call_comment_items(arg_list, indent, ctx)?;
    // A leading run of bare holes packs onto the open-paren line (`fn(,,`)
    // rather than each comma taking its own line; only an interior hole (one
    // preceded by a real arg or comment) breaks. Mirrors `leading_empty_run`
    // in the comment-free path.
    let (lead_commas, rest) = split_leading_holes(&items);
    let lead = Ir::text(",".repeat(lead_commas));
    let lines = layout_call_comment_items(rest);
    if lines.is_empty() {
        return Ok(Ir::concat([Ir::text("("), lead, Ir::text(")")]));
    }
    Ok(Ir::concat([
        Ir::text("("),
        lead,
        Ir::indent(Ir::concat([
            Ir::hard_line(),
            Ir::join(Ir::hard_line(), lines),
        ])),
        Ir::hard_line(),
        Ir::text(")"),
    ]))
}

/// Peels the leading run of bare empty holes (each an `Arg(empty)` immediately
/// followed by a `Comma`) off the front of the item stream. Returns the number
/// of commas peeled (to pack after the open paren) and the remaining items. An
/// interior hole, or one carrying a comment, is left in place to break normally.
fn split_leading_holes(items: &[IrCallItem]) -> (usize, &[IrCallItem]) {
    let mut commas = 0usize;
    let mut rest = items;
    while let [IrCallItem::Arg(arg), IrCallItem::Comma { .. }, ..] = rest
        && arg.is_empty
    {
        commas += 1;
        rest = &rest[2..];
    }
    (commas, rest)
}

fn collect_call_comment_items(
    arg_list: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<Vec<IrCallItem>, FormatError> {
    let elements: Vec<_> = arg_list.children_with_tokens().collect();
    let mut items = Vec::new();
    for (idx, element) in elements.iter().enumerate() {
        match element {
            NodeOrToken::Node(arg) if arg.kind() == SyntaxKind::ARG => {
                let mut arg_info = ir_call_comment_arg(arg, indent, ctx)?;
                arg_info.leading_newline = has_newline_before_arg(&elements, idx);
                items.push(IrCallItem::Arg(arg_info));
            }
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMA => {
                items.push(IrCallItem::Comma {
                    newline_after: comma_followed_by_newline(&elements, idx),
                });
            }
            _ => {}
        }
    }
    Ok(items)
}

/// Whether a comma at `idx` is followed by a newline before the next argument or
/// comma. Mirrors the `newline_after` scan in `collect_call_items`.
fn comma_followed_by_newline(elements: &[SyntaxElement<RLanguage>], idx: usize) -> bool {
    for next in elements.iter().skip(idx + 1) {
        match next {
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::NEWLINE => return true,
            NodeOrToken::Token(tok)
                if tok.kind() == SyntaxKind::WHITESPACE || tok.kind() == SyntaxKind::COMMENT => {}
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::ARG => return false,
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMA => return false,
            _ => return false,
        }
    }
    false
}

/// IR port of [`format_arg`] for the comment layout: classifies the arg and
/// builds its IR (with any leading/internal comments lifted onto their own
/// lines).
fn ir_call_comment_arg(
    arg: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<IrCallArg, FormatError> {
    let elements: Vec<_> = arg.children_with_tokens().collect();
    let significant: Vec<_> = elements
        .iter()
        .filter(|el| !super::super::core::is_trivia(el.kind()))
        .cloned()
        .collect();
    if significant.is_empty() {
        return Ok(IrCallArg {
            ir: Ir::nil(),
            is_empty: true,
            is_comment_only: false,
            comment_text: String::new(),
            leading_newline: false,
            ends_with_eq: false,
        });
    }
    if let [NodeOrToken::Token(tok)] = significant.as_slice()
        && tok.kind() == SyntaxKind::COMMENT
    {
        let text = tok.text().to_string();
        return Ok(IrCallArg {
            ir: Ir::text(text.clone()),
            is_empty: false,
            is_comment_only: true,
            comment_text: text,
            leading_newline: false,
            ends_with_eq: false,
        });
    }

    let (ir, ends_with_eq) = ir_call_arg_value(&elements, &significant, indent, ctx)?;
    Ok(IrCallArg {
        ir,
        is_empty: false,
        is_comment_only: false,
        comment_text: String::new(),
        leading_newline: false,
        ends_with_eq,
    })
}

/// Build the IR for a non-comment-only argument value, plus whether it ends in
/// `=`. Mirrors the curly-curly / named-arg / positional branches of
/// [`format_arg`], lifting any leading/around-`=` comments onto their own lines.
fn ir_call_arg_value(
    elements: &[SyntaxElement<RLanguage>],
    significant: &[SyntaxElement<RLanguage>],
    indent: usize,
    ctx: FormatContext,
) -> Result<(Ir, bool), FormatError> {
    if let Some(curly) = ir_curly_curly_with_comments(significant, indent, ctx)? {
        return Ok((curly, false));
    }

    if let Some(eq_idx) = elements
        .iter()
        .position(|el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::ASSIGN_EQ))
    {
        let lhs_comments: Vec<String> = elements[..eq_idx]
            .iter()
            .filter_map(comment_text_of)
            .collect();
        let lhs_significant: Vec<_> = elements[..eq_idx]
            .iter()
            .filter(|el| {
                !super::super::core::is_trivia(el.kind()) && el.kind() != SyntaxKind::COMMENT
            })
            .cloned()
            .collect();
        let name_empty = lhs_significant.is_empty();
        let name_ir = if name_empty {
            Ir::nil()
        } else {
            ir_expr_segment(&lhs_significant, "named arg name", indent, ctx)?
        };
        let (rhs_comments, value_ir) =
            ir_rhs_with_leading_comments(&elements[eq_idx + 1..], indent, ctx)?;
        let value_empty = value_ir.is_none();
        let base = build_named_arg_ir(name_ir, name_empty, value_ir);
        let mut comments = lhs_comments;
        comments.extend(rhs_comments);
        return Ok((prepend_comment_lines(&comments, base), value_empty));
    }

    let ir = ir_expr_with_optional_comment(elements, "positional arg", indent, ctx)?;
    Ok((ir, false))
}

/// IR port of [`format_assignment_rhs_with_leading_comments`]: peel any leading
/// comments off the value, then build the (optional) value IR with a trailing
/// comment honored.
fn ir_rhs_with_leading_comments(
    elements: &[SyntaxElement<RLanguage>],
    indent: usize,
    ctx: FormatContext,
) -> Result<(Vec<String>, Option<Ir>), FormatError> {
    let mut idx = 0usize;
    let mut leading = Vec::new();
    while idx < elements.len() {
        match &elements[idx] {
            NodeOrToken::Token(tok) if super::super::core::is_trivia(tok.kind()) => idx += 1,
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT => {
                leading.push(tok.text().to_string());
                idx += 1;
            }
            _ => break,
        }
    }
    if idx >= elements.len() {
        return Ok((leading, None));
    }
    // A curly-curly `{{ x }}` value is lifted natively, matching the no-comment
    // named-arg path (`ir_call_argument`); other values render as one expression
    // with an optional trailing comment.
    let value_significant: Vec<_> = elements[idx..]
        .iter()
        .filter(|el| !super::super::core::is_trivia(el.kind()))
        .cloned()
        .collect();
    let value = if let Some(curly) = ir_curly_curly_with_comments(&value_significant, indent, ctx)?
    {
        curly
    } else {
        ir_expr_with_optional_comment(&elements[idx..], "assignment rhs", indent, ctx)?
    };
    Ok((leading, Some(value)))
}

fn comment_text_of(el: &SyntaxElement<RLanguage>) -> Option<String> {
    match el {
        NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT => {
            Some(tok.text().to_string())
        }
        _ => None,
    }
}

/// Prefix `base` with one own-line comment per entry (`# c\n# d\nbase`).
fn prepend_comment_lines(comments: &[String], base: Ir) -> Ir {
    if comments.is_empty() {
        return base;
    }
    let mut parts: Vec<Ir> = Vec::new();
    for comment in comments {
        parts.push(Ir::verbatim_forced(comment.clone()));
        parts.push(Ir::hard_line());
    }
    parts.push(base);
    Ir::concat(parts)
}

/// IR port of [`format_arg_list_multiline`]: turn the item stream into one IR per
/// output line, deciding comment placement. Always emits a fully-broken list.
fn layout_call_comment_items(items: &[IrCallItem]) -> Vec<Ir> {
    let mut out: Vec<Ir> = Vec::new();
    let mut i = 0usize;
    while i < items.len() {
        match &items[i] {
            IrCallItem::Arg(arg) if arg.is_empty => {
                i += 1;
            }
            IrCallItem::Arg(arg) if arg.is_comment_only => {
                out.push(Ir::verbatim_forced(arg.comment_text.clone()));
                i += 1;
            }
            IrCallItem::Arg(arg) => {
                // A comment-only arg directly after this one (no comma between)
                // and on the same source line trails the argument's last line.
                if let Some(IrCallItem::Arg(comment_arg)) = items.get(i + 1)
                    && comment_arg.is_comment_only
                    && !comment_arg.leading_newline
                {
                    out.push(Ir::concat([
                        arg.ir.clone(),
                        Ir::text(" "),
                        Ir::text(comment_arg.comment_text.clone()),
                    ]));
                    i += 2;
                    continue;
                }

                // `arg, # comment` — the comment shares the comma's line. Any
                // further comment-only args sit on their own lines at the base
                // argument indent (air does not align them under the trailing
                // comment).
                if let (
                    Some(IrCallItem::Comma {
                        newline_after: false,
                    }),
                    Some(IrCallItem::Arg(comment_arg)),
                ) = (items.get(i + 1), items.get(i + 2))
                    && comment_arg.is_comment_only
                {
                    let sep = if arg.ends_with_eq { " , " } else { ", " };
                    out.push(Ir::concat([
                        arg.ir.clone(),
                        Ir::text(sep),
                        Ir::text(comment_arg.comment_text.clone()),
                    ]));
                    i += 3;
                    while let Some(IrCallItem::Arg(extra)) = items.get(i) {
                        if !extra.is_comment_only {
                            break;
                        }
                        out.push(Ir::text(extra.comment_text.clone()));
                        i += 1;
                    }
                    continue;
                }

                if matches!(items.get(i + 1), Some(IrCallItem::Comma { .. })) {
                    out.push(Ir::concat([
                        arg.ir.clone(),
                        Ir::text(if arg.ends_with_eq { " ," } else { "," }),
                    ]));
                    i += 2;
                } else {
                    out.push(arg.ir.clone());
                    i += 1;
                }
            }
            IrCallItem::Comma { .. } => {
                out.push(Ir::text(","));
                i += 1;
            }
        }
    }
    out
}

/// IR port of [`try_format_curly_curly`]. Returns `Some` when `significant` is a
/// curly-curly `{{ symbol }}` wrapper; comments are lifted out and placed
/// (leading the `{{`, leading/trailing the symbol, trailing the `}}`, or below
/// it) exactly as the legacy renderer does. With no comments, defers to the flat
/// [`ir_curly_curly`] group. Returns `None` for any non-curly-curly shape.
fn ir_curly_curly_with_comments(
    significant: &[SyntaxElement<RLanguage>],
    indent: usize,
    ctx: FormatContext,
) -> Result<Option<Ir>, FormatError> {
    let [NodeOrToken::Node(outer)] = significant else {
        return Ok(None);
    };
    if outer.kind() != SyntaxKind::BLOCK_EXPR {
        return Ok(None);
    }
    let outer_elements: Vec<_> = outer.children_with_tokens().collect();
    let outer_significant: Vec<_> = outer_elements
        .iter()
        .filter(|el| !super::super::core::is_trivia(el.kind()))
        .cloned()
        .collect();
    let (Some(NodeOrToken::Token(outer_l)), Some(NodeOrToken::Token(outer_r))) =
        (outer_significant.first(), outer_significant.last())
    else {
        return Ok(None);
    };
    if outer_l.kind() != SyntaxKind::LBRACE || outer_r.kind() != SyntaxKind::RBRACE {
        return Ok(None);
    }

    let mut inner_node = None::<SyntaxNode>;
    let mut outer_leading_comments = Vec::new();
    for element in outer_significant
        .iter()
        .skip(1)
        .take(outer_significant.len().saturating_sub(2))
    {
        match element {
            NodeOrToken::Node(node) if node.kind() == SyntaxKind::BLOCK_EXPR => {
                if inner_node.is_some() {
                    return Ok(None);
                }
                inner_node = Some(node.clone());
            }
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT => {
                if inner_node.is_some() {
                    break;
                }
                outer_leading_comments.push(tok.text().to_string());
            }
            _ => return Ok(None),
        }
    }
    let Some(inner) = inner_node else {
        return Ok(None);
    };

    let inner_significant: Vec<_> = inner
        .children_with_tokens()
        .filter(|el| !super::super::core::is_trivia(el.kind()))
        .collect();
    if inner_significant.len() < 3 {
        return Ok(None);
    }
    let (Some(NodeOrToken::Token(inner_l)), Some(NodeOrToken::Token(inner_r))) =
        (inner_significant.first(), inner_significant.last())
    else {
        return Ok(None);
    };
    if inner_l.kind() != SyntaxKind::LBRACE || inner_r.kind() != SyntaxKind::RBRACE {
        return Ok(None);
    }

    let inner_payload = &inner_significant[1..inner_significant.len() - 1];
    let expr_count = inner_payload
        .iter()
        .filter(|el| !matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT))
        .count();
    if expr_count != 1 {
        return Ok(None);
    }

    let mut inner_pre_comments = Vec::new();
    let mut inner_after_comments = Vec::new();
    let mut inner_expr = None::<SyntaxElement<RLanguage>>;
    for element in inner_payload {
        match element {
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT => {
                if inner_expr.is_some() {
                    inner_after_comments.push(tok.text().to_string());
                } else {
                    inner_pre_comments.push(tok.text().to_string());
                }
            }
            _ => {
                if inner_expr.is_some() {
                    return Ok(None);
                }
                inner_expr = Some(element.clone());
            }
        }
    }
    let Some(inner_expr) = inner_expr else {
        return Ok(None);
    };
    if !matches!(&inner_expr, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::IDENT) {
        return Ok(None);
    }

    let inner_idx = outer_elements
        .iter()
        .position(
            |el| matches!(el, NodeOrToken::Node(node) if node.kind() == SyntaxKind::BLOCK_EXPR),
        )
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing inner block for curly-curly",
            snippet: outer.text().to_string(),
        })?;
    let mut inline_trailing = None::<String>;
    let mut outer_post_comments = Vec::new();
    let mut saw_newline = false;
    for element in outer_elements.iter().skip(inner_idx + 1) {
        match element {
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::NEWLINE => saw_newline = true,
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::WHITESPACE => {}
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT => {
                if !saw_newline && inline_trailing.is_none() {
                    inline_trailing = Some(tok.text().to_string());
                } else {
                    outer_post_comments.push(tok.text().to_string());
                }
            }
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::RBRACE => break,
            _ => return Ok(None),
        }
    }

    // No comments anywhere: defer to the flat/group curly-curly form.
    if outer_leading_comments.is_empty()
        && inner_pre_comments.is_empty()
        && inner_after_comments.is_empty()
        && outer_post_comments.is_empty()
        && inline_trailing.is_none()
    {
        return ir_curly_curly(significant, indent, ctx);
    }

    let expr_ir = ir_expr_element(&inner_expr, indent + 1, ctx)?;
    let mut parts: Vec<Ir> = Vec::new();
    for comment in &outer_leading_comments {
        parts.push(Ir::verbatim_forced(comment.clone()));
        parts.push(Ir::hard_line());
    }
    parts.push(Ir::text("{{"));
    let mut body: Vec<Ir> = Vec::new();
    for comment in &inner_pre_comments {
        body.push(Ir::hard_line());
        body.push(Ir::verbatim_forced(comment.clone()));
    }
    body.push(Ir::hard_line());
    body.push(expr_ir);
    for comment in &inner_after_comments {
        body.push(Ir::hard_line());
        body.push(Ir::verbatim_forced(comment.clone()));
    }
    parts.push(Ir::indent(Ir::concat(body)));
    parts.push(Ir::hard_line());
    parts.push(Ir::text("}}"));
    if let Some(comment) = inline_trailing {
        parts.push(Ir::text(" "));
        parts.push(Ir::text(comment));
    }
    for comment in &outer_post_comments {
        parts.push(Ir::hard_line());
        parts.push(Ir::verbatim_forced(comment.clone()));
    }
    Ok(Some(Ir::concat(parts)))
}

fn has_newline_before_arg(elements: &[SyntaxElement<RLanguage>], idx: usize) -> bool {
    for prev in elements[..idx].iter().rev() {
        match prev {
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::NEWLINE => return true,
            NodeOrToken::Token(tok)
                if tok.kind() == SyntaxKind::WHITESPACE || tok.kind() == SyntaxKind::COMMENT => {}
            NodeOrToken::Token(tok)
                if tok.kind() == SyntaxKind::COMMA || tok.kind() == SyntaxKind::LPAREN =>
            {
                return false;
            }
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::ARG => return false,
            _ => return false,
        }
    }
    false
}

pub(crate) fn ir_function_expr(
    node: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    let elements: Vec<_> = node.children_with_tokens().collect();
    let fn_idx = elements
        .iter()
        .position(
            |el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::FUNCTION_KW),
        )
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing 'function' keyword",
            snippet: node.text().to_string(),
        })?;
    let head = match &elements[fn_idx] {
        NodeOrToken::Token(tok) if tok.text() == "\\" => "\\",
        _ => "function",
    };
    let lparen_idx = elements
        .iter()
        .enumerate()
        .skip(fn_idx + 1)
        .find_map(|(i, el)| match el {
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::LPAREN => Some(i),
            _ => None,
        })
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing '(' in function expression",
            snippet: node.text().to_string(),
        })?;
    let mut depth = 0;
    let rparen_idx = elements
        .iter()
        .enumerate()
        .skip(lparen_idx)
        .find_map(|(i, el)| match el {
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::LPAREN => {
                depth += 1;
                None
            }
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::RPAREN => {
                depth -= 1;
                if depth == 0 { Some(i) } else { None }
            }
            _ => None,
        })
        .ok_or_else(|| FormatError::AmbiguousConstruct {
            context: "missing ')' in function expression",
            snippet: node.text().to_string(),
        })?;

    let param_elements = &elements[lparen_idx + 1..rparen_idx];
    let body_elements = &elements[rparen_idx + 1..];

    // Comments are relocated natively: a comment before `(` is hoisted above the
    // whole definition; comments inside `()` keep the param list broken; a comment
    // between `)` and the body is lifted into (or braces) the body. With any such
    // comment the definition stays broken — the bare/flat inline form never applies.
    let leading_fn_comments: Vec<String> = elements[fn_idx + 1..lparen_idx]
        .iter()
        .filter_map(comment_text_of)
        .collect();
    let param_has_comment = param_elements
        .iter()
        .any(|el| el.kind() == SyntaxKind::COMMENT);

    let params_ir = if param_has_comment {
        ir_function_params_with_comments(param_elements)
    } else {
        ir_function_params(param_elements, indent, ctx)?
    };

    // Peel leading comments (between `)` and the body) off the body core.
    let mut body_leading_comments = Vec::new();
    let mut body_start = 0usize;
    while body_start < body_elements.len() {
        match &body_elements[body_start] {
            NodeOrToken::Token(tok) if super::super::core::is_trivia(tok.kind()) => body_start += 1,
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMENT => {
                body_leading_comments.push(tok.text().to_string());
                body_start += 1;
            }
            _ => break,
        }
    }
    let body_core: Vec<_> = body_elements[body_start..]
        .iter()
        .filter(|el| !super::super::core::is_trivia(el.kind()))
        .cloned()
        .collect();
    if body_core.is_empty() {
        return Err(FormatError::AmbiguousConstruct {
            context: "missing function body expression",
            snippet: node.text().to_string(),
        });
    }

    let head_ir = Ir::text(head);
    let body_node = single_node(&body_core);

    let core = if let Some(block) = body_node
        .as_ref()
        .filter(|n| n.kind() == SyntaxKind::BLOCK_EXPR)
    {
        let block_ir =
            ir_block_expr_with_prefixed_comments(block, indent, ctx, &body_leading_comments)?;
        // A curly-curly `{{ x }}` body stays inline when the whole definition
        // fits flat (`function(x) {{ x }}`), otherwise it expands to nested
        // blocks like any other. This only applies when the function is itself a
        // call argument — the embrace is recognised in call-argument position
        // only (a bare `{{ x }}` body, an assignment RHS, etc. expand as ordinary
        // nested blocks), matching air and arity's direct `{{ x }}` arg handling.
        // The flat curly-curly is the bare form, the nested-block `block_ir` the
        // braced fallback.
        let in_call_arg = node.parent().is_some_and(|p| p.kind() == SyntaxKind::ARG);
        let curly_inline = if in_call_arg && !param_has_comment && body_leading_comments.is_empty()
        {
            ir_curly_curly(&body_core, indent, ctx)?
        } else {
            None
        };
        if let Some(curly) = curly_inline {
            function_body_choice(head_ir, params_ir, curly, block_ir)
        }
        // A flattenable block (`function(p) { stmt }` → `function(p) stmt`) only
        // applies with no comments forcing the layout open *and* when the
        // single statement renders flat — a multi-line inner statement (a
        // nested block, an `if` with braced arms) keeps the outer braces, so
        // the user sees the structure they wrote.
        else if !param_has_comment
            && body_leading_comments.is_empty()
            && let Some(stmt_ir) = try_flatten_function_block(block, indent, ctx)?
            && !stmt_ir.contains_forced_break()
        {
            function_body_choice(head_ir, params_ir, stmt_ir, block_ir)
        } else {
            function_braced_hug(head_ir, params_ir, block_ir)
        }
    } else {
        // Bare (non-block) body. Native IR builders are insensitive to the
        // build-time `indent` parameter (they lay out via `Ir::Indent`), so one
        // build serves both the bare splice and the braced fallback: the brace
        // wrapper supplies its own `Ir::indent`, re-indenting the body
        // structurally. The bare/braced choice routes through
        // `function_body_choice`, which measures all lines for forced-break
        // bodies (the IR port of legacy `fits_with_newlines`).
        let body_ir = ir_expr_segment(&body_core, "function body", indent, ctx)?;
        if !param_has_comment && body_leading_comments.is_empty() {
            function_body_choice(
                head_ir,
                params_ir,
                body_ir.clone(),
                brace_wrap_body(body_ir),
            )
        } else {
            function_braced_hug(
                head_ir,
                params_ir,
                brace_wrap_body_with_comments(body_ir, &body_leading_comments),
            )
        }
    };

    Ok(prepend_comment_lines(&leading_fn_comments, core))
}

/// Wrap a bare body in a block, prefixing `comments` on their own lines before
/// the body (`{`, `# c`, …, body, `}`). With no comments this is
/// [`brace_wrap_body`].
fn brace_wrap_body_with_comments(body: Ir, comments: &[String]) -> Ir {
    let mut inner: Vec<Ir> = Vec::new();
    for comment in comments {
        inner.push(Ir::hard_line());
        inner.push(Ir::verbatim_forced(comment.clone()));
    }
    inner.push(Ir::hard_line());
    inner.push(body);
    Ir::concat([
        Ir::text("{"),
        Ir::indent(Ir::break_body(Ir::concat(inner))),
        Ir::hard_line(),
        Ir::text("}"),
    ])
}

/// IR port of [`format_function_parameters`]'s comment branch: with a comment in
/// the param list, emit each comma-delimited segment's raw (trimmed) lines one
/// per line, a comma after the last line of every non-final segment. The list is
/// always broken.
fn ir_function_params_with_comments(param_elements: &[SyntaxElement<RLanguage>]) -> Ir {
    let segments = split_top_level_function_params(param_elements);
    if segments.is_empty() {
        return Ir::text("()");
    }
    let mut lines: Vec<Ir> = Vec::new();
    for (idx, segment) in segments.iter().enumerate() {
        let raw = snippet_from_elements(segment);
        let seg_lines: Vec<&str> = raw
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        let last = seg_lines.len();
        for (line_idx, line) in seg_lines.iter().enumerate() {
            let add_comma = idx + 1 < segments.len() && line_idx + 1 == last;
            lines.push(Ir::text(if add_comma {
                format!("{line},")
            } else {
                (*line).to_string()
            }));
        }
    }
    Ir::concat([
        Ir::text("("),
        Ir::indent(Ir::concat([
            Ir::hard_line(),
            Ir::join(Ir::hard_line(), lines),
        ])),
        Ir::hard_line(),
        Ir::text(")"),
    ])
}

/// The conditional choice between bare inline body and braced-block hug.
/// Two selectors, matched to the bare body's shape:
///
/// * **No forced break in the bare body** — use a single-pass group: bare
///   wins exactly when `head(params) bare_body` fits flat as one line,
///   otherwise the braced hug takes over (matching the legacy
///   `Ir::group(Ir::if_break(...))` shape from before commit 410dd48).
/// * **Forced break in the bare body** (control flow with braced arms) —
///   route through [`Ir::conditional_group_all_lines`]: the bare form wins
///   when every rendered line fits, mirroring legacy's `fits_with_newlines`
///   check.
fn function_body_choice(head: Ir, params: Ir, bare_body: Ir, braced_body: Ir) -> Ir {
    if bare_body.contains_forced_break() {
        // A multi-line control-flow body (`function(p) if (cond) {…}`,
        // `function(p) for (…) {…}`, …) always gets its own braces, matching
        // air: the function body group breaks, so the body is wrapped rather
        // than hugged onto the signature line.
        function_braced_hug(head, params, braced_body)
    } else {
        let bare = Ir::concat([head.clone(), params.clone(), Ir::text(" "), bare_body]);
        let braced = function_braced_hug(head, params, braced_body);
        Ir::group(Ir::if_break(bare, braced))
    }
}

/// `head(params) <block>` as a hug group: the param list stays inline as long as
/// `head(params) {` fits (the printer's fit measurement stops at the block's
/// opening brace), otherwise the params break one per line. The block's own hard
/// breaks lay out its body regardless.
fn function_braced_hug(head: Ir, params: Ir, block: Ir) -> Ir {
    Ir::group_hug(Ir::concat([head, params, Ir::text(" "), block]))
}

/// Wrap a bare body expression in a block on its own indented line.
fn brace_wrap_body(body: Ir) -> Ir {
    Ir::concat([
        Ir::text("{"),
        Ir::indent(Ir::break_body(Ir::concat([Ir::hard_line(), body]))),
        Ir::hard_line(),
        Ir::text("}"),
    ])
}

/// Build the `( ... )` param list as a bare concat (no enclosing group): empty
/// params collapse to `()`, otherwise the params are soft-line separated inside
/// an indent so an enclosing group can lay them inline or one per line. The
/// caller wraps this in the group that owns the break decision.
///
/// One exception forces the list to break: a brace-token default whose inner
/// expression is itself a literal block (`function(a = {{ var }})`). Legacy
/// triggers this on the formatted string (`param.contains("= {\n  {\n")`);
/// here we detect the same shape at the token level and wrap the params in
/// [`Ir::group_expanded`] so the brace default's native IR (see
/// [`ir_brace_token_default`]) renders at `indent + 1` and the nested `{` lines
/// up correctly.
fn ir_function_params(
    param_elements: &[SyntaxElement<RLanguage>],
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    let significant: Vec<_> = param_elements
        .iter()
        .filter(|el| !super::super::core::is_trivia(el.kind()))
        .cloned()
        .collect();
    if significant.is_empty() {
        return Ok(Ir::text("()"));
    }

    let segments = split_function_param_segments(&significant)?;
    // A brace-block default (`a = { … }`) is always rendered multi-line, so the
    // whole parameter list breaks one-per-line rather than hugging the brace
    // onto the signature line (`function(\n  a = {\n    1\n  },\n  b\n)`). This
    // is the parameter-list analogue of the call trailing-block hug rule: a
    // forced-break element never keeps the rest of the list flat. The exploded
    // params render at `indent + 1`.
    let brace_default = segments
        .iter()
        .any(|seg| param_has_breaking_brace_default(seg));
    let param_indent = if brace_default { indent + 1 } else { indent };
    let mut params: Vec<Ir> = Vec::with_capacity(segments.len());
    for param in &segments {
        params.push(ir_function_parameter(param, param_indent, ctx)?);
    }

    let mut body: Vec<Ir> = Vec::new();
    for (idx, param) in params.into_iter().enumerate() {
        if idx > 0 {
            body.push(Ir::if_break(Ir::text(", "), Ir::text(",")));
        }
        body.push(Ir::soft_line());
        body.push(param);
    }
    let inner = Ir::concat([
        Ir::text("("),
        Ir::indent(Ir::concat(body)),
        Ir::soft_line(),
        Ir::text(")"),
    ]);
    if brace_default {
        Ok(Ir::group_expanded(inner))
    } else {
        Ok(inner)
    }
}

/// A param whose default is a non-empty brace block (`a = { … }`). Such a
/// default is always rendered multi-line (an empty `{}` is not — it stays
/// inline), so its presence forces the whole parameter list to expand.
fn param_has_breaking_brace_default(param: &[SyntaxElement<RLanguage>]) -> bool {
    let Some(eq_idx) = param
        .iter()
        .position(|el| matches!(el, NodeOrToken::Token(t) if t.kind() == SyntaxKind::ASSIGN_EQ))
    else {
        return false;
    };
    let default_significant: Vec<_> = param[eq_idx + 1..]
        .iter()
        .filter(|el| !super::super::core::is_trivia(el.kind()))
        .collect();
    // The default is parsed as a single expression node; only a `BLOCK_EXPR`
    // with inner content breaks. An empty `{}` stays inline.
    matches!(
        default_significant.as_slice(),
        [NodeOrToken::Node(node)]
            if node.kind() == SyntaxKind::BLOCK_EXPR && block_default_breaks(node)
    )
}

/// Whether a `{ … }` default renders multi-line: true unless it is an empty
/// `{}`. Any inner expression or comment (even a dangling one) forces a break.
fn block_default_breaks(block: &SyntaxNode) -> bool {
    block.children_with_tokens().any(|el| {
        !matches!(
            el.kind(),
            SyntaxKind::LBRACE | SyntaxKind::RBRACE | SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE
        )
    })
}

fn ir_function_parameter(
    param: &[SyntaxElement<RLanguage>],
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    if let Some(eq_idx) = param
        .iter()
        .position(|el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::ASSIGN_EQ))
    {
        let name = ir_expr_segment(&param[..eq_idx], "function parameter name", indent, ctx)?;
        let value = ir_function_param_default(&param[eq_idx + 1..], indent, ctx)?;
        return Ok(Ir::concat([name, Ir::text(" = "), value]));
    }
    ir_expr_segment(param, "function parameter", indent, ctx)
}

/// Render a parameter default. The parser parses each default value into a
/// single expression node (or a lone atom token), so the common case is exactly
/// one element, formatted in value position — an `if`/call/block default lays
/// out just like the same expression anywhere else. A `{ … }` default is a
/// `BLOCK_EXPR` node handled by the same path; the surrounding param list
/// expands when such a default is present (see [`param_has_breaking_brace_default`]).
///
/// A lenient parse can still leave more than one element (e.g. the invalid
/// `function(a = 1 2)`); reparse that run space-joined (so adjacent tokens keep
/// their boundaries) and format the single resulting expression.
fn ir_function_param_default(
    elements: &[SyntaxElement<RLanguage>],
    indent: usize,
    ctx: FormatContext,
) -> Result<Ir, FormatError> {
    if let [only] = elements {
        return ir_expr_element(only, indent, ctx);
    }
    let snippet = reparse_snippet_from_elements(elements);
    let parsed = parse(&snippet);
    if !parsed.diagnostics.is_empty() {
        return Err(FormatError::AmbiguousConstruct {
            context: "function parameter default",
            snippet,
        });
    }
    let reparsed: Vec<_> = parsed
        .cst
        .children_with_tokens()
        .filter(|el| !super::super::core::is_trivia(el.kind()))
        .collect();
    let [only] = reparsed.as_slice() else {
        return Err(FormatError::AmbiguousConstruct {
            context: "function parameter default",
            snippet,
        });
    };
    ir_expr_element(only, indent, ctx)
}

/// Split params on top-level commas, preserving the legacy splitter's errors on
/// an empty parameter or a trailing comma.
fn split_function_param_segments(
    significant: &[SyntaxElement<RLanguage>],
) -> Result<Vec<Vec<SyntaxElement<RLanguage>>>, FormatError> {
    let mut params: Vec<Vec<SyntaxElement<RLanguage>>> = Vec::new();
    let mut current: Vec<SyntaxElement<RLanguage>> = Vec::new();
    let mut depth = 0usize;
    for element in significant {
        match element {
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMA && depth == 0 => {
                if current.is_empty() {
                    return Err(FormatError::AmbiguousConstruct {
                        context: "empty function parameter",
                        snippet: tok.text().to_string(),
                    });
                }
                params.push(std::mem::take(&mut current));
            }
            NodeOrToken::Token(tok)
                if matches!(
                    tok.kind(),
                    SyntaxKind::LPAREN
                        | SyntaxKind::LBRACE
                        | SyntaxKind::LBRACK
                        | SyntaxKind::LBRACK2
                ) =>
            {
                depth += 1;
                current.push(element.clone());
            }
            NodeOrToken::Token(tok)
                if matches!(
                    tok.kind(),
                    SyntaxKind::RPAREN
                        | SyntaxKind::RBRACE
                        | SyntaxKind::RBRACK
                        | SyntaxKind::RBRACK2
                ) =>
            {
                depth = depth.saturating_sub(1);
                current.push(element.clone());
            }
            _ => current.push(element.clone()),
        }
    }
    if current.is_empty() {
        return Err(FormatError::AmbiguousConstruct {
            context: "trailing comma in function parameters",
            snippet: snippet_from_elements(significant),
        });
    }
    params.push(current);
    Ok(params)
}

/// When a block body is exactly one comment-free statement, return that
/// statement's IR so the printer can flatten `function(p) { stmt }` to
/// `function(p) stmt` when it fits; otherwise `None` (keep it braced).
fn try_flatten_function_block(
    block: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Result<Option<Ir>, FormatError> {
    // A comment in the body keeps the braces (bare `function(p) stmt # c` would
    // dangle the comment). A mid-line `#'` parses as a `ROXYGEN_BLOCK` rather
    // than a `COMMENT` token but is a comment all the same, so it keeps the
    // braces too, matching the plain-comment case.
    if block
        .descendants_with_tokens()
        .any(|el| el.kind() == SyntaxKind::COMMENT || el.kind() == SyntaxKind::ROXYGEN_BLOCK)
    {
        return Ok(None);
    }
    let elements: Vec<_> = block.children_with_tokens().collect();
    let Some(open_idx) = elements
        .iter()
        .position(|el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::LBRACE))
    else {
        return Ok(None);
    };
    let Some(close_idx) = elements
        .iter()
        .rposition(|el| matches!(el, NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::RBRACE))
    else {
        return Ok(None);
    };
    if close_idx <= open_idx {
        return Ok(None);
    }
    let lines = split_lines(
        elements[open_idx + 1..close_idx].to_vec(),
        "function body block",
    )?;
    let mut stmt: Option<Ir> = None;
    for line in &lines {
        let ir = ir_line(line, indent, ctx)?;
        if matches!(ir, Ir::Nil) {
            continue;
        }
        if stmt.is_some() {
            return Ok(None);
        }
        stmt = Some(ir);
    }
    Ok(stmt)
}

fn split_top_level_function_params(
    elements: &[SyntaxElement<RLanguage>],
) -> Vec<Vec<SyntaxElement<RLanguage>>> {
    let mut segments = Vec::new();
    let mut current = Vec::new();
    let mut depth = 0usize;

    for element in elements {
        match element {
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMA && depth == 0 => {
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
            }
            NodeOrToken::Token(tok)
                if matches!(
                    tok.kind(),
                    SyntaxKind::LPAREN
                        | SyntaxKind::LBRACE
                        | SyntaxKind::LBRACK
                        | SyntaxKind::LBRACK2
                ) =>
            {
                depth += 1;
                current.push(element.clone());
            }
            NodeOrToken::Token(tok)
                if matches!(
                    tok.kind(),
                    SyntaxKind::RPAREN
                        | SyntaxKind::RBRACE
                        | SyntaxKind::RBRACK
                        | SyntaxKind::RBRACK2
                ) =>
            {
                depth = depth.saturating_sub(1);
                current.push(element.clone());
            }
            _ => current.push(element.clone()),
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}
