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
//! 3. Add it to its category's list in [`rules_by_category`] below — the single
//!    source of truth. Both the registry ([`all_rules`], and from it the set of
//!    valid rule IDs, [`all_rule_ids`]) and the generated rule reference are
//!    derived from it, so there is no second list to keep in sync.
//!
//! A rule over `DESCRIPTION` implements [`DcfRule`] instead and is registered as
//! [`AnyRule::Dcf`] in the *same* catalogue: two grammars, one list of rules,
//! one namespace of rule IDs. [`run_dcf_rules`] is that grammar's twin of
//! [`run_rules`], with the same single-shared-walk discipline.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

use rowan::ast::AstNode as _;

use crate::ast::{BinaryExpr, CallExpr};
use crate::config::{CompatConfig, CompatVersion, LintConfig, RulesConfig};
use crate::dcf;
use crate::linter::rules::roxygen::RoxygenTopics;
use crate::project::description::{DescriptionCompat, DescriptionFacts};
use crate::project::{ExternalResolution, FileScope, PackageTopics, PackageUsage};
use crate::rindex::provider::CompositeProvider;
use crate::semantic::{FileControlFlow, PackageOrigin, SemanticModel, SymbolProvider};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

use super::diagnostic::{Diagnostic, Severity};
use super::suppression::{DirectiveUsage, SuppressionMap};

pub mod correctness;
pub mod documentation;
pub mod matchers;
pub mod meta;
pub mod packaging;
pub mod performance;
pub mod readability;
pub mod regex;
pub mod roxygen;
pub mod suspicious;

/// The catalogue grouping a rule is listed under in the generated rule
/// reference (`docs/src/reference/rules.md`).
///
/// The grouping lives on the registry rather than on [`Rule`]: it is a property
/// of the catalogue, not of the check, and keeping it here means a rule's
/// category is stated exactly once, next to the rule itself in
/// [`rules_by_category`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleCategory {
    Correctness,
    Suspicious,
    Readability,
    Performance,
    Documentation,
    /// The package's declared metadata, and whether it matches its code. The
    /// one category spanning both grammars — an undeclared dependency and an
    /// unused one are the same defect seen from two sides.
    Packaging,
    Meta,
}

impl RuleCategory {
    /// The section heading this category is rendered under.
    pub fn title(self) -> &'static str {
        match self {
            Self::Correctness => "Correctness",
            Self::Suspicious => "Suspicious",
            Self::Readability => "Readability",
            Self::Performance => "Performance",
            Self::Documentation => "Documentation",
            Self::Packaging => "Packaging",
            Self::Meta => "Meta",
        }
    }
}

/// All rules currently shipped, grouped into the categories the reference page
/// is organized by — the single source of truth. [`all_rules`] flattens this,
/// so the registry order and the catalogue order are one list.
///
/// Both grammars live in this one list. Merging them here rather than keeping a
/// second DCF registry is what keeps the catalogue single-sourced: a parallel
/// registry would have to be merged back per category to render one
/// `## Correctness` section, and that merge would *be* a second source of truth
/// for catalogue order.
pub fn rules_by_category() -> Vec<(RuleCategory, Vec<AnyRule>)> {
    vec![
        (RuleCategory::Correctness, r_rules(correctness_rules())),
        (RuleCategory::Suspicious, r_rules(suspicious_rules())),
        (RuleCategory::Readability, r_rules(readability_rules())),
        (RuleCategory::Performance, r_rules(performance_rules())),
        (RuleCategory::Documentation, r_rules(documentation_rules())),
        (RuleCategory::Packaging, packaging_rules()),
        (RuleCategory::Meta, r_rules(meta_rules())),
    ]
}

/// Lift a category's R rules into the mixed-grammar catalogue.
fn r_rules(rules: Vec<Box<dyn Rule>>) -> Vec<AnyRule> {
    rules.into_iter().map(AnyRule::R).collect()
}

/// All rules currently shipped, in registry order.
pub fn all_rules() -> Vec<AnyRule> {
    rules_by_category()
        .into_iter()
        .flat_map(|(_, rules)| rules)
        .collect()
}

fn correctness_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(correctness::UndefinedSymbol),
        Box::new(correctness::UnusedBinding),
        Box::new(correctness::DuplicateFormal),
        Box::new(correctness::DuplicatedArguments),
        Box::new(correctness::EqualsNa),
        Box::new(correctness::EqualsNan),
        Box::new(correctness::EqualsNull),
        Box::new(correctness::MissingArgument),
        Box::new(correctness::RepTimesIgnored),
        Box::new(correctness::Sprintf),
        Box::new(correctness::VectorLogic),
        Box::new(correctness::UnreachableCode),
        Box::new(correctness::IsNumeric),
        Box::new(correctness::IfAlwaysTrue),
        Box::new(correctness::EmptyAssignment),
        Box::new(correctness::DownloadFile),
        Box::new(correctness::InternalFunction),
        Box::new(correctness::RCompat),
    ]
}

fn suspicious_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(suspicious::AssignmentInCondition),
        Box::new(suspicious::ImplicitAssignment),
        Box::new(suspicious::Browser),
        Box::new(suspicious::ShadowedBuiltin),
        Box::new(suspicious::RedundantEquals),
        Box::new(suspicious::RedundantIfelse),
        Box::new(suspicious::AllEqual),
        Box::new(suspicious::Repeat),
        Box::new(suspicious::UndesirableFunction),
        Box::new(suspicious::ForLoopIndex),
        Box::new(suspicious::ForLoopDupIndex),
        Box::new(suspicious::UnusedFunction),
        Box::new(suspicious::DuplicatedFunctionDefinition),
    ]
}

fn readability_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(readability::TrueFalseSymbol),
        Box::new(readability::ComparisonNegation),
        Box::new(readability::OuterNegation),
        Box::new(readability::StringBoundary),
        Box::new(readability::UnnecessaryNesting),
    ]
}

fn performance_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(performance::AnyIsNa),
        Box::new(performance::AnyDuplicated),
        Box::new(performance::Coalesce),
        Box::new(performance::Crossprod),
        Box::new(performance::Lengths),
        Box::new(performance::Nzchar),
        Box::new(performance::Seq),
        Box::new(performance::ClassEquals),
        Box::new(performance::FixedRegex),
        Box::new(performance::Sort),
    ]
}

fn documentation_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(documentation::RoxygenUnknownTag),
        Box::new(documentation::RoxygenTitle),
        Box::new(documentation::RoxygenReturn),
        Box::new(documentation::RoxygenParam),
        Box::new(documentation::RoxygenExamples),
        Box::new(documentation::Roxygen2Compat),
    ]
}

