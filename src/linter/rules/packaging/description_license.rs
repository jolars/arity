//! Validate license components against R's bundled license database.
//!
//! Only exact historical spellings from R's mapping table can be fixed, and
//! their replacements must still validate against the current database. A fix
//! replaces one component on one value line, preserving surrounding trivia.
//! License file contents and the package's legal choice remain the author's.

use std::cmp::Ordering;
use std::sync::LazyLock;

use rowan::{TextRange, TextSize};

use super::scalar_field::{escape, folded};
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};

pub struct DescriptionLicense;

/// Refreshed by `scripts/dump_license_db.R`, never by production code.
const DATABASE: &str = include_str!("licenses.tsv");
const ALIASES: &str = include_str!("license_aliases.tsv");

struct License {
    name: &'static str,
    abbreviation: &'static str,
    version: &'static str,
    short: &'static str,
}

static LICENSES: LazyLock<Vec<License>> = LazyLock::new(|| {
    DATABASE
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| {
            let mut columns = line.split('\t');
            License {
                name: columns.next().unwrap(),
                abbreviation: columns.next().unwrap_or_default(),
                version: columns.next().unwrap_or_default(),
                short: columns.next().unwrap_or_default(),
            }
        })
        .collect()
});

impl DcfRule for DescriptionLicense {
    fn id(&self) -> &'static str {
        "description-license"
    }

    fn description(&self) -> &'static str {
        "Validate `License` against R's bundled license database. Accept license \
         names and abbreviations, supported versions and version restrictions, \
         `|` alternatives, `+ file LICENSE` or `+ file LICENCE`, standalone file \
         pointers, and `Unlimited`. Version restrictions must select at least \
         one version in the database. Missing or empty fields are left to \
         `description-missing-field`.\n\nExact legacy spellings from R's \
         standardization table, such as `GPL 2.0`, have a safe fix when their \
         replacement is valid. Other unknown or malformed components have no \
         autofix because choosing a license requires the author. This rule \
         checks the specification, not license file existence, file contents, \
         or whether additional terms are legally permitted."
    }

    fn examples(&self) -> &'static [Example] {
        &[
            Example {
                caption: "An exact legacy spelling with a standard replacement:",
                source: "Package: mypkg\nLicense: GPL 2.0\n",
            },
            Example {
                caption: "A version restriction that selects no known license:",
                source: "Package: mypkg\nLicense: GPL (>= 99)\n",
            },
        ]
    }

    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(field) = ctx.document.field("License") else {
            return;
        };
        let Some(value) = folded(&field) else {
            return;
        };
        let mut offset = 0;
        for raw in value.text.split('|') {
            let component = raw.trim();
            let start = offset + raw.len() - raw.trim_start().len();
            let end = start + component.len();
            offset += raw.len() + 1;
            if canonical_component(component) {
                continue;
            }
            // An empty alternative points at its separator rather than an
            // invisible zero-width span at the end of the field.
            let (start, end) = if component.is_empty() {
                let at = start.min(value.text.len() - 1);
                (at, at + 1)
            } else {
                (start, end)
            };
            let range = value.map(TextRange::new(
                TextSize::from(start as u32),
                TextSize::from(end as u32),
            ));
            let replacement = legacy_replacement(component);
            let (message, fix) = if let Some(replacement) = replacement {
                (
                    ViolationData::new(
                        self.id(),
                        format!(
                            "`{}` is a nonstandard license specification",
                            escape(component)
                        ),
                    )
                    .with_suggestion(format!("Use `{replacement}`.")),
                    Some(Fix::safe(
                        range.start().into(),
                        range.end().into(),
                        replacement,
                        format!("Replace with `{replacement}`"),
                    )),
                )
            } else {
                (
                    ViolationData::new(
                        self.id(),
                        if component.is_empty() {
                            "`License` contains an empty alternative".to_string()
                        } else {
                            format!("`{}` is not a valid R license specification", escape(component))
                        },
                    )
                    .with_suggestion("Use a license specification from R's license database or a `file LICENSE` pointer."),
                    None,
                )
            };
            sink.push(Diagnostic {
                rule: self.id(),
                severity: Default::default(),
                path: Default::default(),
                range,
                message,
                fix,
            });
        }
    }
}

