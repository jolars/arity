---
name: linter-investigation
description: Investigate arity's linter (and, secondarily, its parser) against a
  real-world R codebase. Clone a target repo, lint it, and triage the diagnostics
  for false positives, incorrect spans, and unsafe autofixes; parse failures on
  valid R are caught along the way. Every suspected bug is confirmed against real
  R via `Rscript` before it is called a bug. Use when asked to stress-test,
  investigate, or triage the linter (or parser) over an external repo or corpus.
---

Point arity's linter at a large body of real R code and hunt for **linter
quality bugs**: false positives, incorrect spans, and unsafe fixes. This is the
primary goal. **Parse failures are a secondary catch**—a parse error blocks
linting a file, so they surface naturally, and a parse failure on *valid* R is a
real parser bug worth reporting—but the center of gravity is the linter, not a
full parser audit.

This is **distinct from the `smoke-test-triage` skill.** That one reacts to the
weekly automated corpus scan's *formatter* regressions (losslessness,
idempotence, format-error, panic) filed as GitHub issues. This skill is
proactive and interactive: you choose a repo and go looking for linter/parser
quality problems. Formatter losslessness and idempotence are out of scope
here—leave them to `smoke-test-triage`.

## The core principle (read first)

**A finding is only a bug once the oracle says arity is wrong.** The oracle is
real R: `Rscript`. Real codebases contain *intentionally* invalid files
(bug-repro fixtures like `tests/Pkgs/PR*`, deliberately-broken regression
inputs), so a diagnostic on those is correct, not a bug. Before reporting
anything, classify each suspicious finding into exactly one of:

- **True positive** — arity is right; move on.
- **False positive** — arity flags legitimate R. The highest-value find.
- **Incorrect span** — the finding is real but the caret underlines the wrong
  tokens.
- **Unsafe fix** — the autofix produces code that doesn't parse, changes
  semantics, or drops trivia (a comment). Test the fix, don't eyeball it.
- **Parser bug** — valid R that arity fails to parse or mis-parses (surfaces as
  `syntax-error`, or as a wrong CST shape). Confirm validity with `Rscript`.

## Workflow

1. **Target.** Take the repo from the user's argument: a GitHub `owner/name`, a
   full clone URL, or a local path. If none is given, propose a good default
   (`wch/r-source` is large and idiomatic; a tidyverse package is smaller) and
   confirm before cloning.

2. **Setup (parallel/background).** Build the release binary and shallow-clone
   the target into the **session scratchpad directory** (not bare `/tmp`—honor
   the global scratchpad convention), running both at once:

   ```sh
   cargo build --release
   git clone --depth 1 https://github.com/<owner>/<name>.git "$SCRATCH/<name>"
   ```

   Use `target/release/arity` for speed; a debug build over a big corpus is slow.

