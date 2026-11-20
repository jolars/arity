//! `missing-argument`: empty call slots are usually accidental. Trailing and
//! named holes are checked only for established rejecting base calls.
//! Function formals use a different CST shape
//! and are likewise outside this rule. There is no generally safe deletion or
//! replacement, so findings carry no autofix.

use rowan::ast::AstNode as _;

use crate::ast::{
    Arg, AssignmentExpr, BinaryExpr, CallExpr, HasArgList as _, NamespaceAccess, UnaryExpr,
};
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::semantic::calls::{has_dynamic_bindings, omission_checks, stable_closure};
use crate::semantic::symbols::PackageOrigin;
use crate::semantic::{BindingKind, checked_match_args_to_formals};
use crate::syntax::{NodePtr, SyntaxElement, SyntaxKind, SyntaxToken};

pub struct MissingArgument;

impl Rule for MissingArgument {
    fn id(&self) -> &'static str {
        "missing-argument"
    }

    fn description(&self) -> &'static str {
        "Flag empty call arguments, including trailing and named holes in resolved base `list()` calls and `c()` calls with only atomic literal values. A resolved closure can explicitly handle an omitted formal with a `missing()` guard. The first two arguments to ggplot2 `aes()` can be left empty before a tidy-evaluation splice. Unknown or potentially dispatching callees retain the trailing-hole exemption. Function-formal defaults are excluded."
    }

    fn examples(&self) -> &'static [Example] {
        &[
            Example {
                caption: "The second call argument is missing:",
                source: "paste(\"a\", , \"b\")\n",
            },
            Example {
                caption: "Base list and literal c calls reject trailing or named holes:",
                source: "list(1,)\nc(value = )\n",
            },
        ]
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(call) = el.as_node().cloned().and_then(CallExpr::cast) else {
            return;
        };
        let args: Vec<_> = call.args().collect();
        if args.iter().any(|arg| hole_comma(arg).is_some()) {
            let accepted = accepted_holes(&call, &args, ctx);
            for (index, arg) in args.iter().enumerate() {
                if let Some(comma) = hole_comma(arg)
                    && !accepted.get(index).copied().unwrap_or(false)
                {
                    report_positional_hole(comma, sink);
                }
            }
        }
        check_rejecting_call(&call, &args, ctx, sink);
    }
}

fn hole_comma(arg: &Arg) -> Option<SyntaxToken> {
    if arg.is_named() || arg.value().is_some() {
        return None;
    }
    arg.syntax()
        .next_sibling_or_token()
        .and_then(SyntaxElement::into_token)
        .filter(|token| token.kind() == SyntaxKind::COMMA)
}

fn report_positional_hole(comma: SyntaxToken, sink: &mut Vec<Diagnostic>) {
    sink.push(Diagnostic {
        rule: "missing-argument",
        severity: Default::default(),
        path: Default::default(),
        range: comma.text_range(),
        message: ViolationData::new(
            "missing-argument",
            "call contains an empty argument before this comma",
        )
        .with_suggestion("Supply the intended argument explicitly."),
        fix: None,
    });
}

fn namespace_access(call: &CallExpr) -> Option<NamespaceAccess> {
    BinaryExpr::cast(call.base()?.into_node()?)?.namespace_access()
}

fn check_rejecting_call(
    call: &CallExpr,
    args: &[Arg],
    ctx: &RuleContext<'_>,
    sink: &mut Vec<Diagnostic>,
) {
    let Some(name) = call.callee_name() else {
        return;
    };
    if !matches!(name.as_str(), "list" | "c") {
        return;
    }
    let qualified_base = namespace_access(call).is_some_and(|access| access.package == "base");
    if (!qualified_base && !ctx.resolves_to_base(call)) || matchers::in_pipe(call) {
        return;
    }
    // R rejects holes in list() and in the default c() path. Literal-only c()
    // calls cannot supply a classed dispatch receiver; arbitrary values can.
    // Forwarded dots can alter the call, so neither shape is classified here.
    if args.iter().filter_map(Arg::value).any(|value| {
        matchers::element_text(&value) == "..."
            || (name == "c"
                && !matches!(
                    value.kind(),
                    SyntaxKind::INT | SyntaxKind::FLOAT | SyntaxKind::STRING
                ))
    }) {
        return;
    }
    // Non-trailing unnamed holes are reported separately.
    for arg in args {
        if arg.value().is_none() && arg.name().is_some() {
            report_hole(arg.syntax().text_range(), sink);
        }
    }
    let Some(list) = call.arg_list() else {
        return;
    };
    // A trailing comma has no value ARG node. Comments may have their own ARG
    // nodes, but they do not fill the missing slot.
    if let Some(comma) = list
        .syntax()
        .children_with_tokens()
        .filter(|el| {
            !matches!(
                el.kind(),
                SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
            )
        })
        .filter(|el| {
            !el.as_node().is_some_and(|node| {
                Arg::cast(node.clone()).is_some_and(|arg| {
                    arg.name().is_none()
                        && arg.value().is_none()
                        && !arg.syntax().text_range().is_empty()
                })
            })
        })
        .last()
        .filter(|el| el.kind() == SyntaxKind::COMMA)
    {
        report_hole(comma.text_range(), sink);
    }
}

