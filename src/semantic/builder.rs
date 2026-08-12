//! Build a [`SemanticModel`] from a parsed file root.
//!
//! The builder walks the CST once, maintaining a stack of open scopes. At each
//! node it decides whether to:
//! - Push a new scope (`FUNCTION_EXPR` only — R introduces a variable scope at a
//!   `function` and the file top; `for`/`while`/`repeat` bodies share the
//!   enclosing frame, so they push no scope).
//! - Record a binding (`ASSIGNMENT_EXPR` target, `FUNCTION_EXPR` params,
//!   `FOR_EXPR` loop var — the last two in the enclosing frame).
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

use std::collections::{HashMap, HashSet};

use rowan::ast::AstNode as _;
use rowan::{NodeOrToken, SyntaxToken, TextRange};
use smol_str::SmolStr;

use crate::ast::{Arg, AssignmentExpr, AstToken as _, CallExpr, FunctionExpr, Ident, SubsetExpr};
use crate::semantic::binding::{Binding, BindingId, BindingKind};
use crate::semantic::scope::{Scope, ScopeId, ScopeKind};
use crate::semantic::symbols::LoadedPackage;
use crate::semantic::{IdentRef, SemanticModel};
use crate::syntax::{RLanguage, SyntaxElement, SyntaxKind, SyntaxNode};

/// Build a fresh semantic model from a root CST node.
pub fn build(root: &SyntaxNode) -> SemanticModel {
    let mut model = SemanticModel::default();
    let file_scope = push_scope(&mut model, ScopeKind::File, None, root.text_range());
    let ident_gates = {
        let mut ctx = BuildCtx {
            model: &mut model,
            function_depth: 0,
            suppress_read: None,
            mask_depth: 0,
            loop_range: None,
            deferred: false,
            quote_depth: 0,
            gate_depth: 0,
            pin_depth: 0,
            gate_verb: None,
            ident_gates: Vec::new(),
            column_depth: 0,
            data_tables: HashSet::new(),
        };
        walk_generic(&mut ctx, root, file_scope);
        ctx.ident_gates
    };
    resolve_reads(&mut model);
    apply_shadow_gate(&mut model, &ident_gates);
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
    /// How many data-masking call arguments deep we are. Reads recorded while
    /// `mask_depth > 0` are flagged [`IdentRef::data_masked`] — a bare name there
    /// may be a data-frame column, so `undefined-symbol` leaves it alone. Once
    /// inside a masked argument it stays masked through the whole subtree (the
    /// mask is the evaluation environment for the entire expression), the
    /// conservative direction for a false-positive-only rule.
    mask_depth: usize,
    /// Range of the innermost enclosing `for`/`while`/`repeat`, if any. Stamped
    /// onto every binding recorded while walking a loop body so `reads_reached`
    /// can treat a loop-carried read (one textually before its assignment) as a
    /// use. Reset across a `function` boundary, since a closure defined in a loop
    /// does not re-run per iteration.
    loop_range: Option<TextRange>,
    /// Whether reads recorded right now are lazily evaluated (an R promise), so
    /// they carry no intra-frame textual-ordering constraint. Set while walking a
    /// parameter default or an `on.exit(...)` handler; stamped onto each recorded
    /// [`IdentRef::deferred`]. Reset across a `function` boundary (an inner
    /// closure's own body evaluates eagerly in its own frame).
    deferred: bool,
    /// How many quoting-callee argument lists deep we are
    /// (`quote`/`expression`/…). While `> 0`, an inner `<-` target is captured
    /// unevaluated, not a real local binding, so `handle_assignment` records no
    /// binding for it.
    quote_depth: usize,
    /// How many *gateable* masks deep we are — those from a bare data-masking
    /// verb, whose name-only match `apply_shadow_gate` may later retract.
    gate_depth: usize,
    /// How many *pinned* masks deep we are: quoting callees, formulas, opaque
    /// `%op%` operands, model-frame arguments, and data.table `[` subscripts.
    /// None of those hinge on a name that a local binding could shadow, so a
    /// read under one is masked for good.
    pin_depth: usize,
    /// Index into [`SemanticModel::idents`] of the read of the enclosing
    /// gateable verb's own name, when exactly one such mask is open and nothing
    /// pins it. Stamped onto each read via [`ident_gates`](Self::ident_gates).
    gate_verb: Option<u32>,
    /// Parallel to [`SemanticModel::idents`]: which verb read, if any, is solely
    /// responsible for that read being masked. Builder-local so `IdentRef` (and
    /// with it every salsa projection) stays untouched.
    ident_gates: Vec<Option<u32>>,
    /// How many data.table `[` argument lists deep we are. While `> 0` a `:=`
    /// adds a *column*, so `handle_assignment` records a masked read instead of
    /// a binding.
    column_depth: usize,
    /// Names known to hold a data.table, from a constructor call
    /// (`dt <- data.table(…)`) or an in-place `setDT(df)`. Name-only and
    /// scope-free, populated during the same walk — so, exactly like
    /// `loaded_packages`, only conversions appearing textually *earlier* are
    /// visible. Both directions are safe: a miss just leaves `dt[x > 3]`
    /// unmasked, and masking only ever suppresses.
    data_tables: HashSet<SmolStr>,
}

