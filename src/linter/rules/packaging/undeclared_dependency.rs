//! `undeclared-dependency`: package code reaches a package `DESCRIPTION` never
//! declares.
//!
//! `dplyr::filter()` in `R/` works on the author's machine because `dplyr`
//! happens to be installed there. On a clean machine it is a load-time error,
//! and `R CMD check` says so ("Namespace dependency not required"). The
//! declaration is what makes the dependency *installed*, so its absence is a
//! bug that only ever shows up somewhere else.
//!
//! The exempt set is R's, read off `tools:::.check_packages_used`: everything
//! `DESCRIPTION` declares in any of the five fields, the package's own name,
//! and the base-priority packages R ships — **minus `methods` and `stats4`,
//! which R still expects a package to declare**
//! ([`symbols::is_implicitly_available`]). That list is deliberately not
//! `default_packages()`, which answers a different question (what a fresh
//! session *attaches*) and differs in both directions.
//!
//! **Only `R/` is package code.** `package_facts_for` resolves for a
//! `tests/testthat/` file too, since it walks up to the package root, but a
//! test's dependencies belong in `Suggests` and R does not scan `tests/` for
//! this check either ([`RuleContext::is_package_r_source`]).
//!
//! **`Suggests` exempts.** R exempts it here as well. Flagging *unconditional*
//! use of a suggested package is a genuinely different question — is this call
//! reachable without `requireNamespace()` having succeeded? — that belongs to
//! the control-flow graph, not to a name set, and getting it wrong would flag
//! the one idiom careful authors write.
//!
//! Every site is reported, like `internal-function`: each is separately
//! suppressible, and one `DESCRIPTION` line clears them all.
//!
//! **No autofix, structurally.** A [`Fix`] is a contiguous replacement in the
//! diagnostic's own file, and the repair is a line in a different one.
//!
//! [`Fix`]: crate::linter::diagnostic::Fix
//! [`symbols::is_implicitly_available`]: crate::semantic::symbols::is_implicitly_available
//! [`RuleContext::is_package_r_source`]: crate::linter::rules::RuleContext::is_package_r_source

use rowan::ast::AstNode as _;

use crate::ast::{BinaryExpr, CallExpr};
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::semantic::symbols::is_implicitly_available;
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxToken};

pub struct UndeclaredDependency;

const EXAMPLES: &[Example] = &[Example {
    caption: "In `R/` of a package whose `DESCRIPTION` declares only `Imports: rlang`:",
    source: "summarize <- function(data) {\n  dplyr::group_by(data, id)\n}\n",
}];

/// The package the example is linted inside. `undeclared-dependency` is a fact
/// about a package, so its example has to be one.
const EXAMPLE_PACKAGE: &[(&str, &str)] = &[(
    "DESCRIPTION",
    "Package: mypkg\nVersion: 0.1.0\nImports: rlang\n",
)];

impl Rule for UndeclaredDependency {
    fn id(&self) -> &'static str {
        "undeclared-dependency"
    }

    fn description(&self) -> &'static str {
        "Flag package code that reaches a package its `DESCRIPTION` never \
         declares.\n\n`dplyr::filter()` in `R/` works on the author's machine \
         because `dplyr` happens to be installed there; on a clean machine it \
         is a load-time error, and `R CMD check` reports it. Declaring the \
         dependency is what causes it to be installed, so leaving it out is a \
         bug that only ever surfaces somewhere else.\n\nThe rule matches \
         `pkg::name`, `pkg:::name`, and the package argument of `library`, \
         `require`, `requireNamespace`, and `loadNamespace`—at any depth, since \
         the conditional-dependency idiom lives inside a function body.\n\nThe \
         exempt set is R's own: everything declared in any of `Depends`, \
         `Imports`, `Suggests`, `LinkingTo`, or `Enhances`, the package's own \
         name, and the packages R ships at base priority—except `methods` and \
         `stats4`, which `R CMD check` still expects a package to declare. Only \
         files directly in `R/` are checked: a test's or a vignette's \
         dependencies belong in `Suggests`, and R does not scan those \
         directories for this check either.\n\nThere is no autofix. A fix is an \
         edit to the file the finding is in, and the repair here is a line in \
         `DESCRIPTION`."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn doc_package(&self) -> &'static [(&'static str, &'static str)] {
        EXAMPLE_PACKAGE
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::BINARY_EXPR, SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        if !ctx.is_package_r_source() {
            return;
        }
        let facts = ctx.package.expect("gated by is_package_r_source");
        // An unreadable or nameless DESCRIPTION derives all-default facts, which
        // would make every package in the file look undeclared.
        let Some(own) = facts.package.as_deref() else {
            return;
        };
        let Some(node) = el.as_node() else {
            return;
        };

        let (name, token, how) = match el.kind() {
            SyntaxKind::BINARY_EXPR => {
                let Some(access) =
                    BinaryExpr::cast(node.clone()).and_then(|b| b.namespace_access())
                else {
                    return;
                };
                (access.package, access.package_token, How::Qualified)
            }
            SyntaxKind::CALL_EXPR => {
                let Some(call) = CallExpr::cast(node.clone()) else {
                    return;
                };
                let Some((name, token)) = matchers::package_load_arg(&call) else {
                    return;
                };
                (name, token, How::Attached)
            }
            _ => return,
        };

        if name == own || is_implicitly_available(&name) {
            return;
        }
        // A linear scan over a list that is a dozen entries long, rather than
        // `declared_packages()`, which builds a fresh `BTreeSet` per call.
        if facts.dependencies.iter().any(|d| d.name == name) {
            return;
        }

        sink.push(diagnostic(&name, &token, how));
    }
}

/// How the file reached the package — which decides only the wording.
#[derive(Clone, Copy)]
enum How {
    /// `pkg::name` / `pkg:::name`.
    Qualified,
    /// `library(pkg)` and friends.
    Attached,
}

fn diagnostic(name: &str, token: &SyntaxToken, how: How) -> Diagnostic {
    let suggestion = match how {
        How::Qualified => format!("Add `{name}` to `Imports:` in DESCRIPTION."),
        How::Attached => format!(
            "Add `{name}` to `Imports:` in DESCRIPTION, and reach it with `{name}::` \
             rather than attaching it from package code."
        ),
    };
    Diagnostic {
        rule: "undeclared-dependency",
        severity: Default::default(),
        path: Default::default(),
        // The package name alone: it *is* the violation, unlike
        // `internal-function`, where the pair is.
        range: token.text_range(),
        message: ViolationData::new(
            "undeclared-dependency",
            format!("package `{name}` is used here but is not declared in DESCRIPTION"),
        )
        .with_suggestion(suggestion),
        fix: None,
    }
}
