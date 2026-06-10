//! Parse + incremental-reparse benchmarks.
//!
//! `full_parse` measures a from-scratch parse across file sizes. `incremental`
//! measures `reparse` (token and block strategies) against a full parse of the
//! same edited text — the win the reparse path buys on a large file.
//!
//! Run with `cargo bench --bench parse` (or `task bench-parse`).

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

use ravel::parser::{Edit, parse, reparse};

/// One self-contained R unit: a function with nested blocks, a loop, calls, and
/// subset/extract expressions — a fair mix of constructs to lex and parse.
const UNIT: &str = "process <- function(data, n, weight) {\n  total <- 0\n  for (i in seq_len(n)) {\n    value <- data[[i]] * weight + offset\n    total <- total + value\n  }\n  summary <- list(total = total, mean = total / n)\n  summary\n}\n\n";

fn corpus(units: usize) -> String {
    UNIT.repeat(units)
}

fn full_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_parse");
    for (name, units) in [("small", 1), ("medium", 50), ("large", 500)] {
        let src = corpus(units);
        group.throughput(Throughput::Bytes(src.len() as u64));
        group.bench_function(name, |b| b.iter(|| parse(black_box(&src))));
    }
    group.finish();
}

fn incremental(c: &mut Criterion) {
    let src = corpus(500);
    let parsed = parse(&src);
    let old_root = parsed.cst.clone();
    let diags = parsed.diagnostics.clone();

    // Token edit: insert a character inside the last `weight` identifier.
    let token_at = src.rfind("weight").unwrap() + 1;
    let token_edit = Edit {
        range: token_at..token_at,
        insert: "X".to_string(),
    };

    // Block edit: insert a statement just after the last function body's `{`.
    let block_at = src.rfind("{\n  total <- 0").unwrap() + 1;
    let block_edit = Edit {
        range: block_at..block_at,
        insert: "\n  acc <- acc + 1".to_string(),
    };

    // Sanity: these must actually exercise the intended strategies.
    assert_eq!(
        reparse(&old_root, &src, &diags, &token_edit).map(|r| r.kind),
        Some(ravel::parser::ReparseKind::Token),
    );
    assert_eq!(
        reparse(&old_root, &src, &diags, &block_edit).map(|r| r.kind),
        Some(ravel::parser::ReparseKind::Block),
    );

    let mut group = c.benchmark_group("incremental");
    group.bench_function("reparse_token", |b| {
        b.iter(|| reparse(&old_root, &src, &diags, black_box(&token_edit)))
    });
    group.bench_function("reparse_block", |b| {
        b.iter(|| reparse(&old_root, &src, &diags, black_box(&block_edit)))
    });
    // Reference: a full parse of the same edited text (what reparse replaces).
    let token_new = token_edit.apply(&src);
    group.throughput(Throughput::Bytes(token_new.len() as u64));
    group.bench_function("full_parse_baseline", |b| {
        b.iter(|| parse(black_box(&token_new)))
    });
    group.finish();
}

criterion_group!(benches, full_parse, incremental);
criterion_main!(benches);
