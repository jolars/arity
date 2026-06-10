//! Lint rule trait, registry, and per-file dispatch.
//!
//! New rules:
//! 1. Create a module under `src/linter/rules/<category>/<id>.rs`.
//! 2. Define a unit `pub struct` that implements [`Rule`].
//! 3. Add it to [`all_rules`] below.
//! 4. Add the rule ID to [`ALL_RULE_IDS`] so `LintConfig::select` / `ignore`
//!    can validate it.

use std::path::Path;

use crate::project::{ExternalResolution, FileScope};
use crate::rindex::provider::CompositeProvider;
use crate::semantic::{SemanticModel, SymbolProvider};
use crate::syntax::SyntaxNode;

use super::diagnostic::{Diagnostic, Severity};

pub mod correctness;
pub mod suspicious;

/// All rules currently shipped.
pub fn all_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(correctness::UndefinedSymbol),
        Box::new(correctness::UnusedBinding),
        Box::new(correctness::DuplicateFormal),
        Box::new(suspicious::AssignmentInCondition),
        Box::new(suspicious::ShadowedBuiltin),
    ]
}

/// All rule IDs, kept as a constant for `LintConfig` validation.
pub const ALL_RULE_IDS: &[&str] = &[
    "undefined-symbol",
    "unused-binding",
    "duplicate-formal",
    "assignment-in-condition",
    "shadowed-builtin",
];

pub trait Rule: Send + Sync {
    fn id(&self) -> &'static str;
    fn default_severity(&self) -> Severity {
        Severity::Warning
    }
    fn default_enabled(&self) -> bool {
        true
    }
    fn run(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic>;
}

pub struct RuleContext<'a> {
    pub path: &'a Path,
    pub root: &'a SyntaxNode,
    pub model: &'a SemanticModel,
    pub symbols: &'a dyn SymbolProvider,
    /// Cross-file visibility for this file, when linting a multi-file project.
    /// `None` for single-file runs (the LSP per-document path, one-shot checks).
    pub project: Option<&'a FileScope<'a>>,
    /// Salsa-resolved external-symbol verdict for this file, when available (the
    /// cross-file lint path). Carries the backdated set of free-read names that
    /// resolve to no attached package, so `undefined-symbol` consumes a memoized
    /// result instead of re-running masking on every keystroke. `None` on the
    /// single-file paths, where the rule falls back to [`RuleContext::symbols`].
    pub resolution: Option<&'a ExternalResolution>,
}

/// Configured set of rules and severities for a single linting run.
pub struct ResolvedRules {
    pub rules: Vec<Box<dyn Rule>>,
}

impl ResolvedRules {
    /// Build the rule set honoring `select` / `ignore` from `LintConfig`.
    ///
    /// Resolution order:
    /// 1. Start with all rules whose `default_enabled()` is `true`, unless
    ///    `select` is set (then start with the listed rules instead).
    /// 2. Subtract anything in `ignore`.
    /// 3. Unknown rule IDs in `select` or `ignore` are returned via the second
    ///    element of the tuple so the caller can surface them.
    pub fn resolve(select: Option<&[String]>, ignore: &[String]) -> (Self, Vec<String>) {
        let mut unknown = Vec::new();
        for id in select.iter().flat_map(|v| v.iter()).chain(ignore.iter()) {
            if !ALL_RULE_IDS.contains(&id.as_str()) {
                unknown.push(id.clone());
            }
        }
        let mut chosen: Vec<Box<dyn Rule>> = match select {
            Some(picks) => all_rules()
                .into_iter()
                .filter(|r| picks.iter().any(|p| p == r.id()))
                .collect(),
            None => all_rules()
                .into_iter()
                .filter(|r| r.default_enabled())
                .collect(),
        };
        chosen.retain(|r| !ignore.iter().any(|i| i == r.id()));
        (Self { rules: chosen }, unknown)
    }

    pub fn default_set() -> Self {
        let (set, _) = Self::resolve(None, &[]);
        set
    }
}

/// Run every configured rule against a single file's CST + model. Diagnostics
/// are stably sorted by `(start, end, rule)` before returning.
pub fn run_rules(
    rules: &[Box<dyn Rule>],
    path: &Path,
    root: &SyntaxNode,
    model: &SemanticModel,
    symbols: &dyn SymbolProvider,
    project: Option<&FileScope<'_>>,
    resolution: Option<&ExternalResolution>,
) -> Vec<Diagnostic> {
    let ctx = RuleContext {
        path,
        root,
        model,
        symbols,
        project,
        resolution,
    };
    let mut all = Vec::new();
    for rule in rules {
        all.extend(rule.run(&ctx));
    }
    all.sort_by(|a, b| {
        (u32::from(a.range.start()), u32::from(a.range.end()), a.rule).cmp(&(
            u32::from(b.range.start()),
            u32::from(b.range.end()),
            b.rule,
        ))
    });
    all
}

/// Provide a sane default symbol provider: base R only, with no installed-
/// package index. Behaves exactly like the historical `StaticBaseR` for files
/// that don't attach non-default packages.
pub fn default_symbol_provider() -> CompositeProvider {
    CompositeProvider::base_only()
}