/// The one category holding rules over both grammars, so it builds its
/// `AnyRule`s directly rather than going through [`r_rules`].
fn packaging_rules() -> Vec<AnyRule> {
    vec![
        AnyRule::R(Box::new(packaging::UndeclaredDependency)),
        AnyRule::Dcf(Box::new(packaging::DescriptionMissingField)),
        AnyRule::Dcf(Box::new(packaging::DescriptionDuplicateField)),
        AnyRule::Dcf(Box::new(packaging::DescriptionVersionConstraint)),
        AnyRule::Dcf(Box::new(packaging::DescriptionPackageInMultipleFields)),
        AnyRule::Dcf(Box::new(packaging::DescriptionMalformedName)),
        AnyRule::Dcf(Box::new(packaging::DescriptionMalformedVersion)),
        AnyRule::Dcf(Box::new(packaging::DescriptionMalformedMaintainer)),
        AnyRule::Dcf(Box::new(packaging::DescriptionAuthorsAtR)),
        AnyRule::Dcf(Box::new(packaging::DescriptionEmptyPerson)),
        AnyRule::Dcf(Box::new(packaging::UnusedDependency)),
    ]
}

fn meta_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(meta::MisnamedSuppression),
        Box::new(meta::BlanketSuppression),
        Box::new(meta::MisplacedSuppression),
        Box::new(meta::DeprecatedSuppression),
        Box::new(meta::UnexplainedSuppression),
        Box::new(meta::OutdatedSuppression),
    ]
}

/// Every shipped rule's ID, derived from [`all_rules`] so the two never drift.
/// Used to validate `LintConfig::select` / `ignore`.
///
/// Both grammars' IDs, in one namespace: `select`, `ignore`, `# arity-lint`,
/// and `misnamed-suppression` all see one flat set of rule names, and none of
/// them has to learn which file type a rule fires in.
pub fn all_rule_ids() -> Vec<&'static str> {
    all_rules().iter().map(|r| r.id()).collect()
}

/// Whether `id` is a rule arity ships. The `O(1)` membership oracle over
/// [`all_rule_ids`], which instantiates the whole registry on every call.
pub fn is_known_rule(id: &str) -> bool {
    static IDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    IDS.get_or_init(|| all_rule_ids().into_iter().collect())
        .contains(id)
}

/// A documented example for a rule: a snippet of R that triggers the rule.
///
/// The rule reference is generated by running the real linter on `source`, so
/// the "after" state of an autofix is *derived* (by applying the rule's safe
/// fixes) rather than stored — the snippet stays the single source of truth.
pub struct Example {
    /// One-line caption rendered above the snippet (markdown). May be empty.
    pub caption: &'static str,
    /// R source that triggers the rule. Should end with a trailing newline.
    pub source: &'static str,
}

pub trait Rule: Send + Sync {
    fn id(&self) -> &'static str;
    fn default_severity(&self) -> Severity {
        Severity::Warning
    }
    fn default_enabled(&self) -> bool {
        true
    }

    /// One-paragraph (markdown) description of what the rule flags and why,
    /// used to generate the rule reference. Empty means "not yet documented".
    fn description(&self) -> &'static str {
        ""
    }

    /// Worked examples for the rule reference. Each `source` is linted live and
    /// rendered with its diagnostics (and autofix before/after). The default is
    /// empty — a rule with no examples is skipped by the docs generator.
    fn examples(&self) -> &'static [Example] {
        &[]
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

    /// Post-suppression pass, run once after the surviving findings have been
    /// filtered through the file's `# arity` directives. `used` records
    /// which directives actually matched a finding.
    ///
    /// Separate from [`Rule::check_file`] because its input is a *driver* fact —
    /// which suppressions fired — that does not exist until filtering has run.
    /// `outdated-suppression` is the only implementor; the default is a no-op.
    fn check_suppressions(
        &self,
        ctx: &RuleContext<'_>,
        used: &DirectiveUsage,
        sink: &mut Vec<Diagnostic>,
    ) {
        let _ = (ctx, used, sink);
    }

    /// Rule IDs that must also be enabled when this rule's [`Rule::examples`]
    /// are rendered and tested. The docs renderer restricts `select` to the rule
    /// itself so an example cannot trip an unrelated rule; this is the escape
    /// hatch for a rule whose subject *is* another rule's presence in the run
    /// (`outdated-suppression` needs the suppressed rule to have run in order to
    /// know its directive matched nothing). The default is none.
    fn doc_select(&self) -> &'static [&'static str] {
        &[]
    }

    /// The `[compat]` floors this rule's [`Rule::examples`] are linted under.
    /// The default run declares none — which silences the version-aware rules
    /// (`r-compat`, `roxygen2-compat`), so those override this to give their
    /// examples a floor to violate.
    fn doc_compat(&self) -> CompatConfig {
        CompatConfig::default()
    }

    /// A synthetic package this rule's [`Rule::examples`] are linted *inside*:
    /// `(relative path, contents)` pairs written to a temporary directory, with
    /// the example itself placed at `R/example.R`.
    ///
    /// The escape hatch for a rule whose subject is a package-level fact.
    /// `check_document`'s single-file path leaves `RuleContext::package` and
    /// `project` `None`, so such a rule is silent there by construction — and an
    /// example that produces no finding is not documentation. Same shape and
    /// same reason as [`Rule::doc_compat`]. The default is none, which lints the
    /// example as the loose script it looks like.
    fn doc_package(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }
}

/// A rule over the DCF grammar — `DESCRIPTION`, not R.
///
/// Same discipline as [`Rule`], one grammar over: declare the DCF
/// [`SyntaxKind`](dcf::SyntaxKind)s you care about via [`DcfRule::interests`]
/// and implement [`DcfRule::check`], or leave `interests` empty and override
/// [`DcfRule::check_file`]. [`run_dcf_rules`] walks the document once.
///
/// The metadata half is spelled out again rather than factored into a supertrait
/// shared with [`Rule`]: a supertrait would mean splitting every existing `impl
/// Rule` in two for no behavior change, and [`AnyRule`] already gives the
/// catalogue one grammar-blind view of it.
pub trait DcfRule: Send + Sync {
    fn id(&self) -> &'static str;
    fn default_severity(&self) -> Severity {
        Severity::Warning
    }
    fn default_enabled(&self) -> bool {
        true
    }

