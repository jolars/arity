//! `shadowed-builtin`: a binding whose name is exported by a default R package
//! AND that name is later used in a call context in the same scope.
//!
//! Defining `c <- 1` is fine on its own. It's `c <- 1; c(2, 3)` that bites —
//! you think you're calling base `c()`, but R uses your local. The two-step
//! trigger keeps false positives down.

use rowan::{TextRange, TextSize};

use crate::linter::diagnostic::{Diagnostic, Severity, ViolationData};
use crate::linter::rules::matchers::is_callee;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::semantic::BindingKind;
use crate::syntax::{SyntaxKind, SyntaxNode};

/// The end offset of the statement that defines the binding at `def_range` — the
/// enclosing `ASSIGNMENT_EXPR` if there is one, else the def range's own end.
/// Used so a call inside the *defining* assignment's RHS (`sign <- sign(x)`)
/// doesn't count as a "later" call of the shadowing name.
fn defining_stmt_end(root: &SyntaxNode, def_range: TextRange) -> TextSize {
    root.covering_element(def_range)
        .into_token()
        .and_then(|t| t.parent())
        .filter(|p| p.kind() == SyntaxKind::ASSIGNMENT_EXPR)
        .map(|assign| assign.text_range().end())
        .unwrap_or_else(|| def_range.end())
}

pub struct ShadowedBuiltin;

impl Rule for ShadowedBuiltin {
    fn id(&self) -> &'static str {
        "shadowed-builtin"
    }

    fn description(&self) -> &'static str {
        "Flag a local binding whose name is exported by a default R package when \
         that name is later used as a call in the same scope (`c <- 1; \
         c(2, 3)`). The two-step trigger keeps false positives down."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "Shadowing base `c()` and then calling it:",
            source: "c <- 1\nc(2, 3)\n",
        }]
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        for (binding_idx, binding) in ctx.model.bindings().iter().enumerate() {
            // Only local `<-` shadowing is a smell. A parameter named after a base
            // function (`transform = identity`, `round = 3`, `names = ...`) is
            // idiomatic — it's the intended call target, and R's function-vs-value
            // lookup resolves same-named calls correctly — so parameters are exempt.
            if !matches!(binding.kind, BindingKind::Local) {
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
            // Measure "after" from the end of the whole defining statement, not
            // the LHS token: `sign <- sign(x)` evaluates its RHS before the local
            // binding is live, so that call is not a "later" use. Fall back to the
            // def range for bindings without an enclosing assignment (e.g. params).
            let def_end = defining_stmt_end(ctx.root, binding.def_range);
            let triggered = ctx.model.idents().iter().any(|ident| {
                ident.name == binding.name
                    && u32::from(ident.range.start()) >= u32::from(def_end)
                    && is_callee(ctx.root, ident.range)
                    && ctx.model.resolve_local(ident) == Some(id)
            });
            if !triggered {
                continue;
            }
            sink.push(Diagnostic {
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
    }
}
