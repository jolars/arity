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
use rowan::ast::AstNode as _;
use smol_str::SmolStr;

use crate::ast::{Arg, CallExpr, HasArgList as _};
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
         an un-indexed package could export the otherwise-unresolved name. In \
         an analyzed package, a package-local call argument is checked only \
         when its matched formal is proven to evaluate the promise normally; \
         capture, opaque forwarding, and ambiguous behavior stay silent."
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
                    .filter(|ident| ident.name != ".packageName" || !ctx.is_package_r_source())
                    .filter(|ident| resolution.unresolved.contains(ident.name.as_str()))
                    // Last: this one walks the tree, so it should only run for
                    // the names that survived every cheap set lookup.
                    .filter(|ident| {
                        !inside_non_eager_local_argument(
                            ctx.root,
                            ctx.model,
                            ctx.is_package_r_source(),
                            ident.range,
                            &resolution.local_functions,
                        )
                    })
                    .map(|ident| undefined(&ident.name, ident.range)),
            ),
            None => self.run_standalone(ctx, sink),
        }
    }
}

fn inside_non_eager_local_argument(
    root: &crate::syntax::SyntaxNode,
    model: &crate::semantic::SemanticModel,
    is_package_r_source: bool,
    range: TextRange,
    functions: &std::collections::BTreeMap<String, crate::project::FunctionPromiseSummary>,
) -> bool {
    let Some(mut node) = root
        .covering_element(range)
        .into_token()
        .and_then(|token| token.parent())
    else {
        return false;
    };
    loop {
        if let Some(arg) = Arg::cast(node.clone())
            && let Some(call_node) = node.parent().and_then(|list| list.parent())
            && let Some(call) = CallExpr::cast(call_node)
            && let Some(name) = call.callee_name()
            && let Some(summary) = functions.get(name.as_str())
        {
            if let Some(callee) = call.callee_token()
                && let Some(ident) = model
                    .idents()
                    .iter()
                    .find(|ident| ident.range == callee.text_range())
                && let Some(binding) = model.resolve_local(ident)
            {
                let scope = model.scope(model.binding(binding).scope).kind;
                if scope != crate::semantic::ScopeKind::File || !is_package_r_source {
                    return true;
                }
            }
            let args: Vec<Arg> = call.args().collect();
            let Some(argument) = args
                .iter()
                .position(|candidate| candidate.syntax() == arg.syntax())
            else {
                return true;
            };
            let names: Vec<Option<SmolStr>> = args.iter().map(Arg::name).collect();
            let Some(formal) = summary.matched_formal(&names, argument) else {
                return true;
            };
            if !summary.eager.contains(&formal) {
                return true;
            }
        }
        let Some(parent) = node.parent() else {
            return false;
        };
        node = parent;
    }
}

impl UndefinedSymbol {
    /// Inline resolution used when no salsa-backed [`ExternalResolution`] is
    /// available (single-file paths). Mirrors the cross-file path's logic using
    /// the [`RuleContext::symbols`] provider directly.
    fn run_standalone(&self, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        // Fold in packages attached by the file's location (e.g. testthat for a
        // `tests/testthat/` file), which no `library()` call names, and the ones
        // the package's NAMESPACE `import()`s wholesale — every export of those
        // is in scope here, which for resolution is the same thing as attached.
        // Allocate only when there is something to add: the common case keeps
        // the model's slice.
        let implicit = implicit_attached_packages(ctx.path);
        let wildcards = ctx.project.map(|p| p.wildcard_import_packages());
        let augmented: Vec<LoadedPackage>;
        let loaded: &[LoadedPackage] =
            if implicit.is_empty() && wildcards.is_none_or(|packages| packages.is_empty()) {
                ctx.model.loaded_packages()
            } else {
                augmented = ctx
                    .model
                    .loaded_packages()
                    .iter()
                    .cloned()
                    .chain(
                        implicit
                            .iter()
                            .copied()
                            .chain(wildcards.into_iter().flatten().map(String::as_str))
                            .map(|name| LoadedPackage {
                                name: SmolStr::new(name),
                                range: TextRange::default(),
                            }),
                    )
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
        if ctx.project.is_some_and(|p| p.resolution_incomplete) {
            return;
        }
        for ident in ctx.model.idents() {
            // A data-masked read may be a data-frame column, not an undefined
            // symbol — never flag it (see the builder's `mask_depth`).
            if ident.data_masked {
                continue;
            }
            // R injects this binding into every package namespace while its
            // sources are loaded; it is not present in the source text.
            if ident.name == ".packageName" && ctx.is_package_r_source() {
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
