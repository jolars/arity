# Benchmarks

Wall-clock speed of `arity` against other R tooling, measured with [hyperfine].
Two operations are covered:

- the **formatter**, compared against [`air`](https://github.com/posit-dev/air)
  and [`styler`](https://styler.r-lib.org/);
- the **linter**, compared against
  [`jarl`](https://github.com/etiennebacher/jarl) and
  [`lintr`](https://lintr.r-lib.org/).

Each operation is measured at two scopes: single files (the largest source file
of each benchmarked package, plus two synthetic corpus tiers) and whole projects
(real R packages). `arity` is the baseline in every chart, and every other
tool's time is reported relative to it.

The tools also pay very different startup floors: `styler` and `lintr` run
inside an R process, so a large part of their time on small inputs is
interpreter startup rather than real work. Treat the *ratios*, not the absolute
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
  | `lintr`  | `Rscript -e 'lintr::lint(FILE)'`                            |

For projects, each tool walks the package's `R/` source tree in one invocation.
Formatters run in check mode so nothing is mutated, but the full formatting work
is still done:

  | Tool    | Invocation                                  |
  | ------- | ------------------------------------------- |
  | `arity` | `arity format --check R/` / `arity lint R/` |
  | `air`   | `air format --check R/`                     |
  | `jarl`  | `jarl check R/`                             |
  | `lintr` | `Rscript -e 'lintr::lint_dir("R/")'`        |

`arity` is the baseline; every other tool's time is reported relative to it.
Comparison tools absent from the machine are skipped, so a run without `jarl`
simply omits it from the linter charts. The timing backend prefers `hyperfine`
(warmup plus stddev/min/max); without `hyperfine` and `jq` it falls back to a
mean-only shell loop and the min/max columns become blank.

The R-backed tools need two caveats. `styler` and `lintr` pay an interpreter
startup floor plus a steep per-line cost, so they are skipped on documents above
20,000 lines to keep a run tractable; `styler` is additionally absent from the
project charts, because `style_dir` would rewrite the checkout and it has no
check-only directory mode. `styler`'s persistent on-disk cache of already-styled
expressions is deactivated for the measurement, so that every run does the full
work rather than inheriting whatever an earlier run happened to style. Because
the tools do such different work, this is a rough scale comparison, not a
like-for-like one. Set `ARITY_BENCH_NO_R=1` to leave both out of a run.

## Corpus

**Single files** mix real and synthetic input. The real documents are the
largest source file of each benchmarked package, which is the closest stand-in
for the per-file work an editor asks of a formatter or linter. The synthetic
tiers are built by concatenating every formatter fixture's `expected.R`
(`crates/arity-formatter/tests/fixtures/formatter/*/expected.R`, sorted,
blank-line separated) into a base block and repeating it to two sizes. That
content repeats, so it is cache-friendly and not representative of real code; it
exists to amortize process startup and show rough scaling.

**Projects** use real R packages: the [`tidyr`](https://tidyr.tidyverse.org/)
and [`MASS`](https://cran.r-project.org/package=MASS) source trees, cloned once
at pinned tags into a local cache. The two are deliberately unalike---`tidyr` is
modern tidyverse code, `MASS` is long-lived base-R-style code. Point the
benchmark at your own checkout with
`ARITY_BENCH_PROJECT=/path/to/pkg task bench`, which replaces the list entirely;
only a package's `R/` directory is measured.

## Setup

{{#include benchmarks_meta.md}}

## Results

Each operation gets its own section below, split into single files and whole
projects. The `arity` baseline sits on the dashed line at 1 in every chart;
faster tools fall below it, slower tools rise above.

{{#include benchmarks_results.md}}
