# Arity performance measurements

## Commands and modes

```sh
task profile -- --mode parse --path pkg/R/foo.R
task profile -- --mode format --path pkg/R/foo.R
task profile -- --mode format-warm --path pkg/R/foo.R
task profile -- --mode lint --path pkg/R/foo.R
task profile -- --mode lint-dir --path pkg/R
ITERATIONS=2000 task profile -- --mode format-warm --path pkg/R/foo.R
REPORT_ONLY=1 task profile
```

The default concatenated fixture corpus is a harness smoke test, not a basis for
an optimization. Use `--path` with representative real code.

`scripts/profile.sh` builds `benches/profile.rs` under `[profile.profiling]`,
records `target/profile/profile.data`, and prints a phase split, an inclusive
function list, and a self-time leaf list. Its flags and profile are
authoritative.

The directory modes call batch library APIs. They include discovery and rayon,
but not all work performed by `src/main.rs`: project configuration, the format
cache, package-index setup, diagnostics, and output rendering need measurement
on the real CLI.

## Build and recording settings

- Keep `-Cforce-frame-pointers=yes` with `perf --call-graph fp`; otherwise
  inclusive stacks collapse.
- Use `[profile.profiling]`, which combines release codegen with debug info.
- Keep `--no-inline` for the phase table. `INLINE=1` is a drill-down mode and
  can blank module-path phase matches.
- Profile the target with mimalloc, matching the shipping binary.
- Treat allocator and rowan leaves as symptoms until their caller identifies an
  allocation or traversal site.

`perf` requires Linux user-space sampling permission. If it cannot record in the
current environment, report that limitation rather than substituting an
unrelated benchmark.

## End-to-end timings

Use the real release binary for the final comparison:

```sh
cargo build --release
hyperfine --warmup 3 -m 20 \
  'taskset -c 2 ./target/release/arity format --check --no-cache /path/to/pkg/R'
```

Choose a CPU allowed by the current cpuset. `--no-cache` is mandatory for
`format --check`; otherwise warmups turn the timed work into a cache lookup. Use
the median, report the minimum, and check machine load before interpreting a
small delta.

Build and copy the baseline binary before editing. After the change, build and
copy the candidate, then compare those immutable binaries on identical input. If
the baseline must be recovered after editing, use an isolated temporary
worktree; never stash/pop the user's primary working tree.

When the machine is not quiet, alternate baseline and candidate rounds so drift
affects both. Keep any ad hoc runner under `target/profile/`, not in tracked
source.

Criterion benches answer narrower questions:

- `task bench-parse`: parse and incremental reparse.
- `task bench-line-index`: line-index operations.
- `task bench-format-edits`: LSP formatting-edit calculation.
- `cargo bench --bench salsa_keystroke`: persistent keystroke pipeline.

`task bench` rewrites the published cross-tool benchmark artifact. It is a
reporting step, not the inner measurement loop.

## Correctness checks

Read this before handing off or committing a performance change.

### Required checks

```sh
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Run the focused behavioral suites relevant to the changed area:

```sh
cargo test -p arity-formatter --test formatter
cargo test -p arity-parser --test parser_snapshots
cargo test -p arity-parser --test incremental_reparse
cargo test --test salsa_incremental
cargo test --test lint
cargo test --test roxygen_projector
```

For parser or formatter changes, round-trip representative real code with
`task corpus`, `format --verify`, and `parse --verify --quiet` as appropriate.
For formatter changes, compare baseline and candidate output trees byte for
byte.

Never accept an unread insta snapshot. A changed snapshot is a correctness
failure by default in performance work; explain any intentional exception before
accepting it.

## Hotspot notes

Read this only after the phase split identifies a hot subsystem. Re-measure all
shares on the current machine and input.

### Match the cost to the change

- Whole-tree passes that ask only token questions may use one allocation-free
  green-tree traversal. Preserve document order and all error/directive gates.
- Repeated rowan cursor walks or repeated `children_with_tokens().collect()`
  calls suggest eliminating duplicate traversal or materialization.
- `RawVec::grow_one` or `finish_grow` suggests a collection whose size can be
  bounded and reserved. Follow its callers; generic symbol type parameters can
  be merged and are not reliable attribution.
- Allocator symbols identify traffic, not a fix site. Follow callers to the
  allocation that can be removed or reduced.
- Parser tree construction is invasive because fewer nodes change CST shape.
  Treat it as a last resort, and never pool rowan's `NodeCache` across parses;
  it retains green nodes and creates a misleading warm benchmark.
- Formatter profiles on real files have historically put more time in IR
  lowering and rendering than printing. Look for repeated child collection and
  document construction along `ir_statements`, `ir_line`, binary sides, and
  assignments.
- A linter's shared walk fans out to many rules. Attribute cost to a named rule
  before changing dispatch; check `TODO.md` for recorded materializations.
- High salsa or project shares on directory lint point to query granularity and
  durability. Preserve the invalidation expectations in
  `tests/salsa_incremental.rs`.
- Roxygen shares depend heavily on documentation density. Confirm them on the
  affected package before treating them as general parser cost.
- CLI reports may be dominated by discovery, cache behavior, index setup, rayon,
  or rendering. These require the actual CLI, not merely a directory harness
  mode.

### Known traps

- The fixture corpus contains many small, roxygen-heavy files. It exaggerates
  per-file fixed costs and can make parsing dominate a formatter profile.
- A missing sub-phase often means inlining, not absence. Cross-check a different
  call site or use `INLINE=1` for drill-down, then return to normal phase
  output.
- Do not benchmark only a rayon directory path; spare cores can hide a
  single-file regression. Measure one file first, then the package.
- `format --check` without `--no-cache` measures fixed-point cache hits after
  warmup.
- One-shot `lint` is not the language server's persistent lint database.
- Do not replace mimalloc or optimize allocator internals before locating the
  upstream allocation.
- Do not repeat the measured no-win prefilter of `directive::parse` on an
  `arity` prefix; previous results were within timer resolution.

Record a confirmed no-win in the "Performance" section of `TODO.md` only when
that history will prevent a plausible repeated experiment.
