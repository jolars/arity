# Performance benchmarks

The [performance guide](https://arity.cc/guide/performance.html) shows results
from `benchmark_results.json`. To refresh both comparisons and regenerate the
charts, run `task bench` from the repository root. To refresh them separately,
run `task bench-cli` or `task bench-lsp`, followed by `task docs-gen`. Each
suite preserves the other's measurements in the shared artifact. The site build
reads the artifact, and benchmarks are opt-in.

See [performance measurements](PERFORMANCE.md) for profiling, microbenchmarks,
and comparisons between Arity builds.

## Formatter and linter

`task bench-cli` builds Arity in release mode and measures wall-clock time with
[hyperfine] and `jq`. If either is missing, the runner uses a shell loop that
reports the mean only. It skips unavailable comparison tools. Install Air and
Jarl on `PATH` and styler and lintr in the library used by `Rscript` to run the
full comparison. The runner needs Python 3 to preserve existing LSP results.

```sh
task bench-cli
ARITY_BENCH_NO_R=1 task bench-cli
ARITY_BENCH_PROJECT=/path/to/pkg task bench-cli
```

`ARITY_BENCH_NO_R=1` skips styler and lintr. To save a trial run separately, use
`./scripts/bench.sh --out target/bench-cli-trial.json`.

### Corpus

The runner clones tidyr v1.3.2 and MASS 7.3-66 at pinned tags. It caches them
under `$XDG_CACHE_HOME/arity-bench` (by default `~/.cache/arity-bench`), or at
the path set in `ARITY_BENCH_CACHE`. Set `ARITY_BENCH_PROJECT` to replace both
packages with one checkout. The largest source file in each package's `R/`
directory is used for the single-file runs. The project runs cover the full `R/`
directory.

The synthetic files repeat a block containing every formatter fixture's
`expected.R` in sorted order, with blank lines between fixtures. The small file
repeats it twice and the large file repeats it 24 times. The repeated content
helps show how timings change as input grows.

### Commands measured

Single-file formatters read from standard input. Linters receive a file path.

  | Tool   | Single-file invocation                                                                             |
  | ------ | -------------------------------------------------------------------------------------------------- |
  | Arity  | `arity format` or `arity lint FILE`                                                                |
  | Air    | `air format --stdin-file-path bench.R`                                                             |
  | styler | `Rscript -e 'styler::cache_deactivate(); invisible(styler::style_text(readLines(file("stdin"))))'` |
  | Jarl   | `jarl check FILE`                                                                                  |
  | lintr  | `Rscript -e 'lintr::lint(FILE)'`                                                                   |

Project runs pass a directory to each tool. Formatters use check mode to
preserve the source files. Arity uses its format cache if the resolved
configuration enables it.

  | Tool  | Project invocation                           |
  | ----- | -------------------------------------------- |
  | Arity | `arity format --check R/` or `arity lint R/` |
  | Air   | `air format --check R/`                      |
  | Jarl  | `jarl check R/`                              |
  | lintr | `Rscript -e 'lintr::lint_dir("R/")'`         |

With hyperfine, each command gets at least three measured runs. Native tools get
two warmups. The R-backed tools get one warmup and are skipped on inputs above
20,000 lines. Styler runs only on single files because its directory mode
rewrites files. The timings include R interpreter startup and reflect the work
each tool performs under these invocations.

## Language server

The runner requires Linux with `/proc`, Python 3, Git, Cargo, R, and the R
package `languageserver`. The development shell also provides Ark and its Python
dependencies, `jupyter_client` and `pyzmq`. The runner uses Ark when it finds it
on `PATH`. If Ark is present and its Python dependencies are missing, the runner
reports the missing dependency.

```sh
task bench-lsp
task docs-gen
```

Use `--arity /path/to/arity` to measure an existing release binary,
`--rscript /path/to/Rscript` to select R, and `--ark /path/to/ark` to select an
Ark executable. Use `--no-ark` to omit Ark. The runner builds Arity in release
mode by default, stores the corpus under `target/bench-lsp/corpus/`, and writes
server stderr to `target/bench-lsp/logs/`.

This short run writes its results to a separate artifact.

```sh
task bench-lsp -- --runs 1 --edits 10 --latency-runs 2 \
  --out target/bench-lsp/trial.json
```

Run `task bench-lsp -- --help` for other workload and timeout options. The
harness tests use `python3 -m unittest discover -s benches -p 'test_lsp.py'`.

### Sessions and readiness

The runner starts three fresh processes per server in rotating order and opens
the five largest tracked R files in tidyr v1.3.2 at a pinned commit. It adds
benchmark bindings and performs 1,000 edits to the largest file in its open
buffer. The checkout stays unchanged.

The startup measurements cover initialization, workspace readiness, open-file
readiness, and the full edit workload. Arity and languageserver must publish
diagnostics for the opened files. Ark suppresses initially empty and unchanged
diagnostic sets, so its readiness check relies on document-symbol requests and
CPU sampling. All servers answer a document-symbol request for each file.

The runner samples process-tree CPU use every 150 ms and estimates readiness at
the start of a five-second window below 5% of one CPU core. The time spent
confirming the quiet period is excluded. The estimates can miss work that
continues at low CPU usage. A failed request, required diagnostics that never
arrive, an incorrect definition that persists, a disconnected server, or a
settling timeout aborts the run before the artifact is replaced.

### Requests and edits

The timed requests cover document symbols in the three largest open files, hover
and signature help at `mean()`, and definition, references, rename, and
completion at local benchmark bindings. They also cover workspace symbols
matching `pivot_wider`, folding ranges in the largest file, and references to
the `pivot_wider` generic across the package. The runner leaves rename edits
unapplied.

Each request target gets two warmups and 20 measured serial round trips per
process. The runner pools samples across processes to calculate the median and
p95. The results also record sample counts, empty responses, returned items, and
payload sizes. Read those counts alongside response times because the servers
can return different amounts of information. The document-symbol measurements
can include warm outline caches.

For edit-to-definition latency, each edit switches a call between two functions
with equal-length names and declarations on different lines. The timer runs
until the definition response points to the newly selected declaration. The
runner polls every 10 ms after stale or empty responses and includes the wait in
the timing. Arity and Ark receive incremental changes. languageserver receives
full documents. The benchmark measures all 1,000 edits and excludes the final
readiness checks from the total edit workload.

### Memory and environment

The runner samples RSS and PSS for the server and its descendants after
initialization, after opening files, after edits, and throughout the session for
the sampled peak. The chart shows median RSS across fresh processes. RSS counts
shared pages in each process, while PSS apportions them among the processes
using them. Short peaks and short-lived processes can fall between samples. The
peaks for RSS, PSS, and process count can occur at different times.

Arity and languageserver use stdio. Ark runs as a headless Jupyter R kernel
whose language server uses loopback TCP. Its startup time includes the kernel
handshake, and its memory includes the embedded R session and descendants. The
Python client and Jupyter launcher are outside the sampled process tree. The TCP
client sets `TCP_NODELAY` and `TCP_QUICKACK` to reduce delayed acknowledgments.

The servers retain their default analysis settings. Arity runs with
`--no-config` and the remote index URL unset. R runs with `--vanilla`. Each
session gets fresh XDG config and cache directories. Installed R packages and
help files remain available. The runner leaves operating-system file caches in
place and uses the default CPU affinity for the published comparison.

Ark uses the R installation and library paths reported by the selected
`Rscript`. The artifact records its version, R home, library paths, and source
fetching setting. Ark can download package sources on cache misses, so startup
and settling times can include network work. Each session gets a fresh Ark
source cache. Ark 0.1.252 can fetch sources even when
`OAK_SOURCE_FETCHING_ENABLED` is set. Preserve the artifact's recorded setting
when reproducing the run.

[hyperfine]: https://github.com/sharkdp/hyperfine