    /// See [`Rule::description`].
    fn description(&self) -> &'static str {
        ""
    }

    /// See [`Rule::examples`]. A DCF rule's `source` is `DESCRIPTION` text, and
    /// the docs renderer lints it under that file name.
    fn examples(&self) -> &'static [Example] {
        &[]
    }

    /// See [`Rule::doc_select`].
    fn doc_select(&self) -> &'static [&'static str] {
        &[]
    }

    /// See [`Rule::doc_package`]. The example itself is placed at the package's
    /// `DESCRIPTION`, so a fixture only supplies the `R/` sources and NAMESPACE
    /// the rule reads.
    fn doc_package(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }

    /// The DCF `SyntaxKind`s this rule subscribes to, for [`run_dcf_rules`]'
    /// shared traversal. The default opts out of node dispatch entirely.
    fn interests(&self) -> &'static [dcf::SyntaxKind] {
        &[]
    }

    /// Per-element callback, invoked for each DCF element (node *or* token)
    /// whose kind is in [`DcfRule::interests`].
    fn check(&self, el: &dcf::SyntaxElement, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let _ = (el, ctx, sink);
    }

    /// Whole-document pass, run once after the shared traversal. The natural
    /// shape for a rule keyed on a *field* rather than on node shape, which is
    /// most of them.
    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let _ = (ctx, sink);
    }
}

/// A registered rule, whichever grammar it runs over.
///
/// The catalogue is one list ([`rules_by_category`]), so everything derived from
/// it — the valid rule IDs, the reference page, `select`/`ignore` resolution —
/// is written once and sees both grammars. Dispatch is the only place the
/// distinction matters, and [`ResolvedRules`] splits it exactly once.
pub enum AnyRule {
    R(Box<dyn Rule>),
    Dcf(Box<dyn DcfRule>),
}

impl AnyRule {
    pub fn id(&self) -> &'static str {
        match self {
            Self::R(rule) => rule.id(),
            Self::Dcf(rule) => rule.id(),
        }
    }

    pub fn default_severity(&self) -> Severity {
        match self {
            Self::R(rule) => rule.default_severity(),
            Self::Dcf(rule) => rule.default_severity(),
        }
    }

    pub fn default_enabled(&self) -> bool {
        match self {
            Self::R(rule) => rule.default_enabled(),
            Self::Dcf(rule) => rule.default_enabled(),
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::R(rule) => rule.description(),
            Self::Dcf(rule) => rule.description(),
        }
    }

    pub fn examples(&self) -> &'static [Example] {
        match self {
            Self::R(rule) => rule.examples(),
            Self::Dcf(rule) => rule.examples(),
        }
    }

    pub fn doc_select(&self) -> &'static [&'static str] {
        match self {
            Self::R(rule) => rule.doc_select(),
            Self::Dcf(rule) => rule.doc_select(),
        }
    }

    /// The `[compat]` floors this rule's examples are linted under. Only the R
    /// version-aware rules declare any; `DESCRIPTION` *is* the compat source, so
    /// a DCF rule has nothing to say here.
    pub fn doc_compat(&self) -> CompatConfig {
        match self {
            Self::R(rule) => rule.doc_compat(),
            Self::Dcf(_) => CompatConfig::default(),
        }
    }

    pub fn doc_package(&self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::R(rule) => rule.doc_package(),
            Self::Dcf(rule) => rule.doc_package(),
        }
    }
}

/// The rule IDs running in a pass, after `select`/`ignore`. Lets a rule tell
/// "this rule ran and found nothing" from "this rule never ran" — the
/// distinction between a stale suppression and a dormant one.
#[derive(Debug, Clone, Default)]
pub struct EnabledRules(Vec<&'static str>);

impl EnabledRules {
    pub fn contains(&self, id: &str) -> bool {
        self.0.contains(&id)
    }
}

/// The cross-file facts the driver has already resolved for one file, bundled
/// so they travel as a unit.
///
/// Every field is `Option<&_>` and `None` on the single-file paths, which is
/// exactly why they are a struct: passed flat they were three adjacent
/// same-shaped arguments, and a transposed pair type-checked. They are moved
/// onto [`RuleContext`] verbatim — rules keep reading `ctx.project`,
/// `ctx.resolution`, `ctx.package`.
#[derive(Default)]
pub struct FileContext<'a> {
    /// See [`RuleContext::project`].
    pub project: Option<&'a FileScope<'a>>,
    /// See [`RuleContext::resolution`].
    pub resolution: Option<&'a ExternalResolution>,
    /// See [`RuleContext::package`].
    pub package: Option<&'a DescriptionFacts>,
    /// See [`RuleContext::topics`].
    pub topics: Option<&'a PackageTopics>,
}

pub struct RuleContext<'a> {
    pub path: &'a Path,
    pub root: &'a SyntaxNode,
    pub model: &'a SemanticModel,
    /// The per-file control-flow graph (one region per function body plus the
    /// file top-level). Feeds reachability-sensitive rules (`unreachable-code`).
    pub cfg: &'a FileControlFlow,
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
    /// The enclosing package's `DESCRIPTION` facts, when the caller already
    /// resolved them — the cross-file paths read them from the tracked
    /// `DESCRIPTION` input. `None` on the single-file paths, where
    /// [`RuleContext::own_package`] and the compat floors fall back to the lazy
    /// disk walk below.
    pub package: Option<&'a DescriptionFacts>,
    /// The enclosing package's Rd topics, when the caller resolved a project and
    /// this file sits in a package. roxygen2 merges `@rdname`/`@describeIn`
    /// blocks package-wide, so the documentation rules need every `R/` file's
    /// blocks, not just this one's. `None` on the single-file paths and outside
    /// a package, where [`RuleContext::roxygen_topics`] is the file-local
    /// fallback.
    pub topics: Option<&'a PackageTopics>,
    /// Per-rule option tables from `[lint.rules.<id>]`, resolved once per run and
    /// carried on [`ResolvedRules`]. Rules that take no options ignore this.
    pub config: &'a RulesConfig,
    /// The file's parsed `# arity-ignore` directives. Built once per file and
    /// used by [`run_rules`] to drop suppressed findings; the `meta/` rules read
    /// it to lint the directives themselves.
    pub suppressions: &'a SuppressionMap,
    /// The rule IDs running in this pass. A directive naming a rule that did not
    /// run is dormant, not stale.
    pub enabled_rules: &'a EnabledRules,
    /// Lazily-resolved enclosing package name — see [`RuleContext::own_package`].
    /// Private and empty at construction: resolving it touches disk, so the cost
    /// is paid only by the rules that ask, on the files where they match.
    own_package: OnceLock<Option<String>>,
    /// The configured `[compat]` floors (empty when the project sets none),
    /// resolved once per run and carried on [`ResolvedRules`]. Consult
    /// [`RuleContext::r_compat_floor`]/[`RuleContext::roxygen2_compat_floor`],
    /// which layer the per-file `DESCRIPTION` derivation underneath.
    pub compat: &'a CompatConfig,
    /// Lazily-derived `DESCRIPTION` compat facts for this file's package — the
    /// fallback under the configured floors. Same lazy-disk discipline as
    /// [`RuleContext::own_package`]: only the version-aware rules pay the walk.
    description_compat: OnceLock<DescriptionCompat>,
    /// Lazily-built index of the file's Rd topics — see
    /// [`RuleContext::roxygen_topics`]. Same lazy discipline as the two fields
    /// above: a file with no documentation never pays for the walk.
    roxygen_topics: OnceLock<RoxygenTopics>,
}

