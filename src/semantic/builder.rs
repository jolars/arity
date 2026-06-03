//! Build a [`SemanticModel`] from a parsed file root.
//!
//! The builder walks the CST once, maintaining a stack of open scopes. At each
//! node it decides whether to:
//! - Push a new scope (`FUNCTION_EXPR`, `FOR_EXPR`).
//! - Record a binding (`ASSIGNMENT_EXPR` target, `FUNCTION_EXPR` params,
//!   `FOR_EXPR` loop var).
//! - Record an identifier read site for any `IDENT` token in a read position.
//! - Detect a `library()`/`require()`/`requireNamespace()` call at the *file*
//!   level (not nested inside a function), and record it as a `LoadedPackage`.
//!
//! Definition order: the LHS of `<-`/`=`/`:=` is bound *after* recursing into
//! the RHS, so an RHS read sees the pre-assignment scope. For `->`/`->>` the
//! target is on the right; the same rule applies — we recurse into the value
//! side first.
//!
//! After the walk, a separate `resolve_reads` pass marks each binding as
//! `read` if any recorded `IdentRef` resolves to it.

use rowan::ast::AstNode as _;
use rowan::{NodeOrToken, SyntaxToken, TextRange};
use smol_str::SmolStr;

use crate::ast::{AssignmentExpr, CallExpr, FunctionExpr};
use crate::semantic::binding::{Binding, BindingId, BindingKind};
use crate::semantic::scope::{Scope, ScopeId, ScopeKind};
use crate::semantic::symbols::LoadedPackage;
use crate::semantic::{IdentRef, SemanticModel};
use crate::syntax::{RLanguage, SyntaxKind, SyntaxNode};

/// Build a fresh semantic model from a root CST node.
pub fn build(root: &SyntaxNode) -> SemanticModel {
    let mut model = SemanticModel::default();
    let file_scope = push_scope(&mut model, ScopeKind::File, None, root.text_range());
    let mut ctx = BuildCtx {
        model: &mut model,
        function_depth: 0,
        suppress_read: None,
    };
    walk_generic(&mut ctx, root, file_scope);
    resolve_reads(&mut model);
    model
}

struct BuildCtx<'a> {
    model: &'a mut SemanticModel,
    /// How many `FUNCTION_EXPR`s deep we are. Used to decide whether a
    /// `library()` call counts as "top-level."
    function_depth: usize,
    /// A single IDENT range whose read must be suppressed. Set while walking the
    /// package-name argument of a `library()`/`require()` call so the bare
    /// package name isn't recorded as an undefined read.
    suppress_read: Option<TextRange>,
}

fn walk_node(ctx: &mut BuildCtx<'_>, node: &SyntaxNode, scope: ScopeId) {
    match node.kind() {
        SyntaxKind::FUNCTION_EXPR => handle_function(ctx, node, scope),
        SyntaxKind::FOR_EXPR => handle_for(ctx, node, scope),
        SyntaxKind::ASSIGNMENT_EXPR => handle_assignment(ctx, node, scope),
        SyntaxKind::CALL_EXPR => handle_call(ctx, node, scope),
        SyntaxKind::BINARY_EXPR => handle_binary(ctx, node, scope),
        SyntaxKind::ARG => handle_arg(ctx, node, scope),
        _ => walk_generic(ctx, node, scope),
    }
}

/// Default walker: recurse into child nodes, and record every direct-child
/// IDENT token as a read site.
fn walk_generic(ctx: &mut BuildCtx<'_>, parent: &SyntaxNode, scope: ScopeId) {
    for el in parent.children_with_tokens() {
        match el {
            NodeOrToken::Node(child) => walk_node(ctx, &child, scope),
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::IDENT => {
                record_ident_read(ctx, &tok, scope);
            }
            _ => {}
        }
    }
}

fn record_ident_read(ctx: &mut BuildCtx<'_>, tok: &SyntaxToken<RLanguage>, scope: ScopeId) {
    // The package-name argument of a `library()`/`require()` call is not a read.
    if ctx.suppress_read == Some(tok.text_range()) {
        return;
    }
    let name = tok.text();
    // `...`, `..1`, etc. are lexed as IDENT but are not scope-resolvable.
    if name.starts_with('.') && name.chars().all(|c| c == '.' || c.is_ascii_digit()) {
        return;
    }
    ctx.model.idents.push(IdentRef {
        name: SmolStr::new(name),
        range: tok.text_range(),
        scope,
    });
}

