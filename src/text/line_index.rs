//! Byte-offset ↔ (line, column) conversion.
//!
//! Two coordinate systems share the same line-start table:
//! - 1-indexed (line, column) in **code points** for CLI diagnostics.
//! - 0-indexed (line, character) in the negotiated LSP [`PositionEncoding`]
//!   (UTF-8 or UTF-16 units) for LSP positions.
//!
//! The index is **self-contained**: rather than holding a borrow of the text, it
//! precomputes the line-start offsets plus a per-line table of *wide characters*
//! (chars wider than one byte in UTF-8), so every conversion is O(log n) in the
//! line count with no text slicing. Being owned and `Eq`, it can be cached as a
//! `salsa` query (see [`crate::incremental::line_index`]), à la rust-analyzer's
//! `LineIndex`.

use std::collections::HashMap;

use lsp_types::{Position, PositionEncodingKind};

/// The LSP position encoding negotiated at `initialize`: the unit an LSP
/// `Position.character` counts. arity stores text as UTF-8, so [`Utf8`] is a
/// no-op conversion (cheapest) while [`Utf16`] is the protocol default.
///
/// [`Utf8`]: PositionEncoding::Utf8
/// [`Utf16`]: PositionEncoding::Utf16
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PositionEncoding {
    /// UTF-8 code units, i.e. byte offsets. arity's native encoding.
    Utf8,
    /// UTF-16 code units (a surrogate pair counts as 2). The LSP default, and
    /// the only encoding a server may use when the client offers none.
    #[default]
    Utf16,
}

impl PositionEncoding {
    /// The `lsp-types` capability value for this encoding.
    pub fn to_kind(self) -> PositionEncodingKind {
        match self {
            PositionEncoding::Utf8 => PositionEncodingKind::UTF8,
            PositionEncoding::Utf16 => PositionEncodingKind::UTF16,
        }
    }

    fn metric(self) -> Metric {
        match self {
            PositionEncoding::Utf8 => Metric::Utf8,
            PositionEncoding::Utf16 => Metric::Utf16,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    /// 1-indexed line number.
    pub line: usize,
    /// 1-indexed column in code points (not bytes, not UTF-16 units).
    pub column: usize,
}

/// A character wider than one byte in UTF-8, recorded by its **line-relative**
/// byte range. Anything outside a wide char is a 1-byte ASCII char that counts
/// as one unit in every metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WideChar {
    /// Byte offset of the char's start, relative to the start of its line.
    start: u32,
    /// Byte offset just past the char, relative to the start of its line.
    end: u32,
}

impl WideChar {
    /// UTF-8 length in bytes (2, 3, or 4 for a wide char).
    fn len(self) -> u32 {
        self.end - self.start
    }

