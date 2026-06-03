//! `undefined-symbol`: an identifier read that doesn't resolve to any
//! in-scope binding nor any known package export.
//!
//! On by default, but gated: the rule only runs when *every* `library()`-
//! attached package is indexed (a default package, or harvested into the
//! introspection cache). If any attached package's exports are unknown, the
//! rule stays silent for the whole file — an un-indexed package could export
//! any of the otherwise-unresolved names, so flagging them would be a false
//! positive.

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
        true
    }

    fn run(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let loaded = ctx.model.loaded_packages();
        // Conservative gate: bail out entirely if any attached package's exports
        // are unknown, since such a package could define the unresolved names.
        if loaded.iter().any(|p| !ctx.symbols.package_indexed(&p.name)) {
            return out;
        }
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
