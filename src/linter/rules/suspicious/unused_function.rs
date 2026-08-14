//! `unused-function`: an *exported* function that nothing in the project calls.
//!
//! The complement of `unused-binding`, not an overlap. `unused-binding` reports
//! a top-level binding no one reads, but deliberately exempts one that is public
//! API (`FileScope::used_elsewhere` folds in the package's NAMESPACE
//! `export()`s) — otherwise every exported function in a library would flag.
//! This rule reports exactly that exempted set: a function that is declared
//! public yet has no caller anywhere we can see. Between them, a dead top-level
//! function draws at most one finding, and never two. (Not always one: both
//! rules withhold on a name S3 dispatch could reach, so a dead `my.util` that
//! nothing exports draws none.)
//!
//! Dead *public* API is normal for a library — the callers are downstream, out
//! of view — so this is **default-off**: an opt-in sweep for an author auditing
//! their own surface, not a warning to ship on.
//!
//! A function counts as exported when either signal says so:
//! - the package's NAMESPACE `export()`s the name (needs the cross-file
//!   project layer, so this is the multi-file path), or
//! - a roxygen `@export` sits on the block documenting the definition — the
//!   same declaration one step earlier in the pipeline, and available
//!   single-file (roxygen2 is what *generates* that NAMESPACE entry).
//!
//! Report-only. The repair is a judgement call — delete the function, or stop
//! exporting it — and both are breaking changes to a published interface, so
//! neither is a mechanical edit.

use std::collections::HashSet;

use rowan::ast::AstNode as _;
use smol_str::SmolStr;

use crate::ast::{AssignmentExpr, RoxygenBlock};
use crate::linter::diagnostic::{Diagnostic, Severity, ViolationData};
use crate::linter::rules::roxygen::documented_function;
use crate::linter::rules::{Example, Rule, RuleContext, matchers};
use crate::semantic::ScopeKind;
use crate::syntax::SyntaxNode;

const EXAMPLES: &[Example] = &[Example {
    caption: "`add_one` is exported but never called anywhere in the package:",
    source: "#' Add one\n#'\n#' @export\nadd_one <- function(x) {\n  x + 1\n}\n",
}];

pub struct UnusedFunction;

impl Rule for UnusedFunction {
    fn id(&self) -> &'static str {
        "unused-function"
    }

    fn description(&self) -> &'static str {
        "Flag an exported function that nothing in the project calls. The \
         complement of `unused-binding`, which stays quiet on public API: a \
         function is reported here only when it is declared exported (a \
         roxygen `@export`, or a NAMESPACE `export()`) *and* no file that can \
         see it reads it. S3 methods are exempt — dispatch reaches them \
         without a direct call, so having no caller says nothing about them. \
         Disabled by default, since a library's exported functions are meant \
         to be called from outside the project."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn check_file(&self, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let roxygen_exports = roxygen_exported_names(ctx.root);
        sink.extend(
            ctx.model
                .unused_local_bindings()
                .filter(|id| {
                    let b = ctx.model.binding(*id);
                    // Only a top-level definition can be a package export.
                    if ctx.model.scope(b.scope).kind != ScopeKind::File {
                        return false;
                    }
                    if !matchers::binds_a_function(ctx.root, b.def_range) {
                        return false;
                    }
                    // A sibling that calls it is a real use — this is the half of
                    // `used_elsewhere` that is *not* the export exemption, so
                    // asking it separately is the whole point of the split.
                    if ctx.project.is_some_and(|p| p.read_elsewhere(&b.name)) {
                        return false;
                    }
                    if is_s3_method(ctx, &b.name) {
                        return false;
                    }
                    roxygen_exports.contains(&b.name)
                        || ctx
                            .project
                            .is_some_and(|p| p.exported_by_namespace(&b.name))
                })
                .map(|id| {
                    let b = ctx.model.binding(id);
                    Diagnostic {
                        rule: "unused-function",
                        severity: Default::default(),
                        path: Default::default(),
                        range: b.def_range,
                        message: ViolationData::new(
                            "unused-function",
                            format!("exported function `{}` is never called", b.name),
                        )
                        .with_suggestion(
                            "Remove it, or stop exporting it if it is not part of the public API.",
                        ),
                        fix: None,
                    }
                }),
        );
    }
}

/// Whether `name` names an S3 method, which dispatch reaches without any direct
/// call — so "nothing calls it" is not evidence that it is dead.
///
/// Two tiers, mirroring the two export signals. This rule only ever fires on a
/// name declared *exported*, and with a project the NAMESPACE answers that
/// exactly: `S3method(print, foo)` registers `print.foo`, while `export(my.util)`
/// is roxygen2 stating the name is not a method. Without a project arity cannot
/// reproduce that decision — roxygen2 splits `print.foo` into `generic.class`
/// only if `print` is a generic *at build time*, which is a runtime fact — so
/// the name's shape decides and any dotted name is withheld. That under-reports
/// a genuinely dead `my.util`, which is the safe direction for an opt-in audit;
/// the NAMESPACE path still reports it.
///
/// The shape tier is `unused-binding`'s *only* tier, because it asks about
/// reachability rather than about the public surface — see
/// [`matchers::looks_like_s3_method`].
fn is_s3_method(ctx: &RuleContext<'_>, name: &str) -> bool {
    match ctx.project {
        Some(project) => project.is_s3_method(name),
        None => matchers::looks_like_s3_method(name),
    }
}

/// The names a roxygen `@export` declares in this file: for every top-level
/// block carrying the tag, the target name of the definition it documents.
///
/// Reuses `documented_function`'s strictly conservative correlation — anything
/// that is not a plain `name <- function(…)` under the block (an S4
/// `setMethod`, an R6 class, `"_PACKAGE"`) contributes nothing, so an exotic
/// shape is silently skipped rather than mis-attributed.
fn roxygen_exported_names(root: &SyntaxNode) -> HashSet<SmolStr> {
    root.children()
        .filter_map(RoxygenBlock::cast)
        .filter(|block| block.has_tag("export"))
        .filter_map(|block| {
            let function = documented_function(&block)?;
            let assign = function.syntax().parent().and_then(AssignmentExpr::cast)?;
            assign.target_name()
        })
        .collect()
}