impl RuleContext<'_> {
    /// The file's Rd topics, grouped by topic key.
    ///
    /// roxygen2 merges every block resolving to the same topic into one `.Rd`
    /// and judges the merged result, so the three `roxygen-*` topic rules need
    /// the file's other blocks, not just the one they were handed. Built once
    /// per file: the walk is shared, and a file without roxygen never pays it.
    pub(crate) fn roxygen_topics(&self) -> &RoxygenTopics {
        self.roxygen_topics
            .get_or_init(|| RoxygenTopics::build(self.root))
    }

    /// The name of the R package this file belongs to, from the `Package` field
    /// of the DESCRIPTION at the enclosing package root. `None` for a loose
    /// script, a directory that is not a package, or an unreadable DESCRIPTION.
    ///
    /// Resolved lazily and memoized for the file: the walk plus the read touch
    /// disk, and the only consumer (`internal-function`) needs it solely on the
    /// rare files that actually contain a `:::`, so the default path — every
    /// other file, every keystroke in the LSP — pays nothing.
    ///
    /// This is the seam for "is this the package's *own* internals?", which is
    /// a different question from cross-file visibility ([`RuleContext::project`]
    /// answers that, and is `None` on the single-file paths).
    pub fn own_package(&self) -> Option<&str> {
        if let Some(facts) = self.package {
            return facts.package.as_deref();
        }
        self.own_package
            .get_or_init(|| crate::project::description::package_name_for_file(self.path))
            .as_deref()
    }

    /// Whether this file is one of its package's R sources — a direct member of
    /// `<root>/R/`.
    ///
    /// R loads `R/*.R` flat (it does not recurse), so the parent directory's
    /// name plus the presence of package facts is exact. Deliberately narrower
    /// than "belongs to a package": [`RuleContext::package`] resolves for a
    /// `tests/testthat/` file too, since it walks up to the package root — and
    /// a test file is not code R will load, which is exactly the distinction a
    /// dependency check needs and `internal-function` needs the opposite of.
    pub fn is_package_r_source(&self) -> bool {
        self.package.is_some()
            && self
                .path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|dir| dir == "R")
    }

    /// The minimum supported R version this file targets, or `None` when no
    /// floor is declared anywhere — the version-aware rules must then stay
    /// silent. Resolution order: the configured `[compat] r` wins; otherwise
    /// the enclosing package's `Depends: R (>= …)` (lazily resolved and
    /// memoized, like [`RuleContext::own_package`]).
    pub fn r_compat_floor(&self) -> Option<CompatVersion> {
        self.compat
            .r_version()
            .or_else(|| self.description_compat().r.clone())
    }

    /// The roxygen2 version this file's documentation targets, or `None` when
    /// undeclared (rules stay silent). Resolution order: the configured
    /// `[compat] roxygen2` wins; otherwise the enclosing package's
    /// `Config/roxygen2/version`, then its legacy `RoxygenNote`.
    pub fn roxygen2_compat_floor(&self) -> Option<CompatVersion> {
        self.compat
            .roxygen2_version()
            .or_else(|| self.description_compat().roxygen2.clone())
    }

    fn description_compat(&self) -> &DescriptionCompat {
        if let Some(facts) = self.package {
            return &facts.compat;
        }
        self.description_compat
            .get_or_init(|| crate::project::description::description_compat_for_file(self.path))
    }

    /// Whether `call`'s callee is confirmed to invoke a base-R function: a
    /// simple name that is (a) exported by one of R's default packages, (b) not
    /// shadowed by a local binding, and (c) not masked by an attached
    /// non-default package. Computed/qualified callees (`pkg::f(...)`,
    /// `x$f(...)`, `(g())(...)`) and anything we can't confirm return `false`,
    /// keeping callers conservative — no rewrite when unsure (Tenets 3/5).
    ///
    /// This is the Phase 2 namespace-confirmation gate: a call-rewrite rule
    /// matches the shape, then asks this before rewriting a bare name.
    pub fn resolves_to_base(&self, call: &CallExpr) -> bool {
        let Some(name) = matchers::callee_name(call) else {
            return false;
        };
        if !self.symbols.is_base(&name) {
            return false;
        }
        // A namespace-qualified callee (`pkg::f(...)`) is not a bare-name base
        // call: `callee_token` unwraps `pkg::f(...)` to the bare `f`, so guard
        // against it explicitly.
        if is_namespace_qualified(call) {
            return false;
        }
        // The callee read sits in `idents` at the callee token's range; if it
        // resolves to a local binding, the base name is shadowed locally. This
        // is the same `resolve_local` pairing `shadowed-builtin` uses, keyed off
        // the call we already hold.
        if let Some(callee) = call.callee_token()
            && self.is_locally_shadowed(callee.text_range())
        {
            return false;
        }
        // Not masked by an attached non-default package.
        origin_is_default(self.symbols.origin(&name, self.model.loaded_packages()))
    }

    /// Whether the identifier read at `range` resolves to a local binding — the
    /// name is redefined in this file rather than referring to the package
    /// function of the same name. The shadow half of [`resolves_to_base`],
    /// shared with rules that match names arity cannot attribute to a package
    /// (e.g. user-configured `undesirable-function` entries) and so can only
    /// apply this weaker gate.
    ///
    /// [`resolves_to_base`]: RuleContext::resolves_to_base
    pub fn is_locally_shadowed(&self, range: rowan::TextRange) -> bool {
        self.model
            .idents()
            .iter()
            .any(|i| i.range == range && self.model.resolve_local(i).is_some())
    }

    /// Whether a bare value read (an `IDENT` token used as a value, e.g. a
    /// function passed as an argument: `sapply(x, length)`) is confirmed to be
    /// base R: exported by a default package, not shadowed by a local binding,
    /// and not masked by an attached non-default package. The value-position
    /// counterpart of [`RuleContext::resolves_to_base`], sharing its
    /// conservative stance — anything unconfirmed returns `false`.
    pub fn read_resolves_to_base(&self, token: &SyntaxToken) -> bool {
        let name = token.text();
        if !self.symbols.is_base(name) {
            return false;
        }
        if self.is_locally_shadowed(token.text_range()) {
            return false;
        }
        origin_is_default(self.symbols.origin(name, self.model.loaded_packages()))
    }

    /// Whether introducing a bare call to `name` is conservatively known to
    /// reach a default-package function. Unlike [`resolves_to_base`], there is
    /// no existing call-site read to resolve, so any same-name binding anywhere
    /// in the file withholds the rewrite rather than guessing its visibility.
    pub fn introduced_call_resolves_to_base(&self, name: &str) -> bool {
        self.symbols.is_base(name)
            && !self
                .model
                .bindings()
                .iter()
                .any(|binding| binding.name == name)
            && origin_is_default(self.symbols.origin(name, self.model.loaded_packages()))
    }
}

