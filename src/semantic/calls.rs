//! Conservative recovery of signatures from local closure definitions.

use rowan::TextRange;
use rowan::ast::AstNode as _;

use crate::ast::{AssignmentExpr, FunctionExpr};
use crate::semantic::{BindingId, BindingKind, SemanticModel};
use crate::syntax::{SyntaxKind, SyntaxNode};

/// Call effects gathered during the semantic walk, including quoted and
/// qualified callees that do not appear among ordinary identifier reads.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SignatureEffects {
    dynamic_bindings: bool,
    dispatch_calls: Vec<TextRange>,
}

impl SignatureEffects {
    pub(crate) fn record(&mut self, name: &str, range: TextRange) {
        let name = super::symbols::unbacktick(name);
        self.dynamic_bindings |= name.contains('\\')
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
            );
        if matches!(name, "UseMethod" | "NextMethod") {
            self.dispatch_calls.push(range);
        }
    }
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
    if model
        .bindings()
        .iter()
        .any(|other| other.name == binding.name && other.kind == BindingKind::Implicit)
    {
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
