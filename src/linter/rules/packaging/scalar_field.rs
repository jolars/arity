//! Reading a `DESCRIPTION` field that holds one scalar value, the way R does.
//!
//! Shared by the rules that check a single field against one of R's regexps
//! (`description-malformed-name`, `description-malformed-version`), which all
//! need the same two things: the value R would compare, and the source range to
//! put the caret on.

use rowan::{TextRange, TextSize};

use crate::dcf;

/// The field's logical value and the source range spanning it, whitespace
/// excluded on both ends. `None` when the field carries no value at all.
///
/// The fold is `read.dcf`'s rather than [`dcf::Field::folded_value`]'s: an empty
/// value line contributes nothing, so `Package:\n  mypkg` reads as `mypkg` here
/// and in R, instead of arity's leading-`\n` spelling. A value that really does
/// wrap still folds with the `\n` R rejects it for.
pub fn value(field: &dcf::Field) -> Option<(String, TextRange)> {
    let lines: Vec<dcf::ValueLine> = field
        .value_lines()
        .filter(|line| !line.trimmed_text().is_empty())
        .collect();
    let (first, last) = (lines.first()?, lines.last()?);
    let text = lines
        .iter()
        .map(dcf::ValueLine::trimmed_text)
        .collect::<Vec<_>>()
        .join("\n");
    Some((
        text,
        TextRange::new(trimmed_start(first), trimmed_end(last)),
    ))
}

/// The offset of a value line's content, past the whitespace `VALUE_TEXT` may
/// still carry — the caret belongs on the value, not on the space after the
/// colon.
fn trimmed_start(line: &dcf::ValueLine) -> TextSize {
    let range = line.content_range();
    match line.content() {
        Some(tok) => {
            let lead = tok.text().len() - tok.text().trim_start().len();
            range.start() + TextSize::from(lead as u32)
        }
        None => range.start(),
    }
}

/// The offset just past a value line's content. See [`trimmed_start`].
fn trimmed_end(line: &dcf::ValueLine) -> TextSize {
    let range = line.content_range();
    match line.content() {
        Some(tok) => {
            let trail = tok.text().len() - tok.text().trim_end().len();
            range.end() - TextSize::from(trail as u32)
        }
        None => range.end(),
    }
}

/// A value spanning continuation lines carries the fold's `\n`, and a report is
/// line-oriented. Shown the way R's regexps read it: as a character that is not
/// part of any name or version.
pub fn escape(text: &str) -> String {
    text.replace('\n', "\\n")
}
