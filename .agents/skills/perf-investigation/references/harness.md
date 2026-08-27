# Profiling harness

Read this before recording or interpreting a profile.

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
function list, and a self-time leaf list. Its flags and profile are authoritative.

The directory modes call batch library APIs. They include discovery and rayon,
but not all work performed by `src/main.rs`: project configuration, the format
cache, package-index setup, diagnostics, and output rendering need measurement
on the real CLI.

## Load-bearing settings

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

## Wall-time authority

Use the real release binary for the final comparison:

```sh
cargo build --release
hyperfine --warmup 3 -m 20 \
  'taskset -c 2 ./target/release/arity format --check --no-cache /path/to/pkg/R'
```

Choose a CPU allowed by the current cpuset. `--no-cache` is mandatory for
`format --check`; otherwise warmups turn the timed work into a cache lookup.
Use the median, report the minimum, and check machine load before interpreting a
small delta.

Build and copy the baseline binary before editing. After the change, build and
copy the candidate, then compare those immutable binaries on identical input.
If the baseline must be recovered after editing, use an isolated temporary
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