fn handle_function(ctx: &mut BuildCtx<'_>, node: &SyntaxNode, parent: ScopeId) {
    let fn_scope = push_scope(
        ctx.model,
        ScopeKind::Function,
        Some(parent),
        node.text_range(),
    );
    let Some(fn_expr) = FunctionExpr::cast(node.clone()) else {
        walk_generic(ctx, node, parent);
        return;
    };
    for param in fn_expr.params() {
        let range = param.name_token.text_range();
        push_binding(
            ctx.model,
            fn_scope,
            param.name.clone(),
            BindingKind::Param,
            range,
        );
    }
    // Walk the body subtree, plus any param-default expressions. Param-default
    // values live as raw tokens between `=` and the next `,` / `)`, so we walk
    // the entire token range between LPAREN and RPAREN looking for nested
    // expression nodes whose IDENTs are reads.
    walk_function_param_defaults(ctx, &fn_expr, fn_scope);
    if let Some(body) = fn_expr.body() {
        ctx.function_depth += 1;
        match body {
            NodeOrToken::Node(child) => walk_node(ctx, &child, fn_scope),
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::IDENT => {
                record_ident_read(ctx, &tok, fn_scope);
            }
            _ => {}
        }
        ctx.function_depth -= 1;
    }
}

fn walk_function_param_defaults(ctx: &mut BuildCtx<'_>, fn_expr: &FunctionExpr, scope: ScopeId) {
    let Some(lparen) = fn_expr.lparen_index() else {
        return;
    };
    let Some(rparen) = fn_expr.rparen_index() else {
        return;
    };
    let elements: Vec<_> = fn_expr.syntax().children_with_tokens().collect();
    let mut depth = 0usize;
    let mut after_eq = false;
    for el in &elements[lparen + 1..rparen] {
        match el.kind() {
            SyntaxKind::LPAREN | SyntaxKind::LBRACK | SyntaxKind::LBRACK2 | SyntaxKind::LBRACE => {
                depth += 1;
            }
            SyntaxKind::RPAREN | SyntaxKind::RBRACK | SyntaxKind::RBRACK2 | SyntaxKind::RBRACE => {
                depth = depth.saturating_sub(1);
            }
            SyntaxKind::COMMA if depth == 0 => {
                after_eq = false;
                continue;
            }
            SyntaxKind::ASSIGN_EQ if depth == 0 => {
                after_eq = true;
                continue;
            }
            _ => {}
        }
        if !after_eq {
            continue;
        }
        // After `=`, this token belongs to the default expression. Recurse into
        // any node and record IDENT reads.
        match el {
            NodeOrToken::Node(child) => walk_node(ctx, child, scope),
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::IDENT => {
                record_ident_read(ctx, tok, scope);
            }
            _ => {}
        }
    }
}

fn handle_for(ctx: &mut BuildCtx<'_>, node: &SyntaxNode, parent: ScopeId) {
    let for_scope = push_scope(ctx.model, ScopeKind::For, Some(parent), node.text_range());
    let elements: Vec<_> = node.children_with_tokens().collect();

    // Locate `(`, loop-var IDENT, `in`, `)` via a token-level scan.
    let lparen_idx = elements.iter().position(|e| e.kind() == SyntaxKind::LPAREN);
    let in_idx = elements.iter().position(|e| e.kind() == SyntaxKind::IN_KW);
    let rparen_idx = elements.iter().position(|e| e.kind() == SyntaxKind::RPAREN);

    if let Some(lp) = lparen_idx {
        for el in &elements[lp + 1..in_idx.unwrap_or(elements.len())] {
            if let NodeOrToken::Token(tok) = el
                && tok.kind() == SyntaxKind::IDENT
            {
                push_binding(
                    ctx.model,
                    for_scope,
                    SmolStr::new(tok.text()),
                    BindingKind::ForVar,
                    tok.text_range(),
                );
                break;
            }
        }
    }

    // Walk the *sequence* expression (between `in` and `)`).
    if let (Some(in_pos), Some(rp)) = (in_idx, rparen_idx) {
        for el in &elements[in_pos + 1..rp] {
            match el {
                NodeOrToken::Node(child) => walk_node(ctx, child, for_scope),
                NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::IDENT => {
                    record_ident_read(ctx, tok, for_scope);
                }
                _ => {}
            }
        }
    }

    // Walk the body (everything after `)`).
    if let Some(rp) = rparen_idx {
        for el in &elements[rp + 1..] {
            match el {
                NodeOrToken::Node(child) => walk_node(ctx, child, for_scope),
                NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::IDENT => {
                    record_ident_read(ctx, tok, for_scope);
                }
                _ => {}
            }
        }
    }
}

