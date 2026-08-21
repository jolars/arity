//! `description-unknown-field`: a likely misspelling of a standard
//! `DESCRIPTION` field.
//!
//! DCF permits arbitrary field names, and package tooling makes deliberate use
//! of that freedom through names such as `Config/Needs/website`. This rule is
//! therefore a near-miss check, not a whitelist: it reports only names one edit
//! from a standard field, plus whitespace between a standard name and its
//! colon. R includes that whitespace in the field name and silently ignores the
//! metadata under the intended spelling.
//!
//! **No autofix.** The suggested spelling is strong evidence, but changing a
//! field name changes the metadata R reads. The author should confirm that
//! intent explicitly.

use rowan::ast::AstNode;

use crate::dcf::{Field, SyntaxKind};
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};

pub struct DescriptionUnknownField;

const RULE: &str = "description-unknown-field";

const EXAMPLES: &[Example] = &[Example {
    caption: "A misspelled field that R silently ignores:",
    source: "Package: mypkg\nVersion: 0.1.0\nSuggest: testthat\n",
}];

impl DcfRule for DescriptionUnknownField {
    fn id(&self) -> &'static str {
        RULE
    }

    fn description(&self) -> &'static str {
        "Flag a likely misspelling of a standard `DESCRIPTION` field.\n\nThis \
         is deliberately a near-miss check rather than a whitelist: arbitrary \
         fields, including `Config/*`, are legal. A name is reported only when \
         it is one edit from a standard field, or when whitespace separates an \
         otherwise standard name from its colon. R treats that whitespace as \
         part of the name, so `Package : mypkg` declares `Package ` rather than \
         `Package` and is silently ignored.\n\nThere is no autofix: renaming a field \
         changes the metadata R reads, so the author should confirm the intended \
         spelling."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::FIELD]
    }

    fn check(
        &self,
        el: &crate::dcf::SyntaxElement,
        _ctx: &DcfRuleContext<'_>,
        sink: &mut Vec<Diagnostic>,
    ) {
        let Some(field) = el.as_node().and_then(|node| Field::cast(node.clone())) else {
            return;
        };
        let name = field.name();
        if name.is_empty() {
            return;
        }

        let whitespace = field
            .syntax()
            .children_with_tokens()
            .filter_map(|child| child.into_token())
            .find(|token| token.kind() == SyntaxKind::WHITESPACE);
        let standard = crate::formatter::description::field_names();
        let exact = standard.clone().any(|candidate| candidate == name);
        let suggestion = if exact {
            if whitespace.is_some() {
                Some(name.as_str())
            } else {
                None
            }
        } else {
            let mut matches = standard.filter(|candidate| one_edit_apart(&name, candidate));
            let candidate = matches.next();
            (matches.next().is_none()).then_some(candidate).flatten()
        };
        let Some(suggestion) = suggestion else {
            return;
        };

        let range = whitespace.as_ref().map_or_else(
            || field.name_range(),
            |space| rowan::TextRange::new(field.name_range().start(), space.text_range().end()),
        );
        let written = whitespace.map_or_else(
            || name.to_string(),
            |space| format!("{name}{}", space.text()),
        );
        sink.push(Diagnostic {
            rule: RULE,
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new(
                RULE,
                format!(
                    "`{written}` is not a standard DESCRIPTION field; did you mean `{suggestion}`?"
                ),
            )
            .with_suggestion(format!("Rename the field to `{suggestion}`.")),
            fix: None,
        });
    }
}

/// Whether two names have Levenshtein distance exactly one.
fn one_edit_apart(left: &str, right: &str) -> bool {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    match left.len().cmp(&right.len()) {
        std::cmp::Ordering::Equal => left.iter().zip(&right).filter(|(a, b)| a != b).count() == 1,
        std::cmp::Ordering::Less if left.len() + 1 == right.len() => {
            one_insertion_apart(&left, &right)
        }
        std::cmp::Ordering::Greater if right.len() + 1 == left.len() => {
            one_insertion_apart(&right, &left)
        }
        _ => false,
    }
}

/// Whether inserting one character into `shorter` produces `longer`.
fn one_insertion_apart(shorter: &[char], longer: &[char]) -> bool {
    let mut skipped = false;
    let mut short = 0;
    for long in longer {
        if short < shorter.len() && shorter[short] == *long {
            short += 1;
        } else if skipped {
            return false;
        } else {
            skipped = true;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_distance_is_exactly_one() {
        assert!(one_edit_apart("Suggest", "Suggests"));
        assert!(one_edit_apart("Depnds", "Depends"));
        assert!(one_edit_apart("package", "Package"));
        assert!(!one_edit_apart("Depends", "Depends"));
        assert!(!one_edit_apart("Custom", "Package"));
    }
}