/// Whether `call` is the call form of a namespace access (`pkg::f(...)` /
/// `pkg:::f(...)`) — i.e. its `CALL_EXPR` is the RHS of a `::`/`:::` operator.
fn is_namespace_qualified(call: &CallExpr) -> bool {
    let Some(callee) = call.callee_token() else {
        return false;
    };
    // `pkg::fn(...)` parses with the call wrapping a `pkg::fn` `BINARY_EXPR`
    // callee, so the callee token (`callee_token` unwraps it to the bare `fn`)
    // sits under that namespace-access binary.
    callee
        .parent()
        .and_then(BinaryExpr::cast)
        .and_then(|bin| bin.namespace_access())
        .is_some_and(|ns| ns.name_token.text_range() == callee.text_range())
}

/// Whether a resolved origin's effective package (the last/masking one under R's
/// lookup rules) is one of R's default packages.
fn origin_is_default(origin: PackageOrigin) -> bool {
    let pkg = match &origin {
        PackageOrigin::Resolved(pkg) => Some(pkg.as_str()),
        PackageOrigin::Ambiguous(pkgs) => pkgs.last().map(|p| p.as_str()),
        PackageOrigin::Unknown => None,
    };
    pkg.is_some_and(|p| crate::semantic::symbols::default_packages().contains(&p))
}

/// Configured set of rules for a single linting run, plus the derived dispatch
/// state that only depends on the rule set: the node-dispatch table and each
/// rule's stamped severity. Both are computed once here (in [`with_config`], via
/// [`resolve`]) rather than rebuilt per file in [`run_rules`], so reusing one
/// `ResolvedRules` across many files — the CLI batch pass, and the LSP lint
/// worker, which caches it across keystrokes — pays that cost only once.
///
/// It also carries the run's `[lint.rules.<id>]` tables, for the same reason:
/// they are per-run config, so rules read them off [`RuleContext::config`]
/// without widening [`run_rules`].
///
/// [`with_config`]: ResolvedRules::with_config
/// [`resolve`]: ResolvedRules::resolve
pub struct ResolvedRules {
    pub rules: Vec<Box<dyn Rule>>,
    /// Node-dispatch table: `kind as usize` -> indices into `rules` of the rules
    /// that subscribed to that kind via [`Rule::interests`]. `SyntaxKind` is a
    /// contiguous `#[repr(u16)]`, so a flat Vec indexed by kind beats a hash map.
    by_kind: Vec<Vec<usize>>,
    /// Whether any rule subscribed to a node kind at all — lets [`run_rules`]
    /// skip the whole-tree traversal when every rule is `check_file`-only.
    any_node_rules: bool,
    /// The DESCRIPTION rules in this set, and their own dispatch table over the
    /// DCF `SyntaxKind`s. A second, independent table rather than a widened
    /// first one: a run over R files never touches it, so the R hot path below
    /// keeps the exact indices and types it had before DCF existed.
    dcf_rules: Vec<Box<dyn DcfRule>>,
    dcf_by_kind: Vec<Vec<usize>>,
    dcf_any_node_rules: bool,
    /// Each rule ID's default severity, so the severity-stamping pass is an
    /// `O(1)` lookup keyed by the finding's rule ID. Covers **both** grammars —
    /// stamping is one rule, whatever the file type.
    severities: HashMap<&'static str, Severity>,
    /// The chosen rule IDs, handed to rules via [`RuleContext::enabled_rules`].
    enabled: EnabledRules,
    /// The `[lint.rules.<id>]` tables, handed to every rule via
    /// [`RuleContext::config`]. Lives here rather than as a [`run_rules`]
    /// parameter because it is per-*run* config, exactly like the rest of this
    /// struct's derived state — so the hot per-file path carries it for free.
    rules_config: RulesConfig,
    /// The run's `[compat]` floors (mirrored onto `LintConfig` at config parse
    /// time), handed to rules via [`RuleContext::compat`] — same per-run
    /// rationale as `rules_config`.
    compat: CompatConfig,
}

impl ResolvedRules {
    /// Build the derived dispatch state (`by_kind`, `severities`) for a chosen
    /// rule set. The single place that knows how a rule set maps to dispatch.
    fn with_config(chosen: Vec<AnyRule>, rules_config: RulesConfig, compat: CompatConfig) -> Self {
        // Severities and the enabled-ID set are built from the *whole* chosen
        // set, before the grammar split: both are keyed by rule ID, and IDs are
        // one namespace.
        let severities = chosen
            .iter()
            .map(|r| (r.id(), r.default_severity()))
            .collect();
        let enabled = EnabledRules(chosen.iter().map(|r| r.id()).collect());

        let mut rules: Vec<Box<dyn Rule>> = Vec::new();
        let mut dcf_rules: Vec<Box<dyn DcfRule>> = Vec::new();
        for rule in chosen {
            match rule {
                AnyRule::R(rule) => rules.push(rule),
                AnyRule::Dcf(rule) => dcf_rules.push(rule),
            }
        }

        let mut by_kind: Vec<Vec<usize>> = vec![Vec::new(); SyntaxKind::COUNT];
        let mut any_node_rules = false;
        for (i, rule) in rules.iter().enumerate() {
            for kind in rule.interests() {
                by_kind[*kind as usize].push(i);
                any_node_rules = true;
            }
        }

        let mut dcf_by_kind: Vec<Vec<usize>> = vec![Vec::new(); dcf::SyntaxKind::COUNT];
        let mut dcf_any_node_rules = false;
        for (i, rule) in dcf_rules.iter().enumerate() {
            for kind in rule.interests() {
                dcf_by_kind[*kind as usize].push(i);
                dcf_any_node_rules = true;
            }
        }

        Self {
            rules,
            by_kind,
            any_node_rules,
            dcf_rules,
            dcf_by_kind,
            dcf_any_node_rules,
            severities,
            enabled,
            rules_config,
            compat,
        }
    }

    /// The rule IDs in this set.
    pub fn enabled(&self) -> &EnabledRules {
        &self.enabled
    }

