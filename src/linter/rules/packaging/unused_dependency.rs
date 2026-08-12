//! `unused-dependency`: an `Imports:` entry nothing in the package reaches.
//!
//! An `Imports` entry is a promise that installing this package installs that
//! one. An entry no code reaches costs every user of the package a download, a
//! build, and a version constraint to satisfy, for nothing — and `R CMD check`
//! reports it.
//!
//! **`Imports` only.** `Depends` is an API decision (`Depends: R6` re-exports a
//! world to the *user*), and a package's own code may legitimately never name
//! it. `Suggests` is optional by definition, reached from tests, vignettes, and
//! examples rather than from the package's own code. `LinkingTo` is C or C++
//! headers and `Enhances` is optional by definition, both structurally
//! invisible to R-source analysis.
//!
//! **This rule reports on absence**, which is a much stronger claim than every
//! other rule makes, and a wrong one leads a maintainer to delete a dependency
//! their package needs — a package that still installs locally and fails on a
//! clean machine. Hence: default-off, a completeness guard, and a deliberately
//! over-broad notion of "reaches".
//!
//! [`PackageUsage::complete`] is that guard. The rule stays silent unless the
//! run analyzed the package's whole R source set, read its NAMESPACE, and found
//! at least one source — so a single-file lint, a package with a parse error in
//! `R/`, and one mid-`document()` all report nothing rather than everything.
//!
//! The exemptions each close a named false positive:
//!
//! - **`LinkingTo`.** `Imports: Rcpp` + `LinkingTo: Rcpp` with no R-side
//!   reference is the canonical Rcpp/cpp11 skeleton; the entry exists so the
//!   shared library loads.
//! - **A string mention.** `do.call("::", …)`, `system.file(package = "pkg")`,
//!   and `rlang::check_installed("pkg")` all name a package as a plain string.
//! - **`methods`.** An S4 or reference class needs it with nothing naming it,
//!   which is why [`PackageReferences::uses_methods`] exists.
//!
//! **What counts as "the package" is the run's file set.** Usage is folded over
//! every analyzed member under the package root
//! ([`package_usage`](crate::project::package_usage)), so a `tests/`,
//! `inst/`, or `data-raw/` file counts as a use whenever the run covers it — as
//! `arity lint .` does. Narrow the run to `R/` and the same entry is reported.
//! Both readings are defensible (R would want a test-only dependency in
//! `Suggests`), and the quieter one falls out of the default, which is the right
//! way round for a rule that reports on absence. A vignette is never analyzed at
//! all — `.Rmd` is not an R source — so a dependency used only there is
//! reported.
//!
//! **No autofix**, for three independent reasons. The edit is not local
//! (deleting a middle entry leaves a dangling comma; deleting the only one
//! leaves an empty field). The finding is deliberately heuristic, and encoding a
//! heuristic as a destructive edit to package metadata inverts the fix bar. And
//! DESCRIPTION is rewritten by `usethis`, `devtools`, and `R CMD build`, which
//! is why even *formatting* it is opt-in. The seam is that formatter: once a
//! dependency field can be re-emitted canonically, "the field minus one entry"
//! becomes correct by construction.
//!
//! [`PackageUsage::complete`]: crate::project::PackageUsage::complete
//! [`PackageReferences::uses_methods`]: crate::project::PackageReferences::uses_methods

use std::collections::BTreeSet;

use crate::dcf::deps::dependency_entries;
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};
use crate::project::DependencyField;

pub struct UnusedDependency;

const EXAMPLES: &[Example] = &[Example {
    caption: "In a package whose only R source is `f <- function() rlang::abort(\"no\")`:",
    source: "Package: mypkg\nVersion: 0.1.0\nImports: rlang, tibble\n",
}];

/// The package the example's `DESCRIPTION` describes. Without a real package
/// around it the completeness guard would (correctly) keep the rule silent.
const EXAMPLE_PACKAGE: &[(&str, &str)] = &[
    ("NAMESPACE", "export(f)\n"),
    ("R/a.R", "f <- function() rlang::abort(\"no\")\n"),
];

