# Benchmarks

Wall-clock formatting speed of `arity` against other R formatters, measured
with [hyperfine]. The default comparison is against
[`air`](https://github.com/posit-dev/air);
[`styler`](https://styler.r-lib.org/) can be added opt-in (see below). Every
tool formats stdin to stdout (exit 0 regardless of changes), so the comparison
is free of file-mutation and exit-code noise.

**This is not a CI gate and not a parity target.** Timings are machine- and
run-dependent, and these numbers measure *speed only*, never output equivalence
(see `AIR_COMPAT.md` or `task air-compat` for that). The tools also pay very
different startup floors: `styler` runs inside an R process, so a large part of
its time on small inputs is interpreter startup rather than formatting work.
Treat the *ratios*, not the absolute milliseconds, as the takeaway.

The figures below are regenerated manually with `task bench` and committed as a
machine-readable artifact (`benches/benchmark_results.json`); they are never
re-measured when this site is built or in CI.

[hyperfine]: https://github.com/sharkdp/hyperfine

## How it is measured

Each tool is invoked exactly as a user would pipe a file through it, stdin to
stdout:

| Tool     | Invocation                                              |
| -------- | ------------------------------------------------------- |
| `arity`  | `arity format`                                          |
| `air`    | `air format --stdin-file-path bench.R`                  |
| `styler` | `Rscript -e 'styler::style_text(readLines(file("stdin")))'` |

`arity` is the baseline; every other tool's time is reported relative to it.
Comparison tools absent from the machine are skipped, so a run with only `air`
installed compares just `arity` and `air`. The timing backend prefers
`hyperfine` (warmup plus stddev/min/max); without `hyperfine` and `jq` it falls
back to a mean-only shell loop and the min/max columns become blank.

`styler` is an R package: it pays an interpreter startup floor plus a steep
per-line cost (seconds even on the small tier), so it is **not** measured by
default. Opt in with `ARITY_BENCH_STYLER=1 task bench`; even then it is skipped
on tiers too large to format in reasonable time, so it may appear for only some
tiers. Because the tools do such different work, this is a rough scale
comparison, not a like-for-like one.

## Corpus

The corpus is synthetic: every `tests/fixtures/formatter/*/expected.R` is
concatenated (sorted, blank-line separated) into a base block, which is repeated
to two size tiers. The content repeats, so it is cache-friendly and not fully
representative of real code; it exists to amortize process startup and show
rough scaling, not to model a real workload. (Pass `ARITY_BENCH_INPUT` to
benchmark a single real file instead.)

## Setup

{{#include benchmarks_meta.md}}

## Results

{{#include benchmarks_results.md}}