impl BuildCtx<'_> {
    /// Open a mask no shadowing check can retract (see
    /// [`pin_depth`](Self::pin_depth)).
    fn enter_pinned_mask(&mut self) {
        self.mask_depth += 1;
        self.pin_depth += 1;
    }

    fn exit_pinned_mask(&mut self) {
        self.pin_depth -= 1;
        self.mask_depth -= 1;
    }

    /// Open a data-masking verb's mask, attributing the reads inside to
    /// `verb` (an index into `idents`). Returns the previous attribution to hand
    /// back to [`exit_gated_mask`](Self::exit_gated_mask).
    ///
    /// Only the outermost gateable mask attributes: a read nested in a second
    /// verb gets no gate at all, so unmasking the outer verb can't reach it.
    fn enter_gated_mask(&mut self, verb: Option<u32>) -> Option<u32> {
        self.mask_depth += 1;
        self.gate_depth += 1;
        let prev = self.gate_verb;
        if self.gate_depth == 1 && self.pin_depth == 0 {
            self.gate_verb = verb;
        }
        prev
    }

    fn exit_gated_mask(&mut self, prev: Option<u32>) {
        self.gate_verb = prev;
        self.gate_depth -= 1;
        self.mask_depth -= 1;
    }
}

/// Record an identifier read, keeping [`BuildCtx::ident_gates`] in lockstep with
/// the model's `idents`. Every push must go through here.
fn push_ident(ctx: &mut BuildCtx<'_>, ident: IdentRef) {
    let gate = if ctx.pin_depth == 0 && ctx.gate_depth == 1 {
        ctx.gate_verb
    } else {
        None
    };
    ctx.model.idents.push(ident);
    ctx.ident_gates.push(gate);
}

/// Retract the name-only data-masking match wherever the verb turned out to be
/// the file's own function.
///
/// `is_data_masking_callee` matches on name alone, so a file that defines its
/// own non-NSE `filter`/`transform` used to have that call's arguments
/// suppressed. A verb read that resolves to a local binding is *that* function —
/// an ordinary one, which evaluates its arguments — so its reads are genuine
/// again.
///
/// This has to run after `resolve_reads`, not during the walk: R routinely
/// defines a function below its first use. Reusing that pass's `ident_bindings`
/// also gets the frame-ordering rule for free — a top-level call *above* the
/// definition resolves to nothing and stays masked, which is what R does at
/// runtime.
fn apply_shadow_gate(model: &mut SemanticModel, ident_gates: &[Option<u32>]) {
    // One verdict per verb read, not per masked read under it.
    let mut shadowed: HashMap<u32, bool> = HashMap::new();
    for idx in 0..model.idents.len() {
        let Some(verb) = ident_gates.get(idx).copied().flatten() else {
            continue;
        };
        let is_shadowed = *shadowed
            .entry(verb)
            .or_insert_with(|| !model.ident_bindings[verb as usize].is_empty());
        if is_shadowed {
            model.idents[idx].data_masked = false;
        }
    }
}

fn walk_node(ctx: &mut BuildCtx<'_>, node: &SyntaxNode, scope: ScopeId) {
    match node.kind() {
        SyntaxKind::FUNCTION_EXPR => handle_function(ctx, node, scope),
        SyntaxKind::FOR_EXPR => handle_for(ctx, node, scope),
        SyntaxKind::WHILE_EXPR | SyntaxKind::REPEAT_EXPR => handle_loop(ctx, node, scope),
        SyntaxKind::ASSIGNMENT_EXPR => handle_assignment(ctx, node, scope),
        SyntaxKind::CALL_EXPR => handle_call(ctx, node, scope),
        SyntaxKind::BINARY_EXPR => handle_binary(ctx, node, scope),
        SyntaxKind::UNARY_EXPR => handle_unary(ctx, node, scope),
        SyntaxKind::ARG => handle_arg(ctx, node, scope),
        SyntaxKind::SUBSET_EXPR => handle_subset(ctx, node, scope),
        _ => walk_generic(ctx, node, scope),
    }
}

/// Default walker: recurse into child nodes, and record every direct-child
/// IDENT token as a read site.
fn walk_generic(ctx: &mut BuildCtx<'_>, parent: &SyntaxNode, scope: ScopeId) {
    for el in parent.children_with_tokens() {
        walk_element(ctx, &el, scope);
    }
}

/// Walk one child element in read position: recurse into a node, or record a
/// bare `IDENT` token as a read. Trivia and other tokens (operators, literals,
/// punctuation) are ignored. This is the shared arm the structural handlers
/// use once they have carved off their special children.
fn walk_element(ctx: &mut BuildCtx<'_>, el: &SyntaxElement, scope: ScopeId) {
    match el {
        NodeOrToken::Node(child) => walk_node(ctx, child, scope),
        NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::IDENT => {
            record_ident_read(ctx, tok, scope);
        }
        _ => {}
    }
}

fn record_ident_read(ctx: &mut BuildCtx<'_>, tok: &SyntaxToken<RLanguage>, scope: ScopeId) {
    // The package-name argument of a `library()`/`require()` call is not a read.
    if ctx.suppress_read == Some(tok.text_range()) {
        return;
    }
    // `...`, `..1`, etc. are lexed as IDENT but are not scope-resolvable; the
    // reserved literal constants (`TRUE`, `NA`, `NULL`, `Inf`, …) lex as IDENT
    // but are values, not symbol references. (`T`/`F` are *not* excluded: they
    // are rebindable base bindings.)
    // `.Generic`/`.Method`/`.Class` are bound implicitly inside method bodies.
    if let Some(ident) = Ident::cast(tok.clone())
        && (ident.is_dots() || ident.is_reserved_constant() || ident.is_implicit_method_var())
    {
        return;
    }
    push_ident(
        ctx,
        IdentRef {
            name: SmolStr::new(tok.text()),
            range: tok.text_range(),
            scope,
            data_masked: ctx.mask_depth > 0,
            deferred: ctx.deferred,
        },
    );
}

