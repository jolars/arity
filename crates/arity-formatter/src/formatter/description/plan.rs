//! CST to [`Plan`]: classify every field, re-attach every comment, and decline
//! anything that cannot be restyled safely.
//!
//! The plan holds **no** rowan nodes. That makes rendering a pure function of an
//! owned value, and it means every decision about what the output will contain
//! is made here, in one place that can be read end to end.

use rowan::ast::AstNode;

use super::driver::DeclineReason;
use super::order::{collate, compare_fields};
use crate::dcf::{self, SyntaxKind, SyntaxNode};

/// Fields rendered as a sorted one-per-line comma list.
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

/// Fields rendered as a quoted one-per-line list whose **order is preserved**.
const ORDERED_LIST: &[&str] = &["Collate", "Collate.windows", "Collate.unix"];

/// Fields whose value is R code.
const R_CODE: &[&str] = &["Authors@R", "Roxygen"];

/// Fields safe to re-wrap as prose.
///
/// Closed by design. Everything absent from this table is [`FieldBody::Opaque`],
/// whose line structure is preserved exactly — so an unrecognized field's value
/// is byte-identical to `read.dcf` before and after. That default is what makes
/// formatting `DESCRIPTION` by default defensible.
const WRAPPED: &[&str] = &[
    "Type",
    "Package",
    "Title",
    "Version",
    "Date",
    "Author",
    "Maintainer",
    "Description",
    "License",
    "URL",
    "BugReports",
    "Priority",
    "Encoding",
    "Language",
    "OS_type",
    "SystemRequirements",
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Plan {
    /// Comments in a file with no fields at all to anchor them to.
    pub(super) orphan_comments: Vec<String>,
    pub(super) record: Option<RecordPlan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordPlan {
    /// In canonical order.
    pub(super) fields: Vec<FieldPlan>,
    /// A comment run at the end of the record, which anchored nothing in the
    /// input and must go on anchoring nothing.
    pub(super) trailing_comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FieldPlan {
    pub(super) name: String,
    pub(super) leading_comments: Vec<String>,
    pub(super) body: FieldBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FieldBody {
    /// Whitespace-collapsed prose, re-wrapped at the line width.
    Wrapped(String),
    /// Rendered entries, sorted.
    CommaList(Vec<String>),
    /// Quote-stripped tokens, order preserved.
    OrderedList(Vec<String>),
    /// R source, formatted by the R formatter.
    RCode(String),
    /// One trimmed string per source value line; line structure preserved.
    Opaque(Vec<String>),
    /// The exact source bytes of the field's value region.
    Verbatim(String),
}

pub(super) fn build(document: &dcf::Document) -> Result<Plan, DeclineReason> {
    let mut pending = Vec::new();
    let mut records = Vec::new();

    for child in document.syntax().children() {
        match child.kind() {
            SyntaxKind::COMMENT_LINE => pending.push(comment_text(&child)),
            SyntaxKind::BLANK_LINE => {}
            SyntaxKind::RECORD => records.push(child),
            SyntaxKind::MALFORMED_LINE => return Err(DeclineReason::MalformedLine),
            _ => return Err(DeclineReason::UnsupportedStructure),
        }
    }

    if records.len() > 1 {
        return Err(DeclineReason::MultipleRecords {
            count: records.len(),
        });
    }

    let Some(record) = records.pop() else {
        return Ok(Plan {
            orphan_comments: pending,
            record: None,
        });
    };

    let mut fields: Vec<FieldPlan> = Vec::new();
    for child in record.children() {
        match child.kind() {
            SyntaxKind::FIELD => {
                let field = dcf::Field::cast(child).expect("kind checked");
                let name = field.name().to_string();
                if name.is_empty() {
                    return Err(DeclineReason::UnsupportedStructure);
                }
                if has_whitespace_before_colon(&field) {
                    return Err(DeclineReason::NameWhitespace { name });
                }
                if fields.iter().any(|existing| existing.name == name) {
                    return Err(DeclineReason::DuplicateField { name });
                }
                let (body, trailing) = field_body(&field, &name)?;
                fields.push(FieldPlan {
                    name,
                    leading_comments: std::mem::take(&mut pending),
                    body,
                });
                pending = trailing;
            }
            SyntaxKind::COMMENT_LINE => pending.push(comment_text(&child)),
            SyntaxKind::BLANK_LINE => {}
            SyntaxKind::MALFORMED_LINE => return Err(DeclineReason::MalformedLine),
            _ => return Err(DeclineReason::UnsupportedStructure),
        }
    }

    check_encoding(&fields)?;
    fields.sort_by(|left, right| compare_fields(&left.name, &right.name));

    Ok(Plan {
        orphan_comments: Vec::new(),
        record: Some(RecordPlan {
            fields,
            trailing_comments: pending,
        }),
    })
}

/// `Encoding` names how R must read the bytes. We only ever emit UTF-8, and
/// re-wrapping text we cannot decode is guesswork, so anything else is declined.
fn check_encoding(fields: &[FieldPlan]) -> Result<(), DeclineReason> {
    let Some(field) = fields.iter().find(|field| field.name == "Encoding") else {
        return Ok(());
    };
    // Whichever body the field landed in, it still *declares* an encoding, and
    // the guard is about the other fields: re-wrapping prose we cannot decode is
    // the guesswork being refused. Waving it through for any shape but `Wrapped`
    // would disarm the check exactly when the field is unusual.
    let declared = match &field.body {
        FieldBody::Wrapped(value) => collapse_whitespace(value),
        // Frozen by an interior comment. The comment lines are not the value.
        FieldBody::Verbatim(raw) => collapse_whitespace(&strip_comment_lines(raw)),
        FieldBody::Opaque(lines) => collapse_whitespace(&lines.join(" ")),
        // Unreachable while `Encoding` is in `WRAPPED`, but a class table is a
        // thing that changes; decline rather than assume.
        FieldBody::CommaList(_) | FieldBody::OrderedList(_) | FieldBody::RCode(_) => {
            return Err(DeclineReason::UnsupportedStructure);
        }
    };
    if declared.is_empty()
        || declared.eq_ignore_ascii_case("UTF-8")
        || declared.eq_ignore_ascii_case("ASCII")
    {
        return Ok(());
    }
    Err(DeclineReason::Encoding { declared })
}

/// `Package : p` declares a field named `"Package "` to `read.dcf`. arity trims
/// the name (a recorded divergence), so re-emitting `Package:` would *rename*
/// the field rather than reformat it.
fn has_whitespace_before_colon(field: &dcf::Field) -> bool {
    field
        .syntax()
        .children_with_tokens()
        .filter_map(|element| element.into_token())
        .take_while(|token| token.kind() != SyntaxKind::COLON)
        .any(|token| token.kind() == SyntaxKind::WHITESPACE)
}

/// The field's body, plus the trailing comment run that detaches from it.
///
/// A comment run at the end of a field anchored the *next* field in the input
/// (see `next_meaningful_dcf_sibling` in the linter's suppression map), so it
/// must travel forward rather than stay with the field it is nested in.
fn field_body(field: &dcf::Field, name: &str) -> Result<(FieldBody, Vec<String>), DeclineReason> {
    let lines: Vec<SyntaxNode> = field.syntax().children().collect();
    let split = lines
        .iter()
        .rposition(|line| line.kind() == SyntaxKind::VALUE_LINE)
        .map_or(0, |index| index + 1);
    let (body_lines, trailing_lines) = lines.split_at(split);

    let mut trailing = Vec::new();
    for line in trailing_lines {
        match line.kind() {
            SyntaxKind::COMMENT_LINE => trailing.push(comment_text(line)),
            _ => return Err(DeclineReason::UnsupportedStructure),
        }
    }

    // A comment *between* value lines anchors this field and has no meaningful
    // position once the value is reflowed, so the value is left exactly as
    // written. Only the field's position may change.
    if body_lines
        .iter()
        .any(|line| line.kind() == SyntaxKind::COMMENT_LINE)
    {
        let verbatim = body_lines
            .iter()
            .map(|line| line.text().to_string())
            .collect::<String>();
        return Ok((FieldBody::Verbatim(verbatim), trailing));
    }

    Ok((classify(field, name), trailing))
}

fn classify(field: &dcf::Field, name: &str) -> FieldBody {
    let folded = field.folded_value();
    let value = folded.strip_prefix('\n').unwrap_or(&folded);

    if COMMA_LIST.contains(&name) {
        if let Some(entries) = comma_list_entries(field, value, name) {
            return FieldBody::CommaList(entries);
        }
    } else if ORDERED_LIST.contains(&name) {
        if let Some(tokens) = ordered_list_tokens(value) {
            return FieldBody::OrderedList(tokens);
        }
    } else if R_CODE.contains(&name) {
        return FieldBody::RCode(value.to_string());
    } else if WRAPPED.contains(&name) {
        return FieldBody::Wrapped(collapse_whitespace(value));
    }

    FieldBody::Opaque(
        field
            .value_lines()
            .map(|line| line.trimmed_text())
            .collect(),
    )
}

/// Rendered dependency entries in **source** order, or `None` when rendering
/// them would not reproduce the value's non-whitespace bytes.
///
/// That comparison is the whole safety argument for this class: it rejects a
/// value `dependency_entries` cannot round-trip — a doubled comma whose empty
/// entry it drops, an unclosed parenthesis it would let run to the end of the
/// entry, a value that is not a comma list at all — and sends it to `Opaque`,
/// where nothing is rewritten.
fn comma_list_entries(field: &dcf::Field, value: &str, name: &str) -> Option<Vec<String>> {
    let rendered: Vec<String> = dcf::dependency_entries(field)
        .into_iter()
        .map(render_entry)
        .collect();
    if rendered.is_empty() {
        return value.trim().is_empty().then(Vec::new);
    }

    if strip_whitespace(&rendered.join(",")) != strip_whitespace(value) {
        return None;
    }

    let mut sorted = rendered;
    sorted.sort_by(|left, right| sort_key(left, right, name));
    Some(sorted)
}

/// `R` is not a package; it is the interpreter's own floor, and every
/// `DESCRIPTION` in the wild writes it first. `desc` sorts it alphabetically
/// among the packages, which reads as a mistake to anyone who writes R.
fn sort_key(left: &str, right: &str, field: &str) -> std::cmp::Ordering {
    if field == "Depends" {
        let left_is_r = is_r_entry(left);
        let right_is_r = is_r_entry(right);
        if left_is_r != right_is_r {
            return right_is_r.cmp(&left_is_r);
        }
    }
    collate(left, right)
}

fn is_r_entry(entry: &str) -> bool {
    entry
        .split_once(' ')
        .map_or(entry, |(name, _)| name)
        .eq_ignore_ascii_case("R")
}

fn render_entry(entry: dcf::DependencyEntry) -> String {
    let Some((text, _)) = entry.constraint_text else {
        return entry.name.to_string();
    };
    let rebuilt = entry
        .constraints
        .iter()
        .map(|constraint| format!("{} {}", operator(constraint.op), constraint.version))
        .collect::<Vec<_>>()
        .join(", ");
    // Prefer the parsed form, which normalizes `(>=1.0)` to `(>= 1.0)`, but only
    // when it accounts for every non-whitespace byte the author wrote.
    let constraint = if strip_whitespace(&rebuilt) == strip_whitespace(&text) {
        rebuilt
    } else {
        collapse_whitespace(&text)
    };
    format!("{} ({constraint})", entry.name)
}

fn operator(op: dcf::VersionOp) -> &'static str {
    match op {
        dcf::VersionOp::Ge => ">=",
        dcf::VersionOp::Gt => ">",
        dcf::VersionOp::Le => "<=",
        dcf::VersionOp::Lt => "<",
        dcf::VersionOp::Eq => "==",
        dcf::VersionOp::Ne => "!=",
    }
}

/// Whitespace-separated tokens with surrounding quotes stripped, or `None` when
/// a token could not be re-quoted with `'`.
fn ordered_list_tokens(value: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut chars = value.chars().peekable();
    while let Some(&ch) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        let token = if ch == '\'' || ch == '"' {
            chars.next();
            let mut inner = String::new();
            loop {
                match chars.next() {
                    Some(next) if next == ch => break,
                    Some(next) => inner.push(next),
                    // Unterminated quote: not something to guess at.
                    None => return None,
                }
            }
            inner
        } else {
            let mut bare = String::new();
            while let Some(&next) = chars.peek() {
                if next.is_whitespace() {
                    break;
                }
                bare.push(next);
                chars.next();
            }
            bare
        };
        if token.contains('\'') {
            return None;
        }
        tokens.push(token);
    }
    Some(tokens)
}

fn comment_text(node: &SyntaxNode) -> String {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind() == SyntaxKind::COMMENT)
        .map(|token| token.text().trim_end().to_string())
        .unwrap_or_default()
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A frozen field's value with its interleaved comment lines dropped — what
/// `read.dcf` sees, which skips them and resumes the field.
fn strip_comment_lines(raw: &str) -> String {
    raw.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_whitespace(value: &str) -> String {
    value.chars().filter(|ch| !ch.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_of(text: &str) -> Plan {
        build(&dcf::parse(text).document()).expect("plans")
    }

    fn body_of(text: &str, name: &str) -> FieldBody {
        plan_of(text)
            .record
            .expect("record")
            .fields
            .into_iter()
            .find(|field| field.name == name)
            .expect("field")
            .body
    }

    #[test]
    fn a_comment_between_fields_leads_the_following_field() {
        let plan = plan_of("Version: 1.0\n# note\nPackage: p\n");
        let fields = plan.record.expect("record").fields;
        // Canonical order puts Package first, and the comment travels with it.
        assert_eq!(fields[0].name, "Package");
        assert_eq!(fields[0].leading_comments, vec!["# note".to_string()]);
        assert!(fields[1].leading_comments.is_empty());
    }

    #[test]
    fn a_comment_with_nothing_after_it_stays_unanchored() {
        let plan = plan_of("Package: p\n# dangling\n");
        let record = plan.record.expect("record");
        assert!(record.fields[0].leading_comments.is_empty());
        assert_eq!(record.trailing_comments, vec!["# dangling".to_string()]);
    }

    #[test]
    fn an_interior_comment_freezes_its_field() {
        let body = body_of("Collate:\n    'a.R'\n# why\n    'b.R'\n", "Collate");
        assert_eq!(
            body,
            FieldBody::Verbatim("\n    'a.R'\n# why\n    'b.R'\n".to_string())
        );
    }

    #[test]
    fn dependency_entries_normalize_constraint_spacing() {
        assert_eq!(
            body_of("Imports: dplyr(>=1.0.0)\n", "Imports"),
            FieldBody::CommaList(vec!["dplyr (>= 1.0.0)".to_string()])
        );
    }

    #[test]
    fn an_unparsed_constraint_is_kept_verbatim() {
        assert_eq!(
            body_of("Imports: pkg (garbage)\n", "Imports"),
            FieldBody::CommaList(vec!["pkg (garbage)".to_string()])
        );
    }

    #[test]
    fn a_comma_list_that_would_not_round_trip_falls_back_to_opaque() {
        // The empty entry is dropped by `dependency_entries`, so rendering would
        // silently rewrite the value.
        assert_eq!(
            body_of("Imports: a,,b\n", "Imports"),
            FieldBody::Opaque(vec!["a,,b".to_string()])
        );
    }

    #[test]
    fn r_sorts_first_in_depends_only() {
        assert_eq!(
            body_of("Depends: zoo, R (>= 3.5), MASS\n", "Depends"),
            FieldBody::CommaList(vec![
                "R (>= 3.5)".to_string(),
                "MASS".to_string(),
                "zoo".to_string()
            ])
        );
        assert_eq!(
            body_of("Imports: zoo, R6, MASS\n", "Imports"),
            FieldBody::CommaList(vec![
                "MASS".to_string(),
                "R6".to_string(),
                "zoo".to_string()
            ])
        );
    }

    #[test]
    fn an_unknown_field_keeps_its_line_structure() {
        assert_eq!(
            body_of(
                "Config/Needs/website: pkgdown,\n  tidytemplate\n",
                "Config/Needs/website"
            ),
            FieldBody::Opaque(vec!["pkgdown,".to_string(), "tidytemplate".to_string()])
        );
    }

    #[test]
    fn declines_name_whitespace_duplicates_and_extra_records() {
        assert_eq!(
            build(&dcf::parse("Package : p\n").document()),
            Err(DeclineReason::NameWhitespace {
                name: "Package".to_string()
            })
        );
        assert_eq!(
            build(&dcf::parse("Package: p\nPackage: q\n").document()),
            Err(DeclineReason::DuplicateField {
                name: "Package".to_string()
            })
        );
        assert_eq!(
            build(&dcf::parse("Package: p\n\nPackage: q\n").document()),
            Err(DeclineReason::MultipleRecords { count: 2 })
        );
        assert_eq!(
            build(&dcf::parse("Package: p\nEncoding: latin1\n").document()),
            Err(DeclineReason::Encoding {
                declared: "latin1".to_string()
            })
        );
    }

    #[test]
    fn an_encoding_frozen_by_a_comment_still_declines() {
        // The interior comment makes the field `Verbatim`. Reading the guard off
        // `Wrapped` alone would wave the file through and re-wrap every *other*
        // field's prose, which is the guesswork the guard exists to refuse.
        assert_eq!(
            build(&dcf::parse("Package: p\nEncoding:\n# why\n    latin1\n").document()),
            Err(DeclineReason::Encoding {
                declared: "latin1".to_string()
            })
        );
        // A frozen UTF-8 declaration is still fine.
        assert!(build(&dcf::parse("Package: p\nEncoding:\n# why\n    UTF-8\n").document()).is_ok());
    }

    #[test]
    fn quoted_collate_tokens_survive_and_unquotable_ones_do_not() {
        assert_eq!(
            ordered_list_tokens("'b.R' a.R"),
            Some(vec!["b.R".to_string(), "a.R".to_string()])
        );
        assert_eq!(ordered_list_tokens("\"it's.R\""), None);
        assert_eq!(ordered_list_tokens("'unterminated"), None);
    }
}
