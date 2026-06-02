//! `unused-binding`: a local binding that is never read in the same file.
//!
//! Excludes function parameters and `for`-loop variables (those have semantic
//! meaning even when unused — they're part of the API surface). Names starting
//! with `.` are skipped too, following R convention for intentionally unused
//! identifiers.

use crate::linter::diagnostic::{Diagnostic, Severity, ViolationData};
use crate::linter::rules::{Rule, RuleContext};

pub struct UnusedBinding;

impl Rule for UnusedBinding {
    fn id(&self) -> &'static str {
        "unused-binding"
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn run(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        ctx.model
            .unused_local_bindings()
            .map(|id| {
                let b = ctx.model.binding(id);
                Diagnostic {
                    rule: "unused-binding",
                    severity: Severity::Warning,
                    path: Default::default(),
                    range: b.def_range,
                    message: ViolationData::new(
                        "unused-binding",
                        format!("local binding `{}` is assigned but never read", b.name),
                    )
                    .with_suggestion("Remove the assignment, or prefix the name with `.` to mark it intentional."),
                    fix: None,
                }
            })
            .collect()
    }
}
