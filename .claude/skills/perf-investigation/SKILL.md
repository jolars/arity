---
name: perf-investigation
description: >-
  Profile-driven performance work on arity's parser, formatter, linter, or
  language server. Measure with scripts/profile.sh first, read the phase split
  before the leaf list, classify the hotspot, apply the smallest matching fix,
  and prove median wall time moved before committing. Formatter output must stay
  byte-identical, the CST lossless, and the lint findings unchanged: a perf
  change that alters any of them is a bug, not a trade-off.
---

Use this skill when asked to "speed up the formatter", "profile the parser",
"find the hotspots", "why is `lint` slow on this package", "cut LSP latency", or
anything else where the job is *measure where the time goes on real R code and
recover wall time*.

## The one rule that outranks the rest

**Behavior must not change.** `format(x)` is byte-identical before and after,
`format(format(x)) == format(x)`, `reconstruct(text) == text`, and the linter
reports the same findings with the same spans. There is no speed/quality
trade-off available: the formatter is the sole authority on layout (`AGENTS.md`,
tenet 1), so a faster formatter that lays anything out differently has broken
the contract, not optimized it. The fixture and snapshot gates are what prove
it — see §Verify.

## Scope boundaries

- **The member crates must stay `wasm32-unknown-unknown`-clean.**
  `arity-formatter` is embedded by the dprint Wasm plugin, and the `wasm` CI job
  is the only thing keeping it honest. **No threads, no clock, no filesystem, no
  process** in `arity-parser` or `arity-formatter` — which rules out the usual
  "just parallelize it" answer inside the engine, and rules out thread-local
  pools unless you are certain of the wasm story. Rayon parallelism belongs at
  the CLI edge (`formatter::check_paths`, `linter::check_paths`), where it
  already is.
- **Incremental reparse is first-class** (tenet 2). A parser change that speeds
  up the batch path but breaks or bypasses `parser/reparse.rs` is not a win;
  check `crates/arity-parser/tests/incremental_reparse.rs` and
  `tests/salsa_incremental.rs`.
- **Salsa's memo firewall is the LSP's latency structure.** Widening a query's
  inputs to save work inside it can make a keystroke invalidate the project
  graph, which is slower where it matters and invisible in a batch benchmark.
  `tests/salsa_incremental.rs` guards exactly that.
- **Don't fix formatter cost by moving work into the parser or vice versa**
  (tenets 3 and 4). The phase split will tempt you here — resist it.
- **Semantics stay static.** No R evaluation, no matter how much it would save.
- Benchmarks and profiles are **measured, never asserted** and never a CI gate
  (`.claude/rules/docs.md`). Do not add a perf assertion to a test.

## Related rules to read first

- `.claude/rules/docs.md` — the benchmark artifact
  (`benches/benchmark_results.json`) and when a moved number must be re-measured
  and committed.
- `.claude/rules/formatter.md` — the layout engine's contract and the fixture
  gate.
- `.claude/rules/parser.md` — losslessness and the incremental-reparse
  obligation.
- `.claude/rules/lsp.md` — which path a latency question is actually about.

## Harness

```sh
task profile                                     # format, fixture corpus, 300 iters
task profile -- --mode parse                     # a different phase
task profile -- --mode lint --path pkg/R/foo.R   # a file you care about
task profile -- --mode lint-dir --path pkg/R     # the rayon CLI path
ITERATIONS=2000 task profile                     # more samples for a small delta
REPORT_ONLY=1 task profile                       # re-read the last recording
```

`scripts/profile.sh` builds `benches/profile.rs` under the `profiling` cargo
profile, samples it with perf, and prints a **phase split**, a **top-inclusive
list**, and a **self-time leaf list**, leaving `target/profile/profile.data`
(raw) and `profile.svg` (flamegraph) behind. Nothing there is tracked: a profile
is a local observation, unlike `benches/benchmark_results.json`.

