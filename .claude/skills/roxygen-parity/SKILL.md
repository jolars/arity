---
name: roxygen-parity
description: >-
  Grow arity's roxygen2 + markdown CST toward roxygen2 itself using the
  differential oracle. roxygen2 renders #' blocks to .Rd; the projector at
  src/roxygen/project_rd.rs walks arity's CST and emits the same Rd-tree shape,
  and the harness in tests/roxygen_oracle.rs diffs each fixture against a pinned
  expected tree (allowlist) or records a deliberate divergence (blocked). Use
  this skill to pick the next gap from the corpus, add the grammar plus projector
  support, lock it with a fixture + pinned tree, and ratchet the now-passing case
  into the allowlist. The projector is a test-only faithful diagnostic: a
  divergence means the CST (or the encoding translation) is wrong — never patch
  the projector to make a case pass; fix the parser.
---

Use this skill to advance arity's roxygen2 parser parity, work the oracle
backlog, or "take the next gap." The full roadmap is the plan at
`~/.claude/plans/i-want-to-start-snoopy-haven.md` and the `TODO.md` roxygen
section; this skill is the per-session loop within it.

## Why strict (read first)

A roxygen-oracle divergence is **not** an air-compat-style soft "deviation." It
means arity changed what roxygen2 *renders* from the user's docs --- a behavior-
preservation bug, the same family as a losslessness or idempotence failure. So
this mirrors fatou's `parser-parity` (strict conformance, `allowlist` +
`blocked`, faithful projector, pinned expecteds), **not** air-compat's
"subordinate to Tenet 1." Every corpus case is accounted for: PASS (allowlisted)
or BLOCKED (a deliberate/deferred divergence with a one-line rationale). An
unaccounted divergence is RED.

## The oracle in one paragraph

`parse(text)` → lossless rowan CST. `project_to_rd(&cst)` (planned,
`src/roxygen/project_rd.rs`) projects it into roxygen2's Rd-tree shape --- e.g.
`(\format (TEXT "...") (\describe (\item (TEXT "a") (TEXT "first"))))` ---
translating only *encoding* differences and leaving genuine *modeling*
divergences faithful so they surface. The R driver
`tests/oracle/roxygen_oracle.R` is the source of truth: `block-to-tree` runs
`roxygen2::roc_proc_text(rd_roclet(), src)` → `format()` → `tools::parse_Rd` → a
canonical S-expression (drops srcref, the `% Generated` header, and prose
line-wrapping; `\examples` bodies become `...`). The grammar is **Rd-first,
markdown-second** (markdown only under the resolved `@md` mode); markdown is
CommonMark core + the GFM `table` extension, `hardbreaks = TRUE`.

## Two checks (don't conflate them)

1. **Projector parity --- the primary engine, a CI-safe hard gate (planned, the
   build target).** `project_to_rd(parse(x))` vs a **pinned** `expected.rdtree`
   minted once by roxygen2 and committed per fixture. *Pinned ⇒ no R at test
   time ⇒ plain `cargo test`.* R only **refreshes** pins (a script; pinned to a
   roxygen2 version in `.roxygen2-source`). This is what grows the parser.
2. **Formatter fixed-point --- a strict correctness check (exists today,
   `tests/roxygen_oracle.rs`).** `roxygen2(format(x)) == roxygen2(x)` at tree
   level: formatting must never change rendered Rd. `#[ignore]`d only because it
   shells out to R; **strict** (asserts) when R is present; accepted divergences
   in `tests/roxygen_oracle_blocked.toml`. Its pure-Rust analog once the
   projector exists is `project(parse(x)) == project(parse(format(x)))` (no R).

It checks **meaning, not layout.** A cosmetic defect that renders to the *same*
Rd (e.g. a `\describe{}` reflowed into a run-on in non-markdown mode) is
preserving in the fixed-point check --- that is exactly why the projector parity
gate (which compares *structure*, so it sees the un-atomic `\describe`) is the
real driver.

## Current status

Phase 0 is **done**: `devenv.nix` declares `roxygen2` + `commonmark`; the R
driver, seed corpus (`tests/oracle/corpus/roxygen/*.R`), the strict fixed-point
harness, `tests/roxygen_oracle_blocked.toml`, `task roxygen-oracle`, and
`ROXYGEN_ORACLE.md` exist; baseline 100% Rd-preserving. **Next build target
(Phase 1):** the CST nodes for inline/multi-arg Rd macros,
`src/roxygen/project_rd.rs`, and the pinned projector-parity gate
(`expected.rdtree` per fixture, `allowlist` + `blocked`, running in plain
`cargo test`).

## Failure buckets (classify before fixing)

- **Projector gap** --- arity parses fine, but `project_rd.rs` emits the wrong
  Rd shape (missing node arm, wrong macro head, encoding not unwrapped). Fix the
  projector --- but only as a faithful encoding translation.
- **Parser gap** --- arity can't model it (loose tokens, missing block
  structure). Fix `src/parser/roxygen.rs` (+ `src/syntax.rs`, `tree_builder.rs`,
  AST wrappers). This is the bulk of the growth work.
- **Deliberate divergence** --- arity intentionally differs from roxygen2.
  Record: `blocked` it with a rationale; do not "fix."
