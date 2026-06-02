//! `undefined-symbol`: an identifier read that doesn't resolve to any
//! in-scope binding nor any known package export.
//!
//! Off by default this pass — without a CRAN export manifest, any name
//! introduced by a `library()` call resolves as `Unknown` and would generate
//! false positives. Re-enable once the manifest ships.

use crate::linter::diagnostic::{Diagnostic, Severity, ViolationData};
use crate::linter::rules::{Rule, RuleContext};
use crate::semantic::PackageOrigin;

pub struct UndefinedSymbol;

impl Rule for UndefinedSymbol {
    fn id(&self) -> &'static str {
        "undefined-symbol"
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn run(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let loaded = ctx.model.loaded_packages();
        for ident in ctx.model.idents() {
            // Skip if it resolves to a local binding.
            if ctx.model.resolve_local(ident).is_some() {
                continue;
            }
            // Skip if the symbol provider can place it.
            if !matches!(
                ctx.symbols.origin(&ident.name, loaded),
                PackageOrigin::Unknown
            ) {
                continue;
            }
            out.push(Diagnostic {
                rule: "undefined-symbol",
                severity: Severity::Warning,
                path: Default::default(),
                range: ident.range,
                message: ViolationData::new(
                    "undefined-symbol",
                    format!(
                        "no in-scope binding or attached package exports `{}`",
                        ident.name
                    ),
                ),
                fix: None,
            });
        }
        out
    }
}
