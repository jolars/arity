//! Reading a `DESCRIPTION` field value the way R does.
//!
//! Shared by the rules that check a single field against one of R's regexps
//! (`description-malformed-name`, `description-malformed-version`,
//! `description-malformed-maintainer`), which all need the same two things: the
//! value R would compare, and the source range to put the caret on.
//!
//! [`Folded`] adds the third thing a rule that reads *into* a value needs: the
//! source range of a slice of it. `description-authors-at-r` hands the folded
//! text to the R parser and then has to point at one `person(...)` inside a
//! field wrapped across five continuation lines.

use rowan::{TextRange, TextSize};

use crate::dcf;

/// A field's value as R reads it, with the source ranges to map back through.
pub struct Folded {
    /// The logical value: contributing lines joined with `\n`, exactly as
    /// `read.dcf` folds them.
    pub text: String,
    /// The whole value's source range, whitespace excluded on both ends.
    pub range: TextRange,
    /// One `(offset in [`text`], offset in the source, length)` per
    /// contributing line, in order.
    lines: Vec<(usize, TextSize, usize)>,
}

impl Folded {
    /// The source range covering a range of [`text`].
    ///
    /// A range that spans the fold's `\n` maps to a source range spanning the
    /// line break and its indent, which is what the reader wants to see: the
    /// construct as written.
    pub fn map(&self, range: TextRange) -> TextRange {
        let start = self.offset(range.start().into());
        let end = self.offset(range.end().into());
        TextRange::new(start, end.max(start))
    }

    /// The source offset for an offset into [`text`]. An offset landing on a
    /// fold's `\n` belongs to the line it ends.
    fn offset(&self, at: usize) -> TextSize {
        for &(folded, source, len) in &self.lines {
            if at <= folded + len {
                return source + TextSize::from(at.saturating_sub(folded) as u32);
            }
        }
        self.range.end()
    }
}

/// The field's logical value and the source range spanning it, whitespace
/// excluded on both ends. `None` when the field carries no value at all.
///
/// The fold matches [`dcf::Field::folded_value`]: empty value lines contribute
/// nothing, while nonempty continuation lines are joined with `\n`.
pub fn value(field: &dcf::Field) -> Option<(String, TextRange)> {
    let folded = folded(field)?;
    Some((folded.text, folded.range))
}

/// [`value`], keeping the per-line mapping back into the source.
pub fn folded(field: &dcf::Field) -> Option<Folded> {
    let lines: Vec<dcf::ValueLine> = field
        .value_lines()
        .filter(|line| !line.trimmed_text().is_empty())
        .collect();
    let (first, last) = (lines.first()?, lines.last()?);

    let mut text = String::new();
    let mut spans = Vec::with_capacity(lines.len());
    for line in &lines {
        if !text.is_empty() {
            text.push('\n');
        }
        let content = line.trimmed_text();
        spans.push((text.len(), trimmed_start(line), content.len()));
        text.push_str(&content);
    }

    Some(Folded {
        text,
        range: TextRange::new(trimmed_start(first), trimmed_end(last)),
        lines: spans,
    })
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