/// Record the `USER_OP` token of a binary expression (`a %op% b`) as a read of
/// its definition. A user operator is defined and referenced backtick-quoted
/// (`` `%op%` <- function(...)``), so the read name is the operator text wrapped
/// in backticks to match the binding. Recorded with the ambient mask state.
fn record_user_op_read(ctx: &mut BuildCtx<'_>, node: &SyntaxNode, scope: ScopeId) {
    for el in node.children_with_tokens() {
        if let NodeOrToken::Token(t) = el
            && t.kind() == SyntaxKind::USER_OP
        {
            push_ident(
                ctx,
                IdentRef {
                    name: SmolStr::new(format!("`{}`", t.text())),
                    range: t.text_range(),
                    scope,
                    data_masked: ctx.mask_depth > 0,
                    deferred: ctx.deferred,
                },
            );
            return;
        }
    }
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
            None,
        );
    }
    // A closure defined inside a loop does not re-run per iteration, so the
    // loop-carried relaxation stops at the function boundary: clear the enclosing
    // loop range while walking params defaults and body, then restore it. The
    // promise (`deferred`) relaxation likewise stops here — this function's own
    // body evaluates eagerly even when the function is itself a param default.
    let prev_loop = ctx.loop_range.take();
    let prev_deferred = std::mem::replace(&mut ctx.deferred, false);
    // The data mask deliberately does *not* stop here. A closure written inside
    // a masked argument is created in the mask environment, so the mask is its
    // lexical parent and a bare column name in its body resolves:
    // `with(d, sapply(col, function(v) v + other[1]))` finds `other` in `d`.
    // That holds for rlang's data mask too, which is likewise the enclosure of
    // anything defined in it.
    //
    // Walk the body subtree, plus any param-default expressions. Param-default
    // values live as raw tokens between `=` and the next `,` / `)`, so we walk
    // the entire token range between LPAREN and RPAREN looking for nested
    // expression nodes whose IDENTs are reads. A default is a promise evaluated
    // in this frame, so its reads are `deferred` (order-free within the frame).
    ctx.deferred = true;
    walk_function_param_defaults(ctx, &fn_expr, fn_scope);
    ctx.deferred = false;
    if let Some(body) = fn_expr.body() {
        ctx.function_depth += 1;
        walk_element(ctx, &body, fn_scope);
        ctx.function_depth -= 1;
    }
    ctx.loop_range = prev_loop;
    ctx.deferred = prev_deferred;
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
        // After `=`, this element belongs to the default expression.
        walk_element(ctx, el, scope);
    }
}

fn handle_for(ctx: &mut BuildCtx<'_>, node: &SyntaxNode, scope: ScopeId) {
    // R has no loop scope: the loop variable and any body assignments live in the
    // enclosing frame (`scope`) and leak past the loop. Recording them there is
    // what lets a read after the loop resolve to a body binding.
    let elements: Vec<_> = node.children_with_tokens().collect();

    // Locate `(`, loop-var IDENT, `in`, `)` via a token-level scan.
    let lparen_idx = elements.iter().position(|e| e.kind() == SyntaxKind::LPAREN);
    let in_idx = elements.iter().position(|e| e.kind() == SyntaxKind::IN_KW);
    let rparen_idx = elements.iter().position(|e| e.kind() == SyntaxKind::RPAREN);

    // The body re-executes, so bindings recorded inside it are loop-carried:
    // stamp them with this loop's range (innermost wins on nesting).
    let outer_loop = ctx.loop_range.replace(node.text_range());

    if let Some(lp) = lparen_idx {
        for el in &elements[lp + 1..in_idx.unwrap_or(elements.len())] {
            if let NodeOrToken::Token(tok) = el
                && tok.kind() == SyntaxKind::IDENT
            {
                push_binding(
                    ctx.model,
                    scope,
                    SmolStr::new(tok.text()),
                    BindingKind::ForVar,
                    tok.text_range(),
                    ctx.loop_range,
                );
                break;
            }
        }
    }

    // Walk the *sequence* expression (between `in` and `)`).
    if let (Some(in_pos), Some(rp)) = (in_idx, rparen_idx) {
        for el in &elements[in_pos + 1..rp] {
            walk_element(ctx, el, scope);
        }
    }

    // Walk the body (everything after `)`).
    if let Some(rp) = rparen_idx {
        for el in &elements[rp + 1..] {
            walk_element(ctx, el, scope);
        }
    }

    ctx.loop_range = outer_loop;
}

/// `while (cond) body` / `repeat body`. Like `for`, these introduce no scope but
/// do re-execute: stamp bindings in the whole subtree with the loop range so a
/// loop-carried read (textually before its assignment) still counts as a use.
fn handle_loop(ctx: &mut BuildCtx<'_>, node: &SyntaxNode, scope: ScopeId) {
    let outer_loop = ctx.loop_range.replace(node.text_range());
    walk_generic(ctx, node, scope);
    ctx.loop_range = outer_loop;
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
    if let Some(value) = &value {
        walk_element(ctx, value, scope);
    }

    // 2. Record the binding — unless we're inside quoted code (`quote`,
    //    `expression`, …), where an assignment is captured unevaluated and binds
    //    nothing analyzable. The RHS was still walked above (its reads are masked,
    //    hence harmless), matching how the rest of the quoted body is handled.
    if ctx.quote_depth > 0 {
        return;
    }
    // 2b. Inside a data.table `[`, `:=` adds or updates a *column*, not a
    //     variable: `dt[, newcol := 1]` binds nothing in the frame. Record the
    //     target as a masked read (it names a column) so it neither resolves
    //     nor surfaces as an unused binding.
    if ctx.column_depth > 0 && op == Some(SyntaxKind::WALRUS) {
        match (assign.target_name_token(), &target) {
            (Some(tok), _) => record_ident_read(ctx, &tok, scope),
            // A computed LHS (`dt[, c("a", "b") := …]`) holds ordinary reads.
            (None, Some(NodeOrToken::Node(target_node))) => {
                walk_node(ctx, target_node, scope);
            }
            _ => {}
        }
        return;
    }
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
        // Remember a name assigned a data.table, so a later marker-free
        // `dt[x > 3]` is recognized as a column filter.
        if value
            .as_ref()
            .and_then(|v| v.as_node())
            .is_some_and(|v| node_yields_data_table(ctx, v))
        {
            ctx.data_tables.insert(name.clone());
        }
        push_binding(ctx.model, target_scope, name, kind, range, ctx.loop_range);
    } else if let Some(NodeOrToken::Node(target_node)) = target {
        // Complex LHS (e.g. `dim(x) <- ...`): treat contents as reads.
        walk_node(ctx, &target_node, scope);
    }
}

