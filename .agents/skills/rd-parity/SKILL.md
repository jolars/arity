---
name: rd-parity
description: >-
  Grow or investigate Arity's standalone Rd source parser against R's
  tools::parse_Rd oracle. Use for .Rd grammar gaps, incorrect CST structure,
  recovery bugs, or the next Rd parity target. Covers lossless parsing and
  pinned oracle fixtures, not roxygen2 generation, formatting, or rendering.
---

Read [RECAP.md](RECAP.md) for the measured baseline and next investigation.
Prefer the user's reported case. The detailed oracle contract and fixture
provenance live in [the fixture
README](../../../crates/arity-parser/tests/fixtures/rd/README.md). Paths in
commands below are relative to the repository root.

## Source of truth

The parser is `crates/arity-parser/src/rd/`: `parser.rs` owns mode-aware
scanning and recovery, `macros.rs` owns argument shapes and contexts,
`syntax.rs` owns the independent Rd kinds, and `ast.rs` supplies typed source
navigation. Its entry point is `arity_parser::rd::parse`, re-exported as
`arity::rd::parse`.

`scripts/rd-oracle.R` calls `tools::parse_Rd` with UTF-8 input,
`macros = FALSE`, and `fragment = FALSE`. It uses only base R and tools, fails
on any warning or error, and records the R version and invocation in each pin.
`rd-source` and roxygen's Rd scanners can inform an investigation, but neither
defines standalone Rd behavior.

`crates/arity-parser/tests/rd_oracle.rs` projects the CST to the same tree
encoding. Its ordinary tests compare committed pins without R; the ignored test
compares the registered corpus with live R:

```sh
cargo test -p arity-parser --test rd --test rd_oracle
task rd-oracle
```

Every registered curated case must pass. There is no allowlist or measured
external-corpus backlog yet. Preserve existing pins when fixing the parser; do
not refresh them to bless Arity's output.

## Close one confirmed gap

1. Reproduce the target with R before calling it a parser bug. For an accepted
   document, inspect the oracle tree and Arity's CST:

   ```sh
   Rscript --vanilla scripts/rd-oracle.R path/to/input.Rd
   cargo run -- parse --verify path/to/input.Rd
   ```

   The driver deliberately refuses malformed input. For a recovery case, call
   `tools::parse_Rd` directly and inspect its warnings and errors. An R warning
   can accompany a recovered tree and exit status 0. Arity must retain all
   bytes, so do not require its recovery tree to match R's discarded source.

2. Classify the mismatch as a parser defect, an encoding-translation defect, or
   behavior outside the current source-parser scope. Inspect the CST before
   changing the test projector. A projector change may translate parser-owned
   structure or escaping; it must not discover syntax or manufacture missing
   nodes to make a comparison pass.

3. For valid input, add a minimal
   `crates/arity-parser/tests/fixtures/rd/<case>/input.Rd`, mint that case's
   `expected.json` with the R driver, and hand-register it in the parser crate's
   `tests/rd_oracle.rs::fixture_names()`. Run the pinned test to demonstrate the
   failure before fixing the grammar. Review the pin's actual text and shape.
   For malformed input or unexpanded macros, add a regression in the parser
   crate's `tests/rd.rs` that asserts the intended diagnostic behavior and
   unconditional losslessness.

4. Fix the parser or its macro metadata, then extend typed wrappers or the
   faithful projector only when the new CST requires it. Add related cases when
   they exercise the same root cause across input modes or contexts.

5. Run the focused tests and live oracle, then the applicable repository checks
   from `AGENTS.md`. If R is unavailable, report the live comparison as unrun;
   do not invent pins. Update `RECAP.md` with the confirmed result, measured
   coverage, and next target. Report any remaining failures explicitly.

`task rd-oracle-refresh` regenerates the whole curated corpus and bundled
macro-shape pin. Use it for an intentional reference or corpus refresh, and
review every changed pin, including reference-version changes.

## Rd-specific traps

- Prose, R-like, and verbatim arguments have different lexical rules. Equation
  arguments also differ in comment and escape handling. Argument mode belongs to
  the macro and argument position, not to how the text looks.
- `\item` is bare in `\itemize` and `\enumerate`, takes two arguments in
  `\arguments`, `\value`, and `\describe`, and depends on the active context.
  Optional second arguments and zero-argument macros must not swallow unrelated
  following groups.
- R-like Rd text is not ordinary R source. Quoting, raw strings, R comments,
  percent comments, embedded markup, and conditional boundaries interact. Probe
  the exact combination with R instead of substituting the R lexer.
- The standalone oracle preserves comments, whitespace, option contents,
  argument boundaries, source order, and complete example text. Do not import
  roxygen's whitespace trimming, section sorting, comment dropping, or
  `\examples` placeholders. The documented encoding normalization coalesces
  adjacent leaves, decodes parser-owned escapes, normalizes CRLF, and accounts
  for R's supplied final newline.
- Custom macro names stay quiet; their following groups remain source groups
  without guessed argument counts. Definitions and `\Sexpr` remain unexpanded
  and unevaluated. Bundled macro argument counts have a separate R-derived
  `system-macros.json` pin. Expansion differences are not grammar regressions.

Current scope is complete UTF-8 source documents. Formatting, rendering, lint
policy, package macro resolution, encoding conversion, and Rd incremental or
editor integration require their own requested work. Preserve the existing R,
DCF, and roxygen behavior while improving the independent Rd grammar.
