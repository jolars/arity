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

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
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

fn keystroke(c: &mut Criterion) {
    let src = corpus(UNIT, 1024 * 1024);
    // A keystroke in the middle of the buffer: the honest average for the tail
    // shift a patch has to do, and the worst case for a rebuild either way.
    let mut at = src.len() / 2;
    while !src.is_char_boundary(at) {
        at -= 1;
    }

    let mut group = c.benchmark_group("keystroke");
    group.bench_function("rebuild", |b| {
        b.iter(|| {
            let mut text = src.clone();
            text.insert(at, 'x');
            LineIndex::new(black_box(&text))
        })
    });
    // The `didChange` loop re-indexes once *per change*, so a 10-change batch
    // (a multi-cursor edit, or a paste the client splits up) pays 10 rebuilds.
    group.bench_function("batch10/rebuild", |b| {
        b.iter(|| {
            let mut text = src.clone();
            for i in 0..10 {
                let index = LineIndex::new(black_box(&text));
                black_box(&index);
                text.insert(at + i, 'x');
            }
        })
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
    group.bench_function("reparse_only", |b| {
        b.iter(|| reparse(&root, &src, &diags, black_box(&edit)))
    });
    group.bench_function("rebuild_then_reparse", |b| {
        b.iter(|| {
            black_box(LineIndex::new(black_box(&src)));
            reparse(&root, &src, &diags, black_box(&edit))
        })
    });
    group.finish();
}

criterion_group!(benches, build, convert, keystroke, pipeline);
criterion_main!(benches);
