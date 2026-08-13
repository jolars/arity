//! What a `DESCRIPTION` *means*, as a value that formatting must not change.
//!
//! Formatting a `DESCRIPTION` reorders fields, sorts dependency entries,
//! re-wraps prose and reformats embedded R, so no byte comparison can express
//! "the output says the same thing". This module defines the equivalence
//! relation that can: a per-field-class projection, compared before and after.
//!
//! The class table is deliberately **re-derived here** rather than imported from
//! the formatter. A gate that asks the implementation what it meant to do is not
//! a gate. Drift is safe in the direction that matters: a field the formatter
//! newly reflows but this table still calls prose fails loudly, because prose
//! comparison is order-sensitive.
//!
//! Consumed by `#[path]` include, so it never becomes crate API.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use arity_formatter::formatter::{FormatStyle, format_with_style};
use arity_parser::dcf::{self, SyntaxKind};
use rowan::ast::AstNode;

/// One entry per record, each mapping a field's **untrimmed** name — everything
/// between the field's start and its colon — to its values in document order.
///
/// Record order is significant; field order within a record is not, because
/// reordering fields is the point. Values are a `Vec` so a duplicated name does
/// not silently collapse to one entry: the formatter refuses such files, and a
/// relation that hid the duplication could not prove it.
///
/// Untrimmed names are load-bearing: `Package : p` declares a field named
/// `"Package "` to `read.dcf`, so normalizing the header would *rename* it.
/// Keeping the raw bytes here makes that unfixable by accident.
pub type Meaning = Vec<BTreeMap<String, Vec<FieldMeaning>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldMeaning {
    /// Re-wrappable prose: only the word sequence is meaningful.
    Prose(String),
    /// A comma list whose order the formatter is free to change.
    DepSet(BTreeSet<(String, String)>),
    /// A list whose order is semantic (`Collate` is execution order).
    Ordered(Vec<String>),
    /// R code: equal iff both sides format to the same R, or (when the source
    /// does not parse) iff the text is untouched.
    RCode(RCodeMeaning),
    /// Anything else: the folded value must be byte-identical.
    Verbatim(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RCodeMeaning {
    Formatted(String),
    Unparseable(String),
}

const COMMA_LIST: &[&str] = &[
    "Depends",
    "Imports",
    "Suggests",
    "Enhances",
    "LinkingTo",
    "VignetteBuilder",
    "RdMacros",
    "Remotes",
];

const ORDERED: &[&str] = &["Collate", "Collate.windows", "Collate.unix"];

const R_CODE: &[&str] = &["Authors@R", "Roxygen"];

const PROSE: &[&str] = &[
    "Title",
    "Description",
    "License",
    "URL",
    "BugReports",
    "SystemRequirements",
    "Author",
    "Maintainer",
    "Type",
    "Package",
    "Version",
    "Date",
    "Encoding",
    "Language",
    "Priority",
    "OS_type",
    "RoxygenNote",
    "Additional_repositories",
    "LazyData",
    "LazyLoad",
    "KeepSource",
    "ByteCompile",
    "ZipData",
    "Biarch",
    "BuildVignettes",
    "NeedsCompilation",
    "License_is_FOSS",
    "License_restricts_use",
];

pub fn meaning(text: &str) -> Meaning {
    let parsed = dcf::parse(text);
    parsed
        .document()
        .records()
        .map(|record| {
            let mut fields: BTreeMap<String, Vec<FieldMeaning>> = BTreeMap::new();
            for field in record.fields() {
                fields
                    .entry(raw_name(&field))
                    .or_default()
                    .push(field_meaning(&field));
            }
            fields
        })
        .collect()
}

/// Every column-zero comment's text, as a sorted multiset. Formatting may move a
/// comment but must never drop or invent one — the whole differentiator over
/// `desc`, which drops them all.
pub fn comments(text: &str) -> Vec<String> {
    let parsed = dcf::parse(text);
    let mut out: Vec<String> = parsed
        .cst
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::COMMENT_LINE)
        .filter_map(|node| {
            node.children_with_tokens()
                .filter_map(|el| el.into_token())
                .find(|tok| tok.kind() == SyntaxKind::COMMENT)
                .map(|tok| tok.text().trim_end().to_string())
        })
        .collect();
    out.sort();
    out
}

/// Everything between the field's start and its colon, verbatim.
fn raw_name(field: &dcf::Field) -> String {
    let mut out = String::new();
    for token in field
        .syntax()
        .children_with_tokens()
        .filter_map(|el| el.into_token())
    {
        if token.kind() == SyntaxKind::COLON {
            break;
        }
        out.push_str(token.text());
    }
    out
}

fn field_meaning(field: &dcf::Field) -> FieldMeaning {
    let name = field.name();
    let folded = field.folded_value();

    if COMMA_LIST.contains(&name.as_str()) {
        return FieldMeaning::DepSet(
            dcf::dependency_entries(field)
                .into_iter()
                .map(|entry| {
                    // All whitespace, not just runs: the formatter normalizes
                    // `(>=1.0)` to `(>= 1.0)`, and the space between an operator
                    // and its version carries no meaning.
                    let constraint = entry
                        .constraint_text
                        .map(|(text, _)| strip_ws(&text))
                        .unwrap_or_default();
                    (entry.name.to_string(), constraint)
                })
                .collect(),
        );
    }

    if ORDERED.contains(&name.as_str()) {
        return FieldMeaning::Ordered(
            folded
                .split_whitespace()
                .map(|token| token.trim_matches(['\'', '"']).to_string())
                .collect(),
        );
    }

    if R_CODE.contains(&name.as_str()) {
        let source = strip_leading_newline(&folded);
        return FieldMeaning::RCode(match format_with_style(source, FormatStyle::default()) {
            Ok(formatted) => RCodeMeaning::Formatted(formatted),
            Err(_) => RCodeMeaning::Unparseable(source.to_string()),
        });
    }

    if PROSE.contains(&name.as_str()) {
        return FieldMeaning::Prose(collapse_ws(strip_leading_newline(&folded)));
    }

    FieldMeaning::Verbatim(folded)
}

/// `read.dcf` drops the empty leading segment a field with an empty own line
/// folds to; arity keeps it (a recorded divergence, normalized the same way in
/// `tests/dcf_oracle.rs`). The canonical style puts *every* dependency field
/// into that shape, so comparing without this would report a meaning change on
/// every formatted file.
fn strip_leading_newline(value: &str) -> &str {
    value.strip_prefix('\n').unwrap_or(value)
}

fn collapse_ws(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_ws(value: &str) -> String {
    value.chars().filter(|ch| !ch.is_whitespace()).collect()
}

/// A human-readable diff of two meanings, for assertion messages.
pub fn describe_difference(before: &Meaning, after: &Meaning) -> String {
    if before.len() != after.len() {
        return format!("record count {} -> {}", before.len(), after.len());
    }
    for (index, (lhs, rhs)) in before.iter().zip(after).enumerate() {
        for (name, value) in lhs {
            match rhs.get(name) {
                None => return format!("record {index}: field {name:?} disappeared"),
                Some(other) if other != value => {
                    return format!(
                        "record {index}: field {name:?}\n  before: {value:?}\n  after:  {other:?}"
                    );
                }
                Some(_) => {}
            }
        }
        for name in rhs.keys() {
            if !lhs.contains_key(name) {
                return format!("record {index}: field {name:?} appeared");
            }
        }
    }
    "meanings are equal; the assertion should not have fired".to_string()
}
