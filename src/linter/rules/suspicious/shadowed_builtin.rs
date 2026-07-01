//! `shadowed-builtin`: a binding to a **function** whose name is exported by a
//! default R package AND that name is later used in a call context in the same
//! scope.
//!
//! The hazard is narrow. `names <- names(x)` is *not* a footgun: R's
//! call-position lookup skips non-function bindings, so a later `names(y)` still
//! reaches base `names` (verified against R — this is the dominant tidyverse
//! idiom). It only bites when the local is itself a function: `c <- function(...)
//! ...; c(2, 3)` calls the local, not base `c`. So we require both a
//! function-literal RHS and a later call before firing.

use rowan::TextRange;
use rowan::ast::AstNode as _;

use crate::ast::AssignmentExpr;
use crate::linter::diagnostic::{Diagnostic, Severity, ViolationData};
use crate::linter::rules::matchers::is_callee;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::semantic::BindingKind;
use crate::syntax::{SyntaxKind, SyntaxNode};

/// The enclosing `ASSIGNMENT_EXPR` that defines the binding at `def_range`, if
/// any. Local bindings always have one; the `None` fallback is a conservative
/// withhold for anything unexpected.
fn defining_assignment(root: &SyntaxNode, def_range: TextRange) -> Option<SyntaxNode> {
    root.covering_element(def_range)
        .into_token()
        .and_then(|t| t.parent())
        .filter(|p| p.kind() == SyntaxKind::ASSIGNMENT_EXPR)
}

/// Whether the assignment binds a **function literal** (`f <- function(...) ...`
/// or `f <- \(...) ...`). Only a function value can shadow a base function in
/// call position; a value RHS (`names <- names(x)`) cannot, so it is exempt.
/// A function-*producing* expression we can't see through (`f <- Negate(g)`) is
/// conservatively treated as a non-function and withheld.
fn binds_function_literal(assign: &SyntaxNode) -> bool {
    AssignmentExpr::cast(assign.clone())
        .and_then(|a| a.value_element())
        .is_some_and(|value| value.kind() == SyntaxKind::FUNCTION_EXPR)
}

pub struct ShadowedBuiltin;

impl Rule for ShadowedBuiltin {
    fn id(&self) -> &'static str {
        "shadowed-builtin"
    }

    fn description(&self) -> &'static str {
        "Flag a local binding to a function whose name is exported by a default R \
         package when that name is later called in the same scope (`c <- \
         function(...) ...; c(2, 3)`). A value binding (`names <- names(x)`) is \
         exempt: R's call-position lookup skips non-function locals, so it is not \
         a hazard."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "Binding a function over base `c()` and then calling it:",
            source: "c <- function(x, y) x\nc(2, 3)\n",
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
            // Only a binding to a *function* can shadow a base function at a call
            // site. `names <- names(x)` binds a value, and R's call-position
            // lookup skips it — so exempt it (this is the tidyverse-dominant
            // idiom). Requiring a function-literal RHS also gives us the defining
            // assignment we need to bound "later" calls below.
            let Some(assign) = defining_assignment(ctx.root, binding.def_range) else {
                continue;
            };
            if !binds_function_literal(&assign) {
                continue;
            }
            // Look for a *call* of this name in the same scope (or a descendant
            // scope) that occurs *after* the definition: `c <- function(...) ...;
            // c(2, 3)`. A mere value read (`beta[[i]]`, `beta + 1`) carries no "I
            // meant the base function" hazard, so it must not trigger. We
            // approximate "in the same scope" by walking idents and checking that
            // the binding resolves them.
            let id = crate::semantic::BindingId(binding_idx as u32);
            // Measure "after" from the end of the whole defining statement, not
            // the LHS token: `f <- function(x) f(x)` names itself in its own RHS,
            // which isn't a "later" use.
            let def_end = assign.text_range().end();
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