fn handle_assignment(ctx: &mut BuildCtx<'_>, node: &SyntaxNode, scope: ScopeId) {
    let Some(assign) = AssignmentExpr::cast(node.clone()) else {
        walk_generic(ctx, node, scope);
        return;
    };
    let op = assign.op_kind();
    let value = assign.value_element();
    let target = assign.target_element();

    // 1. Recurse the value side FIRST so RHS reads see the pre-assignment scope.
    if let Some(NodeOrToken::Node(value_node)) = &value {
        walk_node(ctx, value_node, scope);
    } else if let Some(NodeOrToken::Token(tok)) = &value
        && tok.kind() == SyntaxKind::IDENT
    {
        record_ident_read(ctx, tok, scope);
    }

    // 2. Record the binding.
    if let Some(name) = assign.target_name() {
        let range = assign
            .target_name_token()
            .map(|t| t.text_range())
            .unwrap_or_else(|| node.text_range());
        let kind = match op {
            Some(SyntaxKind::SUPER_ASSIGN) | Some(SyntaxKind::SUPER_ASSIGN_RIGHT) => {
                BindingKind::Implicit
            }
            _ => BindingKind::Local,
        };
        let target_scope = match kind {
            BindingKind::Implicit => enclosing_function_or_file(ctx.model, scope),
            _ => scope,
        };
        push_binding(ctx.model, target_scope, name, kind, range);
    } else if let Some(NodeOrToken::Node(target_node)) = target {
        // Complex LHS (e.g. `dim(x) <- ...`): treat contents as reads.
        walk_node(ctx, &target_node, scope);
    }
}

fn handle_call(ctx: &mut BuildCtx<'_>, node: &SyntaxNode, scope: ScopeId) {
    // Detect a top-level `library(pkg)` / `require(pkg)` / `requireNamespace("pkg")`.
    if ctx.function_depth == 0
        && let Some(call) = CallExpr::cast(node.clone())
        && let Some(callee) = call_callee_ident(&call)
        && matches!(callee.as_str(), "library" | "require" | "requireNamespace")
        && let Some((pkg_name, pkg_range)) = first_string_or_ident_arg(&call)
    {
        ctx.model.loaded_packages.push(LoadedPackage {
            name: pkg_name,
            range: pkg_range,
        });
        // Don't record the bare package name (e.g. `dplyr` in `library(dplyr)`)
        // as an undefined read. A string arg has no IDENT, so this is a no-op
        // for `requireNamespace("pkg")`.
        let prev = ctx.suppress_read.replace(pkg_range);
        walk_generic(ctx, node, scope);
        ctx.suppress_read = prev;
        return;
    }
    walk_generic(ctx, node, scope);
}

