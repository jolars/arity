# Performance

These benchmarks compare Arity's formatter, linter, and language server with
other R tools. The tools differ in formatting style, lint coverage, and editor
features. The timing charts show milliseconds on a logarithmic scale. Lower
values mean shorter waits.

## Formatter and linter

<a id="corpus"></a>

We compare the formatter with [Air] and [styler] and the linter with [Jarl] and
[lintr]. The single-file benchmarks use the largest R source file from each of
[tidyr] and [MASS], plus two synthetic files. The project benchmarks cover each
package's `R/` directory. Formatters use check mode for projects, and styler
appears only in the single-file comparison.

The charts show mean wall-clock times, including process startup. R startup can
account for much of the time styler and lintr spend on small files.

<a id="setup"></a>

<!--@include: ./benchmarks_meta.md -->

<a id="results"></a>

<!--@include: ./benchmarks_results.md -->

## Language server

<a id="language-server-latency-and-memory"></a>
<a id="what-the-session-measures"></a>

We compare Arity with [languageserver] and [Ark] by opening the same five files
in tidyr. The benchmarks measure startup, readiness after opening files, editor
request latency, and the time to get a correct definition after an edit. The
charts show medians, with the 95th percentile (p95) and result counts in the
request tooltips and tables.

The memory chart includes each server and its child processes. **RSS** counts
shared pages in every process that uses them. **PSS** divides those pages among
the processes and appears in the tooltips and tables.

<!--@include: ./benchmarks_lsp.md -->

## Reproducibility

<a id="how-it-is-measured"></a> <a id="run-the-comparison"></a>

See the [benchmark README] for prerequisites, commands, and measurement details.

[air]: https://github.com/posit-dev/air
[ark]: https://github.com/posit-dev/ark
[benchmark readme]: https://github.com/jolars/arity/blob/main/benches/README.md
[jarl]: https://github.com/etiennebacher/jarl
[languageserver]: https://github.com/REditorSupport/languageserver
[lintr]: https://lintr.r-lib.org/
[mass]: https://cran.r-project.org/package=MASS
[styler]: https://styler.r-lib.org/
[tidyr]: https://tidyr.tidyverse.org/
