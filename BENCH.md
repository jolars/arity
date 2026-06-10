# Formatter benchmark: ravel vs. air

Wall-clock formatting speed of `ravel` against `air` (posit-dev/air), measured
with [hyperfine]. Both tools format stdin → stdout (exit 0 regardless of
changes), so the comparison is free of file-mutation and exit-code noise.

**This is not a CI gate and not an air-parity target.** Timings are machine- and
run-dependent; this file measures speed only, never output equivalence (see
`AIR_COMPAT.md` / `task air-compat` for that). Regenerate with `task bench`.

air is tree-sitter-based; ravel uses a different model (rowan CST + event
pipeline, with incremental reparse as a first-class concern), so matching air's
raw throughput is not a goal. The bar is staying **largely on par** --- not
winning.

[hyperfine]: https://github.com/sharkdp/hyperfine

## Corpus

Synthetic: every `tests/fixtures/formatter/*/expected.R` concatenated (sorted,
blank-line separated) into a base block, repeated to two tiers. Content repeats,
so it is cache-friendly and not fully representative of real code; it exists to
amortize startup and show rough scaling.

- **small**: 6688 lines
- **large**: 80256 lines

## Results

### small (6688 lines)

  | Command | Mean [ms]  | Min [ms] | Max [ms] | Relative    |
  | :------ | ---------: | -------: | -------: | ----------: |
  | `ravel` | 16.1 ± 2.3 |     13.0 |     30.1 |        1.00 |
  | `air`   | 20.4 ± 1.1 |     17.8 |     23.4 | 1.27 ± 0.20 |

### large (80256 lines)

  | Command | Mean [ms]   | Min [ms] | Max [ms] | Relative    |
  | :------ | ----------: | -------: | -------: | ----------: |
  | `ravel` | 286.5 ± 2.6 |    283.0 |    290.2 | 1.30 ± 0.03 |
  | `air`   | 219.7 ± 3.7 |    214.4 |    226.7 |        1.00 |
