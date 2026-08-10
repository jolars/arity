//! Byte-offset ↔ (line, column) conversion.
//!
//! Two coordinate systems share the same line-start table:
//! - 1-indexed (line, column) in **code points** for CLI diagnostics.
//! - 0-indexed (line, character) in the negotiated LSP [`PositionEncoding`]
//!   (UTF-8 or UTF-16 units) for LSP positions.
//!
//! The index is **self-contained**: rather than holding a borrow of the text, it
//! precomputes the line-start offsets plus a table of *wide characters* (chars
//! wider than one byte in UTF-8), so every conversion is O(log n) in the line
//! count with no text slicing. Being owned and `Eq`, it can be cached as a
//! `salsa` query (see [`crate::incremental::line_index`]), à la rust-analyzer's
//! `LineIndex`.
//!
//! Both tables are keyed by **absolute** byte offset and kept sorted, which is
//! what lets an edit *patch* the index instead of rebuilding it: line starts and
//! wide chars splice under one rule (keep the prefix, drop the replaced span,
//! shift the tail by the byte delta) with no renumbering. A per-line wide-char
//! table cannot do that — inserting a line renumbers every later entry.

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

/// A character wider than one byte in UTF-8, recorded by its **absolute** byte
/// range in the buffer. Anything outside a wide char is a 1-byte ASCII char that
/// counts as one unit in every metric.
///
/// `u32` caps a buffer at 4 GiB, the same tradeoff rust-analyzer's `TextSize`
/// makes; [`LineIndex::new`] asserts it in debug builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WideChar {
    /// Byte offset of the char's start.
    start: u32,
    /// Byte offset just past the char.
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
    /// Every wide char in the buffer, by absolute byte offset, ascending. Empty
    /// for pure-ASCII text, which is nearly every R file.
    wide_chars: Vec<WideChar>,
    /// Total byte length of the text.
    len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        debug_assert!(
            text.len() <= u32::MAX as usize,
            "LineIndex records wide chars as u32 offsets"
        );
        let bytes = text.as_bytes();
        let mut line_starts = Vec::with_capacity(bytes.len() / 40 + 1);
        line_starts.push(0);
        line_starts.extend(
            bytes
                .iter()
                .enumerate()
                .filter(|&(_, &b)| b == b'\n')
                .map(|(i, _)| i + 1),
        );
        // Absolute offsets make the two tables independent, so the wide-char
        // scan can be skipped wholesale. `is_ascii` is word-chunked, so the
        // common case costs a fast pass rather than a `char_indices` walk.
        let wide_chars = if bytes.is_ascii() {
            Vec::new()
        } else {
            text.char_indices()
                .filter(|(_, ch)| ch.len_utf8() > 1)
                .map(|(offset, ch)| WideChar {
                    start: offset as u32,
                    end: (offset + ch.len_utf8()) as u32,
                })
                .collect()
        };
        Self {
            line_starts,
            wide_chars,
            len: text.len(),
        }
    }

    /// 1-indexed (line, column-in-code-points). Suitable for CLI diagnostics.
    pub fn byte_to_lc(&self, offset: usize) -> LineCol {
        let clamped = offset.min(self.len);
        let line = self.line_index_for(clamped);
        LineCol {
            line: line + 1,
            column: self.col_in(self.line_starts[line], clamped, Metric::CodePoint) as usize + 1,
        }
    }

    /// 0-indexed LSP `Position`, with `character` measured in `encoding` units.
    pub fn byte_to_position(&self, offset: usize, encoding: PositionEncoding) -> Position {
        let clamped = offset.min(self.len);
        let line = self.line_index_for(clamped);
        let character = self.col_in(self.line_starts[line], clamped, encoding.metric());
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

    /// The wide chars starting at or after `from`, ascending. Callers stop at
    /// their own upper bound; because the table is sorted, the first entry past
    /// that bound ends the run.
    fn wide_chars_from(&self, from: usize) -> &[WideChar] {
        let i = self
            .wide_chars
            .partition_point(|w| (w.start as usize) < from);
        &self.wide_chars[i..]
    }

    /// The column of absolute byte offset `offset` on the line starting at
    /// `line_start`, in `metric` units. Each wide char fully before `offset`
    /// contributes fewer units than its byte length, so the column is the
    /// line-relative offset minus that accumulated shortfall.
    fn col_in(&self, line_start: usize, offset: usize, metric: Metric) -> u32 {
        let rel = (offset - line_start) as u32;
        if self.wide_chars.is_empty() {
            return rel;
        }
        let mut shortfall = 0u32;
        for w in self.wide_chars_from(line_start) {
            // The first char reaching past `offset` ends the run — including
            // any on a later line, which start at or after this line's end.
            if w.end as usize > offset {
                break;
            }
            shortfall += w.len() - metric.wide_units(*w);
        }
        rel - shortfall
    }

    /// The absolute byte offset at column `target_col` (in `metric` units) on
    /// `line`, walking the line's ASCII runs and wide chars. Overshoot clamps to
    /// the line end; a column inside a wide char clamps to the char's start.
    fn byte_at_col(&self, line: usize, target_col: u32, metric: Metric) -> usize {
        let line_start = self.line_starts[line];
        let line_end = self.line_starts.get(line + 1).copied().unwrap_or(self.len);
        if self.wide_chars.is_empty() {
            // Pure ASCII: 1 unit == 1 byte for every metric.
            return (line_start + target_col as usize).min(line_end);
        }
        let mut col = 0u32;
        let mut byte = line_start;
        for w in self.wide_chars_from(line_start) {
            let w_start = w.start as usize;
            if w_start >= line_end {
                break;
            }
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
            byte = w.end as usize;
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
    fn wide_chars_on_late_lines() {
        // Wide chars far from line 0, and a run of empty lines between them: the
        // arrangement a line-keyed wide-char table has to renumber and a flat
        // offset-keyed one does not.
        let text = "a\n\n\n\u{00e1}b\n\n\u{1F600}\u{00e1}c\nd";
        let idx = LineIndex::new(text);
        // Line 3 starts at byte 4 and is "áb", so 'b' is at byte 6: UTF-16
        // column 1 (á is one unit), UTF-8 column 2 (á is two bytes).
        assert_eq!(idx.byte_to_position(6, Utf16), Position::new(3, 1));
        assert_eq!(idx.byte_to_position(6, Utf8), Position::new(3, 2));
        assert_eq!(idx.position_to_byte(Position::new(3, 1), Utf16), 6);
        // Line 5 starts at byte 9 and is "😀ác", so 'c' is at byte 15: UTF-16
        // column 3 (the emoji is a surrogate pair), UTF-8 column 6.
        assert_eq!(idx.byte_to_position(15, Utf16), Position::new(5, 3));
        assert_eq!(idx.byte_to_position(15, Utf8), Position::new(5, 6));
        assert_eq!(idx.position_to_byte(Position::new(5, 3), Utf16), 15);
        // Line 6 is "d", past every wide char.
        assert_eq!(idx.byte_to_position(17, Utf16), Position::new(6, 0));
        assert_eq!(idx.byte_to_lc(17), LineCol { line: 7, column: 1 });
    }

    #[test]
    fn position_to_byte_round_trips_both_encodings() {
        let texts = [
            "ab\ncde\nf\u{00e1}g\n\u{1F600}h",
            // Several wide chars per line, on lines other than the first, with
            // empty lines and a trailing newline in the mix.
            "\u{00e1}\u{1F600}\u{00e1}\n\nx\u{1F600}y\u{00e1}z\n\u{00e1}\u{00e1}\n",
            // Wide chars only, no ASCII to anchor a scan on.
            "\u{1F600}\u{1F600}\n\u{00e1}\u{1F600}",
        ];
        for text in texts {
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
                        "text {text:?} offset {offset} encoding {encoding:?}"
                    );
                }
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
