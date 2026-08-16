//! Lexer-only benchmarks.
//!
//! `full_parse` (benches/parse.rs) conflates lexing, parsing, and green-tree
//! building, so lexer-level changes (token representation, allocation
//! behavior) drown in the other phases there. This bench drives the lexer
//! alone, over corpora weighted toward the token mixes that dominate real R
//! sources: mixed code, operator-dense code, `$`/`::` access chains, and
//! roxygen documentation (plain and markdown).
//!
//! Run with `cargo bench -p arity-parser --bench lex`.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

use arity_parser::parser::lex_token_count;

/// Same shape as the parse bench's unit: a function with nested blocks, a
/// loop, calls, and subset/extract expressions.
const MIXED_UNIT: &str = "process <- function(data, n, weight) {\n  total <- 0\n  for (i in seq_len(n)) {\n    value <- data[[i]] * weight + offset\n    total <- total + value\n  }\n  summary <- list(total = total, mean = total / n)\n  summary\n}\n\n";

/// Operator-dense code: user ops, comparison chains, pipes, and literals with
/// every numeric suffix, so the short fixed-text token paths dominate.
const OPERATOR_UNIT: &str = "res <- a %% b %/% c %*% d; ok <- x <= 1 && y >= 2 || !z != w\nv[i] <- lst[[k]]$field@slot; p <- obj |> f() |> g()\nq <- pkg::fn(1L, 2.5, 3e-4, .5i) - x^2 ** y\n";

/// `$`/`::`/`[[` access chains over longer identifiers, so the identifier
/// slice path dominates.
const FIELD_UNIT: &str = "out$alpha <- frame$column_one + frame$column_two\nstats::median(frame[[\"column_one\"]], na.rm = TRUE)\nconfig$paths$root <- normalizePath(config$paths$root)\n";

/// A plain roxygen block: the sub-lexer carves markers, tags, and prose.
const ROXYGEN_UNIT: &str = "#' Compute weighted totals over a data frame.\n#'\n#' @param data A data frame with numeric columns.\n#' @param weight A single numeric weight applied to every row.\n#' @return A list with the total and the mean.\n#' @examples\n#' process(mtcars, 0.5)\n#' @export\nprocess <- function(data, weight) NULL\n\n";

/// The same block under `@md`, so the markdown inline recognizers (code
/// spans, links, emphasis) run too.
const ROXYGEN_MD_UNIT: &str = "#' Compute *weighted* totals over a data frame.\n#'\n#' @md\n#' @param data A data frame with **numeric** columns.\n#' @param weight A single numeric weight, see [base::sum()].\n#' @return A `list` with the total and the mean.\n#' @export\nprocess <- function(data, weight) NULL\n\n";

/// Repeat `unit` to roughly 100 KiB so every corpus is the same order of
/// magnitude and per-byte throughput is comparable across cases.
fn corpus(unit: &str) -> String {
    const TARGET_BYTES: usize = 100 * 1024;
    unit.repeat(TARGET_BYTES.div_ceil(unit.len()))
}

fn lex(c: &mut Criterion) {
    let mut group = c.benchmark_group("lex");
    for (name, unit) in [
        ("mixed", MIXED_UNIT),
        ("operators", OPERATOR_UNIT),
        ("field_access", FIELD_UNIT),
        ("roxygen", ROXYGEN_UNIT),
        ("roxygen_md", ROXYGEN_MD_UNIT),
    ] {
        let src = corpus(unit);
        group.throughput(Throughput::Bytes(src.len() as u64));
        group.bench_function(name, |b| b.iter(|| lex_token_count(black_box(&src))));
    }
    group.finish();
}

criterion_group!(benches, lex);
criterion_main!(benches);