fn handle_call(ctx: &mut BuildCtx<'_>, node: &SyntaxNode, scope: ScopeId) {
    // Naming a package in a load call *references* it, wherever the call sits —
    // `requireNamespace()` inside a function body is the conditional-dependency
    // idiom. Recorded separately from the attach set below, which is
    // deliberately top-level-only because attachment is the narrower fact.
    if let Some(call) = CallExpr::cast(node.clone())
        && let Some(callee) = call_callee_ident(&call)
        && matches!(
            callee.as_str(),
            "library" | "require" | "requireNamespace" | "loadNamespace"
        )
        && let Some((pkg_name, _)) = first_string_or_ident_arg(&call)
    {
        ctx.model.referenced_packages.push(pkg_name);
    }

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

    // A handful of base callees introduce or reference names in ways the plain
    // scope walk can't see. Match by name (independent of package, like the
    // data-masking set): over-matching only ever *suppresses* an
    // `undefined-symbol` finding, the conservative direction.
    if let Some(call) = CallExpr::cast(node.clone())
        && let Some(callee) = call_callee_ident(&call)
    {
        match callee.as_str() {
            // `attach(df)` puts a data frame's columns on the search path and
            // `load("*.rda")` restores arbitrary names — both introduce bindings
            // arity can't enumerate statically. Flag the file so
            // `undefined-symbol` gates it, then fall through to a normal walk
            // (the call's own arguments are ordinary reads).
            "attach" | "load" => {
                ctx.model.attaches_opaque_env = true;
            }
            // `.C`/`.Call`/`.Fortran`/`.External`: a bare IDENT in the head
            // (first-argument) position names a native routine registered via
            // `useDynLib`, not a scope read. Suppress just that read; the
            // remaining arguments stay ordinary reads. A string head has no
            // IDENT (a no-op), and a compound/`::`-qualified head yields no
            // bare token here, so it walks normally.
            ".C" | ".Call" | ".Fortran" | ".External" => {
                if let Some((_, head_range)) = first_string_or_ident_arg(&call) {
                    let prev = ctx.suppress_read.replace(head_range);
                    walk_generic(ctx, node, scope);
                    ctx.suppress_read = prev;
                    return;
                }
            }
            // `data(name, …)` lazy-loads each named dataset and binds it in the
            // caller's frame, so a later `name$col` read resolves. Introduce a
            // binding for each bare-name argument, then walk normally.
            "data" => introduce_data_bindings(ctx, &call, scope),
            // `setDT(df)` converts in place and returns invisibly, so `df` is a
            // data.table from here on even with no assignment.
            "setDT" | "setalloccol" => {
                if let Some((name, _)) = first_string_or_ident_arg(&call) {
                    ctx.data_tables.insert(name);
                }
            }
            // `on.exit(expr)` registers `expr` as a promise run at function exit,
            // so it may read a local assigned *after* the call. Walk it deferred
            // (order-free within the frame), like a param default.
            "on.exit" => {
                let prev = std::mem::replace(&mut ctx.deferred, true);
                walk_generic(ctx, node, scope);
                ctx.deferred = prev;
                return;
            }
            // `NextMethod()` dispatches to the next method with the *current*
            // frame values of the enclosing function's formals, so each formal
            // (including a reassigned one, `x <- M`) is used. Synthesize a
            // deferred read of each formal name at the call site, then walk the
            // call's own arguments normally.
            "NextMethod" => synthesize_formal_reads(ctx, node, scope),
            _ => {}
        }
    }

    // A data-masking verb (e.g. `mutate`) evaluates its arguments in the data
    // mask, where a bare name may be a column; a quoting callee (`quote`,
    // `substitute`, …) doesn't evaluate its argument body at all. Either way a
    // bare name in the argument list isn't a resolvable read. Walk the callee
    // unmasked (a typo'd verb name is still a genuine undefined read) but mask
    // the argument list so its bare reads aren't flagged.
    if call_masks_arguments(node) {
        // A quoting callee additionally captures its argument body unevaluated, so
        // an inner `<-` there is not a real local binding: track quote depth so
        // `handle_assignment` records no binding for it.
        let quoting = CallExpr::cast(node.clone())
            .and_then(|call| call_callee_ident(&call))
            .is_some_and(|name| is_quoting_callee(&name));
        // The callee is the first read recorded below (it precedes the
        // `ARG_LIST` among the call's children). Remember which one it is so
        // `apply_shadow_gate` can retract the mask if it turns out to name the
        // file's own function. A quoting callee is pinned instead: it evaluates
        // nothing regardless of what the name resolves to.
        let verb_start = ctx.model.idents.len();
        for el in node.children_with_tokens() {
            // Mask the argument list (bare names there may be data columns);
            // walk everything else (the callee) unmasked.
            if let NodeOrToken::Node(child) = &el
                && child.kind() == SyntaxKind::ARG_LIST
            {
                // `None` when no read was recorded for the callee — a
                // `pkg::verb(…)` shape, whose name no local binding can shadow.
                let verb = u32::try_from(verb_start)
                    .ok()
                    .filter(|_| ctx.model.idents.len() > verb_start);
                let prev = if quoting {
                    ctx.enter_pinned_mask();
                    ctx.quote_depth += 1;
                    None
                } else {
                    ctx.enter_gated_mask(verb)
                };
                walk_node(ctx, child, scope);
                if quoting {
                    ctx.quote_depth -= 1;
                    ctx.exit_pinned_mask();
                } else {
                    ctx.exit_gated_mask(prev);
                }
            } else {
                walk_element(ctx, &el, scope);
            }
        }
        return;
    }

    // A model-fitting call with a supplied `data` argument evaluates a few of
    // its arguments (`weights`/`subset`/`offset`) in the model frame built
    // from that data frame, where a bare name is a column. Unlike a
    // data-masking verb this is per-argument: everything else in the arg-list
    // (`data` included) is an ordinary read.
    if let Some(mask) = model_frame_arg_mask(node) {
        for el in node.children_with_tokens() {
            if let NodeOrToken::Node(child) = &el
                && child.kind() == SyntaxKind::ARG_LIST
            {
                walk_model_frame_arg_list(ctx, child, scope, &mask);
            } else {
                walk_element(ctx, &el, scope);
            }
        }
        return;
    }

    walk_generic(ctx, node, scope);
}

