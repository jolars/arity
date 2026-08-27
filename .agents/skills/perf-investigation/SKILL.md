---
name: perf-investigation
description: >-
  Profile-driven performance work on Arity's parser, formatter, linter, and
  specific language-server request paths. Use when asked to find hotspots,
  explain slowness on real R code, recover wall time, or verify a performance
  regression—not for linter correctness or corpus triage. Measure the production
  path before changing it, and preserve formatter bytes, CST losslessness, lint
  findings, and incremental behavior.
---

# Investigate Arity performance

Recover measured wall time without changing behavior. Treat a change that
alters formatter output, CST reconstruction, lint findings or spans, or salsa
invalidation as a bug.

## Establish the path

Read the relevant subsystem section of `AGENTS.md`. Search the "Performance"
section of `TODO.md` for the named subsystem or hotspot, and read matching
entries rather than loading the entire section. Identify the path named by the
report before profiling:

- `parse`: batch parsing and the cost every other cold path pays.
- `format`: cold parse, IR lowering, and printing.
- `format-warm`: `format_node` on a parsed CST—the formatter portion of LSP
  formatting. It does not include edit calculation, scheduling, or transport.
- `lint`: one-shot single-file lint. It is not the persistent LSP lint path.
- `format-dir` / `lint-dir`: batch library drivers with discovery and rayon.
  They use default configuration and omit CLI caching, indexing, and output
  rendering.

For CLI-edge questions, time the real release binary. For non-formatting LSP
latency, identify and measure the actual handler, scheduling path, or persistent
query; do not use `format-warm` or one-shot `lint` as a proxy.

Before recording with `task profile`, read
[references/harness.md](references/harness.md). It defines the modes, required
`perf` settings, and wall-time procedure.

## Preserve the contracts

- Keep `arity-parser` and `arity-formatter` compatible with
  `wasm32-unknown-unknown`: no threads, clocks, filesystem, or processes in
  those crates. Parallelism belongs at the CLI edge.
- Preserve the incremental-reparse ladder and the total handling of invalid
  edits. Run its focused tests after parser changes.
- Preserve salsa query granularity and durability. A body edit must not rebuild
  the project graph.
- Do not move work across parser, formatter, semantic, or project boundaries to
  make one phase look cheaper.
- Keep semantics static; never evaluate R.
- Keep benchmarks and profiles opt-in measurements, not test assertions or CI
  gates.

## Workflow

1. Record a baseline on real input that reproduces the report. Read the phase
   split before inclusive functions, and inclusive functions before self-time
   leaves.
2. Confirm the hotspot on the production-equivalent path. Synthetic fixtures
   are useful for harness checks but can invert the ranking of real phases.
3. Read [references/hotspots.md](references/hotspots.md) after identifying the
   hot phase. Classify the cost and choose the smallest matching change.
4. Make one behavior-preserving change. Add or strengthen a correctness test
   first when the optimization introduces a new fast path or changes traversal.
5. Re-profile to confirm that the intended work moved, then verify wall time on
   the real release binary. A disappearing symbol without a median improvement
   is not a win.
6. If at least 20 pinned-core runs show no shift beyond noise, remove only the
   investigation's own change. Do not ship a flat refactor as performance work.
7. Before handing off or committing, follow
   [references/verification.md](references/verification.md).

Do not stash and pop the user's working tree to build A/B binaries. Prefer
building the baseline before editing; if edits already exist, build the baseline
from an isolated temporary worktree.

## Stop and report

Report:

1. Production path, phase, function, and inclusive/self shares.
2. Hotspot class and the change made.
3. Median and minimum wall-time deltas from at least 20 pinned-core runs,
   including a result in the noise.
4. Behavior-preservation and test status.
5. Experiments removed because they did not pay.
6. The next measured hotspot.

Do not commit unless the user requested a commit. If publishing benchmark
numbers would be in scope, follow the benchmark guidance in `AGENTS.md`.
