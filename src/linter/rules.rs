//! Lint rule trait, registry, and per-file dispatch.
//!
//! Rules are run over a file in a single shared CST traversal: each rule
//! declares the [`SyntaxKind`]s it cares about via [`Rule::interests`], and
//! [`run_rules`] walks the tree once, calling [`Rule::check`] on every element
//! whose kind a rule subscribed to. Rules that work off the whole file rather
//! than node shape (semantic-model queries, comment directives) leave
//! `interests` empty and override [`Rule::check_file`], which runs once per file
//! after the walk.
//!
//! New rules:
//! 1. Create a module under `src/linter/rules/<category>/<id>.rs`.
//! 2. Define a unit `pub struct` that implements [`Rule`] — subscribe to node
//!    kinds via `interests` + `check`, or do a whole-file pass via `check_file`.
//! 3. Add it to [`all_rules`] below.
//! 4. Add the rule ID to [`ALL_RULE_IDS`] so `LintConfig::select` / `ignore`
//!    can validate it.

use std::path::Path;

use crate::project::{ExternalResolution, FileScope};
use crate::rindex::provider::CompositeProvider;
use crate::semantic::{SemanticModel, SymbolProvider};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

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

    /// The `SyntaxKind`s this rule subscribes to. During [`run_rules`]' single
    /// shared traversal, [`Rule::check`] is invoked once for every element whose
    /// kind appears here. The default (`&[]`) opts out of node dispatch entirely
    /// — appropriate for rules that work off the whole file via
    /// [`Rule::check_file`].
    fn interests(&self) -> &'static [SyntaxKind] {
        &[]
    }

    /// Per-element callback, invoked for each CST element (node *or* token) whose
    /// kind is in [`Rule::interests`]. Node-shape rules unwrap `el.as_node()`;
    /// token rules unwrap `el.as_token()`. Push findings onto `sink`.
    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let _ = (el, ctx, sink);
    }

    /// Whole-file pass, run once after the shared traversal. For rules driven by
    /// the semantic model, cross-file scope, or comment directives rather than
    /// node shape. The default is a no-op.
    fn check_file(&self, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let _ = (ctx, sink);
    }
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

    // Build the node-dispatch table: kind discriminant -> indices of subscribed
    // rules. `SyntaxKind` is a contiguous `#[repr(u16)]`, so a flat Vec indexed
    // by `kind as usize` beats a hash map.
    let mut by_kind: Vec<Vec<usize>> = vec![Vec::new(); SyntaxKind::COUNT];
    let mut any_node_rules = false;
    for (i, rule) in rules.iter().enumerate() {
        for kind in rule.interests() {
            by_kind[*kind as usize].push(i);
            any_node_rules = true;
        }
    }

    // Single shared traversal feeding every node-shape rule. Visits tokens too
    // (`descendants_with_tokens`) so token-level rules can subscribe to e.g.
    // `IDENT` or `COMMENT`.
    if any_node_rules {
        for el in root.descendants_with_tokens() {
            for &i in &by_kind[el.kind() as usize] {
                rules[i].check(&el, &ctx, &mut all);
            }
        }
    }

    // Whole-file pass for model-/comment-driven rules.
    for rule in rules {
        rule.check_file(&ctx, &mut all);
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
