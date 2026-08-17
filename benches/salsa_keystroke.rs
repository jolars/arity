//! The keystroke pipeline: `didChange` splice → salsa handoff → reparse.
//!
//! `benches/line_index.rs` times the *pieces* of a keystroke. This one times
//! them joined up, which is the only way to see cost that lives *between* them —
//! the whole-document copies the language server pays purely to move text from
//! the live [`TextBuffer`] into the salsa input, and the whole-document compares
//! the staleness guards pay to decide whether the text moved at all. A change to
//! how document text is stored shows up here and nowhere else, so prefer this
//! bench over `line_index` when judging one.
//!
//! Three rows per corpus size:
//!
//! - `upsert_unchanged` — the staleness guard alone. Salsa's setter never
//!   compares, so `upsert_file` guards the write itself; every re-lint of an
//!   unedited buffer (a `RelintAll` fan-out, a `didSave`, a sibling file) pays
//!   this. It is pure overhead and should be flat.
//! - `write_phase` — splice + `upsert_file` + `stage_edits`: what the lint
//!   thread runs before handing off, with no parse demanded.
//! - `keystroke` — the above plus `parsed_tree`, so the write phase can be read
//!   against the reparse it precedes. That ratio is the one that decides whether
//!   a cost added to the splice matters.
//!
//! Each iteration alternates an insert and a delete of one character, so every
//! round is a genuine text change (a fresh salsa revision, never a memoized
//! no-op) and two rounds return the database to its starting state. That is what
//! lets the timed loop run against one persistent database with no per-iteration
//! setup.
//!
//! Run with `cargo bench --bench salsa_keystroke`.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use std::path::PathBuf;

use arity::incremental::IncrementalDatabase;
use arity::parser::Edit;
use arity::text::TextBuffer;

/// One self-contained R unit, matching the parser and line-index benches so the
/// rows compare like with like.
const UNIT: &str = "process <- function(data, n, weight) {\n  total <- 0\n  for (i in seq_len(n)) {\n    value <- data[[i]] * weight + offset\n    total <- total + value\n  }\n  summary <- list(total = total, mean = total / n)\n  summary\n}\n\n";

/// A corpus of at least `bytes` bytes, built by repeating `unit`.
fn corpus(unit: &str, bytes: usize) -> String {
    unit.repeat(bytes.div_ceil(unit.len()))
}

/// A byte offset *inside an identifier* roughly `frac` of the way through
/// `text`.
///
/// A bare fraction is not good enough: at some corpus sizes it lands in the
/// blank line between two top-level statements, where inserting a character
/// creates a whole new statement and no tier below a full reparse applies. The
/// row would then time a full parse on every other iteration — 19.7 ms rather
/// than 24 µs at 1 MB, which is a fact about where the offset fell, not about
/// the pipeline. Anchoring on a token keeps the edit inside one expression at
/// every size.
fn edit_site(text: &str, frac: f64) -> usize {
    let mark = (text.len() as f64 * frac) as usize;
    let anchor = text[..mark]
        .rfind("weight")
        .expect("the corpus unit contains the anchor");
    anchor + 3
}

const SIZES: [(&str, usize); 2] = [("130k", 130 * 1024), ("1m", 1024 * 1024)];

/// The one line that varies between text-storage strategies: how the live
/// buffer's text reaches the salsa input.
///
/// - an owned `String` document: `buffer.text().to_string()`, an O(N) copy
/// - a shared `Arc<str>` document: `buffer.text_arc()`, a refcount bump
fn handoff(buffer: &TextBuffer) -> String {
    buffer.text().to_string()
}

/// The staleness guard alone: re-upserting text the database already holds.
///
/// Deliberately *not* a `black_box` over the whole call — the point is the
/// compare inside `upsert_file`, and the returned `SourceFile` is a copy handle
/// that cannot be optimized away.
fn upsert_unchanged(c: &mut Criterion) {
    let mut group = c.benchmark_group("upsert_unchanged");
    for (name, bytes) in SIZES {
        let src = corpus(UNIT, bytes);
        let path = PathBuf::from("bench.R");
        let mut db = IncrementalDatabase::default();
        let buffer = TextBuffer::from(src.as_str());
        db.upsert_file(&path, handoff(&buffer));

        group.throughput(Throughput::Bytes(src.len() as u64));
        group.bench_function(name, |b| {
            b.iter(|| black_box(db.upsert_file(black_box(&path), handoff(&buffer))))
        });
    }
    group.finish();
}

/// Splice plus handoff, with no parse demanded — the lint thread's write phase.
fn write_phase(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_phase");
    for (name, bytes) in SIZES {
        let src = corpus(UNIT, bytes);
        let path = PathBuf::from("bench.R");
        // Edit late in the buffer: an early edit would understate the index
        // patch and the tail copy alike.
        let at = edit_site(&src, 0.8);
        let mut db = IncrementalDatabase::default();
        let mut buffer = TextBuffer::from(src.as_str());
        let file = db.upsert_file(&path, handoff(&buffer));
        let mut inserted = false;

        group.throughput(Throughput::Bytes(src.len() as u64));
        group.bench_function(name, |b| {
            b.iter(|| {
                // Alternate insert and delete so each round is a real change and
                // every pair leaves the buffer as it was found.
                let edit = if inserted {
                    buffer.apply_edit(at..at + 1, "");
                    Edit {
                        range: at..at + 1,
                        insert: String::new(),
                    }
                } else {
                    buffer.apply_edit(at..at, "x");
                    Edit {
                        range: at..at,
                        insert: "x".to_string(),
                    }
                };
                inserted = !inserted;
                db.upsert_file(&path, handoff(&buffer));
                db.stage_edits(file, vec![edit]);
            })
        });

        // The loop leaves a staged edit and, on an odd iteration count, a
        // one-character difference. Neither escapes this scope.
        db.stage_edits(file, Vec::new());
    }
    group.finish();
}

/// The write phase plus the reparse it precedes: one whole keystroke.
fn keystroke(c: &mut Criterion) {
    let mut group = c.benchmark_group("keystroke");
    for (name, bytes) in SIZES {
        let src = corpus(UNIT, bytes);
        let path = PathBuf::from("bench.R");
        let at = edit_site(&src, 0.8);
        let mut db = IncrementalDatabase::default();
        let mut buffer = TextBuffer::from(src.as_str());
        let file = db.upsert_file(&path, handoff(&buffer));
        // Prime the reparse base, so the first timed round measures the
        // incremental ladder rather than a cold full parse.
        let _ = db.parsed_tree(file);
        let mut inserted = false;

        group.throughput(Throughput::Bytes(src.len() as u64));
        group.bench_function(name, |b| {
            b.iter(|| {
                let edit = if inserted {
                    buffer.apply_edit(at..at + 1, "");
                    Edit {
                        range: at..at + 1,
                        insert: String::new(),
                    }
                } else {
                    buffer.apply_edit(at..at, "x");
                    Edit {
                        range: at..at,
                        insert: "x".to_string(),
                    }
                };
                inserted = !inserted;
                db.upsert_file(&path, handoff(&buffer));
                db.stage_edits(file, vec![edit]);
                black_box(db.parsed_tree(file));
            })
        });
    }
    group.finish();
}

criterion_group!(benches, upsert_unchanged, write_phase, keystroke);
criterion_main!(benches);