fn report_hole(range: rowan::TextRange, sink: &mut Vec<Diagnostic>) {
    sink.push(Diagnostic {
        rule: "missing-argument",
        severity: Default::default(),
        path: Default::default(),
        range,
        message: ViolationData::new("missing-argument", "base call rejects this empty argument")
            .with_suggestion("Supply the intended argument explicitly."),
        fix: None,
    });
}

/// Project matching ignores trivia-only ARG nodes, which R does not receive.
fn accepted_holes(call: &CallExpr, args: &[Arg], ctx: &RuleContext<'_>) -> Vec<bool> {
    if matchers::in_pipe(call) {
        return vec![];
    }
    let supplied: Vec<_> = args
        .iter()
        .enumerate()
        .filter(|(_, arg)| arg.is_named() || arg.value().is_some() || hole_comma(arg).is_some())
        .collect();
    if supplied.iter().any(|(_, arg)| {
        arg.value()
            .is_some_and(|value| matchers::element_text(&value) == "...")
    }) {
        return vec![];
    }
    if let Some((splice, _)) = supplied
        .iter()
        .find(|(_, arg)| !arg.is_named() && is_splice(arg))
        && is_ggplot2_aes(call, ctx)
    {
        // aes captures x and y separately and discards missing mappings before
        // checking the combined argument list. Dots remain subject to the rule.
        let ordinary: Vec<_> = supplied
            .iter()
            .filter(|(_, arg)| arg.is_named() || !is_splice(arg))
            .collect();
        let names: Vec<_> = ordinary.iter().map(|(_, arg)| arg.name()).collect();
        let mut accepted = vec![false; args.len()];
        if let Ok(matched) = checked_match_args_to_formals(&names, &["x", "y", "..."]) {
            for ((index, arg), formal) in ordinary.into_iter().zip(matched) {
                accepted[*index] = index < splice && formal.is_some() && hole_comma(arg).is_some();
            }
        }
        return accepted;
    }
    let Some(token) = call.base().and_then(SyntaxElement::into_token) else {
        return vec![];
    };
    if has_dynamic_bindings(ctx.model) || supplied.iter().any(|(_, arg)| is_splice(arg)) {
        return vec![];
    }
    let names: Vec<_> = supplied.iter().map(|(_, arg)| arg.name()).collect();
    if names.iter().flatten().any(|name| name.contains('\\')) {
        return vec![];
    }
    let Some(callees) = resolved_callees(&token, ctx) else {
        return vec![];
    };
    let mut accepted = vec![false; args.len()];
    for &(index, arg) in &supplied {
        accepted[index] = hole_comma(arg).is_some();
    }
    for (formals, checks) in callees {
        if checks.is_empty() || formals.iter().any(|name| name.contains('\\')) {
            return vec![];
        }
        let Ok(matched) = checked_match_args_to_formals(&names, &formals) else {
            return vec![];
        };
        for ((index, _), formal) in supplied.iter().zip(matched) {
            accepted[*index] &= formal.is_some_and(|name| checks.contains(name));
        }
    }
    accepted
}

type OmissionSignature = (Vec<String>, std::collections::BTreeSet<String>);

fn resolved_callees(token: &SyntaxToken, ctx: &RuleContext<'_>) -> Option<Vec<OmissionSignature>> {
    if let Some(signature) = closure_signature(token, ctx) {
        return Some(vec![signature]);
    }
    let ident = ctx
        .model
        .idents()
        .iter()
        .find(|ident| ident.range == token.text_range() && !ident.data_masked)?;
    let id = ctx.model.resolve_local(ident)?;
    let binding = ctx.model.binding(id);
    // A local alias can be assigned in multiple branches. Every assignment in
    // its frame must point to a known closure with the same omission behavior.
    let frame = ctx.model.frame_scope(ident.scope);
    if ctx.model.frame_scope(binding.scope) != frame
        || ctx.model.bindings().iter().any(|candidate| {
            candidate.kind == BindingKind::Implicit && candidate.name == binding.name
        })
    {
        return None;
    }
    let mut assignments = Vec::new();
    let signatures = ctx
        .model
        .variable_cohort(id)
        .into_iter()
        .map(|id| {
            let binding = ctx.model.binding(id);
            if binding.kind != BindingKind::Local || binding.available_from > ident.range.start() {
                return None;
            }
            let assignment = ctx
                .root
                .covering_element(binding.def_range)
                .into_token()?
                .parent()
                .and_then(AssignmentExpr::cast)?;
            assignments.push(assignment.syntax().text_range());
            let target = assignment.value_element()?.into_token()?;
            closure_signature(&target, ctx)
        })
        .collect::<Option<Vec<_>>>()?;
    let function = token
        .parent_ancestors()
        .find(|node| node.kind() == SyntaxKind::FUNCTION_EXPR);
    let graph = match function {
        Some(function) => ctx.cfg.function(NodePtr::from_node(&function))?,
        None => ctx.cfg.toplevel(),
    };
    graph
        .must_follow_statement(ident.range, &assignments)
        .then_some(signatures)
}