3. **Lint the tree, capture everything.** Findings print to **stderr**; capture
   both streams to a file:

   ```sh
   target/release/arity lint "$SCRATCH/<name>" >lint.out 2>lint.err
   ```

   **Gotcha:** arity currently *aborts the whole run* on the first non-UTF-8 file
   (`stream did not contain valid UTF-8`). If that happens, find the offender
   (`file` reports the encoding), move/rename it aside, and re-run. (This abort
   is itself a known robustness bug—see arity's `TODO.md`.)

4. **Summarize by rule.** Count findings per rule to prioritize the high-volume
   and high-risk buckets:

   ```sh
   grep -oE '(warning|error): [a-z-]+' lint.err | sort | uniq -c | sort -rn
   ```

   The semantic rules (`undefined-symbol`, `unused-binding`, `shadowed-builtin`)
   are the most false-positive-prone; the `syntax-error`/`error:` bucket is where
   parser bugs hide.

5. **Triage (the heart of the work).** For each priority rule, pull real
   findings (`grep -B1 -A6 'warning: <rule>' lint.err`), open the cited source
   line, and **reduce each suspect to a minimal reproducer** piped to the tool:

   ```sh
   printf 'x <- (a # c\n || b)\n' | target/release/arity lint
   printf '...' | target/release/arity parse            # inspect the CST shape
   printf '...' | target/release/arity parse --verify --quiet   # losslessness
   ```

   When a parse failure looks like a parser bug, **isolate the trigger by
   bisecting context**: vary one axis at a time (paren vs call-arg vs brace,
   comment vs no comment, operator vs operator) until the minimal failing shape
   is pinned. That is how the paren comment-continuation bug was isolated.

6. **Verify against real R.** Ground-truth every suspicion before reporting it.
   Deliver the newline *inside an R string* (`parse(text = "...\n...")`) so R
   sees a real line break—a `\n` typed as bare `-e` code is a literal backslash
   plus `n`, and a bare top-level snippet has different newline rules than the
   same code inside `()`/`[]`, so reproduce the *actual* context:

   ```sh
   # validity: R accepts it (prints "parses") but arity errored -> parser bug
   Rscript -e 'invisible(parse(text = "(a # c\n || b)")); cat("parses\n")'
   # semantics: confirm the value (prints 5) -> the comment-continuation is real
   Rscript -e 'cat(eval(parse(text = "(2 # c\n + 3)")), "\n")'
   ```

   If R parses/evaluates it and arity disagrees, it's a bug. If R also rejects
   it, arity is correct. (For anything fiddly, a heredoc `Rscript file.R` with
   real newlines sidesteps all escaping questions.)

7. **Fan out for volume (recommended).** A big corpus yields thousands of
   findings across many rules. Spawn parallel triage subagents—**one per
   rule-bucket**—each given: the absolute `target/release/arity` path, the
   `lint.err` path, the classification scheme above, and the `Rscript` oracle
   recipe. Have each return minimal reproducers, a verdict per FP category, and
   an overall false-positive-rate assessment. This is how a full sweep gets done
   in one pass instead of serially.

8. **Fix or record.** For the cleanest, well-isolated bugs, fix TDD-style, honoring
   arity's tenets (parsing bugs are fixed in the parser, never papered over in
   the formatter; losslessness is sacred):

   - Add a failing fixture first and **watch it fail** (parser: a case under
     `tests/fixtures/parser/<case>/` registered in `fixture_names()`; linter: a
     case under `tests/fixtures/lint/`). Reduce from the corpus.
   - Fix at the root cause; re-verify against `Rscript`.
   - Run the relevant suites plus gates: `cargo test`, `cargo clippy
     --all-targets --all-features -- -D warnings`, `cargo fmt -- --check`;
     `cargo insta accept` for new snapshots after reviewing them.

   Record everything you don't fix as follow-ups in `TODO.md`, in the house
   style (the parser and linter sections, with a minimal reproducer and the
   confirmed-correct R behavior). Commit only if the user asks—atomic,
   Conventional Commits, imperative subject ≤ ~60 chars.

9. **Report back.** State plainly: bugs found (fixed vs. documented) with
   copy-pasteable reproducers; false-positive categories per rule; any
   incorrect-span issues; which rules you verified clean; and the follow-ups you
   recorded. Be faithful about what was and wasn't oracle-verified.

## Arity-specific notes

- **Findings are on stderr**, not stdout. `lint` exits non-zero when it reports
  anything.
- **`undefined-symbol` needs a package root.** Base R ships `DESCRIPTION.in`,
  not `DESCRIPTION`, so cross-file resolution may not activate and every sibling
  function reads as undefined—a corpus artifact, not a rule bug. Sanity-check by
  dropping a real `DESCRIPTION` next to an `R/` dir and re-linting.
- **NSE is the FP frontier.** Formula `~` terms, `quote`/`substitute`/`bquote`/
  `expression` bodies, and implicit method vars (`.Generic`/`.Method`/`.Class`)
  are common false-positive sources for `undefined-symbol`.
- **Scope asymmetry** drives `unused-binding` FPs (e.g. a `for`-body binding read
  from the enclosing frame). Compare the `for` vs `while` shapes when a binding
  looks wrongly-flagged.
- **Autofix correctness** is *correctness, not layout* (Tenet 1): a fix must keep
  the code parseable and lossless, but need not respect line width. Test a fix by
  applying it and re-parsing, never by reading it.
