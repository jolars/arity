---
name: roxygen-parity
description: >-
  Grow arity's roxygen2 + markdown CST toward roxygen2 itself using a strict
  differential oracle (parser work, not formatting). roxygen2 renders #' blocks to
  .Rd. Use this skill to pick the next gap from the corpus, add the grammar plus
  projector support, lock it with a fixture + pinned tree, and ratchet the
  now-passing case into the allowlist. The projector at src/roxygen/project_rd.rs
  walks arity's CST
  and emits the parser-owned Rd section subtrees; the pure-Rust gate in
  tests/roxygen_projector.rs diffs that against a pinned <stem>.rdtree (no R, plain
  cargo test), allowlist-gated. The projector is a test-only faithful diagnostic: a
  divergence means the CST (or the encoding translation) is wrong—never patch
  the projector to make a case pass; fix the parser.
---

Use this skill to advance arity's roxygen2 parser parity, work the oracle
backlog, or "take the next gap." **Read `RECAP.md` (this directory) first**—it
holds the latest session, persistent traps, settled decisions, and the ranked next
target. The roadmap is `TODO.md` (roxygen section); the full design rationale is the
plan at `~/.claude/plans/i-want-to-start-snoopy-haven.md` (local). This skill is the
per-session loop within them.

## Why strict (read first)

A roxygen-oracle divergence is **not** an air-compat-style soft "deviation." It
means arity changed what roxygen2 *renders* from the user's docs—a behavior-
preservation bug, the same family as a losslessness or idempotence failure. So
this mirrors fatou's `parser-parity` (strict conformance, `allowlist` +
`blocked`, faithful projector, pinned expecteds), **not** air-compat's
"subordinate to Tenet 1."

Two accounting regimes, don't conflate them:
- **Curated corpus = strict.** Every case is PASS (allowlisted) or BLOCKED (a
  deliberate divergence with a one-line rationale); an unaccounted divergence is
  RED.
- **Harvested + whole-CommonMark-spec corpora = measured backlog.** These are
  broad nets; an un-allowlisted divergence is **backlog, never a build failure**.
  The spec is adopted *whole* (all 655 `cm-NNN`) with a per-section burndown in
  `ROXYGEN_PROJECTOR.md` and a `roxygen-projector-blocked.txt` for genuine
  non-targets — panache's conformance model. What is always RED, in both regimes,
  is an **allowlisted** case that regresses. Pick the next target off the
  per-section backlog; close a construct cluster; ratchet the now-passing cases in.

## The oracle in one paragraph

`parse(text)` → lossless rowan CST. `project_to_rd(text)`
(`src/roxygen/project_rd.rs`, built) projects it into roxygen2's Rd **section**
shape—e.g.
`(\format (TEXT "...") (\describe (\item (TEXT "a") (TEXT "first"))))`—translating
only *encoding* differences and leaving genuine *modeling*
divergences faithful so they surface, while **excluding** roclet-*generated*
scaffolding (`\name`/`\alias`/`\usage`/`\arguments`). The R driver
`tests/oracle/roxygen_oracle.R` is the source of truth: `block-to-sections` runs
`roxygen2::roc_proc_text(rd_roclet(), src)` → `format()` → `tools::parse_Rd` → the
parser-owned section subtrees as a canonical, sorted S-expression (drops srcref, the
`% Generated` header, prose line-wrapping, and the roclet-only macros; `\examples`
bodies become `...`). (`block-to-tree` is the whole-topic variant the fixed-point
net still uses.) The grammar is **Rd-first,
markdown-second** (markdown only under the resolved `@md` mode); markdown is
CommonMark core + the GFM `table` extension, `hardbreaks = TRUE`.

## Markdown tenet (non-negotiable)

