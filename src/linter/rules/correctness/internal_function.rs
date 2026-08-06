//! `internal-function`: `pkg:::name` reaches into a package's internals.
//!
//! `:::` bypasses the namespace and binds an object the package never exported.
//! An unexported object is not part of the package's interface: it carries no
//! compatibility promise, no documentation, and no deprecation cycle, so it can
//! be renamed, have its arguments reshuffled, or vanish in any patch release —
//! and the breakage surfaces only at run time, in the caller. CRAN's policy says
//! the same thing from the other side: a package may not use `:::` to reach a
//! package listed in its own `Depends`/`Imports`/`Suggests`.
//!
//! Mostly a shape match on the `:::` operator via
//! [`BinaryExpr::namespace_access`], which reports `internal` for `:::` and
//! resolves the package and name through backticks/quotes. `::` is the exported
//! interface and is never flagged.
//!
//! **The package's own internals are exempt.** `mypkg:::helper` written *inside*
//! `mypkg` reaches nothing external: in `R/` it is merely a redundant qualifier
//! on a name already in scope, and in `tests/testthat/` it is the idiomatic way
//! to unit-test an unexported function. Neither carries the
//! changes-without-notice hazard this rule is about, so the rule resolves the
//! enclosing package's name from `DESCRIPTION`
//! ([`RuleContext::own_package`]) and skips a self-reference. That lookup is
//! lazy and memoized per file, and only a file containing a `:::` triggers it.
//!
//! A loose script outside any package has no own-package name, so nothing is
//! exempt there — the conservative direction for a report-only rule.
//!
//! **No autofix.** The repair is a judgement call the linter cannot make: swap
//! to an exported equivalent (which may not exist), vendor the implementation,
//! or ask upstream to export it. Rewriting `:::` to `::` would turn a working
//! call into a load-time error, so the rule stays report-only.
//!
//! [`BinaryExpr::namespace_access`]: crate::ast::BinaryExpr::namespace_access
//! [`RuleContext::own_package`]: crate::linter::rules::RuleContext::own_package

use rowan::ast::AstNode as _;

use crate::ast::BinaryExpr;
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct InternalFunction;

const EXAMPLES: &[Example] = &[Example {
    caption: "Calling an unexported function through `:::`:",
    source: "utils:::.getHelpFile(path)\n",
}];

impl Rule for InternalFunction {
    fn id(&self) -> &'static str {
        "internal-function"
    }

    fn description(&self) -> &'static str {
        "Flag `pkg:::name`, which reaches past a package's namespace to an \
         object it never exported.\n\nAn unexported object is not part of the \
         package's interface: it carries no compatibility promise, no \
         documentation, and no deprecation cycle, so it can be renamed, \
         reshaped, or removed in any patch release—and the breakage surfaces \
         only at run time. CRAN's policy bars a package from using `:::` on a \
         package in its own `Depends`/`Imports`/`Suggests` for the same \
         reason.\n\nThe exported form `pkg::name` is never flagged, and neither \
         is a package reaching into its *own* internals: `mypkg:::helper` \
         inside `mypkg` is a redundant qualifier in `R/` and the idiomatic way \
         to unit-test an unexported function in `tests/`, so the rule reads the \
         enclosing package's name from `DESCRIPTION` and skips a \
         self-reference.\n\nThere is no autofix: the repair is to find an \
         exported equivalent, vendor the implementation, or ask upstream to \
         export it—rewriting `:::` to `::` would only turn a working call into \
         a load-time error."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::BINARY_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(access) = el
            .as_node()
            .cloned()
            .and_then(BinaryExpr::cast)
            .and_then(|bin| bin.namespace_access())
        else {
            return;
        };
        if !access.internal {
            return;
        }
        // A package reaching into *its own* internals reaches nothing external:
        // redundant in `R/`, idiomatic in `tests/`. Asked only now, so the
        // DESCRIPTION lookup is confined to files that contain a `:::`.
        if ctx.own_package() == Some(access.package.as_str()) {
            return;
        }
        // Span the access itself (`pkg:::name`), not the enclosing call or
        // statement: the call form parses as `CALL_EXPR > BINARY_EXPR`, so this
        // is the same tight range either way.
        let range = access
            .package_token
            .text_range()
            .cover(access.name_token.text_range());
        let (package, name) = (&access.package, &access.name);
        sink.push(Diagnostic {
            rule: "internal-function",
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new(
                "internal-function",
                format!(
                    "`{package}:::{name}` uses an unexported object, which may change \
                     or disappear without notice"
                ),
            )
            .with_suggestion(format!(
                "Use an exported function from `{package}`, or ask upstream to export \
                 `{name}`."
            )),
            fix: None,
        });
    }
}