fn closure_signature(token: &SyntaxToken, ctx: &RuleContext<'_>) -> Option<OmissionSignature> {
    let (index, ident) = ctx
        .model
        .idents()
        .iter()
        .enumerate()
        .find(|(_, ident)| ident.range == token.text_range() && !ident.data_masked)?;
    if let Some(id) = ctx.model.resolve_local(ident) {
        let binding = ctx.model.binding(id);
        if ctx.model.ident_bindings(index) != [id]
            || (ctx.model.frame_scope(ident.scope) == ctx.model.frame_scope(binding.scope)
                && ident.range.start() < binding.def_range.end())
        {
            return None;
        }
        let function = stable_closure(ctx.model, ctx.root, id)?;
        let formals = function
            .params()
            .into_iter()
            .map(|param| param.name.to_string())
            .collect();
        let checks = omission_checks(ctx.model, &function, |call| ctx.resolves_to_base(call));
        return Some((formals, checks));
    }
    let summary = ctx.resolution?.local_functions.get(ident.name.as_str())?;
    if !summary.signature_known || !ctx.introduced_call_resolves_to_base("missing") {
        return None;
    }
    Some((summary.formals.clone(), summary.omission_checks.clone()))
}

fn is_splice(arg: &Arg) -> bool {
    let Some(mut value) = arg.value() else {
        return false;
    };
    for _ in 0..3 {
        let Some(not) = value.into_node().and_then(UnaryExpr::cast) else {
            return false;
        };
        if not.op_kind() != Some(SyntaxKind::BANG) {
            return false;
        }
        let Some(operand) = not.operand() else {
            return false;
        };
        value = operand;
    }
    true
}

fn is_ggplot2_aes(call: &CallExpr, ctx: &RuleContext<'_>) -> bool {
    if call.callee_name().as_deref() != Some("aes") {
        return false;
    }
    if let Some(access) = namespace_access(call) {
        return access.package == "ggplot2";
    }
    let Some(token) = call.callee_token() else {
        return false;
    };
    if ctx.own_package() == Some("ggplot2")
        && ctx
            .project
            .is_some_and(|scope| !scope.resolution_incomplete && scope.exported_by_namespace("aes"))
        && resolves_to_package_aes(&token, ctx)
    {
        return true;
    }
    !ctx.is_locally_shadowed(token.text_range())
        && match ctx.symbols.origin("aes", ctx.model.loaded_packages()) {
            PackageOrigin::Resolved(package) => package == "ggplot2",
            PackageOrigin::Ambiguous(packages) => {
                packages.last().is_some_and(|package| package == "ggplot2")
            }
            PackageOrigin::Unknown => false,
        }
}

fn resolves_to_package_aes(token: &SyntaxToken, ctx: &RuleContext<'_>) -> bool {
    let Some((index, ident)) = ctx
        .model
        .idents()
        .iter()
        .enumerate()
        .find(|(_, ident)| ident.range == token.text_range() && !ident.data_masked)
    else {
        return false;
    };
    if ctx.model.is_free_read(ident) {
        return true;
    }
    // testthat gives each test body its own environment. A binding created in
    // another test cannot shadow the package function here.
    if !crate::semantic::symbols::implicit_attached_packages(ctx.path).contains(&"testthat") {
        return false;
    }
    let Some(current_test) = testthat_body(token.clone(), ctx) else {
        return false;
    };
    let resolution = ctx.model.identifier_resolution(ident);
    let bindings = ctx.model.ident_bindings(index);
    !bindings.is_empty()
        && !resolution.uncertain
        && resolution.generated.is_empty()
        && bindings.iter().all(|&id| {
            let binding = ctx.model.binding(id);
            binding.kind == BindingKind::Local
                && ctx
                    .root
                    .covering_element(binding.def_range)
                    .into_token()
                    .and_then(|token| testthat_body(token, ctx))
                    .is_some_and(|other_test| other_test != current_test)
        })
}

fn testthat_body(token: SyntaxToken, ctx: &RuleContext<'_>) -> Option<rowan::TextRange> {
    let range = token.text_range();
    token
        .parent_ancestors()
        .filter_map(CallExpr::cast)
        .find_map(|call| {
            if call.callee_name()?.as_str() != "test_that" {
                return None;
            }
            if let Some(access) = namespace_access(&call) {
                if access.package != "testthat" {
                    return None;
                }
            } else if ctx.is_locally_shadowed(call.callee_token()?.text_range()) {
                return None;
            }
            let body = call.nth_positional(1)?.text_range();
            body.contains_range(range).then_some(body)
        })
}