The modes are `parse`, `format` (cold: parse + lower + print), `format-warm`
(`format_node` on an already-parsed CST — **the language server's path**),
`lint`, and `format-dir` / `lint-dir` (the CLI drivers, rayon and all). Pick
`format-warm` for LSP latency questions and `format` for `arity format` on a
cold file; they have almost nothing in common.

Four things the harness sets that are load-bearing, and that hand-rolling a perf
invocation will lose:

- **`-Cforce-frame-pointers=yes` plus `--call-graph fp`.** Release codegen omits
  frame pointers, so without this the callchains truncate and every inclusive
  view silently collapses into self time. Never append a bare `-g` after
  `--call-graph dwarf`; `-g` *is* `--call-graph fp` and quietly overrides it.
- **`[profile.profiling]` in the root `Cargo.toml`** — release codegen plus the
  debug info release does not carry. Profiling a plain `--release` build
  resolves fewer symbols and no inline frames at all.
- **`--no-inline` when reading the stacks.** With inline expansion on, perf
  renames *every* frame to its short source name, so
  `arity_formatter::formatter::core::format_with_options` becomes bare
  `format_with_options` and the phase table (which matches module paths) reports
  nothing. `INLINE=1` turns expansion back on when the question is which inlined
  helper inside one function is hot — expect the phase table to go blank then.
- **mimalloc in the profile target.** `src/main.rs` sets it for the shipping
  binary and `benches/profile.rs` mirrors that. It is not cosmetic: the same
  format profile spends **~38%** of total in allocator symbols under glibc
  malloc and **~10%** under mimalloc. A target without it profiles a program
  arity does not ship.

For wall-time verification, not profiling, use hyperfine on the real binary:

```sh
cargo build --release
hyperfine --warmup 3 -m 20 \
  'taskset -c 2 ./target/release/arity format --check --no-cache /path/to/pkg/R'
```

**`--no-cache` is mandatory when timing `format --check`.** A release build
keeps a persistent already-formatted cache (`src/formatter/cache.rs`), so the
warmup runs populate it and every timed run after that is a hash lookup, not a
format. `taskset` pins one core; without it, scheduler migration adds several
percent of jitter and small wins vanish into it. Discard warmups, take the
median.

**Interleave the A/B when the machine is not quiet.** A busy machine drifts far
more between a before and an after measured minutes apart than the win you are
chasing. Build both binaries first, then alternate them round by round so drift
hits both equally, and report min alongside median:

```sh
cargo build --release && cp target/release/arity /tmp/arity_new
git stash push <the changed files>
cargo build --release && cp target/release/arity /tmp/arity_old
git stash pop
# then alternate old/new for N rounds and compare
```

Check `uptime` and `ps -eo pcpu,comm --sort=-pcpu | head` before believing any
number.

The criterion benches answer narrower questions and need no perf:
`task bench-parse` (parse + reparse), `task bench-line-index`,
`task bench-format-edits`. `task bench` is the published cross-tool comparison
and rewrites a tracked artifact — that is a *reporting* step, not a measurement
loop.

## Read the phase split before the leaf list

The leaf list flatters whatever is at the bottom of the stack — the allocator
and rowan's cursor always look enormous and are almost never where a fix goes.
The inclusive split tells you which *phase* to open.

Measured 2026-08-14 on the fixture corpus (63 KB, roxygen-heavy), one machine,
one day. **Re-measure rather than trusting these**; they are a starting map, not
a fact. Sub-entries are shares of total, not of the phase above them.

```text
--mode format (cold, --path tidyr/R/pivot-wide.R: a real 23 KB package file)
  format                 99.9%
    lower               39.4%     <- the formatter's own IR lowering dominates
    render              28.3%
    print                6.7%
    parse               35.9%
      build_tree        11.1%
      parse_expr         8.9%
      structural         7.7%
      roxygen            6.6%
      lex                4.8%
  rowan                 33.0%
  allocator              9.9%

--mode format (cold, the synthetic fixture corpus)
  format                 100%
    parse               86.2%     <- the corpus is tiny files; parse dominates
    format_node          6.6%
      scan_tokens        6.6%
  rowan                 27.5%
  allocator             11.5%

--mode lint (single file, one-shot database)
  lint_rules            69.0%
  salsa                 29.3%
  semantic              14.4%
  parse                 15.6%

--mode lint-dir (the CLI driver over a directory)
  salsa                 46.5%
  lint_rules            31.5%
  semantic              19.4%
  project               18.3%     <- the graph rivals the rules at directory scale
  parse                 13.2%
```

Three readings that follow, and that anyone profiling "the formatter" needs to
absorb before touching anything:

1. **Profile with `--path` on a real package file, not the default corpus.**
   The two columns above are the *same mode on different input*, and they
   disagree about what a cold format even is: parse is 36% on real code and 86%
   on the corpus. The corpus is many tiny fixtures, so it measures per-file
   fixed costs and flatters anything that scales with file count. Every ranking
   below comes from the real-file column.
2. **The formatter's own internals hide inside `format_node`.** In the
   single-file mode they are inlined into it; `--mode format-dir` (a different
   call site) resolves `lower` at ~35%, `render` ~7%, `print` ~5%. If a phase's
   internals are missing, that is inlining, not absence — cross-check with
   `format-dir` or `INLINE=1`.
3. **The fixture corpus is roxygen-heavy and synthetic.** `roxygen` at 10% of a
   format is a property of that input. Confirm a hotspot on a real package
   (`--path pkg/R/big.R`) before optimizing for it.

## Classify the hotspot

Open leads plus known shapes. Add to it as things are confirmed or ruled out —
a lead that was measured and didn't pay belongs in §Don't redo.

- **A whole-tree pass that only asks token questions** — walk the **green**
  tree, not a cursor. `root.green().children()` over an explicit stack of
  `rowan::Children` allocates nothing, where `descendants_with_tokens()`
  allocates and drops a `SyntaxNode` per element. *Closed instance:*
  `validate_supported_tokens` + `file_is_skipped` were two such passes at 31%
  of a cold format; folding them into one green walk (`core::scan_tokens`) took
  `format_node` from 31.3% to 6.6% inclusive and `arity format --check` on
  `tidyr/R` from 39.9 to 36.3 ms. Anything here must keep the *answers*
  identical: the `ERROR` gate is a correctness gate that reports the first token
  in document order and outranks the directive, and `# arity-format skip-file`
  must still return the file byte for byte. Both are pinned by tests in
  `core.rs`.
- **IR lowering — the largest open lead in the formatter.** On real code
  `lower` is 39.4% and `render` 28.3%, against `print` at 6.7%: the cost is
  building the document, not laying it out. `ir_statements`/`ir_line` (54%) and
  the `ir_binary_side`/`ir_assignment_expr` chain (~38%) are the spine.
- **rowan cursor traversal** — now ~9% of a cold format, spread over
  `SyntaxToken::next_sibling_or_token`, `SyntaxElementChildren::next`, and the
  `Vec<SyntaxElement>` that `children_with_tokens().collect()` builds for
  `split_lines`. Proportional to how many times a pass re-walks or re-collects
  the same children. Look for a lowering step that collects the same node's
  children more than once.
- **Green-tree construction** — `build_tree` is 19–26% of a parse, with
  `NodeCache::{token,node}` and `ThinArc::from_header_and_iter` underneath.
  Proportional to token and node count. Reducing it means emitting fewer nodes,
  which is invasive: it changes the CST and therefore the parser snapshots, the
  roxygen projector pins, and possibly formatter output. Last resort, and
  **never by pooling the `NodeCache` across parses** — it holds `Arc`'d green
  nodes, so pooling leaks in the language server and produces a misleadingly
  warm benchmark after iteration one.
- **Allocator traffic** — `_mi_page_malloc_zero`, `mi_free`,
  `mi_theap_malloc_aligned`, ~10% of total. This is a *symptom*; the fix is
  always at an allocation site upstream, found by following callers, never at
  the allocator. Switching allocator again is not the move — that experiment was
  already run and won (glibc ~38% → mimalloc ~10%).
- **`RawVec::finish_grow` / `grow_one`** (~10% inclusive on a format) — a `Vec`
  growing element by element where the final size is known or boundable. Reserve
  up front. **Trap:** the type parameter in a `RawVec::<T>` symbol is not
  reliable evidence of which `Vec` it is; identically-laid-out instantiations get
  merged. Follow the caller chain (`perf report -i … -g graph,caller`), don't
  read the type off the symbol.
- **Salsa and the project layer on the lint paths** — `salsa::` is 29% of a
  single-file lint and 47% over a directory, where `project` alone rivals the
  rule set. At directory scale the question is query granularity and durability,
  not rule bodies. Any change here is guarded by `tests/salsa_incremental.rs`:
  a body edit must not invalidate the project graph.
- **One expensive rule inside the fan-out** — 61 rules share a single walk, so
  `lint_rules` at 31–69% is a *sum*. A rule that materializes text or builds a
  regex per run can dominate it. `TODO.md` records one confirmed instance:
  `suspicious/duplicated_function_definition` builds a line index over
  `ctx.root.text().to_string()` — a full text materialization per rule run.
  Attribute the cost to a named rule before touching the dispatch.
- **Roxygen parsing** — `emit_roxygen_block` and friends run 10–15% of a parse
  on roxygen-dense input. Real R packages vary wildly here; check the input
  before treating it as general.
- **The CLI edge** — file discovery, the format cache, rayon fan-out, and
  rendering. Only visible in the `-dir` modes, and the place where a
  "formatter is slow on this package" report often actually lives.

## Apply the smallest matching fix

- **Don't theorize before measuring.** Pre-sizing a `Vec`, adding a fast-path
  gate, and hoisting an allocation are all changes that *should* help and
  routinely don't. Measure each one.
- **Verify with wall time, not perf.** A change can delete a symbol from the
  top-25 without moving the median — sample relocation, not work eliminated. The
  hyperfine median is the truth; the profile only says where to look.
- **One change per commit**, so one regression can't mask another's win.
- **Revert promptly.** If 20 hyperfine runs show no median shift beyond the
  noise floor, the fix doesn't pay. Don't ship flat refactors as perf.

## Verify

Every perf commit, without exception:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

The suites that specifically catch a perf change that changed behavior:

```sh
cargo test -p arity-formatter --test formatter          # fixtures + idempotence
cargo test -p arity-parser --test parser_snapshots      # CST shape and losslessness
cargo test -p arity-parser --test incremental_reparse   # the reparse strategies
cargo test --test salsa_incremental                     # the memo firewall
cargo test --test lint                                  # rule findings and spans
cargo test --test roxygen_projector                     # CST -> Rd parity pins
```

For a parser or formatter change, also round-trip real code:

```sh
ARITY_CORPUS=<dir of real R sources> task corpus   # losslessness + idempotence
cargo run --release -- format --verify <file.R>    # idempotence on one file
cat file.R | cargo run --release -- parse --verify --quiet
```

The most direct proof for a formatter change is a **differential format**: run
the before and after binaries over a real package and `diff -r` the two output
trees. It catches what a fixture cannot, because the input is code nobody wrote
a fixture for.

```sh
cp -r pkg/R out_old && cp -r pkg/R out_new
./arity_old format --no-cache out_old && ./arity_new format --no-cache out_new
diff -r out_old out_new     # must be silent
```

**Never accept an insta snapshot you have not read**, and on a perf commit a
changed snapshot is a red flag by default: the whole premise is that behavior is
identical.

If the win is large enough to move the published performance page, re-run
`task bench` and commit `benches/benchmark_results.json` in the same commit —
that artifact is the sole source of the docs numbers and is never re-measured at
site build.

## Commit format

Name the bucket and quote the median, including when it's in the noise — that's
the honest record and lets a reviewer decide whether to ship it at all.

`6f8a444` is the worked example — the shape to copy:

```text
perf(formatter): answer both prepasses in one green-tree walk

The profile put 31% of a cold `format()` in two whole-CST cursor walks that
each visit every element before a single layout decision. Both ask only
token questions, so one walk over the green tree answers both.

Median `arity format --check --no-cache tidyr/R` (15 interleaved rounds,
pinned to one core): 39.89 ms -> 36.29 ms (-9.0%); min -8.7%.
Fixtures and parser snapshots unchanged; output over tidyr byte-identical.
```

Keep commits atomic per area — root crate, `crates/arity-parser`,
`crates/arity-formatter` — because the release tooling routes versions by path
(`.claude/rules/release.md`).

## Key files

- `scripts/profile.sh` — the harness; `benches/profile.rs` is what it samples,
  and `[profile.profiling]` in `Cargo.toml` is what makes symbols resolve.
- `crates/arity-formatter/src/formatter/core.rs` — `format_with_options`,
  `format_node`, `scan_tokens`; the phase roots.
- `crates/arity-formatter/src/formatter/{rules.rs,printer.rs,render.rs}` —
  lowering, the best-fit layout engine, and rendering.
- `crates/arity-parser/src/parser/{core.rs,lexer.rs,expr.rs,structural.rs,tree_builder.rs}`
  — the parse pipeline; `reparse.rs` is the incremental path.
- `src/linter/check.rs` — the lint driver, its salsa passes and rayon fan-out.
- `src/project/graph.rs` — the queries behind the `project` phase.
- `src/formatter/cache.rs` — the persistent `--check` cache that will lie to
  your benchmark.
- `TODO.md` — the "Performance" section records what has already been done and
  the follow-ups that were deliberately deferred. Read it before proposing work.

## Don't redo / known traps

- **A share measured on the default corpus can be pure timer resolution.** The
  whole `format-warm` corpus run is ~0.08 ms/iter, and the harness reports to
  0.01 ms — so a "18.8% of the warm path" reading there is worth ~one tick.
  Confirm any share on a real file with `--path` before acting on it.
- **Prefiltering `directive::parse` on `body.starts_with("arity")` does not
  pay.** It looks certain to: the formatter and linter run it over every comment
  token, and all five spellings need that prefix. Measured on
  `tidyr/R/pivot-wide.R` (237 comments, 9 interleaved rounds): `format-warm`
  1.310 -> 1.300 ms/iter and cold `format` 2.090 -> 2.080, i.e. one resolution
  tick with identical minima. It only looked hot on the synthetic corpus.
- **`arity format --check` without `--no-cache` measures a hash lookup.** In a
  release build the persistent already-formatted cache is on by default (it is
  compiled out in debug builds, which is its own reason not to time a debug
  binary).
- **`INLINE=1` blanks the phase table.** perf renames every frame to its short
  source name under inline expansion. Use it to drill into one function, then go
  back.
- **A missing sub-phase usually means inlining, not absence.** `lower` and
  `print` are invisible under `--mode format` and plain under `--mode
  format-dir`.
- **Don't profile a target that doesn't set mimalloc.** The allocator share
  changes by ~4x and the shape of every leaf list with it.
- **Don't read a generic's type parameter off a symbol name** — see the
  `RawVec` trap above.
- **Don't pool rowan's `NodeCache` across parses.** It holds `Arc`'d green
  nodes: a leak in the language server, and a warm cache after iteration one
  makes the benchmark lie.
- **Don't benchmark a formatter or linter change on a directory alone.** Those
  paths are rayon-parallel across files, so a many-core machine hides a per-file
  regression behind spare cores. Single file first, directory second.
- **The `lint` mode builds a one-shot database.** The language server's hot path
  is `check_tracked_file` against a persistent one; a win in the one-shot setup
  may not exist in the editor.
- **perf needs `kernel.perf_event_paranoid <= 2`** for user-space sampling. At 2
  (the usual default) the recording is user-space only, which is what we want
  anyway; kernel frames simply won't appear.

## Report-back format

1. Phase and hotspot addressed (function + inclusive and self %).
2. Bucket from §Classify.
3. Median wall-time delta (hyperfine, pinned, ≥20 runs, `--no-cache` where it
   applies) — even if in the noise.
4. Test/clippy/fmt status, and explicitly that the formatter fixtures, parser
   snapshots, and lint findings are unchanged.
5. What was tried and reverted, with the measurement that killed it.
6. Next hotspot, ranked.