fn handle_binary(ctx: &mut BuildCtx<'_>, node: &SyntaxNode, scope: ScopeId) {
    // Detect namespace / member access patterns.
    let mut operator_kind: Option<SyntaxKind> = None;
    for el in node.children_with_tokens() {
        if let NodeOrToken::Token(t) = el {
            match t.kind() {
                SyntaxKind::COLON2 | SyntaxKind::COLON3 | SyntaxKind::DOLLAR | SyntaxKind::AT => {
                    operator_kind = Some(t.kind());
                    break;
                }
                _ => {}
            }
        }
    }
    match operator_kind {
        Some(SyntaxKind::COLON2 | SyntaxKind::COLON3) => {
            // `pkg::name` / `pkg:::name`: neither operand is a scope-resolvable
            // read. If the RHS is a CALL_EXPR (`pkg::name(args)`), suppress its
            // callee IDENT but record its arguments as reads.
            let elements: Vec<_> = node.children_with_tokens().collect();
            let op_idx = elements
                .iter()
                .position(|e| matches!(e.kind(), SyntaxKind::COLON2 | SyntaxKind::COLON3));
            // The LHS names a referenced (not attached) package; record it.
            if let Some(op) = op_idx
                && let Some(pkg) = lhs_package_name(&elements[..op])
            {
                ctx.model.referenced_packages.push(pkg);
            }
            if let Some(op) = op_idx {
                for el in &elements[op + 1..] {
                    match el {
                        NodeOrToken::Token(_) => {}
                        NodeOrToken::Node(child) if child.kind() == SyntaxKind::CALL_EXPR => {
                            // Skip the first IDENT (callee); recurse into everything else.
                            let mut skipped_callee = false;
                            for cel in child.children_with_tokens() {
                                match cel {
                                    NodeOrToken::Token(t)
                                        if t.kind() == SyntaxKind::IDENT && !skipped_callee =>
                                    {
                                        skipped_callee = true;
                                    }
                                    NodeOrToken::Node(grandchild) => {
                                        walk_node(ctx, &grandchild, scope);
                                    }
                                    _ => {}
                                }
                            }
                        }
                        NodeOrToken::Node(child) => walk_node(ctx, child, scope),
                    }
                }
            }
        }
        Some(SyntaxKind::DOLLAR | SyntaxKind::AT) => {
            // `obj$field` / `obj@slot`: LHS is a read, RHS is a member name.
            let mut seen_op = false;
            for el in node.children_with_tokens() {
                match el {
                    NodeOrToken::Token(t)
                        if matches!(t.kind(), SyntaxKind::DOLLAR | SyntaxKind::AT) =>
                    {
                        seen_op = true;
                    }
                    NodeOrToken::Token(t) if t.kind() == SyntaxKind::IDENT && !seen_op => {
                        record_ident_read(ctx, &t, scope);
                    }
                    NodeOrToken::Node(child) if !seen_op => {
                        walk_node(ctx, &child, scope);
                    }
                    _ => {}
                }
            }
        }
        _ => walk_generic(ctx, node, scope),
    }
}

