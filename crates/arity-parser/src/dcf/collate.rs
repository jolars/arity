//! R's whitespace-separated, optionally quoted `Collate` field grammar.
//!
//! R reads this with `scan(what = character())`, after replacing folded
//! newlines with spaces. Quotes start an entry only at its first byte, and
//! backslashes escape the matching quote only inside a quoted entry. This
//! helper preserves entry order, duplicates, and source spans without changing
//! the lossless DCF tree.

use rowan::{TextRange, TextSize};
use smol_str::SmolStr;

use super::Field;

/// One decoded filename and its complete source span, including quotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollateEntry {
    pub name: SmolStr,
    pub range: TextRange,
}

/// Read one field's Collate list, preserving repeated filenames.
///
/// An unclosed quoted entry includes `textConnection`'s final newline, as R's
/// `scan` does with a warning.
/// This navigation helper does not add diagnostics to the DCF parser.
pub fn collate_entries(field: &Field) -> Vec<CollateEntry> {
    let mut text = String::new();
    let mut offsets = Vec::new();
    for line in field.value_lines() {
        let Some(token) = line.content() else {
            continue;
        };
        let content = token.text().trim();
        if content.is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
            offsets.push(None);
        }
        let leading = token.text().len() - token.text().trim_start().len();
        let source = token.text_range().start() + TextSize::from(leading as u32);
        text.push_str(content);
        offsets.extend((0..content.len()).map(|i| Some(source + TextSize::from(i as u32))));
    }

    let mut entries = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, first)) = chars.next() {
        if matches!(first, ' ' | '\t' | '\n' | '\r') {
            continue;
        }
        let mut name = String::new();
        let mut end = start + first.len_utf8();
        if matches!(first, '\'' | '"') {
            let mut closed = false;
            while let Some((at, ch)) = chars.next() {
                end = at + ch.len_utf8();
                if ch == first {
                    closed = true;
                    break;
                }
                if ch == '\\' {
                    let Some((at, escaped)) = chars.next() else {
                        name.push('\\');
                        break;
                    };
                    end = at + escaped.len_utf8();
                    if escaped != first {
                        name.push('\\');
                    }
                    name.push(escaped);
                } else {
                    name.push(ch);
                }
            }
            if !closed {
                name.push('\n');
            }
        } else {
            name.push(first);
            while let Some(&(at, ch)) = chars.peek() {
                if matches!(ch, ' ' | '\t' | '\n' | '\r') {
                    break;
                }
                chars.next();
                name.push(ch);
                end = at + ch.len_utf8();
            }
        }
        // `scan` skips an empty first item when it is also the final item on
        // the line supplied by `textConnection`.
        if name.is_empty() && entries.is_empty() && chars.peek().is_none() {
            continue;
        }
        // The boundary bytes belong to value tokens even when an entry spans
        // continuation lines. Fold separators have no source byte of their own.
        let range = TextRange::new(
            offsets[start].expect("entry begins in a value token"),
            offsets[end - 1].expect("entry ends in a value token") + TextSize::from(1),
        );
        entries.push(CollateEntry {
            name: name.into(),
            range,
        });
    }
    entries
}
