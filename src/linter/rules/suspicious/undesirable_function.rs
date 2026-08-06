//! `undesirable-function`: a call to a function the project has banned.
//!
//! Which functions are undesirable is a **project policy**, not a fact about R,
//! so this rule is the linter's first config-driven one: the name -> suggestion
//! map comes from `[lint.rules.undesirable-function]` (see
//! [`UndesirableFunctionConfig`]). It ships a conservative built-in set of base-R
//! functions that reach outside the current evaluation to mutate global state
//! (`attach`, `setwd`, `options`, `Sys.setenv`, ...) plus the debugging entry
//! points that should not survive into committed code (`debug`, `trace`, ...).
//! `functions` replaces that set, `extend-functions` adds to it.
//!
//! **Default-off.** Even with a defensible default set, banning a function is a
//! per-project call — `setwd()` in an analysis script is not the same mistake as
//! `setwd()` in a package. Enable it with `select`.
//!
//! **No autofix.** The rule knows a call is unwanted, never what should replace
//! it: the suggestion is prose written by whoever configured the entry. Rewriting
//! `attach(df)` into `with(df, ...)` is a semantic restructuring, not a textual
//! edit the linter may perform.
//!
//! **Namespace confirmation** is two-tier, because the configured names are
//! open-ended. For a name arity can attribute to base R
//! (`SymbolProvider::is_base`), the full [`RuleContext::resolves_to_base`] gate
//! applies, so a user redefinition or an attached package masking the name is
//! left alone. For a user-added name arity cannot attribute to any package
//! (`my_helper`, a function from an unindexed package), only the local-shadow
//! half ([`RuleContext::is_locally_shadowed`]) can be checked — demanding
//! base-R confirmation there would make user config silently no-op, which is
//! worse than the weaker gate.
//!
//! v1 matches **bare-name calls only**: `base::attach(x)` is not flagged,
//! consistent with the namespace-confirmation gate everywhere else in arity. A
//! bare symbol *read* (`sapply(x, setwd)`) is likewise not a call and is not
//! flagged — lintr's `symbol_is_undesirable` behavior is a possible follow-up.
//!
//! [`UndesirableFunctionConfig`]: crate::config::UndesirableFunctionConfig

use rowan::ast::AstNode as _;

use crate::ast::CallExpr;
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct UndesirableFunction;

const EXAMPLES: &[Example] = &[Example {
    caption: "`attach()` is in the built-in set — it puts a data frame's columns \
              on the search path, so later code silently depends on load order:",
    source: "attach(mtcars)\nmean(mpg)\n",
}];

impl Rule for UndesirableFunction {
    fn id(&self) -> &'static str {
        "undesirable-function"
    }

    fn description(&self) -> &'static str {
        "Flag a call to a function the project has banned, with the configured \
         alternative as the suggestion.\n\nThe name -> suggestion map is set in \
         `[lint.rules.undesirable-function]`: `functions` replaces the built-in \
         set, `extend-functions` adds to it. The built-in set covers base-R \
         functions that mutate global state (`attach`, `setwd`, `options`, \
         `Sys.setenv`, ...) and the debugging entry points (`debug`, `trace`, \
         ...); `browser()` is left to the dedicated `browser` rule.\n\nOnly \
         bare-name calls are flagged, and a locally redefined name is skipped. \
         There is no autofix — the rule knows the call is unwanted, not what \
         should replace it."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    /// Opt-in: which functions are undesirable is a per-project policy, so even
    /// the built-in set is a starting point rather than a default judgment.
    fn default_enabled(&self) -> bool {
        false
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(call) = el.as_node().cloned().and_then(CallExpr::cast) else {
            return;
        };
        let Some(name) = matchers::callee_name(&call) else {
            return;
        };
        let Some(suggestion) = ctx.config.undesirable_function.lookup(&name) else {
            return;
        };

        // Two-tier namespace confirmation (see the module doc): a name arity can
        // place in base R gets the full gate; anything else gets the shadow
        // check alone, so a user-configured name is not silently inert.
        let confirmed = if ctx.symbols.is_base(&name) {
            ctx.resolves_to_base(&call)
        } else {
            call.callee_token()
                .is_some_and(|t| !ctx.is_locally_shadowed(t.text_range()))
        };
        if !confirmed {
            return;
        }

        // Tight span: the callee name, not the whole call — the argument list can
        // run for lines and is not what is wrong.
        let range = call
            .callee_token()
            .map_or_else(|| call.syntax().text_range(), |t| t.text_range());

        let message = ViolationData::new(
            "undesirable-function",
            format!("call to undesirable function `{name}`"),
        );
        // An empty suggestion is the configured "no alternative, just don't call
        // this" — render no clause rather than an empty one.
        let message = if suggestion.is_empty() {
            message
        } else {
            message.with_suggestion(format!("Avoid `{name}()`: {suggestion}."))
        };

        sink.push(Diagnostic {
            rule: "undesirable-function",
            severity: Default::default(),
            path: Default::default(),
            range,
            message,
            fix: None,
        });
    }
}
