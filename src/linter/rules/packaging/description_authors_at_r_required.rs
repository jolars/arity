//! `description-authors-at-r-required`: CRAN requires an `Authors@R` field,
//! even when `Author` and `Maintainer` already satisfy R's requirements.
//!
//! This policy is opt-in because packages need not target CRAN. Presence is
//! enough here; `description-authors-at-r` checks the field's contents.
//! There is no autofix because contributor names, roles, and contact details
//! require the author's input.

use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};

pub struct DescriptionAuthorsAtRRequired;

const RULE: &str = "description-authors-at-r-required";

const EXAMPLES: &[Example] = &[Example {
    caption: "Legacy author fields satisfy R, but CRAN also requires `Authors@R`:",
    source: "Package: mypkg\nVersion: 0.1.0\nTitle: A Test Package\n\
             Description: Demonstrate package metadata checks.\nLicense: GPL-3\n\
             Author: Jane Doe [aut, cre]\nMaintainer: Jane Doe <jane@example.com>\n",
}];

impl DcfRule for DescriptionAuthorsAtRRequired {
    fn id(&self) -> &'static str {
        RULE
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn description(&self) -> &'static str {
        "Flag a `DESCRIPTION` without an `Authors@R` field.\n\n\
         [Writing R Extensions](https://cran.r-project.org/doc/manuals/r-devel/R-exts.html#The-DESCRIPTION-file) \
         requires this field for CRAN submissions, even when valid `Author` \
         and `Maintainer` fields are present. This rule is **off by default** \
         because packages need not target CRAN. Enable it with \
         `--select description-authors-at-r-required`.\n\n\
         `description-missing-field` checks R's required metadata independently. \
         A present `Authors@R` field, even an empty or invalid one, is left to \
         `description-authors-at-r` to validate. Files with no fields are left \
         alone.\n\nThere is no autofix: contributor names, roles, and contact \
         details require the author's input."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        if ctx.document.field("Authors@R").is_some() {
            return;
        }
        let Some(first) = ctx.document.fields().next() else {
            return;
        };

        sink.push(Diagnostic {
            rule: RULE,
            severity: Default::default(),
            path: Default::default(),
            // Anchor the absent field to the record's first field, as the
            // required-metadata rule does.
            range: first.name_range(),
            message: ViolationData::new(
                RULE,
                "CRAN submissions require an `Authors@R` field".to_string(),
            )
            .with_suggestion(
                "Add `Authors@R` with the package's contributors and their roles.".to_string(),
            ),
            fix: None,
        });
    }
}
