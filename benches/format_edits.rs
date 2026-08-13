//! What line-scoped formatting edits cost, and what they save.
//!
//! `textDocument/formatting` used to answer with one `TextEdit` replacing the
//! whole document. That is always correct, but it makes the client throw away
//! and rebuild everything anchored to the old text — the cursor, the selection,
//! folds, and diagnostic markers — even when a single line changed. The server
//! now line-diffs the formatted output against the buffer and sends a hunk per
//! changed run (`src/lsp/format.rs`), falling back to the whole-document
//! replacement when the diff covers more than half the file.
//!
//! **This buys the client's anchors, not server time.** The `didChange` the
//! client echoes back was measured both ways on the corpus below: the parse
//! costs the same to within noise (0.06 ms when `diff_edit` recovers a tight
//! spanning edit, 2.0 ms when it does not — identical for one whole-document
//! change and for 43 scoped ones), and the buffer splice saves ~20 µs. The
//! staged multi-edit reparse never fires for format hunks, because a whole-line
//! replacement misses the single-edit ladder. Measure again before claiming
//! otherwise.
//!
//! Two questions, and this bench is the evidence for both: what the diff adds
//! to a format request, and how much smaller the answer gets. `payload` is the
//! edits' `new_text` as a share of the document, so 100% is what the old
//! whole-document replacement always sent.
//!
//! Measured on 2026-08-13 (release, otherwise-idle machine). Absolute times
//! drift with CPU frequency across rows, so **the comparison that means
//! something is `format` against `+diff` within a row**:
//!
//! ```text
//! === formatter fixtures, concatenated (62 KB, 4076 lines) ===
//!                                   format        +diff   edits    payload
//! already formatted                7.51 ms      7.45 ms       0      0.0 %
//! one line dirtied                 7.46 ms      8.27 ms       1      0.0 %
//! five lines dirtied               7.60 ms      7.95 ms       5      0.2 %
//! fifty lines dirtied              7.45 ms      8.27 ms      43      2.6 %
//! every assignment dirtied         7.50 ms      8.23 ms     129      5.6 %
//! CRLF, line ending forced LF      7.57 ms      8.32 ms       1     94.1 %
//! ```
//!
//! The diff costs about 10% of the format it follows, which is the price of
//! turning a 62 KB payload into a few hundred bytes. It is charged only when
//! the document actually changed: an already-formatted document short-circuits
//! on the string comparison before any diffing, which is the first row's ~0%.
//!
//! The last row is the one the fallback exists for. Forcing a line-ending
//! conversion changes every line, so there is nothing to scope: the diff
//! collapses to one edit rather than shipping a hunk per line. (Its payload is
//! under 100% only because the CRLF source is longer than the LF output it is
//! measured against.) Real R in the wild is closer to the middle rows — even a
//! file nobody has run through arity comes back as scattered small hunks, so
//! the fallback is for genuine wholesale changes, not for ordinary first
//! contact.
//!
//! Plain `main` (`harness = false`) rather than criterion: the numbers that
//! matter here are a ratio and a payload share, not a distribution.
//!
//! ```sh
//! cargo bench --bench format_edits   # or `task bench-format-edits`
//! ```

use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;

use arity::formatter::{FormatStyle, LineEnding, format_with_options};
use arity::lsp::compute_format_edits;
use arity::parser::ParseOptions;
use arity::text::PositionEncoding;

const UTF16: PositionEncoding = PositionEncoding::Utf16;

/// Every formatter fixture's `expected.R`, in sorted order — the same
/// deterministic base block `scripts/bench.sh` builds its synthetic tiers from,
/// so this bench and the published numbers measure the same bytes.
fn corpus() -> String {
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/arity-formatter/tests/fixtures/formatter");
    let mut files: Vec<PathBuf> = fs::read_dir(&root)
        .expect("formatter fixtures")
        .filter_map(|entry| {
            let path = entry.expect("fixture dir entry").path().join("expected.R");
            path.is_file().then_some(path)
        })
        .collect();
    files.sort();
    files
        .iter()
        .map(|path| fs::read_to_string(path).expect("fixture"))
        .collect()
}

/// Time `f` in a warm loop, returning nanoseconds per iteration.
fn time<T>(iters: usize, mut f: impl FnMut() -> T) -> f64 {
    for _ in 0..(iters / 10).max(1) {
        black_box(f());
    }
    let start = Instant::now();
    for _ in 0..iters {
        black_box(f());
    }
    start.elapsed().as_nanos() as f64 / iters as f64
}

/// `text` with the first `n` ` <- ` assignments tightened to `<-`, which is a
/// change the formatter always undoes — one dirtied line each, spread through
/// the document rather than bunched at the top.
fn dirty(text: &str, n: usize) -> String {
    let mut left = n;
    text.lines()
        .map(|line| {
            if left > 0 && line.contains(" <- ") {
                left -= 1;
                format!("{}\n", line.replacen(" <- ", "<-", 1))
            } else {
                format!("{line}\n")
            }
        })
        .collect()
}

fn row(label: &str, text: &str, style: FormatStyle, iters: usize) {
    let options = ParseOptions::default();
    let format = time(iters, || {
        format_with_options(black_box(text), style, &options)
    });
    let with_diff = time(iters, || {
        compute_format_edits(black_box(text), style, UTF16, &options)
    });
    let edits = compute_format_edits(text, style, UTF16, &options).expect("formats");
    let payload: usize = edits.iter().map(|edit| edit.new_text.len()).sum();
    println!(
        "{label:<30}{:>7.2} ms{:>10.2} ms{:>8}{:>9.1} %",
        format / 1e6,
        with_diff / 1e6,
        edits.len(),
        100.0 * payload as f64 / text.len() as f64,
    );
}

fn main() {
    let style = FormatStyle::default();
    let options = ParseOptions::default();
    // Format the concatenation once: the fixtures are formatted individually,
    // but only their formatted concatenation is a fixed point, and the clean
    // row has to actually be clean.
    let clean = format_with_options(&corpus(), style, &options).expect("corpus formats");
    let lines = clean.lines().count();

    println!(
        "\n=== formatter fixtures, concatenated ({} KB, {lines} lines) ===",
        clean.len() / 1024
    );
    println!(
        "{:<30}{:>10}{:>13}{:>8}{:>11}",
        "", "format", "+diff", "edits", "payload"
    );

    let iters = 30;
    row("already formatted", &clean, style, iters);
    row("one line dirtied", &dirty(&clean, 1), style, iters);
    row("five lines dirtied", &dirty(&clean, 5), style, iters);
    row("fifty lines dirtied", &dirty(&clean, 50), style, iters);
    row(
        "every assignment dirtied",
        &dirty(&clean, usize::MAX),
        style,
        iters,
    );

    // The wholesale case the fallback exists for: every line differs, so hunks
    // would be a hunk per line and the single replacement is what comes back.
    let crlf = clean.replace('\n', "\r\n");
    let force_lf = FormatStyle {
        line_ending: LineEnding::Lf,
        ..style
    };
    row("CRLF, line ending forced LF", &crlf, force_lf, iters);
    println!();
}
