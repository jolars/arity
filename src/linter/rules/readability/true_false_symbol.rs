//! `true-false-symbol`: prefer the reserved literals `TRUE`/`FALSE` over the
//! rebindable base symbols `T`/`F`.
//!
//! `T`/`F` are not reserved words — they are ordinary base-R bindings (`T <-
//! FALSE` is legal), so a blind rewrite could change behavior. We lean on the
//! semantic model to stay safe: [`SemanticModel::idents`] records only value
//! *reads* (excluding named-arg names, `$`/`@` members, `::` operands, and
//! assignment targets), and a read that [`resolve_local`]s to a same-file
//! binding is the user's own variable, not the boolean shorthand — so we skip
//! it. What remains resolves to base R, and the same-span token swap never
//! alters layout, so the fix is `Safe`.
//!
//! [`SemanticModel::idents`]: crate::semantic::SemanticModel::idents
//! [`resolve_local`]: crate::semantic::SemanticModel::resolve_local

use crate::linter::diagnostic::{Diagnostic, Fix, Severity, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext};

pub struct TrueFalseSymbol;

const EXAMPLES: &[Example] = &[Example {
    caption: "`T` and `F` used as boolean shorthand:",
    source: "x <- T\ny <- F\n",
}];

impl Rule for TrueFalseSymbol {
    fn id(&self) -> &'static str {
        "true-false-symbol"
    }

    fn description(&self) -> &'static str {
        "Prefer the reserved literals `TRUE`/`FALSE` over the rebindable base \
         symbols `T`/`F`.\n\n`T` and `F` are ordinary base-R bindings, not \
         reserved words — `T <- FALSE` is legal — so relying on them as boolean \
         shorthand is fragile. The fix is withheld when the name resolves to a \
         local binding, since that is the user's own variable rather than the \
         shorthand."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        for ident in ctx.model.idents() {
            let replacement = match ident.name.as_str() {
                "T" => "TRUE",
                "F" => "FALSE",
                _ => continue,
            };
            // A read that resolves to a same-file binding is a rebound `T`/`F`,
            // not the boolean shorthand — flagging it would be a false positive.
            if ctx.model.resolve_local(ident).is_some() {
                continue;
            }
            let r = ident.range;
            sink.push(Diagnostic {
                rule: "true-false-symbol",
                severity: Severity::Warning,
                path: Default::default(),
                range: r,
                message: ViolationData::new(
                    "true-false-symbol",
                    format!("use `{replacement}` instead of `{}`", ident.name),
                )
                .with_suggestion("`T`/`F` are rebindable; prefer the reserved literals."),
                fix: Some(Fix::safe(
                    usize::from(r.start()),
                    usize::from(r.end()),
                    replacement.to_string(),
                    format!("Replace `{}` with `{replacement}`", ident.name),
                )),
            });
        }
    }
}