The **end goal for the markdown layer is full CommonMark parity—nothing less**.
roxygen2 delegates to `cmark`/`cmark-gfm`, so any "pragmatic subset" is a parity
*gap*, never an acceptable end state. The early inline recognizers were written as
local, line-scoped span scanners in the lexer (`scan_md_emphasis` etc.); that
**shape is wrong** for CommonMark, whose inline grammar is a non-local,
whole-block **delimiter-stack** pass (block parse → inline parse). The agreed
direction is a real **block→inline pass** (`docs/design/roxygen-inline-pass.md`)
that emphasis migrates into first, then links/code/etc. When you touch a markdown
construct: if the current local scanner can't model it correctly, do **not** widen
the scanner with heuristics—that entrenches the wrong shape. Either land it in
the inline pass or record the gap as backlog toward full parity. A bail-to-literal
is a stopgap (keeps structure never *wrong*), never a target.

## Two checks (don't conflate them)

1. **Projector parity—the primary engine, a CI-safe hard gate (EXISTS, Phase 1
   skeleton; `tests/roxygen_projector.rs`).** `project_to_rd(parse(x))`
   (`src/roxygen/project_rd.rs`) vs a **pinned** `<stem>.rdtree` minted once by
   roxygen2 and committed per curated case. *Pinned ⇒ no R at test time ⇒ plain
   `cargo test`* (`task roxygen-projector`). R only **refreshes** pins
   (`task roxygen-projector-refresh`, the driver's `block-to-sections` op; roxygen2
   version in `.roxygen2-source`). It compares **section-body subtrees**, excluding
   roclet-*generated* scaffolding (`\name`/`\alias`/`\usage`/`\arguments`)—those
   are generation, not parsing. This is what grows the parser; allowlist-gated by
   `tests/oracle/roxygen-projector-allowlist.txt`.
2. **Formatter fixed-point—a secondary correctness net (exists,
   `tests/roxygen_oracle.rs`).** `roxygen2(format(x)) == roxygen2(x)` at tree
   level: formatting must never change rendered Rd. `#[ignore]`d only because it
   shells out to R; **strict** (asserts) when R is present; accepted divergences
   in `tests/roxygen_oracle_blocked.toml`. **Cosmetic-blind by design** (a reflowed
   `\describe` renders identical Rd → passes here), so it is *not* the parser-growth
   driver—the projector is. Its pure-Rust analog is
   `project(parse(x)) == project(parse(format(x)))` (no R).

It checks **meaning, not layout.** A cosmetic defect that renders to the *same*
Rd (e.g. a `\describe{}` reflowed into a run-on in non-markdown mode) is
preserving in the fixed-point check—that is exactly why the projector parity
gate (which compares *structure*, so it sees the un-atomic `\describe`) is the
real driver.

Current phase, baseline, and the ranked next target live in `RECAP.md` (not
duplicated here, so this skill stays timeless).

## Failure buckets (classify before fixing)

- **Projector gap**—arity parses fine, but `project_rd.rs` emits the wrong
  Rd shape (missing node arm, wrong macro head, encoding not unwrapped). Fix the
  projector—but only as a faithful encoding translation.
- **Parser gap**—arity can't model it (loose tokens, missing block
  structure). Fix `src/parser/roxygen.rs` (+ `src/syntax.rs`, `tree_builder.rs`,
  AST wrappers). This is the bulk of the growth work.
- **Deliberate divergence**—arity intentionally differs from roxygen2.
  Record: `blocked` it with a rationale; do not "fix."
- **Cosmetic-only**—renders identical Rd (fixed-point preserving). The
  projector parity gate catches it as a *structural* divergence; that is the
  right signal.

## Workflow (per session)

1. **Read `RECAP.md`** (traps, settled decisions, latest session, ranked next
   target). Prefer a user-named target.

