//! `description-encoding`: text outside R's byte-level ISO-8859 set whose
//! encoding is undeclared, or non-ASCII text in a field R requires to be ASCII.
//!
//! R expects `Package`, `Version`, `License`, and `Encoding` to contain ASCII
//! only. Elsewhere, R requires an encoding declaration when its UTF-8 bytes
//! include the 0x80..=0x9f range rejected by `tools:::.is_ISO_8859`.
//!
//! Adding `Encoding: UTF-8` is a safe fix because arity has already decoded the
//! input as UTF-8. Invalid UTF-8 never reaches lint rules. Content in an
//! ASCII-only field has no mechanical repair, so those findings carry no fix.

use rowan::{TextRange, TextSize};

use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};

pub struct DescriptionEncoding;

const ASCII_ONLY_FIELDS: [&str; 4] = ["Package", "Version", "License", "Encoding"];

const EXAMPLES: &[Example] = &[Example {
    caption: "A package containing UTF-8 text without declaring its encoding:",
    source: "Package: mypkg\nVersion: 0.1.0\nTitle: A 日本語 package\nLicense: MIT\n",
}];

impl DcfRule for DescriptionEncoding {
    fn id(&self) -> &'static str {
        "description-encoding"
    }

    fn description(&self) -> &'static str {
        "Flag text outside R's ISO-8859 byte set in a `DESCRIPTION` with no \
         `Encoding` field, and \
         non-ASCII text in fields R requires to be ASCII: `Package`, `Version`, \
         `License`, and `Encoding`.\n\nA missing declaration has a safe fix: \
         arity only lints text it has already decoded as UTF-8, so it can append \
         `Encoding: UTF-8` without guessing. Non-ASCII content in an ASCII-only \
         field has no autofix because choosing replacement text requires the \
         author."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let text = ctx.root.text().to_string();

        for name in ASCII_ONLY_FIELDS {
            let Some(field) = ctx.document.field(name) else {
                continue;
            };
            let range = field.value_range();
            let Some(range) = first_non_ascii_range(&text, range) else {
                continue;
            };
            sink.push(Diagnostic {
                rule: "description-encoding",
                severity: Default::default(),
                path: Default::default(),
                range,
                message: ViolationData::new(
                    "description-encoding",
                    format!("`{name}` must contain ASCII text only"),
                )
                .with_suggestion(format!("Replace the non-ASCII text in `{name}`.")),
                fix: None,
            });
        }

        if ctx.document.field("Encoding").is_some() {
            return;
        }
        let Some(range) = first_non_iso_8859_range(
            &text,
            TextRange::new(TextSize::from(0), ctx.root.text_range().end()),
        ) else {
            return;
        };
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let prefix = if text.is_empty() || text.ends_with('\n') || text.ends_with('\r') {
            ""
        } else {
            eol
        };
        let at = text.len();
        sink.push(Diagnostic {
            rule: "description-encoding",
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new(
                "description-encoding",
                "DESCRIPTION contains text that requires an encoding declaration",
            )
            .with_suggestion("Add `Encoding: UTF-8`."),
            fix: Some(Fix::safe(
                at,
                at,
                format!("{prefix}Encoding: UTF-8{eol}"),
                "Add `Encoding: UTF-8`",
            )),
        });
    }
}

fn first_non_ascii_range(text: &str, range: TextRange) -> Option<TextRange> {
    let start: usize = range.start().into();
    let end: usize = range.end().into();
    text.get(start..end)?
        .char_indices()
        .find_map(|(offset, ch)| {
            (!ch.is_ascii()).then(|| {
                TextRange::new(
                    TextSize::try_from(start + offset).expect("source offset fits in TextSize"),
                    TextSize::try_from(start + offset + ch.len_utf8())
                        .expect("source offset fits in TextSize"),
                )
            })
        })
}

/// Match the byte-level predicate used by `tools:::.is_ISO_8859`.
fn first_non_iso_8859_range(text: &str, range: TextRange) -> Option<TextRange> {
    let start: usize = range.start().into();
    let end: usize = range.end().into();
    text.get(start..end)?
        .char_indices()
        .find_map(|(offset, ch)| {
            ch.encode_utf8(&mut [0; 4])
                .as_bytes()
                .iter()
                .any(|byte| (0x80..=0x9f).contains(byte))
                .then(|| {
                    TextRange::new(
                        TextSize::try_from(start + offset).expect("source offset fits in TextSize"),
                        TextSize::try_from(start + offset + ch.len_utf8())
                            .expect("source offset fits in TextSize"),
                    )
                })
        })
}
