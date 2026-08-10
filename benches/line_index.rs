//! Line-index construction, patching, and conversion benchmarks.
//!
//! `LineIndex::new` is linear in the *document*, not in the edit, and the LSP
//! live buffer pays it repeatedly: once per ranged change in the `didChange`
//! loop, then again in every handler answering against the buffer. These groups
//! size that cost and the patch-on-edit alternative:
//!
//! - `build` / `build_wide` — from-scratch construction across file sizes.
//! - `convert` — `byte_to_position` / `position_to_byte`, the conversion path a
//!   diagnostic-heavy file walks thousands of times. Guards that making the
//!   wide-char lookup a binary search does not regress the ASCII case.
//! - `keystroke` — one edit's index cost, rebuild versus patch.
//! - `keystroke_batch` — the same at 10 content changes, which is what the
//!   `didChange` loop actually does (it re-indexes *per change*).
//! - `pipeline` — index cost next to the incremental reparse it precedes. This
//!   is the ratio that decides whether patching is worth anything.
//!
//! Run with `cargo bench --bench line_index` (or `task bench-line-index`).

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

use arity::parser::{Edit, parse, reparse};
use arity::text::{LineIndex, PositionEncoding};

/// One self-contained R unit, matching the parser bench's corpus so the
/// `pipeline` group compares like with like.
const UNIT: &str = "process <- function(data, n, weight) {\n  total <- 0\n  for (i in seq_len(n)) {\n    value <- data[[i]] * weight + offset\n    total <- total + value\n  }\n  summary <- list(total = total, mean = total / n)\n  summary\n}\n\n";

/// The same unit with a CJK comment and a non-ASCII string literal, so the
/// wide-char table is densely populated rather than empty.
const WIDE_UNIT: &str = "# 数据处理函数 — 计算加权总和\nprocess <- function(data, n, weight) {\n  label <- \"平均值（加权）\"\n  total <- 0\n  for (i in seq_len(n)) {\n    total <- total + data[[i]] * weight\n  }\n  list(label = label, total = total)\n}\n\n";

/// A corpus of at least `bytes` bytes, built by repeating `unit`.
fn corpus(unit: &str, bytes: usize) -> String {
    unit.repeat(bytes.div_ceil(unit.len()))
}

const SIZES: [(&str, usize); 3] = [
    ("16k", 16 * 1024),
    ("130k", 130 * 1024),
    ("1m", 1024 * 1024),
];

fn build(c: &mut Criterion) {
    let mut group = c.benchmark_group("build");
    for (name, bytes) in SIZES {
        let src = corpus(UNIT, bytes);
        group.throughput(Throughput::Bytes(src.len() as u64));
        group.bench_function(name, |b| b.iter(|| LineIndex::new(black_box(&src))));
    }
    group.finish();

    let mut group = c.benchmark_group("build_wide");
    let src = corpus(WIDE_UNIT, 130 * 1024);
    group.throughput(Throughput::Bytes(src.len() as u64));
    group.bench_function("130k", |b| b.iter(|| LineIndex::new(black_box(&src))));
    group.finish();
}

fn convert(c: &mut Criterion) {
    let mut group = c.benchmark_group("convert");
    for (name, unit) in [("ascii", UNIT), ("cjk", WIDE_UNIT)] {
        let src = corpus(unit, 130 * 1024);
        let index = LineIndex::new(&src);
        // Spread the probes over the whole buffer so neither the binary search
        // nor the per-line scan is measured at a lucky offset. Snap to char
        // boundaries so `byte_to_position` is not fed a split code point.
        let offsets: Vec<usize> = (0..1000)
            .map(|i| {
                let mut off = src.len() * i / 1000;
                while !src.is_char_boundary(off) {
                    off -= 1;
                }
                off
            })
            .collect();
        let positions: Vec<_> = offsets
            .iter()
            .map(|&o| index.byte_to_position(o, PositionEncoding::Utf16))
            .collect();

        group.bench_function(format!("byte_to_position/{name}"), |b| {
            b.iter(|| {
                for &off in &offsets {
                    black_box(index.byte_to_position(black_box(off), PositionEncoding::Utf16));
                }
            })
        });
        group.bench_function(format!("position_to_byte/{name}"), |b| {
            b.iter(|| {
                for &pos in &positions {
                    black_box(index.position_to_byte(black_box(pos), PositionEncoding::Utf16));
                }
            })
        });
    }
    group.finish();
}

