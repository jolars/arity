//! `undefined-symbol`: an identifier read that doesn't resolve to any
//! in-scope binding nor any known package export.
//!
//! On by default, but gated: the rule only runs when *every* `library()`-
//! attached package is indexed (a default package, or harvested into the
//! introspection cache). If any attached package's exports are unknown, the
//! rule stays silent for the whole file — an un-indexed package could export
//! any of the otherwise-unresolved names, so flagging them would be a false
//! positive.

use rowan::TextRange;
use smol_str::SmolStr;

use crate::linter::diagnostic::{Diagnostic, Severity, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::semantic::{LoadedPackage, PackageOrigin, implicit_attached_packages};

pub struct UndefinedSymbol;

impl Rule for UndefinedSymbol {
    fn id(&self) -> &'static str {
        "undefined-symbol"
    }

    fn description(&self) -> &'static str {
        "Flag an identifier read that resolves to no in-scope binding and no \
         known package export.\n\nGated for safety: the rule stays silent for a \
         whole file unless every `library()`-attached package is indexed, since \
         an un-indexed package could export the otherwise-unresolved name."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "`subtotal` resolves to nothing:",
            source: "total <- subtotal\n",
        }]
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn default_enabled(&self) -> bool {
        true
    }

    fn check_file(&self, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        // Conservative gate: `attach()`/`load()` introduce bindings arity can't
        // enumerate (a data frame's columns on the search path; arbitrary names
        // from an `.rda`), so any otherwise-unresolved bare name in the file
        // might be one of them. Stay silent for the whole file. Applies to both
        // resolution paths below.
        if ctx.model.attaches_opaque_env() {
            return;
        }
        match ctx.resolution {
            // Cross-file path: the salsa `external_resolution` query already
            // applied the conservative gates and the project + package masking,
            // memoized and backdated across edits. We only re-apply the cheap,
            // always-fresh per-occurrence local-binding check and re-attach the
            // diagnostic span — the resolved set is range-free.
            Some(resolution) => sink.extend(
                ctx.model
                    .idents()
                    .iter()
                    // A data-masked read may be a data-frame column, not an
                    // undefined symbol — never flag it (see the builder).
                    .filter(|ident| !ident.data_masked)
                    .filter(|ident| ctx.model.resolve_local(ident).is_none())
                    .filter(|ident| resolution.unresolved.contains(ident.name.as_str()))
                    .map(|ident| undefined(&ident.name, ident.range)),
            ),
            // Single-file fallback (no project / no manifest): resolve inline
            // against the provided `SymbolProvider`, preserving the historical
            // behavior for one-shot checks and the LSP per-document path.
            None => self.run_standalone(ctx, sink),
        }
    }
}

impl UndefinedSymbol {
    /// Inline resolution used when no salsa-backed [`ExternalResolution`] is
    /// available (single-file paths). Mirrors the cross-file path's logic using
    /// the [`RuleContext::symbols`] provider directly.
    fn run_standalone(&self, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        // Fold in packages attached by the file's location (e.g. testthat for a
        // `tests/testthat/` file), which no `library()` call names. Allocate only
        // when there is something to add — the common case keeps the model's slice.
        let implicit = implicit_attached_packages(ctx.path);
        let augmented: Vec<LoadedPackage>;
        let loaded: &[LoadedPackage] = if implicit.is_empty() {
            ctx.model.loaded_packages()
        } else {
            augmented = ctx
                .model
                .loaded_packages()
                .iter()
                .cloned()
                .chain(implicit.iter().map(|name| LoadedPackage {
                    name: SmolStr::new(name),
                    range: TextRange::default(),
                }))
                .collect();
            &augmented
        };
        // Conservative gate: bail out entirely if any attached package's exports
        // are unknown, since such a package could define the unresolved names. A
        // meta-package (e.g. tidyverse) also attaches its core members (the
        // provider prefers the harvested attach set, falling back to the static
        // table), so each of those must be indexed too.
        if loaded.iter().any(|p| {
            !ctx.symbols.package_indexed(&p.name)
                || ctx
                    .symbols
                    .attached_packages(&p.name)
                    .iter()
                    .any(|m| !ctx.symbols.package_indexed(m))
        }) {
            return;
        }
        // Conservative gate: incomplete cross-file visibility (an unresolved
        // `source()`, or a wholesale `import(pkg)`) could define any of the
        // names below.
        if ctx.project.is_some_and(|p| p.resolution_incomplete) {
            return;
        }
        for ident in ctx.model.idents() {
            // A data-masked read may be a data-frame column, not an undefined
            // symbol — never flag it (see the builder's `mask_depth`).
            if ident.data_masked {
                continue;
            }
            // Skip if it resolves to a local binding.
            if ctx.model.resolve_local(ident).is_some() {
                continue;
            }
            // Skip if a sibling file in the same package or source-closure binds
            // it at top level.
            if ctx.project.is_some_and(|p| p.resolves(&ident.name)) {
                continue;
            }
            // Skip if the symbol provider can place it.
            if !matches!(
                ctx.symbols.origin(&ident.name, loaded),
                PackageOrigin::Unknown
            ) {
                continue;
            }
            sink.push(undefined(&ident.name, ident.range));
        }
    }
}

/// Build an `undefined-symbol` diagnostic for `name` at `range`.
fn undefined(name: &str, range: rowan::TextRange) -> Diagnostic {
    Diagnostic {
        rule: "undefined-symbol",
        severity: Default::default(),
        path: Default::default(),
        range,
        message: ViolationData::new(
            "undefined-symbol",
            format!("no in-scope binding or attached package exports `{name}`"),
        ),
        fix: None,
    }
}