/// Per-argument model-frame mask for a model-fitting call (`lm`, `glm`,
/// `polr`, …), or `None` when the call builds no model frame. The callee's
/// formals table drives a simulation of R's argument matching, so `data`
/// counts whether supplied by exact name, unique prefix (`dat = d`), or
/// position (`lm`'s second argument, `glm`'s third). The `data` requirement is
/// what makes this faithful rather than merely suppressive: with no data
/// frame, R evaluates `weights`/`subset`/`offset` in the calling environment,
/// where an unresolved bare name is genuinely undefined.
///
/// An argument is masked when it binds a model-frame formal (by name, prefix,
/// or position) or when it is named into `...` under a (prefix of a)
/// model-frame name — dots-forwarded arguments are re-matched at the inner
/// fitting call (`aov` forwards `weights` to `lm`).
fn model_frame_arg_mask(node: &SyntaxNode) -> Option<Vec<bool>> {
    let call = CallExpr::cast(node.clone())?;
    let callee = call_callee_ident(&call)?;
    let formals = crate::semantic::model_frame_formals(&callee)?;
    let args: Vec<Arg> = call.arg_list()?.args().collect();
    let names: Vec<Option<SmolStr>> = args.iter().map(Arg::name).collect();
    let matched = crate::semantic::match_args_to_formals(&names, formals);
    // An argument hole (`lm(y ~ x, , weights = w)`) consumes `data`'s position
    // but supplies nothing, so it does not open the gate.
    let data_supplied = matched
        .iter()
        .zip(&args)
        .any(|(m, arg)| *m == Some("data") && arg.value().is_some());
    if !data_supplied {
        return None;
    }
    Some(
        matched
            .iter()
            .zip(&names)
            .map(|(m, name)| match m {
                Some(formal) => crate::semantic::is_model_frame_arg(formal),
                None => name
                    .as_deref()
                    .is_some_and(crate::semantic::is_model_frame_arg_prefix),
            })
            .collect(),
    )
}

/// Walk an `ARG_LIST` of a model-fitting call, masking only the arguments
/// [`model_frame_arg_mask`] marked (indexed in `ARG` order). Masking the whole
/// `ARG` is fine: its name token and `=` are skipped by [`handle_arg`]
/// regardless.
fn walk_model_frame_arg_list(
    ctx: &mut BuildCtx<'_>,
    arg_list: &SyntaxNode,
    scope: ScopeId,
    mask: &[bool],
) {
    let mut arg_idx = 0;
    for el in arg_list.children_with_tokens() {
        let masked = match &el {
            NodeOrToken::Node(arg) if arg.kind() == SyntaxKind::ARG => {
                arg_idx += 1;
                mask.get(arg_idx - 1).copied().unwrap_or(false)
            }
            _ => false,
        };
        if masked {
            ctx.enter_pinned_mask();
        }
        walk_element(ctx, &el, scope);
        if masked {
            ctx.exit_pinned_mask();
        }
    }
}

/// Introduce a binding for each bare-name positional argument of a `data()`
/// call. `data(sole)` lazy-loads the `sole` dataset and binds it in the calling
/// frame, so later reads (`sole$off`) resolve. Bound `Implicit` (like `<<-`
/// targets): opaquely introduced, and thus excluded from `unused-binding`.
/// String / named (`package = "…"`, `list = …`) arguments introduce nothing.
fn introduce_data_bindings(ctx: &mut BuildCtx<'_>, call: &CallExpr, scope: ScopeId) {
    let Some(arg_list) = call.arg_list() else {
        return;
    };
    for arg in arg_list.args() {
        if arg.is_named() {
            continue;
        }
        let Some(NodeOrToken::Token(tok)) = arg.value() else {
            continue;
        };
        if tok.kind() != SyntaxKind::IDENT {
            continue;
        }
        // Skip `...`/`..1` and reserved constants (`data(NULL)` etc.): not names
        // to bind. Mirrors `record_ident_read`'s exclusions.
        if let Some(ident) = Ident::cast(tok.clone())
            && (ident.is_dots() || ident.is_reserved_constant())
        {
            continue;
        }
        push_binding(
            ctx.model,
            scope,
            SmolStr::new(tok.text()),
            BindingKind::Implicit,
            tok.text_range(),
            ctx.loop_range,
        );
    }
}

