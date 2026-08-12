//! `description-version-constraint`: a dependency entry whose parenthesized
//! part is not a version constraint.
//!
//! R reads `pkg (>= 1.0.0)` — an operator and a version. Anything else in those
//! parentheses (`dplyr (1.0.0)`, `dplyr (>=)`, `dplyr (latest)`) states no
//! bound, and R's own dependency check simply does not enforce it. The
//! declaration reads as a requirement and behaves as none, which is the whole
//! defect: the package installs against a version its author believed was
//! excluded.
//!
//! The fact comes straight off the grammar
//! ([`DependencyEntry::malformed_constraint`]): the parser deliberately keeps
//! the package name when the constraint fails to parse, so a broken constraint
//! never hides a dependency from resolution — and leaves reporting it to a
//! lint, which is this one.
//!
//! Checked in all five dependency fields, `R` included: `Depends: R (>= 4.1)`
//! is the most common constraint in any `DESCRIPTION`, and a typo there is
//! worth exactly as much attention.
//!
//! **The span is the whole entry**, parentheses included. The name alone would
//! point away from the part that is wrong.
//!
//! **No autofix.** `dplyr (1.0.0)` most likely means `>=`, but it could mean
//! `==` or `>`; guessing an operator would silently invent a requirement.
//!
//! Known limit: an entry with two constraints of which only one parses
//! (`(>= 1.0, whatever)`) is not flagged, because the entry does state a bound.
//!
//! [`DependencyEntry::malformed_constraint`]: crate::dcf::DependencyEntry::malformed_constraint

use crate::dcf::deps::{dependency_entries, is_dependency_field};
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};

pub struct DescriptionVersionConstraint;

const EXAMPLES: &[Example] = &[Example {
    caption: "A version requirement R will not enforce:",
    source: "Package: mypkg\nDepends: R (4.1)\nImports: dplyr (1.0.0)\n",
}];

impl DcfRule for DescriptionVersionConstraint {
    fn id(&self) -> &'static str {
        "description-version-constraint"
    }

    fn description(&self) -> &'static str {
        "Flag a dependency entry whose parenthesized part is not a version \
         constraint.\n\nR reads `pkg (>= 1.0.0)`: an operator and a version. \
         Anything else—`dplyr (1.0.0)`, `dplyr (>=)`, `dplyr (latest)`—states no \
         bound at all, and R's dependency check enforces nothing. The line \
         reads as a requirement and behaves as none, so the package installs \
         against exactly the versions its author meant to \
         exclude.\n\nChecked in all five dependency fields, `R` included: \
         `Depends: R (>= 4.1)` is the most common constraint in any \
         `DESCRIPTION`.\n\nThere is no autofix. `dplyr (1.0.0)` most likely \
         means `>=`, but it could mean `==` or `>`, and guessing would invent a \
         requirement the author never wrote."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        for field in ctx.document.fields() {
            if !is_dependency_field(&field.name()) {
                continue;
            }
            for entry in dependency_entries(&field) {
                if !entry.malformed_constraint() {
                    continue;
                }
                let name = &entry.name;
                sink.push(Diagnostic {
                    rule: "description-version-constraint",
                    severity: Default::default(),
                    path: Default::default(),
                    range: entry.range,
                    message: ViolationData::new(
                        "description-version-constraint",
                        format!(
                            "the version constraint on `{name}` states no bound, so R \
                             enforces nothing here"
                        ),
                    )
                    // No package name in the suggestion: `R (>= 1.0.0)` would
                    // be nonsense advice for the one entry that is not a
                    // package.
                    .with_suggestion(
                        "Write a comparison operator and a version, as in `(>= 1.0.0)`."
                            .to_string(),
                    ),
                    fix: None,
                });
            }
        }
    }
}
