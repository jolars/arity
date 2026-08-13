//! [`FieldPlan`] to output lines.
//!
//! Every function here is total: rendering never fails, because every decision
//! that could fail was already made in `plan.rs`.

use super::plan::{FieldBody, FieldPlan};
use super::rcode;
use super::wrap;
use crate::formatter::style::FormatStyle;

pub(super) fn render(field: &FieldPlan, style: FormatStyle, indent: &str) -> Vec<String> {
    let name = field.name.as_str();
    match &field.body {
        FieldBody::Wrapped(value) => wrap::fill(name, value, style.line_width, indent),
        FieldBody::CommaList(entries) => comma_list(name, entries, indent),
        FieldBody::OrderedList(tokens) => ordered_list(name, tokens, indent),
        FieldBody::RCode(source) => rcode::render(name, source, style, indent)
            .unwrap_or_else(|| opaque(name, &split_folded(source), indent)),
        FieldBody::Opaque(lines) => opaque(name, lines, indent),
        FieldBody::Verbatim(raw) => verbatim(name, raw),
    }
}

/// One entry per line, always broken. A comma list is a comma list: whether it
/// happens to fit on the key's line is not a reason to lay it out differently
/// from the next package's.
fn comma_list(name: &str, entries: &[String], indent: &str) -> Vec<String> {
    if entries.is_empty() {
        return vec![format!("{name}:")];
    }
    let mut lines = Vec::with_capacity(entries.len() + 1);
    lines.push(format!("{name}:"));
    for (index, entry) in entries.iter().enumerate() {
        let comma = if index + 1 < entries.len() { "," } else { "" };
        lines.push(format!("{indent}{entry}{comma}"));
    }
    lines
}

fn ordered_list(name: &str, tokens: &[String], indent: &str) -> Vec<String> {
    if tokens.is_empty() {
        return vec![format!("{name}:")];
    }
    let mut lines = Vec::with_capacity(tokens.len() + 1);
    lines.push(format!("{name}:"));
    lines.extend(tokens.iter().map(|token| format!("{indent}'{token}'")));
    lines
}

/// Line structure preserved 1:1; only the continuation indent is normalized.
/// `read.dcf` strips a continuation's leading whitespace unconditionally, so
/// this is provably value-preserving.
fn opaque(name: &str, lines: &[String], indent: &str) -> Vec<String> {
    let mut iter = lines.iter();
    let head = match iter.next() {
        Some(first) if !first.is_empty() => format!("{name}: {first}"),
        _ => format!("{name}:"),
    };
    let mut out = vec![head];
    out.extend(iter.map(|line| format!("{indent}{line}")));
    out
}

/// The field's own bytes, appended straight after the colon.
fn verbatim(name: &str, raw: &str) -> Vec<String> {
    let joined = format!("{name}:{raw}");
    // `strip_suffix`, not `trim_end_matches`: exactly one `\r` is this line's
    // CRLF terminator. Any further one is a byte of the value, and replaying the
    // value's bytes is the whole point of this shape.
    let mut lines: Vec<String> = joined
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// Recover a folded value's line structure for the opaque fallback.
fn split_folded(source: &str) -> Vec<String> {
    source.split('\n').map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDENT: &str = "    ";

    fn field(name: &str, body: FieldBody) -> FieldPlan {
        FieldPlan {
            name: name.to_string(),
            leading_comments: Vec::new(),
            body,
        }
    }

    fn rendered(name: &str, body: FieldBody) -> Vec<String> {
        render(&field(name, body), FormatStyle::default(), INDENT)
    }

    #[test]
    fn a_single_entry_comma_list_still_breaks() {
        assert_eq!(
            rendered("Imports", FieldBody::CommaList(vec!["cli".to_string()])),
            vec!["Imports:", "    cli"]
        );
    }

    #[test]
    fn an_empty_comma_list_keeps_the_field() {
        assert_eq!(
            rendered("Suggests", FieldBody::CommaList(Vec::new())),
            vec!["Suggests:"]
        );
    }

    #[test]
    fn ordered_lists_are_quoted_in_place() {
        assert_eq!(
            rendered(
                "Collate",
                FieldBody::OrderedList(vec!["b.R".to_string(), "a.R".to_string()])
            ),
            vec!["Collate:", "    'b.R'", "    'a.R'"]
        );
    }

    #[test]
    fn an_opaque_field_with_an_empty_own_line_keeps_it_empty() {
        assert_eq!(
            rendered(
                "Config/x",
                FieldBody::Opaque(vec![String::new(), "value".to_string()])
            ),
            vec!["Config/x:", "    value"]
        );
    }

    #[test]
    fn verbatim_replays_the_source_bytes() {
        assert_eq!(
            rendered(
                "Collate",
                FieldBody::Verbatim("\n    'a.R'\n# why\n    'b.R'\n".to_string())
            ),
            vec!["Collate:", "    'a.R'", "# why", "    'b.R'"]
        );
    }

    #[test]
    fn unparseable_r_falls_back_to_the_opaque_shape() {
        assert_eq!(
            rendered("Authors@R", FieldBody::RCode("person(\"Jo\",".to_string())),
            vec!["Authors@R: person(\"Jo\","]
        );
    }
}
