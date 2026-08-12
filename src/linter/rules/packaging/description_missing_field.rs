//! `description-missing-field`: a `DESCRIPTION` without the fields R requires.
//!
//! `R CMD build` refuses a package whose `DESCRIPTION` omits `Package`,
//! `Version`, `Title`, `Description`, `Author`, `Maintainer`, or `License` —
//! and `Authors@R`, which modern packages write instead, is what `Author` and
//! `Maintainer` are derived from, so it satisfies both.
//!
//! A field present but empty declares nothing and counts as missing, which is
//! also what `R CMD check` concludes.
//!
//! **One finding, not one per field.** The defect is that this `DESCRIPTION` is
//! incomplete; stacking five diagnostics on the same line says it five times,
//! and takes five suppressions to silence.
//!
//! A file with no fields at all is left alone: that is not an incomplete
//! package description, it is not a package description.
//!
//! **No autofix.** Every one of these fields needs a value only the author has.

use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};

pub struct DescriptionMissingField;

/// The fields `R CMD build` requires, in the order R writes them.
const REQUIRED: [&str; 7] = [
    "Package",
    "Version",
    "Title",
    "Description",
    "Author",
    "Maintainer",
    "License",
];

/// `Authors@R` is the modern spelling of these two: `R CMD build` derives both
/// from it, so declaring it satisfies both.
const DERIVED_FROM_AUTHORS_AT_R: [&str; 2] = ["Author", "Maintainer"];

const EXAMPLES: &[Example] = &[Example {
    caption: "A `DESCRIPTION` that `R CMD build` would reject:",
    source: "Package: mypkg\nVersion: 0.1.0\n",
}];

impl DcfRule for DescriptionMissingField {
    fn id(&self) -> &'static str {
        "description-missing-field"
    }

    fn description(&self) -> &'static str {
        "Flag a `DESCRIPTION` missing a field R requires.\n\n`R CMD build` \
         refuses a package whose `DESCRIPTION` omits `Package`, `Version`, \
         `Title`, `Description`, `Author`, `Maintainer`, or `License`. \
         `Authors@R` satisfies `Author` and `Maintainer`, since `R CMD build` \
         derives both from it. A field that is present but empty declares \
         nothing and counts as missing.\n\nEvery missing field is reported as \
         one finding rather than one each: the defect is that the file is \
         incomplete, and it takes one decision—and one suppression—to \
         settle.\n\nA file with no fields at all is left alone; that is not an \
         incomplete package description.\n\nThere is no autofix: each of these \
         fields needs a value only the author has."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let declared = |name: &str| {
            ctx.document
                .field(name)
                .is_some_and(|field| !field.folded_value().trim().is_empty())
        };

        // Nothing declared at all: not a package description, so not an
        // incomplete one.
        let Some(first) = ctx.document.fields().find(|f| !f.name().is_empty()) else {
            return;
        };

        let has_authors_at_r = declared("Authors@R");
        let missing: Vec<&str> = REQUIRED
            .into_iter()
            .filter(|name| {
                if has_authors_at_r && DERIVED_FROM_AUTHORS_AT_R.contains(name) {
                    return false;
                }
                !declared(name)
            })
            .collect();
        if missing.is_empty() {
            return;
        }

        let list = missing
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let plural = if missing.len() == 1 {
            "field"
        } else {
            "fields"
        };
        sink.push(Diagnostic {
            rule: "description-missing-field",
            severity: Default::default(),
            path: Default::default(),
            // Nothing marks an absence, so the finding points at the first
            // field — the head of the record the missing ones belong to.
            range: first.name_range(),
            message: ViolationData::new(
                "description-missing-field",
                format!("DESCRIPTION is missing the required {plural} {list}"),
            )
            .with_suggestion(if missing.iter().any(|name| {
                DERIVED_FROM_AUTHORS_AT_R.contains(name)
            }) {
                format!("Add the {plural} {list}, or declare `Authors@R` in place of `Author` and `Maintainer`.")
            } else {
                format!("Add the {plural} {list}.")
            }),
            fix: None,
        });
    }
}
