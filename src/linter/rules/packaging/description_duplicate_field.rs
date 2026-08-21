//! `description-duplicate-field`: a `DESCRIPTION` field declared more than once.
//!
//! A repeated field is always a mistake, but it is a *silent* one: nothing
//! errors, and the last value quietly replaces the earlier value. Both arity
//! and R's `read.dcf` apply that rule.
//!
//! **The span is the later occurrence.** It is both the repeat and the value
//! that takes effect.
//!
//! Record-blind, like every other `DESCRIPTION` reader: a stray blank line
//! splits the file into two DCF records, and a field repeated across that split
//! is still a repeated field.
//!
//! **No autofix.** Deleting a duplicate means choosing a value. A fix would
//! silently make that choice for the author.

use std::collections::HashMap;

use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};

pub struct DescriptionDuplicateField;

const EXAMPLES: &[Example] = &[Example {
    caption: "A field declared twice, with the later value silently taking effect:",
    source: "Package: mypkg\nVersion: 0.1.0\nLicense: MIT + file LICENSE\nVersion: 0.2.0\n",
}];

impl DcfRule for DescriptionDuplicateField {
    fn id(&self) -> &'static str {
        "description-duplicate-field"
    }

    fn description(&self) -> &'static str {
        "Flag a `DESCRIPTION` field declared more than once.\n\nA repeated \
         field is a silent mistake: nothing errors, and the last value quietly \
         replaces the earlier value. Both arity and R's `read.dcf` apply that \
         rule.\n\nThe finding is reported on the *later* occurrence, which is \
         both the repeat and the value that takes effect. Duplicates are \
         detected across DCF records, so a stray blank line does not hide \
         one.\n\nThere is no autofix: removing a duplicate means choosing a \
         value, and a fix would silently make that choice for the author."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let text = ctx.root.text().to_string();
        // Field name -> the line its first occurrence sits on, which is what
        // the message points the author back at.
        let mut first_line: HashMap<String, u32> = HashMap::new();

        for field in ctx.document.fields() {
            let name = field.name();
            // A nameless field (`: value`) is the parser's `empty field name`
            // diagnostic, not a duplicate of anything.
            if name.is_empty() {
                continue;
            }
            let Some(&first) = first_line.get(name.as_str()) else {
                first_line.insert(name.to_string(), line_of(&text, field.name_range().start()));
                continue;
            };
            sink.push(Diagnostic {
                rule: "description-duplicate-field",
                severity: Default::default(),
                path: Default::default(),
                range: field.name_range(),
                message: ViolationData::new(
                    "description-duplicate-field",
                    format!(
                        "`{name}` is already declared on line {first}; this later \
                         occurrence silently replaces its value"
                    ),
                )
                .with_suggestion(format!("Keep one `{name}` field and delete the other.")),
                fix: None,
            });
        }
    }
}

/// The 1-based line `offset` falls on.
fn line_of(text: &str, offset: rowan::TextSize) -> u32 {
    let at: usize = offset.into();
    1 + text[..at.min(text.len())].matches('\n').count() as u32
}