fn handle_arg(ctx: &mut BuildCtx<'_>, node: &SyntaxNode, scope: ScopeId) {
    // ARG forms: `value`, `name = value`, `name`. Skip the leading IDENT/STRING
    // that names the argument; everything after `=` is normal.
    let elements: Vec<_> = node.children_with_tokens().collect();
    let eq_idx = elements
        .iter()
        .position(|el| matches!(el, NodeOrToken::Token(t) if t.kind() == SyntaxKind::ASSIGN_EQ));
    let mut name_idx: Option<usize> = None;
    if let Some(eq) = eq_idx {
        // Validate the prefix is "[trivia*] IDENT|STRING [trivia*]".
        let mut name_token_count = 0;
        let mut name_position: Option<usize> = None;
        let mut ok = true;
        for (i, el) in elements[..eq].iter().enumerate() {
            match el.kind() {
                SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT => {}
                SyntaxKind::IDENT | SyntaxKind::STRING => {
                    name_token_count += 1;
                    name_position = Some(i);
                }
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok && name_token_count == 1 {
            name_idx = name_position;
        }
    }
    let skip_until = name_idx.map(|i| i + 1).unwrap_or(0);
    for (i, el) in elements.iter().enumerate() {
        if i < skip_until {
            continue;
        }
        // Also skip the `=` itself.
        if name_idx.is_some()
            && matches!(el, NodeOrToken::Token(t) if t.kind() == SyntaxKind::ASSIGN_EQ)
        {
            continue;
        }
        match el {
            NodeOrToken::Node(child) => walk_node(ctx, child, scope),
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::IDENT => {
                record_ident_read(ctx, tok, scope);
            }
            _ => {}
        }
    }
}

fn call_callee_ident(call: &CallExpr) -> Option<SmolStr> {
    for el in call.syntax().children_with_tokens() {
        match el {
            NodeOrToken::Token(t)
                if matches!(
                    t.kind(),
                    SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
                ) =>
            {
                continue;
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::IDENT => {
                return Some(SmolStr::new(t.text()));
            }
            _ => return None,
        }
    }
    None
}

fn first_string_or_ident_arg(call: &CallExpr) -> Option<(SmolStr, TextRange)> {
    let arg_list = call.arg_list()?;
    let first_arg = arg_list.args().next()?;
    for el in first_arg.syntax().children_with_tokens() {
        match el {
            NodeOrToken::Token(t)
                if matches!(
                    t.kind(),
                    SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
                ) =>
            {
                continue;
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::IDENT => {
                return Some((SmolStr::new(t.text()), t.text_range()));
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::STRING => {
                let stripped = strip_quotes(t.text())?;
                return Some((SmolStr::new(stripped), t.text_range()));
            }
            _ => return None,
        }
    }
    None
}

/// The package named on the left of `::` / `:::`: the last `IDENT`/`STRING`
/// token in the left-hand-side elements (ignoring trivia).
fn lhs_package_name(lhs: &[NodeOrToken<SyntaxNode, SyntaxToken<RLanguage>>]) -> Option<SmolStr> {
    for el in lhs.iter().rev() {
        if let NodeOrToken::Token(t) = el {
            match t.kind() {
                SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT => continue,
                SyntaxKind::IDENT => return Some(SmolStr::new(t.text())),
                SyntaxKind::STRING => return strip_quotes(t.text()).map(SmolStr::new),
                _ => return None,
            }
        }
    }
    None
}

fn strip_quotes(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' || first == b'\'' || first == b'`') && first == last {
            return Some(&s[1..s.len() - 1]);
        }
    }
    None
}

fn enclosing_function_or_file(model: &SemanticModel, scope: ScopeId) -> ScopeId {
    let mut current = scope;
    loop {
        let scope_ref = &model.scopes[current.0 as usize];
        match scope_ref.kind {
            ScopeKind::File => return current,
            ScopeKind::Function => {
                return scope_ref.parent.unwrap_or(current);
            }
            _ => match scope_ref.parent {
                Some(p) => current = p,
                None => return current,
            },
        }
    }
}

fn push_scope(
    model: &mut SemanticModel,
    kind: ScopeKind,
    parent: Option<ScopeId>,
    range: TextRange,
) -> ScopeId {
    let id = ScopeId::from_index(model.scopes.len());
    model.scopes.push(Scope {
        kind,
        parent,
        range,
        bindings: Vec::new(),
    });
    id
}

fn push_binding(
    model: &mut SemanticModel,
    scope: ScopeId,
    name: SmolStr,
    kind: BindingKind,
    def_range: TextRange,
) -> BindingId {
    let id = BindingId::from_index(model.bindings.len());
    model.bindings.push(Binding {
        name,
        kind,
        scope,
        def_range,
        read: false,
    });
    model.scopes[scope.0 as usize].bindings.push(id);
    id
}

/// Walk every recorded identifier read and, if it resolves to a binding in any
/// enclosing scope, mark that binding as `read`. Used by `unused-binding`.
fn resolve_reads(model: &mut SemanticModel) {
    for ident_idx in 0..model.idents.len() {
        let ident = model.idents[ident_idx].clone();
        let resolved = {
            let mut current = Some(ident.scope);
            let mut found: Option<BindingId> = None;
            while let Some(scope_id) = current {
                let scope_ref = &model.scopes[scope_id.0 as usize];
                for binding_id in &scope_ref.bindings {
                    let binding = &model.bindings[binding_id.0 as usize];
                    if binding.name == ident.name && binding.def_range != ident.range {
                        found = Some(*binding_id);
                        break;
                    }
                }
                if found.is_some() {
                    break;
                }
                current = scope_ref.parent;
            }
            found
        };
        if let Some(id) = resolved {
            model.bindings[id.0 as usize].read = true;
        }
    }
}