    /// Build the rule set honoring `select` / `ignore` from `LintConfig`.
    ///
    /// Resolution order:
    /// 1. Start with all rules whose `default_enabled()` is `true`, unless
    ///    `select` is set (then start with the listed rules instead).
    /// 2. Subtract anything in `ignore`.
    /// 3. Unknown rule IDs in `select` or `ignore` are returned via the second
    ///    element of the tuple so the caller can surface them.
    ///
    /// `config.rules` (the `[lint.rules.<id>]` tables) is carried through onto
    /// the result, reaching rules via [`RuleContext::config`]. Unknown *rule
    /// tables* are rejected earlier, when the config is parsed — unlike unknown
    /// IDs in `select`/`ignore`, which are data and so surface here.
    pub fn resolve(config: &LintConfig) -> (Self, Vec<String>) {
        let select = config.select.as_deref();
        let ignore = &config.ignore;
        // Instantiate the registry once and derive the known-ID set from it —
        // rather than calling `all_rule_ids()` (a second `all_rules()`).
        let all = all_rules();
        let mut unknown = Vec::new();
        for id in select.iter().flat_map(|v| v.iter()).chain(ignore.iter()) {
            if !all.iter().any(|r| r.id() == id.as_str()) {
                unknown.push(id.clone());
            }
        }
        let mut chosen: Vec<AnyRule> = match select {
            Some(picks) => all
                .into_iter()
                .filter(|r| picks.iter().any(|p| p == r.id()))
                .collect(),
            None => all.into_iter().filter(|r| r.default_enabled()).collect(),
        };
        chosen.retain(|r| !ignore.iter().any(|i| i == r.id()));
        (
            Self::with_config(chosen, config.rules.clone(), config.compat.clone()),
            unknown,
        )
    }

    pub fn default_set() -> Self {
        let (set, _) = Self::resolve(&LintConfig::default());
        set
    }
}

/// Run every configured rule against a single file's CST + model, dropping the
/// findings the file's `# arity-ignore` directives suppress. Diagnostics are
/// stably sorted by `(start, end, rule)` before returning.
///
/// Suppression is filtered *here*, not by the caller, for two reasons: the
/// directive list has to reach rules on [`RuleContext`], and the post-suppression
/// pass ([`Rule::check_suppressions`]) needs the *result* of filtering — which
/// directives fired — a fact that does not exist any earlier.
///
/// The dispatch table (`resolved.by_kind`) and severity map are precomputed on
/// `resolved`, so this is on the hot path only for the per-file traversal and
/// the rules' own work, not for rebuilding the rule-set-derived state.
pub fn run_rules(
    resolved: &ResolvedRules,
    path: &Path,
    root: &SyntaxNode,
    model: &SemanticModel,
    cfg: &FileControlFlow,
    symbols: &dyn SymbolProvider,
    file: &FileContext<'_>,
) -> Vec<Diagnostic> {
    let suppressions = SuppressionMap::build(root);
    let ctx = RuleContext {
        path,
        root,
        model,
        cfg,
        symbols,
        project: file.project,
        resolution: file.resolution,
        package: file.package,
        topics: file.topics,
        config: &resolved.rules_config,
        suppressions: &suppressions,
        enabled_rules: &resolved.enabled,
        own_package: OnceLock::new(),
        compat: &resolved.compat,
        description_compat: OnceLock::new(),
        roxygen_topics: OnceLock::new(),
    };
    let rules = &resolved.rules;
    let mut all = Vec::new();

    // Single shared traversal feeding every node-shape rule. Visits tokens too
    // (`descendants_with_tokens`) so token-level rules can subscribe to e.g.
    // `IDENT` or `COMMENT`.
    if resolved.any_node_rules {
        for el in root.descendants_with_tokens() {
            for &i in &resolved.by_kind[el.kind() as usize] {
                rules[i].check(&el, &ctx, &mut all);
            }
        }
    }

    // Whole-file pass for model-/comment-driven rules.
    for rule in rules {
        rule.check_file(&ctx, &mut all);
    }

    // Drop the suppressed findings, recording which directives did the work.
    let used = suppressions.filter(&mut all);

    // Post-suppression pass. Its own findings are suppressible too, but against
    // the *frozen* usage record — a directive that only ever silenced an
    // `outdated-suppression` finding is not thereby "used".
    let mut post = Vec::new();
    for rule in rules {
        rule.check_suppressions(&ctx, &used, &mut post);
    }
    if !post.is_empty() {
        post.retain(|d| !suppressions.is_suppressed(d.rule, d.range));
        all.append(&mut post);
    }

    stamp_and_sort(resolved, &mut all);
    all
}

/// Stamp each finding's severity from its rule's `default_severity()`, then
/// apply the stable `(start, end, rule)` ordering.
///
/// Rules build findings with a placeholder severity (`Default::default()`); the
/// authoritative value lives on the rule, so overriding `default_severity()`
/// actually takes effect here (and is the natural seam for a future per-rule
/// severity config override). Keyed by rule ID rather than by emit order — a
/// whole-file pass may interleave findings from several rules.
///
/// Shared by both grammars' drivers so the two can never drift on severity or
/// ordering.
fn stamp_and_sort(resolved: &ResolvedRules, all: &mut [Diagnostic]) {
    for d in all.iter_mut() {
        if let Some(&sev) = resolved.severities.get(d.rule) {
            d.severity = sev;
        }
    }

    all.sort_by(|a, b| {
        (u32::from(a.range.start()), u32::from(a.range.end()), a.rule).cmp(&(
            u32::from(b.range.start()),
            u32::from(b.range.end()),
            b.rule,
        ))
    });
}

/// What a [`DcfRule`] gets to see: one parsed `DESCRIPTION`, and the facts
/// derived from it.
///
/// Deliberately narrow. There is no `compat` (this file *is* the compat
/// source), none of the R-grammar state, and no lazily-resolved disk fallback —
/// unlike [`RuleContext`], whose `own_package`/`description_compat` exist
/// precisely because an R file has to go looking for the document a DCF rule is
/// already holding.
pub struct DcfRuleContext<'a> {
    /// The `DESCRIPTION`'s path. Its parent is the package root.
    pub path: &'a Path,
    /// The DCF CST root, for a rule that wants raw tokens or ranges.
    pub root: &'a dcf::SyntaxNode,
    /// The typed view — how a rule reads fields.
    pub document: &'a dcf::Document,
    /// The facts this document declares, folded once per file so several rules
    /// don't refold the same fields. Derived from `document`, never from disk.
    pub facts: &'a DescriptionFacts,
    /// The enclosing package's R-side dependency usage, when the caller
    /// computed one — the cross-file driver does. `None` on the single-file
    /// paths (`check_description_document`, the docs renderer), where a rule
    /// that needs it must stay silent, exactly as the version-aware rules do
    /// without a floor.
    pub usage: Option<&'a PackageUsage>,
    /// Per-rule option tables from `[lint.rules.<id>]`. See [`RuleContext::config`].
    pub config: &'a RulesConfig,
    /// The document's `# arity-ignore` directives, used by [`run_dcf_rules`] to
    /// drop suppressed findings.
    pub suppressions: &'a SuppressionMap,
    /// The rule IDs running in this pass. See [`RuleContext::enabled_rules`].
    pub enabled_rules: &'a EnabledRules,
}