fn legacy_replacement(component: &str) -> Option<&'static str> {
    let (_, replacement) = ALIASES
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .find(|(source, _)| *source == component)?;
    // Some historical mappings now produce unknown names. Do not turn one
    // invalid specification into another, or change a license file pointer.
    (!replacement.contains("file ")
        && replacement
            .split('|')
            .all(|c| canonical_component(c.trim())))
    .then_some(replacement)
}

fn canonical_component(component: &str) -> bool {
    if matches!(component, "Unlimited" | "file LICENSE" | "file LICENCE") {
        return true;
    }
    let base = if let Some((base, extension)) = component.split_once('+') {
        if !matches!(extension.trim(), "file LICENSE" | "file LICENCE") {
            return false;
        }
        base.trim_end()
    } else {
        component
    };
    LICENSES.iter().any(|license| license.matches(base))
}

impl License {
    fn matches(&self, text: &str) -> bool {
        if !self.short.is_empty() && text == self.short {
            return true;
        }
        if self.version.is_empty() {
            return text == self.name
                || (!self.abbreviation.is_empty() && text == self.abbreviation);
        }
        if !self.abbreviation.is_empty()
            && text
                .strip_prefix(self.abbreviation)
                .and_then(|rest| rest.strip_prefix('-'))
                == Some(self.version)
        {
            return true;
        }
        if let Some(rest) = text.strip_prefix(self.name) {
            let version = rest.trim_start();
            if version.len() < rest.len()
                && version
                    .strip_prefix("Version")
                    .or_else(|| version.strip_prefix("version"))
                    .filter(|suffix| suffix.len() > suffix.trim_start().len())
                    .map_or(version, str::trim_start)
                    == self.version
            {
                return true;
            }
        }
        [self.name, self.abbreviation].into_iter().any(|name| {
            !name.is_empty()
                && text.strip_prefix(name).is_some_and(|rest| {
                    let rest = rest.trim_start();
                    rest.is_empty() || restrictions_match(rest, self.version)
                })
        })
    }
}

fn restrictions_match(mut text: &str, version: &str) -> bool {
    let version = numeric_version(version).expect("bundled license versions are numeric");
    let mut matches = false;
    while let Some(rest) = text.strip_prefix('(') {
        let Some((constraints, rest)) = rest.split_once(')') else {
            return false;
        };
        let mut accepted = true;
        for constraint in constraints.split(',') {
            let constraint = constraint.trim();
            let Some(operator) = ["<=", ">=", "==", "!=", "<", ">"]
                .into_iter()
                .find(|op| constraint.starts_with(op))
            else {
                return false;
            };
            let Some(target) = numeric_version(constraint[operator.len()..].trim()) else {
                return false;
            };
            let order = compare_versions(&version, &target);
            accepted &= match operator {
                "<" => order.is_lt(),
                "<=" => order.is_le(),
                ">" => order.is_gt(),
                ">=" => order.is_ge(),
                "==" => order.is_eq(),
                "!=" => !order.is_eq(),
                _ => unreachable!(),
            };
        }
        // R accepts adjacent restriction groups and uses the final group.
        matches = accepted;
        text = rest;
    }
    text.is_empty() && matches
}

fn numeric_version(version: &str) -> Option<Vec<u32>> {
    version
        .split(['.', '-'])
        .map(|part| {
            (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
                .then(|| part.parse::<i32>().ok().map(|n| n as u32))
                .flatten()
        })
        .collect()
}

fn compare_versions(a: &[u32], b: &[u32]) -> Ordering {
    (0..a.len().max(b.len()))
        .map(|i| a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0)))
        .find(|order| !order.is_eq())
        .unwrap_or(Ordering::Equal)
}
