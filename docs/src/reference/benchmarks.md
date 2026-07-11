# Benchmarks

Wall-clock speed of `arity` against other R tooling, measured with [hyperfine].
Two operations are covered:

- the **formatter**, compared against [`air`](https://github.com/posit-dev/air)
  (with [`styler`](https://styler.r-lib.org/) available opt-in);
- the **linter**, compared against
  [`jarl`](https://github.com/etiennebacher/jarl).

Each operation is measured at two scopes: **single files** (synthetic corpus
tiers) and a whole **project** (a real R package). `arity` is the baseline in
every chart, and every other tool's time is reported relative to it.

**This is not a CI gate and not a parity target.** Timings are machine- and
run-dependent, and these numbers measure *speed only*, never output or finding
equivalence (see `AIR_COMPAT.md` or `task air-compat` for formatter output
comparison). The tools also pay very different startup floors: `styler` runs
inside an R process, so a large part of its time on small inputs is interpreter
startup rather than real work. Treat the *ratios*, not the absolute
milliseconds, as the takeaway.

The figures below are regenerated manually with `task bench` and committed as a
machine-readable artifact (`benches/benchmark_results.json`); they are never
re-measured when this site is built or in CI.

[hyperfine]: https://github.com/sharkdp/hyperfine

## How it is measured

For single files, each tool is invoked as a user would pipe (formatters) or
point it at a file (linters):

  | Tool     | Invocation                                                  |
  | -------- | ----------------------------------------------------------- |
  | `arity`  | `arity format` / `arity lint FILE`                          |
  | `air`    | `air format --stdin-file-path bench.R`                      |
  | `styler` | `Rscript -e 'styler::style_text(readLines(file("stdin")))'` |
  | `jarl`   | `jarl check FILE`                                           |

For projects, each tool walks the package's `R/` source tree in one invocation.
Formatters run in check mode so nothing is mutated, but the full formatting work
is still done:

  | Tool    | Invocation                                  |
  | ------- | ------------------------------------------- |
  | `arity` | `arity format --check R/` / `arity lint R/` |
  | `air`   | `air format --check R/`                     |
  | `jarl`  | `jarl check R/`                             |

`arity` is the baseline; every other tool's time is reported relative to it.
Comparison tools absent from the machine are skipped, so a run without `jarl`
simply omits the linter comparison. The timing backend prefers `hyperfine`
(warmup plus stddev/min/max); without `hyperfine` and `jq` it falls back to a
mean-only shell loop and the min/max columns become blank.

`styler` is an R package: it pays an interpreter startup floor plus a steep
per-line cost, so it is **not** measured by default and only ever appears on the
formatter single-file tiers (never on projects, where `style_dir` would mutate
the checkout). Opt in with `ARITY_BENCH_STYLER=1 task bench`; even then it is
skipped on tiers too large to format in reasonable time. Because the tools do
such different work, this is a rough scale comparison, not a like-for-like one.

## Corpus

**Single files** are synthetic: every `tests/fixtures/formatter/*/expected.R` is
concatenated (sorted, blank-line separated) into a base block, which is repeated
to two size tiers. The content repeats, so it is cache-friendly and not fully
representative of real code; it exists to amortize process startup and show
rough scaling, not to model a real workload.

**Projects** use a real R package (the [`tidyr`](https://tidyr.tidyverse.org/)
source tree by default), cloned once at a pinned tag into a local cache. Point
the benchmark at your own checkout with
`ARITY_BENCH_PROJECT=/path/to/pkg task bench`; only its `R/` directory is
measured.

## Setup

{{#include benchmarks_meta.md}}

## Results

Each operation gets its own section below, split into single files and a whole
project. The `arity` baseline sits on the dashed line at 1 in every chart;
faster tools fall below it, slower tools rise above.

{{#include benchmarks_results.md}}