- **Cosmetic-only** --- renders identical Rd (fixed-point preserving). The
  projector parity gate catches it as a *structural* divergence; that is the
  right signal.

## Workflow (per session)

1. **Read the plan** (`~/.claude/plans/i-want-to-start-snoopy-haven.md`) for the
   phase you're in and the next target. Prefer a user-named target.

2. **Baseline:** `cargo test` is green; `task roxygen-oracle` passes (or note
   the blocked set). "No regression" = still green at the end.

3. **Probe roxygen2 for the exact target shape** before coding --- it is the
   oracle:
   ```sh
   printf '%s\n' "#' @details" "#' \\itemize{" "#'   \\item one" "#' }" "#' @name x" "NULL" \
     | Rscript tests/oracle/roxygen_oracle.R block-to-tree
   ```

4. **Pick a target**: a cluster of divergences/unsupported cases sharing one
   root cause, or a small high-value construct (e.g. multi-line `\describe`).

5. **Classify** into a bucket, apply the **smallest** parser fix. Inspect the
   CST via `cargo run -q -- parse <file>` and (once it exists) the projection.

6. **TDD fixture** --- add `tests/fixtures/parser/roxygen_<name>/input.R`,
   assert losslessness (`cat file | cargo run -q -- parse --verify --quiet`),
   review + accept the snapshot (`cargo insta review`). **Read the CST before
   accepting.**

7. **Wire into the oracle corpus + pin** --- add the case under
   `tests/oracle/corpus/roxygen/`, mint its `expected.rdtree` from roxygen2 (the
   refresh script), and confirm `project_to_rd(parse(x))` matches it exactly.

8. **Ratchet** --- move the now-passing case into the allowlist; for a genuine
   deliberate divergence, `blocked` it with a rationale instead. Pass count must
   go **up** (or hold); unaccounted divergences must stay 0.

9. **Guardrails:**
   ```sh
   cargo test
   cargo clippy --all-targets --all-features -- -D warnings
   cargo fmt -- --check
   task roxygen-oracle   # needs R
   ```

10. **Update `TODO.md`** (mark the grammar bullet, trim the backlog) and the
    plan's phase status. **Commit** (Conventional Commits; `feat(parser)` for
    new capability, `test(roxygen)` for test-infra-only; the pre-commit hook
    runs clippy + rustfmt + panache-format --- never `--no-verify`). Don't push
    unless asked; commit straight to `main` (trunk-based).

## Key files

- `src/parser/roxygen.rs` --- roxygen lexing → block-tree → events. The growth
  site.
- `src/syntax.rs` --- `SyntaxKind` (append after `ROXYGEN_TAG`; bump `COUNT`).
- `src/parser/tree_builder.rs` --- `TokKind` → `SyntaxKind` (single source of
  truth).
- `src/ast/nodes.rs` --- typed wrappers (`ast_node!`).
- `src/roxygen/project_rd.rs` --- the projector (**to build**). Faithful; never
  patched to pass.
- `tests/roxygen_oracle.rs` --- harness (strict; `#[ignore]`d; skip-if-no-R).
- `tests/oracle/roxygen_oracle.R` --- the R driver (oracle + pin minting).
- `tests/oracle/corpus/roxygen/` --- corpus;
  `tests/roxygen_oracle_blocked.toml`.

## Traps

- **R is needed for the oracle, not for the gate.** The projector-parity gate
  runs on pinned `expected.rdtree` with no R. Only minting/refreshing pins and
  the fixed-point check need `Rscript`. devenv ships R 4.x + roxygen2 7.3 +
  commonmark.
- **parse_Rd tags brace-group arg wrappers as `TEXT` but they are *lists*.** In
  the canonical serializer, coalesce only genuine **character** TEXT leaves
  (prose); never merge across list-wrapped groups, or `\item{term}{def}`
  collapses to one atom. (`is_text_leaf` in `roxygen_oracle.R` is the guard.)
- **`hardbreaks = TRUE`but soft-wrapping prose is semantically safe** ---
  roxygen2 inserts no `\cr` for a soft-wrapped paragraph, so wrapping must
  canonicalize identically (coalesce TEXT runs). A *real* hard break (trailing
  ``  `` / `\\`) is a distinct node --- preserve it.
- **`\examples`bodies are reformatted R** (Tenet 1), so the serializer replaces
  their content with `...`. Don't try to match example text.
- **Cosmetic ≠ semantic.** Don't expect the fixed-point check to catch layout
  bugs; that's the projector parity gate's job.
- **Mode matters to structure, not just interpretation.** With markdown off, a
  `-` list is literal Rd prose (no `\itemize`); the parser is mode-keyed, so the
  CST (and thus the projected Rd) differs by mode. Pin both modes where
  relevant.

## Report-back format

1. Construct landed (e.g. "multi-line `\describe` block macro").
2. Oracle: divergence / unsupported before → after (regressions must be zero).
3. Allowlist + blocked counts before → after.
4. Files changed, by failure bucket.
5. New fixtures + new blocked entries (with rationale).
6. Ranked next target. If ending uncommitted or red, say so and list the red
   tests.