impl DcfRule for UnusedDependency {
    fn id(&self) -> &'static str {
        "unused-dependency"
    }

    fn default_enabled(&self) -> bool {
        // Every other rule reports on something present in the file it is
        // looking at. This one claims nothing anywhere reaches an entry — a
        // claim whose truth depends on the shape of the run, on NAMESPACE
        // freshness, and on several over-approximations. And acting on a wrong
        // one yields a package that installs fine locally and fails on a clean
        // machine. Opt in.
        false
    }

    fn description(&self) -> &'static str {
        "Flag an `Imports:` entry that nothing in the package reaches.\n\nAn \
         `Imports` entry promises that installing this package installs that \
         one, so an entry no code reaches costs every user a download, a build, \
         and a constraint to satisfy for nothing. `R CMD check` reports it \
         too.\n\nA package counts as reached by `pkg::`, `pkg:::`, a `library`, \
         `require`, `requireNamespace`, or `loadNamespace` call at any depth, a \
         NAMESPACE `import()`/`importFrom()`/`importClassesFrom()`/\
         `importMethodsFrom()`, or a roxygen `@import`/`@importFrom` tag. \
         Exempt on top of that: anything also in `LinkingTo` (the Rcpp \
         skeleton), `methods` when the package defines an S4 or reference \
         class, and any package whose name appears as a plain string (a dynamic \
         `do.call(\"::\", …)` or `system.file(package = …)`).\n\nOnly `Imports` \
         is checked. `Depends` is an API decision the package's own code may \
         never name; `Suggests` is for tests, vignettes, and examples, reached \
         from outside the package's own code; `LinkingTo` and `Enhances` are \
         invisible to R-source analysis.\n\nUsage is folded over every R file \
         the run analyzed under the package root, so a package reached only \
         from `tests/`, `inst/`, or `data-raw/` counts as used when those files \
         are in the run (`arity lint .`) and is reported when they are not \
         (`arity lint R/`). A vignette's R code is never analyzed either way, \
         so a dependency used only there is reported—it belongs in \
         `Suggests`.\n\nIt reports on \
         *absence*, and a wrong finding would have a maintainer delete a \
         dependency their package needs, so it stays silent unless the run \
         analyzed the package's whole `R/` source set and read its \
         NAMESPACE—which is also why it is off by default.\n\nThere is no \
         autofix: removing an \
         entry from a comma-separated list is not a local edit, and this is not \
         a claim a tool should act on destructively."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn doc_package(&self) -> &'static [(&'static str, &'static str)] {
        EXAMPLE_PACKAGE
    }

    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        // Silent on the single-file paths, and on any run that did not analyze
        // the whole package. Reporting on absence is only sound when the run
        // has actually seen everything.
        let Some(usage) = ctx.usage.filter(|usage| usage.complete) else {
            return;
        };
        let Some(own) = ctx.facts.package.as_deref() else {
            return;
        };
        let Some(field) = ctx.document.field(DependencyField::Imports.name()) else {
            return;
        };

        let linking_to: BTreeSet<&str> = ctx
            .facts
            .in_field(DependencyField::LinkingTo)
            .map(|d| d.name.as_str())
            .collect();

        // Spans are re-derived from the CST: `DescriptionFacts` is range-free by
        // construction, being the salsa backdating firewall for DESCRIPTION.
        for entry in dependency_entries(&field) {
            let name = entry.name.as_str();
            if name == "R" || name == own {
                continue;
            }
            if usage.used.contains(name)
                || usage.mentioned.contains(name)
                || linking_to.contains(name)
            {
                continue;
            }
            sink.push(Diagnostic {
                rule: "unused-dependency",
                severity: Default::default(),
                path: Default::default(),
                // The name, not the version constraint: the constraint is not
                // part of what is unused.
                range: entry.name_range,
                message: ViolationData::new(
                    "unused-dependency",
                    format!(
                        "`{name}` is declared in `Imports:` but nothing in the package uses it"
                    ),
                )
                .with_suggestion(format!(
                    "Remove `{name}` from `Imports:`, or move it to `Suggests:` if only \
                     tests, vignettes, or examples need it."
                )),
                fix: None,
            });
        }
    }
}