/// A char-boundary offset at `frac` of the way through `text`.
fn boundary_at(text: &str, frac: f64) -> usize {
    let mut at = (text.len() as f64 * frac) as usize;
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn keystroke(c: &mut Criterion) {
    let src = corpus(UNIT, 1024 * 1024);
    // Mid-buffer: the honest average for the tail shift a patch has to do.
    let at = boundary_at(&src, 0.5);
    let index = LineIndex::new(&src);

    // Both arms time *index work only* — the text splice is common to either
    // strategy, so including it would understate the difference.
    let mut group = c.benchmark_group("keystroke");
    group.bench_function("rebuild", |b| b.iter(|| LineIndex::new(black_box(&src))));
    group.bench_function("patch", |b| {
        b.iter_batched_ref(
            || index.clone(),
            |idx| idx.apply_edit(black_box(at..at), "x"),
            BatchSize::SmallInput,
        )
    });
    // The `didChange` loop re-indexes once *per change*, so a 10-change batch
    // (a multi-cursor edit, or a paste the client splits up) pays 10 rebuilds.
    group.bench_function("batch10/rebuild", |b| {
        b.iter(|| {
            for _ in 0..10 {
                black_box(LineIndex::new(black_box(&src)));
            }
        })
    });
    group.bench_function("batch10/patch", |b| {
        b.iter_batched_ref(
            || index.clone(),
            |idx| {
                for i in 0..10 {
                    idx.apply_edit(black_box(at + i..at + i), "x");
                }
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

fn patch(c: &mut Criterion) {
    // Where the edit lands decides how much tail has to shift: offset 0 is the
    // worst case, the end is the best.
    let src = corpus(UNIT, 1024 * 1024);
    let index = LineIndex::new(&src);
    let mut group = c.benchmark_group("patch");

    for (name, frac) in [("start", 0.0), ("middle", 0.5), ("end", 1.0)] {
        let at = boundary_at(&src, frac);
        group.bench_function(name, |b| {
            b.iter_batched_ref(
                || index.clone(),
                |idx| idx.apply_edit(black_box(at..at), "x"),
                BatchSize::SmallInput,
            )
        });
    }

    let at = boundary_at(&src, 0.5);
    group.bench_function("newline", |b| {
        b.iter_batched_ref(
            || index.clone(),
            |idx| idx.apply_edit(black_box(at..at), "\n"),
            BatchSize::SmallInput,
        )
    });
    // Delete the line containing `at`, newline included.
    let line_start = src[..at].rfind('\n').map_or(0, |i| i + 1);
    let line_end = src[at..].find('\n').map_or(src.len(), |i| at + i + 1);
    group.bench_function("delete_line", |b| {
        b.iter_batched_ref(
            || index.clone(),
            |idx| idx.apply_edit(black_box(line_start..line_end), ""),
            BatchSize::SmallInput,
        )
    });
    let paste = "value <- transform(data)\n".repeat(100);
    group.bench_function("paste_100_lines", |b| {
        b.iter_batched_ref(
            || index.clone(),
            |idx| idx.apply_edit(black_box(at..at), &paste),
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

fn pipeline(c: &mut Criterion) {
    // The go/no-go number: what fraction of a keystroke's cost is the index,
    // measured against the incremental reparse it precedes.
    let src = corpus(UNIT, 1024 * 1024);
    let parsed = parse(&src);
    let root = parsed.cst.clone();
    let diags = parsed.diagnostics.clone();

    let at = src.rfind("weight").unwrap() + 1;
    let edit = Edit {
        range: at..at,
        insert: "X".to_string(),
    };
    // Sanity: this must exercise the cheap strategy, else the ratio is meaningless.
    assert_eq!(
        reparse(&root, &src, &diags, &edit).map(|r| r.kind),
        Some(arity::parser::ReparseKind::Token),
    );

    let mut group = c.benchmark_group("pipeline");
    // All three arms run under `iter_batched_ref` with the same clone-in-setup.
    // That matters for more than symmetry: `iter` drops the returned CST inside
    // the timed region while `iter_batched_ref` drops it after, and dropping a
    // green tree is not free — mixing the two understates the reparse by ~30%.
    let index = LineIndex::new(&src);
    group.bench_function("reparse_only", |b| {
        b.iter_batched_ref(
            || index.clone(),
            |_| reparse(&root, &src, &diags, black_box(&edit)),
            BatchSize::SmallInput,
        )
    });
    group.bench_function("rebuild_then_reparse", |b| {
        b.iter_batched_ref(
            || index.clone(),
            |idx| {
                *idx = LineIndex::new(black_box(&src));
                reparse(&root, &src, &diags, black_box(&edit))
            },
            BatchSize::SmallInput,
        )
    });
    group.bench_function("patch_then_reparse", |b| {
        b.iter_batched_ref(
            || index.clone(),
            |idx| {
                idx.apply_edit(black_box(at..at), "X");
                reparse(&root, &src, &diags, black_box(&edit))
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

criterion_group!(benches, build, convert, keystroke, patch, pipeline);
criterion_main!(benches);
