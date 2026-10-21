//! String native-routine names retain runtime lookup where a registered symbol
//! makes the intended entry point explicit. Registration and symbol bindings
//! already belong to the project graph; this rule never scans native sources.

use rowan::ast::AstNode as _;

use crate::ast::{BinaryExpr, CallExpr};
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext, matchers};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct RoutineRegistration;

impl Rule for RoutineRegistration {
    fn id(&self) -> &'static str {
        "routine-registration"
    }

    fn description(&self) -> &'static str {
        "Flag string-named `.Call`, `.C`, `.Fortran`, and `.External` calls in \
         package R sources. Prefer registered native symbols to runtime string \
         lookup. A string still triggers the rule when the routine is registered: \
         registration alone does not make the R call use the symbol binding.\n\n\
         Only files directly in a package's `R/` directory are checked. Dynamic \
         names and symbol arguments are left alone. Existing NAMESPACE and native \
         registration facts supply symbol visibility through the project graph; \
         the rule never runs R or inspects an installed package.\n\n\
         There is no automatic fix: the symbol's binding may depend on \
         `useDynLib()` aliases or `.fixes`, and changing registration can require \
         edits to NAMESPACE and native sources."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "In a package's `R/` directory:",
            source: "f <- function(x) .Call(\"native_fun\", x)\n",
        }]
    }

    fn doc_package(&self) -> &'static [(&'static str, &'static str)] {
        &[("DESCRIPTION", "Package: mypkg\nVersion: 0.1.0\n")]
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        if !ctx.is_package_r_source() || ctx.own_package().is_none() {
            return;
        }
        let Some(call) = el.as_node().cloned().and_then(CallExpr::cast) else {
            return;
        };
        let Some(name) = matchers::callee_name(&call) else {
            return;
        };
        if !matches!(name.as_str(), ".Call" | ".C" | ".Fortran" | ".External") {
            return;
        }
        let qualified_base = call
            .base()
            .and_then(|el| el.into_node())
            .and_then(BinaryExpr::cast)
            .and_then(|expr| expr.namespace_access())
            .is_some_and(|access| access.package == "base");
        if !qualified_base
            && (!ctx.resolves_to_base(&call)
                || ctx.project.is_some_and(|scope| scope.resolves(&name)))
        {
            return;
        }
        let Some(head) = matchers::named_arg(&call, ".NAME")
            .or_else(|| matchers::nth_arg(&call, 0))
            .and_then(|el| el.into_token())
            .filter(|token| token.kind() == SyntaxKind::STRING)
        else {
            return;
        };
        sink.push(Diagnostic {
            rule: self.id(),
            severity: Default::default(),
            path: Default::default(),
            range: head.text_range(),
            message: ViolationData::new(
                self.id(),
                format!("`{name}` looks up a native routine by string name"),
            )
            .with_suggestion("Register the native routine and pass its registered symbol binding."),
            fix: None,
        });
    }
}