2. **Baseline:** `cargo test` is green—this includes `task roxygen-projector`
   (the pure-Rust projector-parity gate; **the primary driver**), whose
   `ROXYGEN_PROJECTOR.md` backlog (the **divergent** curated cases) is the usual
   source of next targets. Optionally, with R: `task roxygen-oracle` (curated
   fixed-point, strict) and `task roxygen-harvest` (broad coverage net). "No
   regression" = still green + the projector allowlist count holds-or-grows at the end.

3. **Probe roxygen2 for the exact target shape** before coding—it is the
   oracle:
   ```sh
   printf '%s\n' "#' @details" "#' \\itemize{" "#'   \\item one" "#' }" "#' @name x" "NULL" \
     | Rscript tests/oracle/roxygen_oracle.R block-to-tree
   ```

4. **Pick a target**: a cluster of divergences/unsupported cases sharing one
   root cause, or a small high-value construct (e.g. multi-line `\describe`).

5. **Classify** into a bucket, apply the **smallest** parser fix. Inspect the
   CST via `cargo run -q -- parse <file>` and (once it exists) the projection.

6. **TDD fixture**—add `tests/fixtures/parser/roxygen_<name>/input.R`,
   assert losslessness (`cat file | cargo run -q -- parse --verify --quiet`),
   review + accept the snapshot (`cargo insta review`). **Read the CST before
   accepting.**

7. **Wire into the projector corpus + pin**—add the case as
   `tests/oracle/corpus/roxygen/<stem>.R`, mint its `<stem>.rdtree` from roxygen2
   (`task roxygen-projector-refresh`), and confirm `project_to_rd(parse(x))`
   matches it exactly (`task roxygen-projector`). Grow a **faithful** projector arm
   for any new node—never widen the projector to paper over a CST gap.

8. **Ratchet**—add the now-matching case's stem to
   `tests/oracle/roxygen-projector-allowlist.txt`. Matching count must go **up** (or
   hold); allowlisted regressions must stay 0. (The curated fixed-point corpus keeps
   its `blocked` discipline for deliberate semantic divergences.)

9. **Guardrails:**
   ```sh
   cargo test                 # includes the projector gate (no R)
   cargo clippy --all-targets --all-features -- -D warnings
   cargo fmt -- --check
   task roxygen-oracle        # optional, needs R (fixed-point net)
   ```

10. **Update `RECAP.md`** (write the new "Latest session", demote the old one to a
    one-liner under "Earlier sessions", refresh the ranked next target and any new
    trap). **Commit**
    (Conventional Commits; `feat(parser)` for new capability, `test(roxygen)` for
    test-infra-only; the pre-commit hook runs clippy + rustfmt + panache-format—never
    `--no-verify`). Don't push unless asked; commit straight to `main`
    (trunk-based).

## Session boundaries

