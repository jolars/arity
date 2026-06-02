//! Byte-offset → (line, column) conversion.
//!
//! Two coordinate systems share the same line-start table:
//! - 1-indexed (line, column) in **code points** for CLI diagnostics.
//! - 0-indexed (line, character) in **UTF-16 units** for LSP positions.

use tower_lsp_server::ls_types::Position;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    /// 1-indexed line number.
    pub line: usize,
    /// 1-indexed column in code points (not bytes, not UTF-16 units).
    pub column: usize,
}

/// Precomputed line-start byte offsets for a text buffer.
///
/// `line_starts[i]` is the byte offset of the first character of line `i`
/// (0-indexed). `line_starts` always starts with `0`.
#[derive(Debug, Clone)]
pub struct LineIndex<'a> {
    text: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub fn new(text: &'a str) -> Self {
        let mut line_starts = Vec::with_capacity(text.len() / 40 + 1);
        line_starts.push(0);
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(offset + 1);
            }
        }
        Self { text, line_starts }
    }

    /// 1-indexed (line, column-in-code-points). Suitable for CLI diagnostics.
    pub fn byte_to_lc(&self, offset: usize) -> LineCol {
        let clamped = offset.min(self.text.len());
        let line_idx = self.line_index_for(clamped);
        let line_start = self.line_starts[line_idx];
        let column = self.text[line_start..clamped].chars().count() + 1;
        LineCol {
            line: line_idx + 1,
            column,
        }
    }

    /// 0-indexed LSP `Position` (UTF-16 character offsets).
    pub fn byte_to_position(&self, offset: usize) -> Position {
        let clamped = offset.min(self.text.len());
        let line_idx = self.line_index_for(clamped);
        let line_start = self.line_starts[line_idx];
        let character = self.text[line_start..clamped].encode_utf16().count() as u32;
        Position::new(line_idx as u32, character)
    }

    /// Total line count (1 even for empty text).
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
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

    #[test]
    fn empty_string() {
        let idx = LineIndex::new("");
        assert_eq!(idx.byte_to_lc(0), LineCol { line: 1, column: 1 });
        assert_eq!(idx.byte_to_position(0), Position::new(0, 0));
    }

    #[test]
    fn single_line() {
        let idx = LineIndex::new("abc");
        assert_eq!(idx.byte_to_lc(0).column, 1);
        assert_eq!(idx.byte_to_lc(2).column, 3);
        assert_eq!(idx.byte_to_lc(3).column, 4);
        assert_eq!(idx.byte_to_position(2), Position::new(0, 2));
    }

    #[test]
    fn multi_line() {
        let idx = LineIndex::new("ab\ncd\nef");
        assert_eq!(idx.byte_to_lc(0), LineCol { line: 1, column: 1 });
        assert_eq!(idx.byte_to_lc(2), LineCol { line: 1, column: 3 }); // the newline
        assert_eq!(idx.byte_to_lc(3), LineCol { line: 2, column: 1 });
        assert_eq!(idx.byte_to_lc(6), LineCol { line: 3, column: 1 });
        assert_eq!(idx.byte_to_position(6), Position::new(2, 0));
    }

    #[test]
    fn utf8_multibyte() {
        // Each non-ASCII char below is 2 bytes in UTF-8, 1 UTF-16 unit.
        let idx = LineIndex::new("\u{00e1}b\nc");
        // offset 2 = after á (2 bytes), column should be 2 (code points)
        assert_eq!(idx.byte_to_lc(2), LineCol { line: 1, column: 2 });
        assert_eq!(idx.byte_to_position(2), Position::new(0, 1));
        // offset 3 = at 'b'
        assert_eq!(idx.byte_to_lc(3), LineCol { line: 1, column: 3 });
        assert_eq!(idx.byte_to_position(3), Position::new(0, 2));
    }

    #[test]
    fn utf16_surrogate_pair() {
        // U+1F600 (emoji) is 4 bytes in UTF-8, 2 UTF-16 units (surrogate pair).
        let idx = LineIndex::new("\u{1F600}x");
        // offset 4 = after emoji
        assert_eq!(idx.byte_to_lc(4), LineCol { line: 1, column: 2 });
        assert_eq!(idx.byte_to_position(4), Position::new(0, 2));
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
}
