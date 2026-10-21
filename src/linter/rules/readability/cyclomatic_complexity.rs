//! Function complexity comes from the shared CFG, including potential expression
//! flow. Refactoring requires judgment, so this policy rule offers no rewrite.

use rowan::ast::AstNode as _;

use crate::ast::{BinaryExpr, CallExpr};
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext};

pub struct CyclomaticComplexity;

const EXAMPLES: &[Example] = &[Example {
    caption: "Many independent conditions make a function harder to maintain:",
    source: "dispatch <- function(x) {\n  if (x$a && x$b) a()\n  if (x$c && x$d) b()\n  if (x$e && x$f) c()\n  if (x$g && x$h) d()\n  if (x$i && x$j) e()\n  if (x$k && x$l) f()\n  if (x$m && x$n) g()\n  if (x$o && x$p) h()\n  if (x$q && x$r) i()\n  if (x$s && x$t) j()\n}\n",
}];

impl Rule for CyclomaticComplexity {
    fn id(&self) -> &'static str {
        "cyclomatic-complexity"
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn description(&self) -> &'static str {
        "Report function bodies whose native CFG decision complexity exceeds \
         `[lint.rules.cyclomatic-complexity] max-complexity` (20 by default, at \
         least 1). This maintainability policy is opt-in and has no autofix.\n\n\
         The score starts at 1. Each reachable `if`, `for`, `while`, `&&`, or \
         `||` adds 1. Each value-bearing arm of a base-resolved `switch()` adds \
         1, including an unnamed arm: a numeric selector can still match none. \
         Empty fallthrough labels add nothing. `else`, `return()`, `stop()`, \
         `break`, `next`, and unconditional `repeat` add nothing themselves. \
         Decisions in nested functions are scored independently; formal defaults, \
         quoted bodies, and unreachable tails are excluded. Decisions in ordinary \
         call arguments count as potential flow, without assuming eager evaluation.\n\n\
         A simple loop or an early-return guard scores 2. These native scores \
         intentionally differ from cyclocomp and Jarl, which score those shapes \
         as 3. Consider splitting a complex function into smaller helpers."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let maximum = ctx.config.cyclomatic_complexity.max_complexity.get() as usize;
        for (ptr, graph) in ctx.cfg.functions() {
            let score = graph.cyclomatic_complexity(&|ptr| {
                let Some(call) = ptr.try_to_node(ctx.root).and_then(CallExpr::cast) else {
                    return false;
                };
                if let Some(callee) = call.base().and_then(|el| el.into_node())
                    && let Some(access) =
                        BinaryExpr::cast(callee).and_then(|binary| binary.namespace_access())
                {
                    return access.package == "base" && access.name == "switch";
                }
                ctx.resolves_to_base(&call)
            });
            if score <= maximum {
                continue;
            }
            let Some(node) = ptr.try_to_node(ctx.root) else {
                continue;
            };
            if node
                .ancestors()
                .skip(1)
                .any(|ancestor| crate::semantic::builder::call_quote_kind(&ancestor).is_some())
            {
                continue;
            }
            let Some(keyword) = node.first_token() else {
                continue;
            };
            sink.push(Diagnostic {
                rule: self.id(),
                severity: self.default_severity(),
                path: Default::default(),
                range: keyword.text_range(),
                message: ViolationData::new(self.id(), format!(
                    "function has a cyclomatic complexity of {score}, exceeding the maximum of {maximum}"
                )).with_suggestion("Split this function into smaller helpers."),
                fix: None,
            });
        }
    }
}
