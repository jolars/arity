---
name: smoke-test-triage
description: Triage and fix arity corpus smoke-test regressions (losslessness,
  idempotence, format-error, panic) reported by the weekly scan of real R
  package repos and the GitHub issues it files.
---

Use this skill when asked to investigate failures reported by the corpus
smoke-test scan (`.github/workflows/smoke-test.yml`) or the `CI: corpus
regression in <repo> (<category>)` issues it files—especially losslessness and
idempotence regressions.

## How the scan works (read this before triaging)

The workflow shallow-clones every repo in `TARGET_REPOS`, then runs the Tier 0
corpus test (`tests/corpus.rs`) **once, in-process, over all of them** in CI
mode (`ARITY_CORPUS=<dir> ARITY_CORPUS_REPORT=<tsv>`). The test writes one
tab-separated record per failure—`relative-key \t category \t message`—and
returns cleanly; the workflow enriches those with the repo + SHA and files
issues. Consequences that shape triage:

- The unit of work is a **category bucket per repo**, not a file. One issue
  covers every file in that bucket; it lists at most ten samples, and the named
  sample is often not the most actionable one. **Re-run the scan yourself over
  the whole clone** and triage by root cause, not by the sample.
- Failure categories are `losslessness`, `idempotence`, `format-error`, and
  `panic`.
- **A file arity cannot parse is skipped, not failed.** Parse diagnostics are a
  known limitation, not a regression. So a `format-error` is never "just parse
  errors"—it is the formatter refusing input it *was* able to parse.
- Losslessness is checked on **every** file, including the skipped-unparseable
  ones.
- The scan is weekly and issues are deduped by a marker
  (`arity-corpus-key:repo=…;type=…`). An issue that stops reproducing gets a
  "No longer reproducing" comment rather than being closed automatically.

**Check the `ALLOWLIST` in `.github/workflows/smoke-test.yml` first.** It holds
`repo|path|category` entries for failures already triaged as out of scope, and
matching failures are suppressed from the report. An issue naming an
already-listed file means the scan predates the entry—confirm and close rather
than re-triaging.

## Goals

1. Reproduce the exact failure from the report.
2. Minimize to a stable local fixture.
3. Add regression coverage in the right test surface.
4. Fix root cause (not symptom).
5. Validate targeted cases, then the whole target repo, then the full suite.

## Triage workflow