/// Run every configured `DESCRIPTION` rule against one parsed document — the
/// DCF twin of [`run_rules`], and subject to the same contract: one shared
/// traversal, suppression filtered here rather than by the caller, findings
/// stamped and stably sorted.
///
/// No post-suppression pass: `Rule::check_suppressions` exists for
/// `outdated-suppression`, which has no DCF counterpart yet. Adding one is a
/// default method away.
pub fn run_dcf_rules(
    resolved: &ResolvedRules,
    path: &Path,
    root: &dcf::SyntaxNode,
    document: &dcf::Document,
    facts: &DescriptionFacts,
    usage: Option<&PackageUsage>,
) -> Vec<Diagnostic> {
    let suppressions = SuppressionMap::build_dcf(root);
    let ctx = DcfRuleContext {
        path,
        root,
        document,
        facts,
        usage,
        config: &resolved.rules_config,
        suppressions: &suppressions,
        enabled_rules: &resolved.enabled,
    };
    let rules = &resolved.dcf_rules;
    let mut all = Vec::new();

    if resolved.dcf_any_node_rules {
        for el in root.descendants_with_tokens() {
            for &i in &resolved.dcf_by_kind[el.kind() as usize] {
                rules[i].check(&el, &ctx, &mut all);
            }
        }
    }

    for rule in rules {
        rule.check_file(&ctx, &mut all);
    }

    suppressions.filter(&mut all);
    stamp_and_sort(resolved, &mut all);
    all
}

