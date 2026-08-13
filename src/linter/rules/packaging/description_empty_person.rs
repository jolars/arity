//! `description-empty-person`: a `person()` in `Authors@R` that supplies
//! nothing.
//!
//! `person()` with no arguments — and `person(NULL)`, which takes the same
//! early return — is a **zero-length** person vector, not a nameless person. R
//! concatenates it away without a word: `c(person("Jane", …), person())` is a
//! one-element vector, `Author` and `Maintainer` derive exactly as they would
//! have, and nothing in `R CMD check` will ever mention it. It really does sit
//! at the end of `xfun`'s `Authors@R`, which is where this rule came from.
//!
//! So the finding is **style, not correctness**: the call is a contributor
//! someone opened and never filled in, left in shipped metadata where it reads
//! as an intention. Nothing is broken, and that is the point — it is invisible
//! to every tool that reads the field, this one included until it is looked
//! for.
//!
//! **This is the one packaging rule R does not back**, which is why it is its
//! own rule and not a clause of `description-authors-at-r`. That rule is
//! gated against `R CMD check` in `tests/description_oracle.rs` — every finding
//! it makes must be one of R's — and folding an opinion R does not share into
//! it would make the gate's claim conditional. Keeping the ids apart keeps that
//! claim exact and lets this one be suppressed on its own.
//!
//! **No autofix.** Deleting the call means deleting a comma that belongs to its
//! neighbor, and the other repair — filling the person in — is the author's.

use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::packaging::authors::{self, Authors};
use crate::linter::rules::packaging::scalar_field::folded;
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};

pub struct DescriptionEmptyPerson;

const RULE: &str = "description-empty-person";

const EXAMPLES: &[Example] = &[Example {
    caption: "A contributor opened and never filled in. R drops the call \
              silently, so the credit was never going to appear:",
    source: "Package: mypkg\nVersion: 0.1.0\n\
             Authors@R: c(\n    \
             person(\"Jane\", \"Doe\", , \"jane@example.com\", c(\"aut\", \"cre\")),\n    \
             person()\n  )\n",
}];

impl DcfRule for DescriptionEmptyPerson {
    fn id(&self) -> &'static str {
        RULE
    }

    fn description(&self) -> &'static str {
        "Flag a `person()` in `Authors@R` that supplies no arguments.\n\n\
         `person()`—and `person(NULL)`, which takes the same early \
         return—is a **zero-length** person vector, not a nameless person. R \
         concatenates it away without a word: `c(person(\"Jane\", …), person())` \
         is a one-element vector, `Author` and `Maintainer` derive exactly as \
         they would have, and nothing in `R CMD check` will ever mention \
         it.\n\nSo this is style rather than correctness. The call is a \
         contributor someone opened and never filled in, left in shipped \
         metadata where it reads as an intention—and it is invisible to every \
         tool that reads the field unless one looks for it.\n\nIt is a rule of \
         its own rather than a clause of `description-authors-at-r` because it \
         is the one packaging finding `R CMD check` does not back, and keeping \
         the ids apart keeps that rule's claim exact—as well as letting this \
         one be suppressed on its own.\n\nA person carrying anything at all is \
         a person, however little R can make of them: `person(role = \"ctb\")` \
         and even `person(\"\")` are `description-authors-at-r`'s subject, not \
         this rule's. A computed argument could be `NULL` and could be a name, \
         so the rule stays silent there.\n\nThere is no autofix: deleting the \
         call means deleting a comma that belongs to its neighbor, and filling \
         the person in is the author's."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(field) = ctx.document.field("Authors@R") else {
            return;
        };
        let Some(source) = folded(&field) else {
            return;
        };
        // Every other outcome — unparseable, an unsafe call, a value no static
        // reading resolves — is a field this rule cannot see people in at all,
        // and `description-authors-at-r` owns saying so.
        let Authors::Persons(resolved) = authors::resolve(&source) else {
            return;
        };

        for range in resolved.empty {
            sink.push(Diagnostic {
                rule: RULE,
                severity: Default::default(),
                path: Default::default(),
                range,
                message: ViolationData::new(
                    RULE,
                    "this `person()` supplies nothing, so it names nobody".to_string(),
                )
                .with_suggestion(
                    "Fill the person in, or delete the call: R reads `person()` as a \
                     zero-length person vector and drops it silently."
                        .to_string(),
                ),
                fix: None,
            });
        }
    }
}