1. **Read the issue.** Note the category, the target repo + commit SHA, the
   arity version/commit used by the scan, the sample files, and the per-file
   detail messages (the `message` column—for `format-error` and `panic` this
   is the actual error text and is usually the fastest route to the root
   cause). The `corpus-scan-results` artifact holds the full `failures.tsv`
   plus `corpus.log` (the run's totals, including how many files were skipped
   as unparseable); fetch it with
   `gh run download <run-id> -n corpus-scan-results` (the run id is in the
   issue's workflow-run link).

2. **Reproduce.** Clone the target repo at the exact SHA from the issue, then
   re-run the whole scan against it—the bucket, not the sample, is the thing
   you are triaging:

   ```sh
   git clone https://github.com/<repo>.git /tmp/target
   git -C /tmp/target checkout <repo_sha>
   ARITY_CORPUS=/tmp/target ARITY_CORPUS_REPORT=/tmp/before.tsv \
     cargo test --release --test corpus -- --ignored --nocapture
   cut -f2 /tmp/before.tsv | sort | uniq -c     # bucket sizes
   cut -f2,3 /tmp/before.tsv | sort | uniq -c   # distinct messages per category
   ```

   Then narrow to one file with the CLI. `arity` here means the current
   checkout (`cargo run --release --` or `target/release/arity`), never an
   installed release:

   ```sh
   arity parse --verify --quiet <file>   # losslessness
   arity format --verify <file>          # idempotence
   arity format < <file> > /tmp/once.R   # pass 1 (stdin form!)
   arity format < /tmp/once.R > /tmp/twice.R
   diff /tmp/once.R /tmp/twice.R         # the idempotence divergence
   ```

   Two traps:
   - `arity format <path>` **rewrites the file in place**. Always use the
     stdin form (`arity format < file`) when capturing passes out of a clone.
   - `arity parse --verify` exits 1 on *parse diagnostics* before it ever runs
     the losslessness check, so on an unparseable file it tells you nothing
     about losslessness. The corpus runner checks `reconstruct(raw) == raw`
     independently—trust that, or call `reconstruct` directly.
   - `arity format --verify` stops at the first failing path, so give it one
     file at a time.

3. **Minimize.** Reduce to the smallest snippet that still reproduces, keeping
   the source realistic. Common triggers in R sources: roxygen `#'` blocks and
   mid-line `#'` markers, comments in awkward positions (assignment RHS, inside
   argument lists, before `else`), `if`/`else` chains, pipes (`|>`, `%>%`) and
   user `%op%` operators, `[`/`[[` subsetting, formulas, `function` defaults,
   backticked and non-ASCII names, `...`/`..1`, trailing commas, `;`-separated
   statements, and CRLF line endings. Confirm the reproduction is deterministic
   across repeated runs.

4. **Classify before fixing—check the CST before any formatter-side fix.**

   - **Losslessness ⇒ always a parser bug** (Tenet 4: `reconstruct(text)` is
     `text`). Fix in `src/parser/`, never by compensating in the formatter.
   - **Idempotence ⇒ find which pass diverges and why.** Diff pass 1 against
     pass 2 (above), then run `arity parse` on the input *and* on the pass-1
     output. If the CST of the formatted output is structurally wrong—an
     argument list re-shaped after reflow, a comment bound to the wrong node, a
     construct that re-parses differently once the line breaks moved—**the bug
     is parser-side no matter which pass shows the symptom** (Tenet 3).
     Idempotence drift is usually the downstream symptom of upstream shape
     divergence. `crates/arity-parser/tests/air_parser_harness.rs` and
     `task air-compat` are useful
     structural references for a suspicious parse.
   - **Anti-pattern: fixing in the formatter because the symptom lives there.**
     If you find yourself reaching for a formatter special case to make
     pass1 == pass2 (normalizing a whitespace run, hard-coding a node shape),
     stop and re-check the parse. A formatter fix is only correct when the CST
     is already right and the divergence is purely in rendering—and it must be
     a *rule*, not an exception (Tenet 1). Arity does **not** honor persistent
     line breaks, so "make the output follow the input's layout" is never the
     fix.
   - **`format-error` ⇒ the formatter refused input it parsed.** The message
     is the classifier; in practice it is nearly always
     `ambiguous construct (<site>)`—a shape the IR rules at that site do not
     cover. Two outcomes, and they are decided by looking at the CST:
     - a genuine formatter gap: add the rule in `src/formatter/rules/` plus a
       fixture;
     - a parser gap that hands the formatter a shape no rule could lay out. Fix
       the parser (Tenet 3). Precedent: juxtaposed statements
       (`12 14` on one line) were silently accepted, leaving the formatter a
       line with two elements and no rule; the fix was a parser diagnostic, and
       the file then correctly became "unparseable, skipped" (issue #68, commit
       `e98b877`).
   - **`panic` ⇒ always a bug and the highest priority**, in parsing or
     formatting. Re-run with `RUST_BACKTRACE=1` to get the frame; a panic on
     real-world input means an invariant the code assumes is not one.
   - **Genuinely invalid R ⇒ out of scope; allowlist it, do not "fix" it.** The
     bar is that *R itself* rejects the file—check with
     `R --vanilla -q -e 'parse("<file>")'` (R is in the devenv shell)—or that
     the file is not R at all (a template with placeholder syntax, generated
     output committed by mistake). Record it in the `ALLOWLIST` in
     `.github/workflows/smoke-test.yml` as `repo|path|category`, grouped under
     a comment naming the root cause and the issue. Only the named category is
     suppressed, so the file still reports if it later fails a *different*
     way—an allowlist entry can never mask a new regression. Be strict: a real
     gap you simply have not fixed yet is **not** out of scope, and
     allowlisting it buries the work. When in doubt leave it failing and say so
     in the report.
   - If uncertain, state the best hypothesis and why before implementing—and
     include the relevant `arity parse` output in the hypothesis.

5. **Add regression fixture(s) first (TDD—watch them fail before fixing).**
   Both fixture suites are **hand-registered**; a directory alone does nothing.

   - Parser bugs (losslessness, mis-parse, panic-in-parse):
     `crates/arity-parser/tests/fixtures/parser/<case>/input.R`, then add
     `"<case>"` to `fixture_names()` in
     `crates/arity-parser/tests/parser_snapshots.rs`. The harness snapshots the
     CST and the diagnostics and asserts the lossless round-trip (only
     `air_error_*` cases are exempt, and those must produce diagnostics).
     Accept snapshots with `cargo insta review`—never hand-write a `.snap`.
   - Formatter bugs (idempotence, `format-error`):
     `crates/arity-formatter/tests/fixtures/formatter/<case>/{input.R,expected.R}`,
     then add `"<case>"` to `fixture_names()` in
     `crates/arity-formatter/tests/formatter.rs`. That harness asserts the
     expected output, and separately that the input parses clean, the output
     parses clean, the output round-trips losslessly, and formatting is
     idempotent—so an idempotence bug is pinned by the fixture alone.
   - Prefer one focused fixture per bug; do not update unrelated golden files.

6. **Fix at root cause.**
   - parser/CST and losslessness bugs → `src/parser/`
   - layout, idempotence, and `ambiguous construct` bugs → `src/formatter/`
   - never paper over by editing expected outputs or accepting a changed
     snapshot without understanding the diff
   - preserve behavior for unrelated fixtures

7. **Validate.**
   - Targeted first: the new fixture
     (`cargo test --test parser_snapshots <case>` /
     `cargo test --test formatter <case>`) and the CLI reproduction from step 2.
   - Then the whole suite: `cargo test`,
     `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt`.
   - **Re-measure over the whole target repo, not just the sample.** Build a
     baseline binary from the unfixed tree (`git stash && cargo build
     --release`, copy it aside, restore), re-run the corpus scan before and
     after, and diff the per-file records:

     ```sh
     ARITY_CORPUS=/tmp/target ARITY_CORPUS_REPORT=/tmp/after.tsv \
       cargo test --release --test corpus -- --ignored --nocapture
     diff <(sort /tmp/before.tsv) <(sort /tmp/after.tsv)
     ```

     A parser gate that fixes one shape routinely makes another *worse*, and
     only a per-file diff catches it. Report the counts and treat any file that
     regressed as a blocker, not a footnote.
   - **Watch the skipped-unparseable count** in the corpus runner's stderr
     summary. Making files parse *moves them into the checked set*, where they
     can hit pre-existing formatter gaps. Those are not regressions, but they
     are new failures the next scan will file—record them as distinct items in
     `TODO.md` under the affected subsystem, retain the originating issue link,
     and say so in the report.
   - For formatter rule changes, run `task air-compat` and triage any new
     divergence per `AGENTS.md` (adopt, or record in
     `crates/arity-formatter/tests/air_compat_allowlist.toml` with a
     rationale). It is a gauge, never
     a gate.
   - Drop any `ALLOWLIST` entry the fix makes stale—the file now simply passes
     and records nothing. A stale entry is the one way the allowlist *can* mask
     a real regression.

## Arity-specific guidance

- Formatting is deterministic and rule-based (Tenet 1): the input's line breaks
  never influence the result, unlike air. Never "fix" an idempotence failure by
  mirroring the input's layout, and push back on per-construct special cases.
- Comments are trivia the parser preserves losslessly; a losslessness diff that
  drops or relocates a comment is a CST attachment bug, and a comment that
  moves between format passes is an idempotence bug in comment relocation
  (`src/formatter/trivia.rs` and the `rules/` comment paths).
- Roxygen (`#'`) blocks have their own formatting path and their own oracle
  (`roxygen-parity` skill, `tests/roxygen_format_stability.rs`). An idempotence
  failure whose diff sits inside a `#'` block belongs there.
- Line endings: the corpus contains CRLF files; keep the original ending when
  minimizing (`tests/line_endings.rs` is the home for ending-specific cases).
- The corpus test catches panics per file, so a panic bucket does not stop the
  scan—but it does mean the run's other results came from a process that hit a
  bug. Fix panics first.

## Report-back format

When done, report:

1. Whether the issue reproduced, and the exact command.
2. Minimal reproducer summary.
3. Fixture(s) added, and where they are registered.
4. Root cause and the code path changed.
5. Validation commands and outcomes, including the before/after failure counts
   over the whole target repo and explicit confirmation that no file regressed.
6. What you did **not** fix: the remaining buckets by root cause, which were
   allowlisted as out of scope (with the reason), which are open gaps still
   worth work, and any newly-checked files that surfaced pre-existing formatter
   gaps. Say plainly whether the issue can be closed or should stay open—a scan
   issue is rarely one bug, and a partial fix reported as a whole one is worse
   than no fix.