/// Record a deferred read of every formal (parameter) of the enclosing function
/// at a `NextMethod()` call. `NextMethod` re-dispatches with the current frame
/// values of the formals, so each one — including a formal reassigned in the body
/// (`x <- M`, a `Local` shadowing the `Param`) — is used. The formals are the
/// `Param` bindings already recorded in `scope` (params are pushed before the
/// body walk, and `{}`/`for` introduce no scope, so a call in the body sees the
/// function scope directly). A no-op at file scope, where there are no formals.
fn synthesize_formal_reads(ctx: &mut BuildCtx<'_>, node: &SyntaxNode, scope: ScopeId) {
    let range = node.text_range();
    let formals: Vec<SmolStr> = ctx.model.scopes[scope.0 as usize]
        .bindings
        .iter()
        .filter_map(|id| {
            let b = &ctx.model.bindings[id.0 as usize];
            (b.kind == BindingKind::Param).then(|| b.name.clone())
        })
        .collect();
    for name in formals {
        push_ident(
            ctx,
            IdentRef {
                name,
                range,
                scope,
                data_masked: false,
                deferred: true,
            },
        );
    }
}

/// A `[` subscript. data.table is the one masking construct that is
/// `[`-shaped rather than a call: `dt[i, j, by]` evaluates every slot against
/// the table's columns. Mask the argument list when the subscript is
/// recognizably that form, walking the base unmasked (a typo'd `dt` is still a
/// genuine read); otherwise this is ordinary indexing and walks normally.
///
/// `[[` is deliberately excluded — it is not data.table's NSE form.
fn handle_subset(ctx: &mut BuildCtx<'_>, node: &SyntaxNode, scope: ScopeId) {
    if !subset_masks_arguments(ctx, node) {
        walk_generic(ctx, node, scope);
        return;
    }
    for el in node.children_with_tokens() {
        if let NodeOrToken::Node(child) = &el
            && child.kind() == SyntaxKind::ARG_LIST
        {
            // Pinned: the form was identified by syntax, not by a name a local
            // binding could shadow.
            ctx.enter_pinned_mask();
            ctx.column_depth += 1;
            walk_node(ctx, child, scope);
            ctx.column_depth -= 1;
            ctx.exit_pinned_mask();
        } else {
            walk_element(ctx, &el, scope);
        }
    }
}

/// Whether a `SUBSET_EXPR` is data.table's masking form. Two independent
/// prongs, since the syntax alone can't always tell:
///
/// - A **marker** unique to `[.data.table`: one of [`is_data_table_arg_name`]'s
///   named arguments, a `:=` anywhere inside, or a pronoun like `.N`/`.SD`.
/// - A **known table base**, which is what identifies the marker-free filter
///   idiom `dt[x > 3]` — indistinguishable from vector indexing otherwise.
///
/// The marker scan covers the whole argument subtree, so a nested table
/// expression can mark its enclosing subscript too. That over-matches, which
/// only ever suppresses — the safe direction for a false-positive-only rule.
fn subset_masks_arguments(ctx: &BuildCtx<'_>, node: &SyntaxNode) -> bool {
    let Some(subset) = SubsetExpr::cast(node.clone()) else {
        return false;
    };
    let Some(arg_list) = subset.arg_list() else {
        return false;
    };
    let named_marker = arg_list.args().any(|arg| {
        arg.name()
            .is_some_and(|name| crate::semantic::is_data_table_arg_name(&name))
    });
    if named_marker {
        return true;
    }
    let inner_marker = arg_list
        .syntax()
        .descendants_with_tokens()
        .any(|el| match el {
            NodeOrToken::Token(t) => {
                t.kind() == SyntaxKind::WALRUS
                    || (t.kind() == SyntaxKind::IDENT
                        && crate::semantic::is_data_table_pronoun(t.text()))
            }
            NodeOrToken::Node(_) => false,
        });
    inner_marker || base_is_data_table(ctx, &subset)
}

/// Whether a subscript's base is known to hold a data.table: a name recorded in
/// [`BuildCtx::data_tables`], a constructor call subscripted directly
/// (`data.table(…)[…]`), or a chained subscript (`dt[…][…]`) that is itself
/// data.table-shaped.
fn base_is_data_table(ctx: &BuildCtx<'_>, subset: &SubsetExpr) -> bool {
    match subset.base() {
        Some(NodeOrToken::Token(t)) if t.kind() == SyntaxKind::IDENT => {
            ctx.data_tables.contains(t.text())
        }
        Some(NodeOrToken::Node(inner)) => node_yields_data_table(ctx, &inner),
        _ => false,
    }
}

/// Whether an expression node evaluates to a data.table: a constructor call, or
/// a data.table-shaped subscript (which returns another table). Drives both
/// [`base_is_data_table`] and the assignment tracking that fills
/// [`BuildCtx::data_tables`], so `en.dt <- data.table(…)[, x := y][]` registers
/// `en.dt` and every later `en.dt[…]` masks.
fn node_yields_data_table(ctx: &BuildCtx<'_>, node: &SyntaxNode) -> bool {
    match node.kind() {
        SyntaxKind::CALL_EXPR => CallExpr::cast(node.clone())
            .and_then(|call| call_callee_ident(&call))
            .is_some_and(|callee| crate::semantic::is_data_table_constructor(&callee)),
        SyntaxKind::SUBSET_EXPR => subset_masks_arguments(ctx, node),
        _ => false,
    }
}

/// Whether the `CALL_EXPR` `node`'s bare reads in its argument list should be
/// masked (not recorded as resolvable reads): either the callee data-masks its
/// arguments (`mutate`) or it quotes them without evaluating (`quote`). The
/// callee is the first non-trivia IDENT token directly under the call — which,
/// given how `pkg::fn(args)` parses (the `CALL_EXPR` nests *under* the `::`),
/// is the bare function name for both `mutate(...)` and `dplyr::mutate(...)`.
fn call_masks_arguments(node: &SyntaxNode) -> bool {
    CallExpr::cast(node.clone())
        .and_then(|call| call_callee_ident(&call))
        .is_some_and(|name| {
            crate::semantic::is_data_masking_callee(&name) || is_quoting_callee(&name)
        })
}

