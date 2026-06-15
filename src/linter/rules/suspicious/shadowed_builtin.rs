//! `shadowed-builtin`: a binding whose name is exported by a default R package
//! AND that name is later used in a call context in the same scope.
//!
//! Defining `c <- 1` is fine on its own. It's `c <- 1; c(2, 3)` that bites —
//! you think you're calling base `c()`, but R uses your local. The two-step
//! trigger keeps false positives down.

use rowan::TextRange;
use rowan::ast::AstNode;

use crate::ast::CallExpr;
use crate::linter::diagnostic::{Diagnostic, Severity, ViolationData};
use crate::linter::rules::{Rule, RuleContext};
use crate::semantic::BindingKind;
use crate::syntax::{SyntaxKind, SyntaxNode};

pub struct ShadowedBuiltin;

/// Whether the identifier covering `range` sits in the callee position of a
/// call (`name(…)`) — as opposed to a value read like `name[[i]]` or `name + 1`.
/// This is the rule's "you meant the base function but got your local" trigger.
fn is_callee(root: &SyntaxNode, range: TextRange) -> bool {
    let rowan::SyntaxElement::Token(token) = root.covering_element(range) else {
        return false;
    };
    let Some(parent) = token.parent() else {
        return false;
    };
    parent.kind() == SyntaxKind::CALL_EXPR
        && CallExpr::cast(parent)
            .and_then(|call| call.callee_token())
            .is_some_and(|callee| callee.text_range() == range)
}

impl Rule for ShadowedBuiltin {
    fn id(&self) -> &'static str {
        "shadowed-builtin"
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn run(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for (binding_idx, binding) in ctx.model.bindings().iter().enumerate() {
            if !matches!(binding.kind, BindingKind::Local | BindingKind::Param) {
                continue;
            }
            if !ctx.symbols.is_base(&binding.name) {
                continue;
            }
            // Look for a *call* of this name in the same scope (or a descendant
            // scope) that occurs *after* the definition: `c <- 1; c(2, 3)`. A
            // mere value read (`beta[[i]]`, `beta + 1`) carries no "I meant the
            // base function" hazard, so it must not trigger. We approximate "in
            // the same scope" by walking idents and checking that the binding
            // resolves them.
            let id = crate::semantic::BindingId(binding_idx as u32);
            let triggered = ctx.model.idents().iter().any(|ident| {
                ident.name == binding.name
                    && u32::from(ident.range.start()) > u32::from(binding.def_range.end())
                    && is_callee(ctx.root, ident.range)
                    && ctx.model.resolve_local(ident) == Some(id)
            });
            if !triggered {
                continue;
            }
            out.push(Diagnostic {
                rule: "shadowed-builtin",
                severity: Severity::Warning,
                path: Default::default(),
                range: binding.def_range,
                message: ViolationData::new(
                    "shadowed-builtin",
                    format!(
                        "local binding `{}` shadows a base-R name later used in this scope",
                        binding.name
                    ),
                )
                .with_suggestion(
                    "Rename the local, or fully qualify the base call (e.g. `base::c`).",
                ),
                fix: None,
            });
        }
        out
    }
}