`RECAP.md` is the handoff—a committed target with `RECAP.md` updated (the end of
step 10) is a **clean stop**, so nothing load-bearing lives only in chat. One
target per context window is the intended cadence; the rolling log exists so the
next session re-reads `RECAP.md` (step 1) and continues on a lean context. Only
continue mid-target within one context (uncommitted work, a half-applied fix, a
failing test you're chasing).

## Key files

- `src/parser/roxygen.rs`—roxygen lexing → block-tree → events. The growth
  site.
- `src/syntax.rs`—`SyntaxKind` (append after `ROXYGEN_TAG`; bump `COUNT`).
- `src/parser/tree_builder.rs`—`TokKind` → `SyntaxKind` (single source of
  truth).
- `src/ast/nodes.rs`—typed wrappers (`ast_node!`).
- `src/roxygen/project_rd.rs`—the projector (**built, Phase 1 skeleton**).
  Faithful encoding translation; never patched to pass. The growth site for new
  faithful arms as the parser learns structure.
- `tests/roxygen_projector.rs`—**the primary gate** (pure Rust, no R, plain
  `cargo test`): `project_to_rd(parse(x))` vs pinned `<stem>.rdtree`, allowlist-gated;
  writes `ROXYGEN_PROJECTOR.md`.
- `tests/oracle/roxygen-projector-allowlist.txt`—projector allowlist (by file
  stem or slug); `tests/oracle/corpus/roxygen/<stem>.rdtree`—the curated pins.
- `tests/oracle/roxygen-projector-blocked.txt`—projector `blocked` list
  (deliberate non-targets, inline `# reason`; asserted disjoint from the
  allowlist, excluded from the backlog).
- `tests/oracle/corpus/commonmark-spec.jsonl` (+ `-sections.jsonl` pins)—the
  **whole** CommonMark spec as a measured backlog, `{slug, input, section}` built
  by `scripts/build-commonmark-corpus.R ... ALL` (`task roxygen-spec-corpus`).
- `tests/roxygen_oracle.rs`—secondary R harness: curated `roxygen_oracle_report`
  (strict) + harvested `roxygen_harvested_{report,allowlist}` (opt-in). `#[ignore]`d;
  skip-if-no-R.
- `tests/oracle/roxygen_oracle.R`—the R driver (`block-to-sections`/`sections-batch`
  for projector pins; `block-to-tree`/`rd-to-tree`/`trees-batch` for fixed-point).
- `tests/oracle/corpus/roxygen/`—**curated** dir corpus (strict);
  `tests/roxygen_oracle_blocked.toml`.
- `tests/oracle/corpus/roxygen.jsonl`—**harvested** corpus (slug-keyed), gated by
  `tests/oracle/roxygen-allowlist.txt`. Mined by `scripts/harvest-roxygen-corpus.R` from
  `roxygen2-ref/` (gitignored clone; version in `tests/oracle/.roxygen2-source`).
  Re-seed allowlist: `task roxygen-harvest-seed`.
- `.claude/skills/roxygen-parity/RECAP.md`—rolling session log (read first).
- `.claude/skills/roxygen-parity/ROXYGEN_ORACLE.md` (curated) +
  `ROXYGEN_HARVEST.md` (harvested backlog)—generated reports (regenerated by
  `task roxygen-oracle`/`task roxygen-harvest`; tracked, do not hand-edit).

## Traps

- **R is needed for the oracle, not for the gate.** The projector-parity gate
  runs on pinned `expected.rdtree` with no R. Only minting/refreshing pins and
  the fixed-point check need `Rscript`. devenv ships R 4.x + roxygen2 7.3 +
  commonmark.
- **parse_Rd tags brace-group arg wrappers as `TEXT` but they are *lists*.** In
  the canonical serializer, coalesce only genuine **character** TEXT leaves
  (prose); never merge across list-wrapped groups, or `\item{term}{def}`
  collapses to one atom. (`is_text_leaf` in `roxygen_oracle.R` is the guard.)
- **`hardbreaks = TRUE` but soft-wrapping prose is semantically safe**—roxygen2
  inserts no `\cr` for a soft-wrapped paragraph, so wrapping must
  canonicalize identically (coalesce TEXT runs). A *real* hard break (trailing
  ``  ``/`\\`) is a distinct node—preserve it.
- **`\examples` bodies are reformatted R** (Tenet 1), so the serializer replaces
  their content with `...`. Don't try to match example text.
- **Cosmetic ≠ semantic.** Don't expect the fixed-point check to catch layout
  bugs; that's the projector parity gate's job.
- **Mode matters to structure, not just interpretation.** With markdown off, a
  `-` list is literal Rd prose (no `\itemize`); the parser is mode-keyed, so the
  CST (and thus the projected Rd) differs by mode. Pin both modes where
  relevant.

## Report-back format

1. Construct landed (e.g. "multi-line `\describe` block macro").
2. Oracle: divergence/unsupported before → after (regressions must be zero).
3. Allowlist + blocked counts before → after.
4. Files changed, by failure bucket.
5. New fixtures + new blocked entries (with rationale).
6. Ranked next target. If ending uncommitted or red, say so and list the red
   tests.
