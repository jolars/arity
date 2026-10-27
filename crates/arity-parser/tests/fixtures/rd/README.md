# Rd source oracle fixtures

`tests/rd_oracle.rs::fixture_names()` hand-registers these cases. Every
`input.Rd` has an `expected.json` minted by R's `tools::parse_Rd()`, with the R
version and invocation recorded in the pin. Ordinary Rust tests compare those
pins with a faithful projection of Arity's lossless CST; R is not needed in CI.

From the repository root:

```sh
task rd-oracle
task rd-oracle-refresh
```

The live task requires Rscript. The refresh task parses the entire corpus before
replacing pins and fails on any R warning or error. Review every changed pin
before keeping it. The driver uses only base R and tools.

The comparison retains source order, comments, options, argument groups,
whitespace, and R example contents. It coalesces adjacent leaves of the same
kind, preserving their full contents. The projection decodes parser-owned escape
tokens, translates CRLF to LF, and supplies a final newline when absent,
matching R's physical-line reader. It does not trim text or interpret syntax
that the CST has not modeled.

Oracle calls disable additional macros with `macros = FALSE`. Macro definitions
and unexpanded calls have separate source-preservation tests in `tests/rd.rs`;
their source trees cannot be compared directly with R's expanded trees.
`system-macros.json` pins the bundled macros' argument counts from R's own
definitions. The tests check those source shapes without expanding the calls.

The grammar vocabulary follows R 4.6.1. R's [grammar
source](https://github.com/wch/r-source/blob/R-4-6-branch/src/library/tools/src/gramRd.y)
and [parser
documentation](https://stat.ethz.ch/R-manual/R-devel/library/tools/html/parse_Rd.html)
describe the reference behavior. The `rd-source` crate is an additional
reference, not the oracle or a production dependency.

## Real help pages

- `cli_ansi_strip/input.Rd` is `man/ansi_strip.Rd` from
  [cli](https://github.com/r-lib/cli/blob/633bc27770f3357238d0f797132ad000cd1a8eb4/man/ansi_strip.Rd),
  commit `633bc27770f3357238d0f797132ad000cd1a8eb4`.
- `rlang_type_predicates/input.Rd` is `man/type-predicates.Rd` from
  [rlang](https://github.com/r-lib/rlang/blob/b4a2a057ea67613cc13cdbaed9eb0c421400e96a/man/type-predicates.Rd),
  commit `b4a2a057ea67613cc13cdbaed9eb0c421400e96a`.

Both files are distributed under the MIT license below. The upstream notices are
Copyright (c) 2025 cli authors and Copyright (c) 2020 rlang authors.

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