    /// UTF-16 length in code units: 2 for an astral char (4-byte UTF-8,
    /// encoded as a surrogate pair), 1 for a BMP char (2- or 3-byte UTF-8).
    fn len_utf16(self) -> u32 {
        if self.len() == 4 { 2 } else { 1 }
    }
}

/// How a character's width is measured when converting a byte offset to a column.
#[derive(Clone, Copy)]
enum Metric {
    /// UTF-8 code units (bytes).
    Utf8,
    /// UTF-16 code units.
    Utf16,
    /// Unicode scalar values (code points).
    CodePoint,
}

impl Metric {
    /// The number of units a wide char occupies in this metric.
    fn wide_units(self, w: WideChar) -> u32 {
        match self {
            Metric::Utf8 => w.len(),
            Metric::Utf16 => w.len_utf16(),
            Metric::CodePoint => 1,
        }
    }
}

/// Precomputed line-start byte offsets plus per-line wide-char tables for a text
/// buffer. `line_starts[i]` is the byte offset of the first character of line `i`
/// (0-indexed); `line_starts` always starts with `0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    line_starts: Vec<usize>,
    /// Wide chars per 0-indexed line, in ascending order; a line with none is
    /// simply absent from the map.
    line_wide_chars: HashMap<usize, Vec<WideChar>>,
    /// Total byte length of the text.
    len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = Vec::with_capacity(text.len() / 40 + 1);
        line_starts.push(0);
        let mut line_wide_chars: HashMap<usize, Vec<WideChar>> = HashMap::new();
        let mut cur_line = 0usize;
        let mut line_start = 0usize;
        for (offset, ch) in text.char_indices() {
            if ch == '\n' {
                line_starts.push(offset + 1);
                cur_line += 1;
                line_start = offset + 1;
                continue;
            }
            let bytes = ch.len_utf8();
            if bytes > 1 {
                line_wide_chars.entry(cur_line).or_default().push(WideChar {
                    start: (offset - line_start) as u32,
                    end: (offset + bytes - line_start) as u32,
                });
            }
        }
        Self {
            line_starts,
            line_wide_chars,
            len: text.len(),
        }
    }

    /// 1-indexed (line, column-in-code-points). Suitable for CLI diagnostics.
    pub fn byte_to_lc(&self, offset: usize) -> LineCol {
        let clamped = offset.min(self.len);
        let line = self.line_index_for(clamped);
        let rel = clamped - self.line_starts[line];
        LineCol {
            line: line + 1,
            column: self.col_in(line, rel, Metric::CodePoint) as usize + 1,
        }
    }

    /// 0-indexed LSP `Position`, with `character` measured in `encoding` units.
    pub fn byte_to_position(&self, offset: usize, encoding: PositionEncoding) -> Position {
        let clamped = offset.min(self.len);
        let line = self.line_index_for(clamped);
        let rel = clamped - self.line_starts[line];
        let character = self.col_in(line, rel, encoding.metric());
        Position::new(line as u32, character)
    }

    /// Inverse of [`byte_to_position`](Self::byte_to_position): a 0-indexed LSP
    /// `Position` (its `character` in `encoding` units) back to a byte offset. A
    /// line or character past the end clamps to the end of the line / buffer, and
    /// a character landing inside a wide char clamps to that char's start.
    pub fn position_to_byte(&self, position: Position, encoding: PositionEncoding) -> usize {
        let line = position.line as usize;
        if line >= self.line_starts.len() {
            return self.len;
        }
        self.byte_at_col(line, position.character, encoding.metric())
    }

    /// The 0-indexed line containing `offset`. Encoding-independent — the line a
    /// byte falls on does not depend on how columns are counted.
    pub fn byte_to_line(&self, offset: usize) -> u32 {
        self.line_index_for(offset.min(self.len)) as u32
    }

    /// Total line count (1 even for empty text).
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Byte offset of the start of the 0-indexed `line`. A line past the end
    /// clamps to the buffer end, so `line_start(n)..line_start(n + 1)` is
    /// always a valid slice range covering line `n` (including its newline).
    pub fn line_start(&self, line: usize) -> usize {
        self.line_starts.get(line).copied().unwrap_or(self.len)
    }

    /// The column of a line-relative byte offset `rel` on `line`, in `metric`
    /// units. Each wide char fully before `rel` contributes fewer units than its
    /// byte length, so the column is `rel` minus that accumulated shortfall.
    fn col_in(&self, line: usize, rel: usize, metric: Metric) -> u32 {
        let mut shortfall = 0u32;
        if let Some(wides) = self.line_wide_chars.get(&line) {
            for w in wides {
                if w.end as usize <= rel {
                    shortfall += w.len() - metric.wide_units(*w);
                } else {
                    break;
                }
            }
        }
        rel as u32 - shortfall
    }

    /// The absolute byte offset at column `target_col` (in `metric` units) on
    /// `line`, walking the line's ASCII runs and wide chars. Overshoot clamps to
    /// the line end; a column inside a wide char clamps to the char's start.
    fn byte_at_col(&self, line: usize, target_col: u32, metric: Metric) -> usize {
        let line_start = self.line_starts[line];
        let line_end = self.line_starts.get(line + 1).copied().unwrap_or(self.len);
        let mut col = 0u32;
        let mut byte = line_start;
        if let Some(wides) = self.line_wide_chars.get(&line) {
            for w in wides {
                let w_start = line_start + w.start as usize;
                // The ASCII run before this wide char: 1 unit == 1 byte each.
                let ascii = (w_start - byte) as u32;
                if col + ascii >= target_col {
                    return byte + (target_col - col) as usize;
                }
                col += ascii;
                byte = w_start;
                let units = metric.wide_units(*w);
                if col + units > target_col {
                    // Column lands inside the wide char: clamp to its start.
                    return byte;
                }
                col += units;
                byte = line_start + w.end as usize;
            }
        }
        // Trailing ASCII after the last wide char (or the whole line if none):
        // 1 unit == 1 byte, clamped to the line end on overshoot.
        (byte + (target_col - col) as usize).min(line_end)
    }

    fn line_index_for(&self, offset: usize) -> usize {
        match self.line_starts.binary_search(&offset) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use PositionEncoding::{Utf8, Utf16};

    #[test]
    fn empty_string() {
        let idx = LineIndex::new("");
        assert_eq!(idx.byte_to_lc(0), LineCol { line: 1, column: 1 });
        assert_eq!(idx.byte_to_position(0, Utf16), Position::new(0, 0));
        assert_eq!(idx.byte_to_position(0, Utf8), Position::new(0, 0));
    }

    #[test]
    fn single_line() {
        let idx = LineIndex::new("abc");
        assert_eq!(idx.byte_to_lc(0).column, 1);
        assert_eq!(idx.byte_to_lc(2).column, 3);
        assert_eq!(idx.byte_to_lc(3).column, 4);
        assert_eq!(idx.byte_to_position(2, Utf16), Position::new(0, 2));
        assert_eq!(idx.byte_to_position(2, Utf8), Position::new(0, 2));
    }

    #[test]
    fn multi_line() {
        let idx = LineIndex::new("ab\ncd\nef");
        assert_eq!(idx.byte_to_lc(0), LineCol { line: 1, column: 1 });
        assert_eq!(idx.byte_to_lc(2), LineCol { line: 1, column: 3 }); // the newline
        assert_eq!(idx.byte_to_lc(3), LineCol { line: 2, column: 1 });
        assert_eq!(idx.byte_to_lc(6), LineCol { line: 3, column: 1 });
        assert_eq!(idx.byte_to_position(6, Utf16), Position::new(2, 0));
        assert_eq!(idx.byte_to_position(6, Utf8), Position::new(2, 0));
    }

    #[test]
    fn utf8_multibyte() {
        // Each non-ASCII char below is 2 bytes in UTF-8, 1 UTF-16 unit, 1 code point.
        let idx = LineIndex::new("\u{00e1}b\nc");
        // offset 2 = after á (2 bytes)
        assert_eq!(idx.byte_to_lc(2), LineCol { line: 1, column: 2 });
        assert_eq!(idx.byte_to_position(2, Utf16), Position::new(0, 1));
        // In UTF-8 the character column is the byte offset: 2.
        assert_eq!(idx.byte_to_position(2, Utf8), Position::new(0, 2));
        // offset 3 = at 'b'
        assert_eq!(idx.byte_to_lc(3), LineCol { line: 1, column: 3 });
        assert_eq!(idx.byte_to_position(3, Utf16), Position::new(0, 2));
        assert_eq!(idx.byte_to_position(3, Utf8), Position::new(0, 3));
    }

    #[test]
    fn utf16_surrogate_pair() {
        // U+1F600 (emoji) is 4 bytes in UTF-8, 2 UTF-16 units, 1 code point.
        let idx = LineIndex::new("\u{1F600}x");
        // offset 4 = after emoji
        assert_eq!(idx.byte_to_lc(4), LineCol { line: 1, column: 2 });
        assert_eq!(idx.byte_to_position(4, Utf16), Position::new(0, 2));
        assert_eq!(idx.byte_to_position(4, Utf8), Position::new(0, 4));
        // offset 5 = after 'x'
        assert_eq!(idx.byte_to_position(5, Utf16), Position::new(0, 3));
        assert_eq!(idx.byte_to_position(5, Utf8), Position::new(0, 5));
    }

    #[test]
    fn offset_past_end_clamps() {
        let idx = LineIndex::new("abc");
        assert_eq!(idx.byte_to_lc(100), LineCol { line: 1, column: 4 });
    }

    #[test]
    fn trailing_newline() {
        let idx = LineIndex::new("ab\n");
        assert_eq!(idx.byte_to_lc(3), LineCol { line: 2, column: 1 });
    }

    #[test]
    fn line_start_clamps_past_the_end() {
        let idx = LineIndex::new("ab\ncd");
        assert_eq!(idx.line_start(0), 0);
        assert_eq!(idx.line_start(1), 3);
        assert_eq!(idx.line_start(2), 5);
        assert_eq!(idx.line_start(99), 5);
    }

    #[test]
    fn position_to_byte_round_trips_both_encodings() {
        let text = "ab\ncde\nf\u{00e1}g\n\u{1F600}h";
        let idx = LineIndex::new(text);
        for encoding in [Utf8, Utf16] {
            for offset in 0..=text.len() {
                // Only byte offsets on char boundaries round-trip exactly.
                if !text.is_char_boundary(offset) {
                    continue;
                }
                let pos = idx.byte_to_position(offset, encoding);
                assert_eq!(
                    idx.position_to_byte(pos, encoding),
                    offset,
                    "offset {offset} encoding {encoding:?}"
                );
            }
        }
    }

    #[test]
    fn position_to_byte_handles_wide_chars_and_overshoot() {
        // Emoji is 4 UTF-8 bytes, 2 UTF-16 units.
        let idx = LineIndex::new("\u{1F600}x\ny");
        // UTF-16.
        assert_eq!(idx.position_to_byte(Position::new(0, 0), Utf16), 0);
        assert_eq!(idx.position_to_byte(Position::new(0, 2), Utf16), 4); // at 'x'
        assert_eq!(idx.position_to_byte(Position::new(1, 0), Utf16), 6); // at 'y'
        // A UTF-16 unit landing inside the surrogate pair clamps to its start.
        assert_eq!(idx.position_to_byte(Position::new(0, 1), Utf16), 0);
        // A character past the line end clamps to the line end.
        assert_eq!(idx.position_to_byte(Position::new(0, 99), Utf16), 6);
        // A line past the end clamps to the buffer end.
        assert_eq!(idx.position_to_byte(Position::new(9, 0), Utf16), 7);
        // UTF-8: the character column is the byte offset within the line.
        assert_eq!(idx.position_to_byte(Position::new(0, 4), Utf8), 4); // at 'x'
        assert_eq!(idx.position_to_byte(Position::new(0, 5), Utf8), 5); // at line end
    }

    #[test]
    fn to_kind_maps_to_lsp() {
        assert_eq!(Utf8.to_kind(), PositionEncodingKind::UTF8);
        assert_eq!(Utf16.to_kind(), PositionEncodingKind::UTF16);
    }
}
