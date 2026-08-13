//! `description-package-in-multiple-fields`: a package listed in more than one
//! of `Depends`, `Imports`, `Suggests`, and `Enhances`.
//!
//! *Writing R Extensions* is blunt about it: a package should be listed in only
//! one of these fields. They are not additive, they are a choice, and each pair
//! is contradictory in its own way — `Imports` plus `Suggests` says the package
//! is both required and optional, `Depends` plus `Imports` says it is both
//! attached and not. R resolves the contradiction by picking one, so the
//! declaration the author wrote second is simply inert, and `R CMD check`
//! reports the pair (`.check_package_description2`'s `duplicates`).
//!
//! **`LinkingTo` is deliberately not one of the four.** A package supplying C++
//! headers *and* R code belongs in both `LinkingTo` and `Imports` — that is the
//! Rcpp idiom, not a defect — and R's own check leaves `LinkingTo` out of the
//! comparison for exactly that reason.
//!
//! `R` is never a dependency: it names the language, and its floor is the
//! `Depends: R (>= x.y)` entry the compat layer reads.
//!
//! A package repeated *within* one field is a different defect and not this
//! rule's: R uniques each field before comparing them, so `Imports: dplyr,
//! dplyr` raises no `duplicates` signal either.
//!
//! **The span is the later listing's package name**, in source order. The
//! earlier one is what the reader is being pointed back at, and the bare name
//! is what R reports, which keeps the differential oracle comparing like with
//! like. The message names the earlier *field* rather than a line number: a
//! `DESCRIPTION` has one `Imports`, so naming it locates the other listing.
//!
//! **No autofix.** Deleting a listing means deciding which field the package
//! belongs in, and `Imports` versus `Suggests` is the decision about whether
//! the code may rely on the package at all. A fix would make that call silently.

use std::collections::{HashMap, HashSet};

use rowan::TextRange;
use smol_str::SmolStr;

use crate::dcf::deps::dependency_entries;
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};
use crate::project::description::DependencyField;

pub struct DescriptionPackageInMultipleFields;

/// The four fields a package must appear in only one of. `LinkingTo` is absent
/// on purpose — see the module docs.
const FIELDS: [DependencyField; 4] = [
    DependencyField::Depends,
    DependencyField::Imports,
    DependencyField::Suggests,
    DependencyField::Enhances,
];

const EXAMPLES: &[Example] = &[Example {
    caption: "A package declared as both a hard requirement and an optional one:",
    source: "Package: mypkg\nVersion: 0.1.0\nImports: dplyr, rlang\nSuggests: dplyr, testthat\n",
}];

impl DcfRule for DescriptionPackageInMultipleFields {
    fn id(&self) -> &'static str {
        "description-package-in-multiple-fields"
    }

    fn description(&self) -> &'static str {
        "Flag a package listed in more than one of `Depends`, `Imports`, \
         `Suggests`, and `Enhances`.\n\n*Writing R Extensions* says a package \
         should be listed in only one of these fields. They are a choice, not \
         an accumulation, and every pair contradicts itself: `Imports` plus \
         `Suggests` declares the package both required and optional, `Depends` \
         plus `Imports` both attached and not. R settles it by picking one \
         field, so the second declaration is inert, and `R CMD check` reports \
         the pair.\n\n`LinkingTo` is deliberately excluded. A package that \
         supplies headers *and* R code belongs in both `LinkingTo` and \
         `Imports`—the Rcpp idiom—and R's own check leaves it out of the \
         comparison for the same reason. `R` is excluded too: it names the \
         language, not a package.\n\nThe finding sits on the *later* listing, \
         and names the field holding the earlier one. There is no autofix: \
         which field to keep is a decision about whether the code may rely on \
         the package at all."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        // Every distinct package each field declares. First occurrence of a
        // field name only, matching `Document::field` and `DescriptionFacts`;
        // a repeated field is `description-duplicate-field`'s finding.
        let mut listings: Vec<(DependencyField, SmolStr, TextRange)> = Vec::new();
        for field in FIELDS {
            let Some(node) = ctx.document.field(field.name()) else {
                continue;
            };
            let mut seen_here: HashSet<SmolStr> = HashSet::new();
            for entry in dependency_entries(&node) {
                if entry.name == "R" || !seen_here.insert(entry.name.clone()) {
                    continue;
                }
                listings.push((field, entry.name, entry.name_range));
            }
        }
        // Source order, so a finding always points backwards up the file: the
        // fields themselves may be written in any order.
        listings.sort_by_key(|(_, _, range)| range.start());

        let mut first: HashMap<SmolStr, DependencyField> = HashMap::new();
        for (field, name, range) in listings {
            let Some(&earlier) = first.get(&name) else {
                first.insert(name, field);
                continue;
            };
            let (earlier, later) = (earlier.name(), field.name());
            sink.push(Diagnostic {
                rule: "description-package-in-multiple-fields",
                severity: Default::default(),
                path: Default::default(),
                range,
                message: ViolationData::new(
                    "description-package-in-multiple-fields",
                    format!(
                        "`{name}` is already listed in `{earlier}`; a package belongs \
                         in only one dependency field"
                    ),
                )
                .with_suggestion(format!(
                    "List `{name}` in either `{earlier}` or `{later}`, and delete the \
                     other entry."
                )),
                fix: None,
            });
        }
    }
}
