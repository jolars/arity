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
        line_starts.extend(memchr::memchr_iter(b'\n', bytes).map(|i| i + 1));
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

    /// Patch the index for an edit replacing `range` with `insert`, instead of
    /// rebuilding it from the edited text. Cost is `O(log n + insert.len() +
    /// lines after the edit)` rather than linear in the whole document.
    ///
    /// The result is **bit-identical** to `LineIndex::new(edited_text)`; that is
    /// what `apply_edit_matches_rebuild_exhaustively` pins, and what lets a
    /// patched index and the salsa `line_index` query be used interchangeably.
    ///
    /// `range` must be within the *pre-edit* text and both ends must fall on
    /// char boundaries. The LSP path cannot violate this: [`position_to_byte`]
    /// clamps a column landing inside a wide char to that char's start.
    ///
    /// [`position_to_byte`]: Self::position_to_byte
    pub fn apply_edit(&mut self, range: std::ops::Range<usize>, insert: &str) {
        let (start, end) = (range.start, range.end);
        debug_assert!(
            start <= end && end <= self.len,
            "edit {range:?} out of range"
        );
        debug_assert!(
            self.is_char_boundary(start) && self.is_char_boundary(end),
            "edit {range:?} splits a wide char"
        );

        let removed = end - start;
        let inserted = insert.len();
        // A length-preserving edit (typing over a selection) shifts nothing, so
        // both tails stay put and the splice is all that is needed.
        let shift = removed != inserted;

        // `line_starts[i]` for `i > 0` is `newline_offset + 1`, so an entry `s`
        // dies exactly when its newline at `s - 1` falls in `start..end`, i.e.
        // when `start < s <= end`.
        let first = self.line_starts.partition_point(|&s| s <= start);
        let last = self.line_starts.partition_point(|&s| s <= end);
        // Shift before splicing, while these indices still address the old Vec.
        if shift {
            for s in &mut self.line_starts[last..] {
                *s = *s - removed + inserted;
            }
        }
        // `line_starts[0]` is 0, which is `<= start` for every edit, so `first`
        // is at least 1: the leading zero is structurally never touched.
        self.line_starts.splice(
            first..last,
            memchr::memchr_iter(b'\n', insert.as_bytes()).map(|i| start + i + 1),
        );

        // Wide chars splice the same way. Under the char-boundary precondition,
        // a wide char starting inside the edit also ends inside it, so testing
        // `start` alone selects exactly the wholly-replaced run.
        let wfirst = self
            .wide_chars
            .partition_point(|w| (w.start as usize) < start);
        let wlast = self
            .wide_chars
            .partition_point(|w| (w.start as usize) < end);
        if shift {
            for w in &mut self.wide_chars[wlast..] {
                w.start = (w.start as usize - removed + inserted) as u32;
                w.end = (w.end as usize - removed + inserted) as u32;
            }
        }
        if insert.is_ascii() {
            self.wide_chars.splice(wfirst..wlast, std::iter::empty());
        } else {
            let new: Vec<WideChar> = insert
                .char_indices()
                .filter(|(_, ch)| ch.len_utf8() > 1)
                .map(|(i, ch)| WideChar {
                    start: (start + i) as u32,
                    end: (start + i + ch.len_utf8()) as u32,
                })
                .collect();
            self.wide_chars.splice(wfirst..wlast, new);
        }

        self.len = self.len - removed + inserted;
    }

    /// Whether `offset` falls on a char boundary, decided from the wide-char
    /// table alone — no access to the text. Only a wide char can straddle an
    /// offset, so an offset strictly inside one is the only non-boundary.
    fn is_char_boundary(&self, offset: usize) -> bool {
        !self
            .wide_chars_from(offset.saturating_sub(3))
            .iter()
            .take_while(|w| (w.start as usize) < offset)
            .any(|w| (w.end as usize) > offset)
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

    /// Assert the representation invariants a rebuild always satisfies. Equality
    /// against a rebuilt index proves a patch agrees with `new`; this proves the
    /// pair are not *both* wrong.
    #[cfg(test)]
    fn assert_canonical(&self) {
        assert_eq!(self.line_starts.first(), Some(&0), "missing leading zero");
        assert!(
            self.line_starts.windows(2).all(|w| w[0] < w[1]),
            "line starts not strictly increasing: {:?}",
            self.line_starts
        );
        assert!(
            self.line_starts.iter().all(|&s| s <= self.len),
            "line start past the end: {:?} len {}",
            self.line_starts,
            self.len
        );
        assert!(
            self.wide_chars
                .windows(2)
                .all(|w| w[0].end <= w[1].start && w[0].start < w[0].end),
            "wide chars overlap or are unsorted: {:?}",
            self.wide_chars
        );
        assert!(
            self.wide_chars
                .iter()
                .all(|w| (2..=4).contains(&w.len()) && w.end as usize <= self.len),
            "wide char out of range: {:?} len {}",
            self.wide_chars,
            self.len
        );
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

    /// Splice `insert` over `range` in `base` and assert that patching an index
    /// built from `base` lands on exactly the index a rebuild would produce.
    /// Bit-identity is the bar: a divergence makes live-buffer handlers report
    /// wrong positions, and would silently break the salsa query's backdating.
    #[track_caller]
    fn assert_patch_matches_rebuild(base: &str, range: std::ops::Range<usize>, insert: &str) {
        let mut spliced = base.to_string();
        spliced.replace_range(range.clone(), insert);

        let mut patched = LineIndex::new(base);
        patched.apply_edit(range.clone(), insert);
        patched.assert_canonical();

        assert_eq!(
            patched,
            LineIndex::new(&spliced),
            "base {base:?} range {range:?} insert {insert:?}"
        );
    }

    /// Bases spanning the structural cases: empty, no newline, only newlines,
    /// with and without a trailing newline, wide chars early and late, CRLF,
    /// and a line made entirely of wide chars.
    const BASES: [&str; 9] = [
        "",
        "a",
        "\n",
        "\n\n\n",
        "ab\ncd\nef",
        "ab\ncd\nef\n",
        "\u{00e1}b\nc\u{1F600}\nd",
        "a\r\nb\r\n",
        "\u{1F600}\n\u{1F600}",
    ];

    /// Inserts covering: nothing, plain text, newlines at either end, several
    /// newlines, both wide-char widths, a multi-line insert carrying wide
    /// chars, and a CRLF pair.
    const INSERTS: [&str; 11] = [
        "",
        "x",
        "\n",
        "\nx",
        "x\n",
        "\n\n",
        "xy",
        "\u{00e1}",
        "\u{1F600}",
        "a\u{00e1}\nb\u{1F600}\n",
        "\r\n",
    ];

    #[test]
    fn apply_edit_matches_rebuild_exhaustively() {
        for base in BASES {
            let bounds: Vec<usize> = (0..=base.len())
                .filter(|&o| base.is_char_boundary(o))
                .collect();
            for (i, &start) in bounds.iter().enumerate() {
                for &end in &bounds[i..] {
                    for insert in INSERTS {
                        assert_patch_matches_rebuild(base, start..end, insert);
                    }
                }
            }
        }
    }

    #[test]
    fn apply_edit_sequences_match_rebuild() {
        // State that is only wrong on the *second* patch — a stale `len`, a tail
        // shifted twice — survives a single-edit test. Chain three edits and
        // check against a rebuild after every step.
        type Recipe = fn(&str) -> (std::ops::Range<usize>, &'static str);
        let recipes: [Recipe; 6] = [
            |_| (0..0, "\n"),
            |t| (t.len()..t.len(), "x"),
            // Delete the first line, newline included, if there is one.
            |t| (0..t.find('\n').map_or(0, |i| i + 1), ""),
            // Delete the last byte, snapped to a char boundary.
            |t| {
                let mut at = t.len();
                while at > 0 && !t.is_char_boundary(at - 1) {
                    at -= 1;
                }
                (at.saturating_sub(1)..t.len(), "")
            },
            // Replace the middle byte with a wide char.
            |t| {
                let mut at = t.len() / 2;
                while !t.is_char_boundary(at) {
                    at -= 1;
                }
                (at..at, "\u{00e1}")
            },
            |t| {
                let mut at = t.len() / 2;
                while !t.is_char_boundary(at) {
                    at -= 1;
                }
                (at..at, "p\nq\n")
            },
        ];

        for base in ["ab\ncd\nef\n", "\u{1F600}x\n\u{00e1}", ""] {
            for a in recipes {
                for b in recipes {
                    for c in recipes {
                        let mut text = base.to_string();
                        let mut index = LineIndex::new(&text);
                        for step in [a, b, c] {
                            let (range, insert) = step(&text);
                            index.apply_edit(range.clone(), insert);
                            text.replace_range(range, insert);
                            index.assert_canonical();
                            assert_eq!(index, LineIndex::new(&text), "after {text:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn apply_edit_no_op_leaves_the_index_untouched() {
        let before = LineIndex::new("ab\ncd\n");
        let mut after = before.clone();
        after.apply_edit(3..3, "");
        assert_eq!(before, after);
    }

    #[test]
    fn apply_edit_pure_insert_and_pure_delete() {
        assert_patch_matches_rebuild("ab\ncd", 2..2, "XY");
        assert_patch_matches_rebuild("ab\ncd", 1..3, "");
    }

    #[test]
    fn apply_edit_at_the_buffer_end() {
        assert_patch_matches_rebuild("ab\ncd", 5..5, "e");
        // Appending after a trailing newline starts a new line.
        assert_patch_matches_rebuild("ab\n", 3..3, "c");
    }

    #[test]
    fn apply_edit_deleting_the_trailing_newline() {
        assert_patch_matches_rebuild("ab\ncd\n", 5..6, "");
    }

    #[test]
    fn apply_edit_inserting_a_newline_at_offset_zero() {
        assert_patch_matches_rebuild("ab\ncd", 0..0, "\n");
    }

    #[test]
    fn apply_edit_deleting_from_zero_across_a_newline() {
        assert_patch_matches_rebuild("ab\ncd\nef", 0..4, "");
    }

    #[test]
    fn apply_edit_spanning_several_newlines() {
        assert_patch_matches_rebuild("a\nb\nc\nd\ne", 1..7, "Z");
    }

    #[test]
    fn apply_edit_multi_line_paste() {
        assert_patch_matches_rebuild("ab\ncd", 2..2, "1\n2\n3\n4");
    }

    #[test]
    fn apply_edit_swapping_wide_chars_and_ascii() {
        // A wide char replaced by ASCII, and the reverse: the wide-char run
        // splices out or in while the tail shifts by a different byte delta.
        assert_patch_matches_rebuild("a\u{1F600}b\n\u{00e1}c", 1..5, "z");
        assert_patch_matches_rebuild("azb\n\u{00e1}c", 1..2, "\u{1F600}");
    }

    #[test]
    fn apply_edit_keeps_crlf_line_starts() {
        // Only `\n` starts a line; `\r` is an ordinary byte in both producers.
        assert_patch_matches_rebuild("a\r\nb", 1..1, "\r\n");
    }
}