/// Provide a sane default symbol provider: base R only, with no installed-
/// package index. Behaves exactly like the historical `StaticBaseR` for files
/// that don't attach non-default packages.
pub fn default_symbol_provider() -> CompositeProvider {
    CompositeProvider::base_only()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linter::diagnostic::ViolationData;

    /// A rule that subscribes to every `CALL_EXPR` and emits a finding carrying
    /// the *placeholder* severity (`Default::default()` == `Warning`). Its
    /// `default_severity` is overridden to `Error`, so a run that respects the
    /// override must stamp `Error` — proving `default_severity` is live, not the
    /// dead trait method it used to be.
    struct FakeError;
    impl Rule for FakeError {
        fn id(&self) -> &'static str {
            "fake-error"
        }
        fn default_severity(&self) -> Severity {
            Severity::Error
        }
        fn interests(&self) -> &'static [SyntaxKind] {
            &[SyntaxKind::CALL_EXPR]
        }
        fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
            sink.push(Diagnostic {
                rule: "fake-error",
                severity: Default::default(),
                path: Default::default(),
                range: el.text_range(),
                message: ViolationData::new("fake-error", "boom"),
                fix: None,
            });
        }
    }

    /// The DCF twin of [`FakeError`]: subscribes to every `FIELD` and emits a
    /// finding with the placeholder severity, so the same stamping and
    /// suppression contract can be asserted over the second grammar.
    struct FakeDcfError;
    impl DcfRule for FakeDcfError {
        fn id(&self) -> &'static str {
            "fake-dcf-error"
        }
        fn default_severity(&self) -> Severity {
            Severity::Error
        }
        fn interests(&self) -> &'static [dcf::SyntaxKind] {
            &[dcf::SyntaxKind::FIELD]
        }
        fn check(
            &self,
            el: &dcf::SyntaxElement,
            _ctx: &DcfRuleContext<'_>,
            sink: &mut Vec<Diagnostic>,
        ) {
            sink.push(Diagnostic {
                rule: "fake-dcf-error",
                severity: Default::default(),
                path: Default::default(),
                range: el.text_range(),
                message: ViolationData::new("fake-dcf-error", "boom"),
                fix: None,
            });
        }
    }

    fn dcf_resolved() -> ResolvedRules {
        ResolvedRules::with_config(
            vec![AnyRule::Dcf(Box::new(FakeDcfError))],
            RulesConfig::default(),
            CompatConfig::default(),
        )
    }

    fn run_dcf(resolved: &ResolvedRules, text: &str) -> Vec<Diagnostic> {
        let parsed = crate::dcf::parse(text);
        let document = parsed.document();
        let facts = DescriptionFacts::from_document(&document);
        run_dcf_rules(
            resolved,
            Path::new("DESCRIPTION"),
            &parsed.cst,
            &document,
            &facts,
            None,
        )
    }

    /// The grammar split happens exactly once, in `with_config`: a DCF rule
    /// lands in the DCF dispatch table and never in the R one, so the R hot
    /// path cannot pay for it.
    #[test]
    fn with_config_files_rules_by_grammar() {
        let resolved = ResolvedRules::with_config(
            vec![
                AnyRule::R(Box::new(FakeError)),
                AnyRule::Dcf(Box::new(FakeDcfError)),
            ],
            RulesConfig::default(),
            CompatConfig::default(),
        );
        assert_eq!(resolved.rules.len(), 1);
        assert_eq!(resolved.dcf_rules.len(), 1);
        assert_eq!(resolved.by_kind[SyntaxKind::CALL_EXPR as usize], vec![0]);
        assert_eq!(
            resolved.dcf_by_kind[dcf::SyntaxKind::FIELD as usize],
            vec![0]
        );
        // One namespace of IDs and one severity map, whatever the grammar.
        assert!(resolved.enabled().contains("fake-error"));
        assert!(resolved.enabled().contains("fake-dcf-error"));
    }

    #[test]
    fn run_dcf_rules_stamps_default_severity() {
        let diags = run_dcf(&dcf_resolved(), "Package: p\n");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Severity::Error);
    }

    /// A `# arity-ignore` line in a `DESCRIPTION` suppresses the field that
    /// follows it, the DCF answer to "the next non-trivia sibling".
    #[test]
    fn run_dcf_rules_filters_suppressed_findings() {
        let diags = run_dcf(
            &dcf_resolved(),
            "# arity-ignore fake-dcf-error: quiet\nPackage: p\n",
        );
        assert!(diags.is_empty(), "expected no findings, got {diags:?}");
    }

    /// The range of `field` in `text`, for asserting *which* finding survived.
    fn field_range(text: &str, name: &str) -> rowan::TextRange {
        crate::dcf::parse(text)
            .document()
            .field(name)
            .unwrap_or_else(|| panic!("a `{name}` field"))
            .syntax()
            .text_range()
    }

    /// The directive reaches only the field it precedes — the next-item scope
    /// must not silently widen to the whole record.
    #[test]
    fn dcf_node_directive_covers_only_the_next_field() {
        let text = "# arity-ignore fake-dcf-error: quiet\nPackage: p\nVersion: 1.0\n";
        let diags = run_dcf(&dcf_resolved(), text);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].range, field_range(text, "Version"));
    }

    /// A comment *between* two fields is grammatically a child of the earlier
    /// one — a `FIELD` stays open across its continuation lines — but it points
    /// at the field that follows, which is what its author meant.
    #[test]
    fn dcf_trailing_directive_covers_the_following_field() {
        let text = "Package: p\n# arity-ignore fake-dcf-error: quiet\nVersion: 1.0\n";
        let diags = run_dcf(&dcf_resolved(), text);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].range, field_range(text, "Package"));
    }

    /// A comment *inside* a field's continuation attaches to that field: R
    /// skips the line and resumes the value, so there is no "next line" that
    /// means anything else.
    #[test]
    fn dcf_directive_inside_a_field_covers_that_field() {
        let diags = run_dcf(
            &dcf_resolved(),
            "Imports:\n    dplyr,\n# arity-ignore fake-dcf-error: quiet\n    rlang\n",
        );
        assert!(diags.is_empty(), "expected no findings, got {diags:?}");
    }

    #[test]
    fn dcf_file_directive_covers_the_whole_document() {
        let diags = run_dcf(
            &dcf_resolved(),
            "# arity-ignore-file: quiet\nPackage: p\nVersion: 1.0\n",
        );
        assert!(diags.is_empty(), "expected no findings, got {diags:?}");
    }

    /// A directive with nothing after it is dangling, exactly as in R — it is
    /// recorded (so the `meta` rules can see it) but suppresses nothing. The
    /// blank line closes the record, and a comment never opens one, so nothing
    /// follows this directive even though the file continues.
    #[test]
    fn dcf_dangling_directive_suppresses_nothing() {
        let text = "Package: p\n\n# arity-ignore fake-dcf-error: quiet\n";
        let map = SuppressionMap::build_dcf(&crate::dcf::parse(text).cst);
        assert_eq!(map.directives().len(), 1);
        assert!(map.directives()[0].is_dangling());
        assert_eq!(run_dcf(&dcf_resolved(), text).len(), 1);
    }

    #[test]
    fn run_rules_stamps_default_severity() {
        let root = crate::parser::parse("f(1)").cst;
        let model = SemanticModel::build(&root);
        let cfg = FileControlFlow::build(&root);
        let symbols = crate::semantic::StaticBaseR::new();
        let resolved = ResolvedRules::with_config(
            vec![AnyRule::R(Box::new(FakeError))],
            RulesConfig::default(),
            CompatConfig::default(),
        );
        let diags = run_rules(
            &resolved,
            Path::new("test.R"),
            &root,
            &model,
            &cfg,
            &symbols,
            &FileContext::default(),
        );
        assert_eq!(diags.len(), 1);
        // Emitted with the `Warning` placeholder; the override stamps `Error`.
        assert_eq!(diags[0].severity, Severity::Error);
    }

    /// Suppression filtering lives in `run_rules`, not in `check.rs` — the rules
    /// need the directive list on `RuleContext`, and `outdated-suppression`
    /// needs the *result* of filtering.
    #[test]
    fn run_rules_filters_suppressed_findings() {
        let root = crate::parser::parse("# arity-ignore fake-error: quiet\nf(1)\n").cst;
        let model = SemanticModel::build(&root);
        let cfg = FileControlFlow::build(&root);
        let symbols = crate::semantic::StaticBaseR::new();
        let resolved = ResolvedRules::with_config(
            vec![AnyRule::R(Box::new(FakeError))],
            RulesConfig::default(),
            CompatConfig::default(),
        );
        let diags = run_rules(
            &resolved,
            Path::new("test.R"),
            &root,
            &model,
            &cfg,
            &symbols,
            &FileContext::default(),
        );
        assert!(diags.is_empty(), "expected no findings, got {diags:?}");
    }

    /// The rule set reaches rules through the context, so a post-suppression
    /// pass can tell "this rule found nothing" from "this rule never ran".
    #[test]
    fn enabled_rules_reflects_the_resolved_set() {
        let resolved = ResolvedRules::with_config(
            vec![AnyRule::R(Box::new(FakeError))],
            RulesConfig::default(),
            CompatConfig::default(),
        );
        assert!(resolved.enabled().contains("fake-error"));
        assert!(!resolved.enabled().contains("unused-binding"));
    }

    /// `resolves_to_base` for the first `CallExpr` in `src`, over the base-only
    /// `StaticBaseR` provider (the single-file / LSP path).
    fn resolves(src: &str) -> bool {
        let root = crate::parser::parse(src).cst;
        let model = SemanticModel::build(&root);
        let cfg = FileControlFlow::build(&root);
        let symbols = crate::semantic::StaticBaseR::new();
        let ctx = RuleContext {
            path: Path::new("test.R"),
            root: &root,
            model: &model,
            cfg: &cfg,
            symbols: &symbols,
            project: None,
            resolution: None,
            package: None,
            topics: None,
            config: &RulesConfig::default(),
            suppressions: &SuppressionMap::default(),
            enabled_rules: &EnabledRules::default(),
            own_package: OnceLock::new(),
            compat: &CompatConfig::default(),
            description_compat: OnceLock::new(),
            roxygen_topics: OnceLock::new(),
        };
        let call = root
            .descendants()
            .find_map(CallExpr::cast)
            .expect("a call in the source");
        ctx.resolves_to_base(&call)
    }

    #[test]
    fn confirms_unshadowed_base_call() {
        assert!(resolves("c(1, 2)"));
        assert!(resolves("f <- function() sum(a)"));
    }

    #[test]
    fn rejects_local_value_shadow() {
        // The first call is `c(2, 3)`; the local `c <- 1` shadows base `c`.
        assert!(!resolves("c <- 1\nc(2, 3)"));
    }

    #[test]
    fn rejects_function_redefinition() {
        assert!(!resolves("any <- function(x) x\nany(z)"));
    }

    #[test]
    fn rejects_nested_scope_shadow() {
        assert!(!resolves("f <- function() {\n  sum <- 1\n  sum(a)\n}"));
    }

    #[test]
    fn rejects_non_base_name() {
        assert!(!resolves("frobnicate(1)"));
    }

    #[test]
    fn rejects_qualified_callee() {
        assert!(!resolves("dplyr::filter(x)"));
    }

    #[test]
    fn rejects_computed_callee() {
        assert!(!resolves("(g())(1)"));
    }
}