/// Whether a call to `name` quotes its argument body rather than evaluating it:
/// `quote`/`bquote`/`substitute`/`expression` capture their arguments as
/// unevaluated language objects, so a bare name inside is not a resolvable read.
/// Name-only (independent of package), matching `is_data_masking_callee`;
/// over-matching only ever suppresses a finding, the conservative direction.
fn is_quoting_callee(name: &str) -> bool {
    matches!(name, "quote" | "bquote" | "substitute" | "expression")
}

fn handle_binary(ctx: &mut BuildCtx<'_>, node: &SyntaxNode, scope: ScopeId) {
    // Detect namespace / member access patterns and opaque custom operators.
    let mut operator_kind: Option<SyntaxKind> = None;
    for el in node.children_with_tokens() {
        if let NodeOrToken::Token(t) = el {
            match t.kind() {
                SyntaxKind::COLON2
                | SyntaxKind::COLON3
                | SyntaxKind::DOLLAR
                | SyntaxKind::AT
                | SyntaxKind::TILDE => {
                    operator_kind = Some(t.kind());
                    break;
                }
                // A user-defined `%...%` operator is opaque: arity does no R
                // evaluation, so it can't know whether the operator captures its
                // operands symbolically (NSE) — as caugi's `A %---% B` does. The
                // base special operators (`%%`, `%in%`, the pipes, …) provably
                // evaluate both operands normally, so only those stay transparent.
                SyntaxKind::USER_OP if !is_transparent_infix(t.text()) => {
                    operator_kind = Some(SyntaxKind::USER_OP);
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
                        // Bare `pkg::name`: the RHS IDENT is the accessed name.
                        NodeOrToken::Token(t) if t.kind() == SyntaxKind::IDENT => {
                            ctx.model.qualified_reads.push(t.text().into());
                        }
                        NodeOrToken::Token(_) => {}
                        NodeOrToken::Node(child) if child.kind() == SyntaxKind::CALL_EXPR => {
                            // Skip the first IDENT (callee); recurse into everything else.
                            // Mask the arguments when the qualified callee is a
                            // data-masking verb (`dplyr::mutate(...)`) or a
                            // quoting callee (`base::quote(...)`); mask just the
                            // model-frame arguments for a qualified model fit
                            // (`MASS::polr(..., data = d, weights = w)`).
                            let masked = call_masks_arguments(child);
                            let model_frame_mask = if masked {
                                None
                            } else {
                                model_frame_arg_mask(child)
                            };
                            let mut skipped_callee = false;
                            for cel in child.children_with_tokens() {
                                match cel {
                                    NodeOrToken::Token(t)
                                        if t.kind() == SyntaxKind::IDENT && !skipped_callee =>
                                    {
                                        // `pkg::name(args)`: the callee is the
                                        // accessed name — a cross-file use.
                                        ctx.model.qualified_reads.push(t.text().into());
                                        skipped_callee = true;
                                    }
                                    NodeOrToken::Node(grandchild)
                                        if model_frame_mask.is_some()
                                            && grandchild.kind() == SyntaxKind::ARG_LIST =>
                                    {
                                        let mask = model_frame_mask.as_deref().unwrap_or(&[]);
                                        walk_model_frame_arg_list(ctx, &grandchild, scope, mask);
                                    }
                                    NodeOrToken::Node(grandchild) => {
                                        // A qualified verb names the package's
                                        // function outright, so no local
                                        // binding can shadow it: pinned.
                                        if masked {
                                            ctx.enter_pinned_mask();
                                        }
                                        walk_node(ctx, &grandchild, scope);
                                        if masked {
                                            ctx.exit_pinned_mask();
                                        }
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
        Some(SyntaxKind::TILDE) => {
            // A formula (`y ~ x`) captures its operands symbolically: the names
            // are model terms (typically data-frame columns), never in-scope
            // reads. Mask the whole subtree so `undefined-symbol` leaves them
            // alone — the same suppress-only direction as data masking.
            ctx.enter_pinned_mask();
            walk_generic(ctx, node, scope);
            ctx.exit_pinned_mask();
        }
        Some(SyntaxKind::USER_OP) => {
            // Opaque custom operator: walk operands with the data mask bumped so
            // their bare names are recorded as reads (an enclosing binding used
            // only here isn't mis-flagged unused) but skipped by `undefined-symbol`.
            // Over-masking only ever suppresses — the safe direction for a
            // false-positive-only rule.
            ctx.enter_pinned_mask();
            // The operator itself is a read of its (backtick-quoted) definition:
            // `a %||% b` uses `` `%||%` ``. Record it masked, matching the operand
            // policy — so a locally- or cross-file-defined operator isn't flagged
            // unused, while an external one stays out of `undefined-symbol`.
            record_user_op_read(ctx, node, scope);
            walk_generic(ctx, node, scope);
            ctx.exit_pinned_mask();
        }
        _ => walk_generic(ctx, node, scope),
    }
}

/// A prefix operator (`!x`, `-x`, `~x`). Only the one-sided formula `~x` needs
/// special handling: like a two-sided formula, its operand is a symbolic model
/// term, so mask the subtree. Every other unary operator evaluates its operand
/// normally and falls through to the default walk.
fn handle_unary(ctx: &mut BuildCtx<'_>, node: &SyntaxNode, scope: ScopeId) {
    let is_formula = node
        .children_with_tokens()
        .any(|el| el.kind() == SyntaxKind::TILDE);
    if is_formula {
        ctx.enter_pinned_mask();
        walk_generic(ctx, node, scope);
        ctx.exit_pinned_mask();
    } else {
        walk_generic(ctx, node, scope);
    }
}

/// True when a `%...%` operator provably evaluates both operands as ordinary
/// value reads, so its operands stay flaggable by `undefined-symbol`. Covers the
/// base special operators and the common pipes; every other (user-defined)
/// operator is treated as opaque and its operands are masked.
fn is_transparent_infix(text: &str) -> bool {
    matches!(
        text,
        // base R special operators
        "%%" | "%/%" | "%*%" | "%o%" | "%in%"
        // magrittr pipes (operands are ordinary value reads; any nested call
        // does its own data-masking)
        | "%>%" | "%T>%" | "%<>%"
    )
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
        walk_element(ctx, el, scope);
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
            // `pkg::fn(args)` parses as a `CALL_EXPR` whose callee is the
            // `pkg::fn` `BINARY_EXPR`; the bare function name is that binary's
            // RHS member. Recover it so data-masking detection (`dplyr::mutate`)
            // and `library()`-shape checks see the same name as an unqualified
            // call.
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::BINARY_EXPR => {
                return namespace_member_name(&n);
            }
            _ => return None,
        }
    }
    None
}

/// The RHS member name of a `pkg::name` / `pkg:::name` `BINARY_EXPR`, if that is
/// its shape (a `::`/`:::` operator with a trailing `IDENT`). `None` for any
/// other binary expression.
fn namespace_member_name(node: &SyntaxNode) -> Option<SmolStr> {
    let mut seen_ns_op = false;
    for el in node.children_with_tokens() {
        match el {
            NodeOrToken::Token(t)
                if matches!(t.kind(), SyntaxKind::COLON2 | SyntaxKind::COLON3) =>
            {
                seen_ns_op = true;
            }
            NodeOrToken::Token(t) if seen_ns_op && t.kind() == SyntaxKind::IDENT => {
                return Some(SmolStr::new(t.text()));
            }
            _ => {}
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
    loop_range: Option<TextRange>,
) -> BindingId {
    let id = BindingId::from_index(model.bindings.len());
    model
        .bindings_by_name
        .entry((scope, name.clone()))
        .or_default()
        .push(id);
    model.bindings.push(Binding {
        name,
        kind,
        scope,
        def_range,
        loop_range,
        read: false,
    });
    model.scopes[scope.0 as usize].bindings.push(id);
    id
}

/// Walk every recorded identifier read and mark the binding(s) it reaches as
/// `read`. Used by `unused-binding`.
fn resolve_reads(model: &mut SemanticModel) {
    model.binding_reads = vec![Vec::new(); model.bindings.len()];
    model.ident_bindings = Vec::with_capacity(model.idents.len());
    for ident_idx in 0..model.idents.len() {
        let ident = model.idents[ident_idx].clone();
        // Compute the reached bindings first (immutable borrow of `model` via
        // `reads_reached`), then record both edge directions.
        let reached = reads_reached(model, &ident);
        for id in &reached {
            model.bindings[id.0 as usize].read = true;
            model.binding_reads[id.0 as usize].push(ident_idx as u32);
        }
        model.ident_bindings.push(reached);
    }
}

/// The binding(s) a single identifier read marks as `read`, found by walking
/// from the read's own scope outward.
///
/// R only introduces a new variable scope at a `function` (and the file top):
/// `for`/`{}` blocks share their enclosing function's execution *frame*, where
/// statements run in source order. So within the read's own frame — every scope
/// up to and including the nearest enclosing `function`/file — a read can only
/// refer to a same-name binding assigned *before* it. We resolve to the
/// innermost frame scope that holds such a binding and mark *every* preceding
/// one there: marking all (not just the nearest) keeps a reassignment
/// conservative — in `x <- a; f(x); x <- b; f(x)` both `x`s have a later read,
/// so neither is a false unused-binding.
///
/// Past the frame boundary lie *enclosing* functions, reached only through a
/// closure, whose body runs when the closure is later called. Textual position
/// carries no ordering there (the closure can read a binding defined after it),
/// so the first match suffices.
fn reads_reached(model: &SemanticModel, ident: &IdentRef) -> Vec<BindingId> {
    let mut current = Some(ident.scope);
    // Whether the scope under inspection still belongs to the read's own frame.
    // It does until we step *past* the first `function`/file scope (a closure
    // boundary).
    let mut in_frame = true;
    while let Some(scope_id) = current {
        let scope_ref = &model.scopes[scope_id.0 as usize];
        // Only this scope's same-name bindings, via the name index; their
        // order is `scope_ref.bindings` order by construction.
        let named = model.scope_bindings_named(scope_id, &ident.name);
        let matches = || {
            named
                .iter()
                .copied()
                .filter(|id| model.bindings[id.0 as usize].def_range != ident.range)
        };

        if in_frame {
            // A same-frame read reaches a binding assigned *before* it in source
            // order — or, when the binding sits in a loop body that also contains
            // the read, one assigned *after* it too: the loop re-executes, so on a
            // later iteration the assignment precedes the read (loop-carried use).
            let preceding: Vec<BindingId> = matches()
                .filter(|id| {
                    let b = &model.bindings[id.0 as usize];
                    b.def_range.start() < ident.range.start()
                        || b.loop_range
                            .is_some_and(|lr| lr.contains_range(ident.range))
                        // A deferred (promise) read carries no textual ordering
                        // within its frame: the default / `on.exit` / `NextMethod`
                        // expression runs after body statements may have assigned a
                        // same-name local, so an assignment *after* the read counts.
                        || ident.deferred
                })
                .collect();
            if !preceding.is_empty() {
                return preceding;
            }
        } else if let Some(id) = matches().next() {
            return vec![id];
        }

        // The frame ends at the first `function`/file scope: its parent (if any)
        // is an enclosing function, visible only via a closure.
        if matches!(scope_ref.kind, ScopeKind::Function | ScopeKind::File) {
            in_frame = false;
        }
        current = scope_ref.parent;
    }
    Vec::new()
}
