//! Text paired with its line index, kept in sync across edits.
//!
//! An open LSP document is a [`TextBuffer`] behind an `Arc`. Three properties
//! follow from that and are what the LSP layer relies on:
//!
//! - **Text and index never disagree.** Every mutation goes through
//!   [`TextBuffer::apply_edit`], which splices both. Handing out `&str` and
//!   `&LineIndex` separately would let a caller pair one buffer's text with
//!   another's index, and a mismatch there is a silently wrong position rather
//!   than a compile error.
//! - **A shared buffer is immutable.** The main loop mutates only a uniquely
//!   owned one (via `Arc::make_mut`), so a read job holding an `Arc` observes
//!   exactly the bytes of the version it was dispatched at.
//! - **The text itself is shared past the buffer.** It is an `Arc<str>`, and the
//!   salsa `SourceFile` input and the reparse base hold that same allocation, so
//!   handing the document to the analysis layer is a refcount bump rather than a
//!   copy, and the staleness guards can settle the common case with
//!   `Arc::ptr_eq`. The price is that an edit rebuilds the string instead of
//!   splicing in place — see [`TextBuffer::apply_edit`].

use std::ops::Range;
use std::sync::Arc;

use crate::text::LineIndex;

/// A text buffer and its [`LineIndex`], maintained together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextBuffer {
    text: Arc<str>,
    index: LineIndex,
}

impl TextBuffer {
    pub fn new(text: impl Into<Arc<str>>) -> Self {
        let text = text.into();
        let index = LineIndex::new(&text);
        Self { text, index }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The text as a shared handle: an O(1) clone, for the salsa boundary and
    /// anything else that *stores* the document rather than borrowing it.
    ///
    /// Reintroducing a `text().to_string()` on a dispatch path is the regression
    /// this exists to prevent.
    pub fn text_arc(&self) -> Arc<str> {
        Arc::clone(&self.text)
    }

    /// The line index for [`text`](Self::text). Always current: it is patched in
    /// step with every edit, never rebuilt per request.
    pub fn line_index(&self) -> &LineIndex {
        &self.index
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Replace `range` with `insert`, splicing the index rather than rebuilding
    /// it.
    ///
    /// `range` is **clamped** into the buffer and de-inverted first. A client
    /// may send a `didChange` whose range is inverted or past the end, and both
    /// are nonsense a server must not die on, so the range is coerced rather
    /// than trusted. Offsets are snapped to char boundaries for the same reason.
    ///
    /// The text is rebuilt rather than spliced in place: an `Arc<str>` cannot be
    /// mutated, and sharing one allocation with the salsa layer is worth more
    /// than an in-place splice (see the module docs). This is the one linear
    /// pass over the document a keystroke pays for text, and it sits in front of
    /// a reparse that costs an order of magnitude more.
    ///
    /// The clamp above is what makes the capacity arithmetic safe — with `start
    /// <= end <= len` guaranteed it can neither underflow nor size a buffer that
    /// duplicates a region. Keep the rebuild below it.
    pub fn apply_edit(&mut self, range: Range<usize>, insert: &str) {
        let mut start = range.start.min(self.text.len());
        let mut end = range.end.clamp(start, self.text.len());
        while !self.text.is_char_boundary(start) {
            start -= 1;
        }
        while !self.text.is_char_boundary(end) {
            end += 1;
        }

        let mut next = String::with_capacity(self.text.len() - (end - start) + insert.len());
        next.push_str(&self.text[..start]);
        next.push_str(insert);
        next.push_str(&self.text[end..]);
        self.text = Arc::from(next);
        self.index.apply_edit(start..end, insert);

        // The patch must land exactly where a rebuild would; anything else is a
        // wrong position reported to the editor. Cheap insurance in debug
        // builds, and it turns every LSP test into a check on `apply_edit`.
        debug_assert_eq!(self.index, LineIndex::new(&self.text));
    }
}

impl From<String> for TextBuffer {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

impl From<&str> for TextBuffer {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl From<Arc<str>> for TextBuffer {
    fn from(text: Arc<str>) -> Self {
        Self::new(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_edit_keeps_text_and_index_in_sync() {
        let mut buf = TextBuffer::from("ab\ncd\nef");
        buf.apply_edit(3..5, "XYZ\nW");
        assert_eq!(buf.text(), "ab\nXYZ\nW\nef");
        assert_eq!(buf, TextBuffer::from("ab\nXYZ\nW\nef"));
    }

    #[test]
    fn an_inverted_range_is_coerced_rather_than_panicking() {
        let mut buf = TextBuffer::from("ab\ncd");
        // Built from variables: a literal `4..1` trips `reversed_empty_ranges`,
        // and an inverted range is exactly what this test is about.
        let (start, end) = (4, 1);
        buf.apply_edit(start..end, "X");
        // The inverted end collapses onto the start: an insertion at 4.
        assert_eq!(buf.text(), "ab\ncXd");
        assert_eq!(buf, TextBuffer::from("ab\ncXd"));
    }

    #[test]
    fn a_range_past_the_end_clamps() {
        let mut buf = TextBuffer::from("ab");
        buf.apply_edit(1..99, "X");
        assert_eq!(buf.text(), "aX");
        assert_eq!(buf, TextBuffer::from("aX"));
    }

    #[test]
    fn an_offset_inside_a_wide_char_snaps_to_a_boundary() {
        // Splitting the emoji would panic `replace_range`; snap outward instead.
        let mut buf = TextBuffer::from("a\u{1F600}b");
        buf.apply_edit(2..3, "");
        assert_eq!(buf.text(), "ab");
        assert_eq!(buf, TextBuffer::from("ab"));
    }

    #[test]
    fn a_long_edit_script_stays_in_sync() {
        // A seeded LCG walk: hundreds of arbitrary edits over text mixing ASCII,
        // a 2-byte char, and an astral char, each checked against a rebuild.
        // This is the test that would catch a patch bug the fixed cases miss.
        let mut buf = TextBuffer::from("x <- 1\ny <- \"\u{00e1}\"\nz <- \"\u{1F600}\"\n");
        let inserts = [
            "",
            "q",
            "\n",
            "\nq",
            "q\n",
            "\u{00e1}",
            "\u{1F600}",
            "a\nb\nc",
            "\r\n",
        ];
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 33) as usize
        };

        for step in 0..500 {
            let len = buf.len();
            let mut start = if len == 0 { 0 } else { next() % (len + 1) };
            let mut end = if len == 0 { 0 } else { next() % (len + 1) };
            if start > end {
                std::mem::swap(&mut start, &mut end);
            }
            let insert = inserts[next() % inserts.len()];
            buf.apply_edit(start..end, insert);
            assert_eq!(
                buf.line_index(),
                &LineIndex::new(buf.text()),
                "step {step}: {start}..{end} insert {insert:?} text {:?}",
                buf.text()
            );
        }
    }
}
