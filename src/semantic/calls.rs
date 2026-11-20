//! Conservative recovery of signatures from local closure definitions.

use std::collections::BTreeSet;

use rowan::TextRange;
use rowan::ast::AstNode as _;

use crate::ast::{
    AssignmentExpr, BinaryExpr, CallExpr, FunctionExpr, HasArgList as _, IfExpr, ParenExpr,
    UnaryExpr,
};
use crate::semantic::{BindingId, BindingKind, SemanticModel};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

/// Call effects gathered during the semantic walk, including quoted and
/// qualified callees that do not appear among ordinary identifier reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SignatureEffects {
    dynamic_bindings: bool,
    dispatch_calls: Vec<TextRange>,
    generator_mutations: bool,
}

impl SignatureEffects {
    pub(crate) fn record_member_call(&mut self, name: &str) {
        self.generator_mutations |= matches!(
            super::symbols::unbacktick(name),
            "set" | "methods" | "lock" | "unlock" | "accessors"
        );
    }
    pub(crate) fn record(&mut self, name: &str, range: TextRange) {
        let name = super::symbols::unbacktick(name);
        self.dynamic_bindings |= changes_bindings(name);
        if matches!(name, "UseMethod" | "NextMethod") {
            self.dispatch_calls.push(range);
        }
    }
}

pub(crate) fn has_generator_mutations(model: &SemanticModel) -> bool {
    model.signature_effects.generator_mutations
}

pub(crate) fn changes_bindings(name: &str) -> bool {
    name.contains('\\')
        || matches!(
            name,
            "assign"
                | "delayedAssign"
                | "makeActiveBinding"
                | "eval"
                | "evalq"
                | "source"
                | "sys.source"
                | "load"
                | "attach"
                | "rm"
                | "remove"
                | "formals<-"
                | "body<-"
                | "environment<-"
        )
}

/// Known operations can replace bindings or change closure signatures, so
/// static consumers must withhold signature-based diagnostics in their presence.
pub(crate) fn has_dynamic_bindings(model: &SemanticModel) -> bool {
    model.attaches_opaque_env() || model.signature_effects.dynamic_bindings
}

/// A unique, unconditional closure literal for this binding. This recovers
/// syntax through the semantic definition site, without another CST traversal.
pub(crate) fn stable_closure(
    model: &SemanticModel,
    root: &SyntaxNode,
    id: BindingId,
) -> Option<FunctionExpr> {
    let binding = model.binding(id);
    if binding.kind != BindingKind::Local || model.variable_cohort(id).len() != 1 {
        return None;
    }
    // A super-assignment from another frame can replace the closure as well.
    if model.bindings().iter().any(|other| {
        super::symbols::unbacktick(&other.name) == super::symbols::unbacktick(&binding.name)
            && other.kind == BindingKind::Implicit
    }) {
        return None;
    }
    let assignment = root
        .covering_element(binding.def_range)
        .into_token()?
        .parent()
        .and_then(AssignmentExpr::cast)?;
    let function = assignment
        .value_element()?
        .into_node()
        .and_then(FunctionExpr::cast)?;
    for ancestor in assignment.syntax().ancestors().skip(1) {
        match ancestor.kind() {
            SyntaxKind::ROOT => break,
            SyntaxKind::FUNCTION_EXPR => {
                // An assignment in a formal's default is deferred and may
                // never run. Only a definition in the body establishes a local.
                if !FunctionExpr::cast(ancestor)?.body().is_some_and(|body| {
                    body.text_range()
                        .contains_range(assignment.syntax().text_range())
                }) {
                    return None;
                }
                break;
            }
            SyntaxKind::BLOCK_EXPR => {}
            _ => return None,
        }
    }
    // A dispatching closure does not establish the method's argument behavior.
    if model
        .signature_effects
        .dispatch_calls
        .iter()
        .any(|range| function.syntax().text_range().contains_range(*range))
    {
        return None;
    }
    Some(function)
}

/// Formals explicitly tested by a missingness guard in this closure's body.
pub(crate) fn omission_checks(
    model: &SemanticModel,
    function: &FunctionExpr,
    bare_missing: impl Fn(&CallExpr) -> bool,
) -> BTreeSet<String> {
    let Some(body) = function.body().and_then(SyntaxElement::into_node) else {
        return BTreeSet::new();
    };
    body.descendants()
        .filter_map(IfExpr::cast)
        .filter_map(|branch| {
            let condition = branch
                .condition_elements()?
                .into_iter()
                .find_map(SyntaxElement::into_node)?;
            let mut node = condition;
            loop {
                node = if let Some(paren) = ParenExpr::cast(node.clone()) {
                    paren.inner()?.into_node()?
                } else if let Some(not) = UnaryExpr::cast(node.clone())
                    && not.op_kind() == Some(SyntaxKind::BANG)
                {
                    not.operand()?.into_node()?
                } else {
                    break;
                };
            }
            let call = CallExpr::cast(node)?;
            if call.callee_name()?.as_str() != "missing"
                || call
                    .syntax()
                    .ancestors()
                    .find(|node| node.kind() == SyntaxKind::FUNCTION_EXPR)
                    .as_ref()
                    != Some(function.syntax())
                || super::builder::is_quoted(call.syntax())
            {
                return None;
            }
            let base = call.base()?;
            let is_base = match base {
                SyntaxElement::Node(node) => BinaryExpr::cast(node)?
                    .namespace_access()
                    .is_some_and(|access| access.package == "base"),
                SyntaxElement::Token(_) => bare_missing(&call),
            };
            if !is_base {
                return None;
            }
            let mut args = call.args();
            let arg = args.next()?;
            if arg.is_named() || args.next().is_some() {
                return None;
            }
            let token = arg.value()?.into_token()?;
            let ident = model
                .idents()
                .iter()
                .find(|ident| ident.range == token.text_range())?;
            let binding = model.binding(model.resolve_local(ident)?);
            (binding.kind == BindingKind::Param
                && model.scope(model.frame_scope(binding.scope)).range
                    == function.syntax().text_range())
            .then(|| binding.name.to_string())
        })
        .collect()
}
