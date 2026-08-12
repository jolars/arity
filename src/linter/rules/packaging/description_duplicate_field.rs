//! `description-duplicate-field`: a `DESCRIPTION` field declared more than once.
//!
//! A repeated field is always a mistake, but it is a *silent* one: nothing
//! errors, and the file keeps whichever value the reader happens to pick. Which
//! is the problem — the readers disagree. R's `read.dcf` takes the **last**
//! occurrence; arity takes the **first**, deliberately and consistently
//! (`dcf::Document::field`). So a duplicated `Version` means R and arity are
//! reading two different packages, and every tool downstream of either inherits
//! that split.
//!
//! This rule is where the divergence becomes visible instead of silent. It does
//! not resolve it: which reading is right is a question about the DCF parser,
//! answered by its own change with the `read.dcf` oracle to prove it (see
//! `TODO.md`). Until then, saying so at the exact duplicate is worth more than
//! either reader quietly winning.
//!
//! **The span is the later occurrence.** The first is the one arity reads, so
//! the repeat is the line whose author has to decide.
//!
//! Record-blind, like every other `DESCRIPTION` reader: a stray blank line
//! splits the file into two DCF records, and a field repeated across that split
//! is still a repeated field.
//!
//! **No autofix.** Deleting a duplicate means choosing a value, and the whole
//! point of the finding is that the two readers do not agree on which one is
//! already in effect. A fix would silently pick a side.

use std::collections::HashMap;

use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};

pub struct DescriptionDuplicateField;

const EXAMPLES: &[Example] = &[Example {
    caption: "A field declared twice, which arity and `read.dcf` read differently:",
    source: "Package: mypkg\nVersion: 0.1.0\nLicense: MIT + file LICENSE\nVersion: 0.2.0\n",
}];

impl DcfRule for DescriptionDuplicateField {
    fn id(&self) -> &'static str {
        "description-duplicate-field"
    }

    fn description(&self) -> &'static str {
        "Flag a `DESCRIPTION` field declared more than once.\n\nA repeated \
         field is a silent mistake: nothing errors, and the file keeps \
         whichever value the reader picks—except the readers disagree. R's \
         `read.dcf` takes the **last** occurrence; arity takes the **first**. A \
         duplicated `Version` therefore means R and arity are describing two \
         different packages, and every tool downstream of either inherits the \
         split.\n\nThe finding is reported on the *later* occurrence, since the \
         earlier one is what arity already read. Duplicates are detected across \
         DCF records, so a stray blank line does not hide one.\n\nThere is no \
         autofix: removing a duplicate means choosing a value, and this rule \
         exists precisely because it is not settled which value is already in \
         effect."
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
                        "`{name}` is already declared on line {first}; arity reads the \
                         first occurrence and R's `read.dcf` reads the last, so the two \
                         disagree about this file"
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
