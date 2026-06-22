# roxygen-parity recap

Rolling log. Read top-to-bottom: persistent traps → settled decisions → progress →
latest session → earlier log. Keep ≤ ~300 lines; demote "Latest session" to a
one-liner under "Earlier sessions" each new session. The `roxygen-parity` skill
reads this first.

## Persistent traps & invariants

- **Projector is faithful, never compensating.** `project_rd.rs` translates encoding
  only; a divergence means the CST is wrong — fix the parser, never the projector.
- **Strict gate, not soft.** A divergence = a behavior-preservation bug. Every corpus
  case is allowlisted (pinned-pass) or blocked (deliberate, with rationale).
- **`parse_Rd` tags brace-group arg wrappers as `TEXT` but they are *lists*.**
  Coalesce only genuine character TEXT leaves (`is_text_leaf` in `roxygen_oracle.R`),
  or `\item{term}{def}` collapses into one atom.
- **`hardbreaks = TRUE`, yet soft-wrapped prose is semantically safe** (roxygen2 emits
  no `\cr`) — coalesce TEXT runs. A real hard break (trailing `  ` / `\\`) is a
  distinct node; preserve it.
- **`\examples` bodies are reformatted R** (Tenet 1); the serializer replaces their
  content with `...`. Don't try to match example text.
- **Cosmetic ≠ semantic.** The fixed-point check won't catch layout bugs (a reflowed
  `\describe` renders identical Rd); that's the projector parity gate's job.
- **`ROXYGEN_RD_MACRO` is a NODE, not a leaf** (since 2026-06-22e). Code that classifies
  it must use `el.kind()` (works for node or token), never `as_token()`. The macro token
  is still lexed atomically; the *tree builder* (`push_token`) expands it. Verbatim macros
  (`VERBATIM_RD_MACROS` in `parser/roxygen.rs`: `url,verb,samp,env,kbd,option`) don't
  recurse — body is one `…_VERB` leaf → projector emits `(VERB …)`. New verbatim macro in
  backlog? add it there (confirm via `parse_Rd`: a `{VERB}` child = verbatim).
- **Mode-keyed parse.** Markdown structure exists in the CST only when `@md` is on;
  the CST (and projected Rd) differs by mode — pin both modes where relevant.
- **Two corpora, two disciplines.** *Curated* dir corpus = strict (every case allowlisted
  or `blocked`). *Harvested* JSONL corpus = opt-in allowlist; un-allowlisted divergences
  are just the **backlog**, never a build failure, never need a rationale. Don't `blocked`
  harvested cases. Ratchet a fixed slug into `roxygen-allowlist.txt` via
  `task roxygen-harvest-seed` (re-seeds from PASS; preserves the header).
- **Harvested gate needs R and is `#[ignore]`d** (like the whole oracle today); the
  projector, once built, gives it a pure-Rust CI analog. Slugs are content hashes
  (`rx-`+sha1) so re-harvesting is allowlist-stable.
- **`roc_proc_text` needs the block attached to an object** (a function, or `@name` +
  `NULL`). A bare block errors.
- **`@md` must stand alone** — a prose line treated as its value errors in roxygen2.
- **pre-commit `panache-format` reformats `.md`** and mangles long inline-code spans
  on wrap; put commands in fenced blocks.

## Settled decisions (don't relitigate without reason)

Mode-keyed parse (one `markdown_default` salsa input; `@md`/`@noMd` per-block
override; loose-file default ON). CommonMark reference-spec two-pass (block tree →
inlines); **no crate dependency** (panache secondary). Projector is the **primary
conformance engine** (now built, Phase 1 skeleton); `pub` rather than `pub(crate)`
because the gate lives in an integration-test crate (`tests/roxygen_projector.rs`),
but it remains a **test-only faithful diagnostic** — no user-facing CLI, never patched
to pass. **Projection granularity: section-body subtrees, excluding roclet-generated
scaffolding** (`\name`/`\alias`/`\usage`/`\arguments`) — settled with the user
(2026-06-22c); the `block-to-sections` op drops the same set so the two stay aligned.
Markdown = CommonMark core + GFM `table`, `hardbreaks = TRUE`. Full design rationale:
`~/.claude/plans/i-want-to-start-snoopy-haven.md` (local); roadmap: `TODO.md` roxygen
section.

## Progress

Phase 0 **done**. **Phase 1 skeleton done:** the projector + pinned projector-parity
gate now exist and are the **primary driver** (parser-first, structural, CI-safe).
`src/roxygen/project_rd.rs` projects the CST to the parser-owned Rd section subtrees;
`tests/roxygen_projector.rs` diffs that against roxygen2 section pins — pure Rust,
**no R, runs in plain `cargo test`**, allowlist-gated
(`tests/oracle/roxygen-projector-allowlist.txt`). **Two pin sources:** the curated dir
corpus (`<stem>.rdtree`) and the **harvested corpus's projector-eligible subset**
(`roxygen-sections.jsonl` — the 151/217 single-topic, self-contained blocks;
`@inherit`/`@template`/`@eval`/`@example`/… filtered out as resolve-from-elsewhere, so
they stay in the R↔R fixed-point net, not false-positive backlog). Baseline (after
inline Rd macros, 2026-06-22e): **56 matching (allowlisted), 102 divergent (backlog)**
of 158 pinned cases. The
divergences are **structural/parser** gaps, not fixed-point cosmetics — exactly the
re-pointing, now a real 116-case worklist. Tasks: `task roxygen-projector` (the gate),
`task roxygen-projector-refresh` (re-mint all pins), `task roxygen-projector-pins`
(harvested pins only), `task roxygen-projector-seed` (re-seed allowlist from matches).
Report: `ROXYGEN_PROJECTOR.md` (this dir).

**Three checks, three roles** (don't conflate):
1. **Projector parity** (`tests/roxygen_projector.rs`, pure Rust) — the **primary,
   parser-growth driver**. Compares Rd *structure*, so it sees block-structure gaps
   the fixed-point check is blind to. Curated corpus + harvested projector-eligible
   subset (151 cases). The 116 divergences are the worklist.
2. **Curated fixed-point** (`tests/roxygen_oracle.rs::roxygen_oracle_report`, needs R,
   `#[ignore]`d) — strict semantic preservation of the formatter; 7/7 preserving, 0
   blocked. *Meaning, not layout.*
3. **Harvested fixed-point** (`tests/oracle/corpus/roxygen.jsonl`, 217 cases, needs R,
   `#[ignore]`d) — broad **opt-in** backlog gated by `roxygen-allowlist.txt`
   (212 preserving, 4 divergent, 1 skipped). A coverage net, **not** the parser driver
   (it's cosmetic-blind + R-dependent). Reports: `task roxygen-oracle` /
   `task roxygen-harvest`.

## Latest session (2026-06-22e)

**Inline Rd macros: `ROXYGEN_RD_MACRO` is now a structured CST node, projector emits
nested Rd.** Closed the `rd_macros` curated case (the recap's ranked target). The lexer
still carves the macro span as one `RoxygenRdMacro` token (all its fuzz/round-trip tests
intact); the **tree builder** (`push_token` → `build_rd_macro`/`build_rd_content`) now
expands that token into a `ROXYGEN_RD_MACRO` *node* with new leaves
`ROXYGEN_RD_MACRO_{NAME,OPT,DELIM,VERB}` (syntax.rs +4 kinds, COUNT bumped, both
`kind_from_raw` arms). Content is sub-parsed for nested `\macro` calls; **verbatim**
macros (`url,verb,samp,env,kbd,option` — `VERBATIM_RD_MACROS`, confirmed against
`parse_Rd`) keep their body as one `…_VERB` leaf (no recursion). The projector
(`project_rd.rs`) was rewritten from string-bodies to an `Inline{Text,Macro}` sequence:
prose coalesces to `(TEXT …)`, macros recurse to `(\code (\link (TEXT "add")))`, `[pkg]`
dropped, verbatim → `(VERB …)`. Formatter kept atomic (3 token-only predicates →
kind-based, since `chunk_elements` already glues a node's text). **42→56 matching,
116→102 backlog, 0 regressions**; +1 curated case (`rd_macros`) and 13 harvested cases
ratcheted. New fixture `roxygen_rd_macro_nested`; 6 macro CST snapshots updated
(leaf→node). Curated fixed-point still 7/7. All guardrails green; committed.

**Next (ranked):** the remaining curated divergences are all **multi-line block / list
structure** the CST doesn't model yet — pick the simplest first: **`\itemize`/`\enumerate`**
(`itemize_enumerate`), single-arg `\item` lists spanning many `#'` lines as one atomic
nested unit; then **multi-arg `\item{term}{def}`** for `\describe` (`describe_format`),
then `\tabular` (`tabular`, `\tab`/`\cr` cells), then markdown lists under `@md`
(`markdown_list`). These are block macros across line boundaries (TODO.md "Block macros"
bullet) — bigger than inline: needs block grouping in `src/parser/roxygen.rs`, not just a
token expansion. **Fix the parser, never the projector.**

## Earlier sessions

- **2026-06-22d:** Filtered bulk-pin (`58ad5e4`): turned the projector backlog into a real
  116-case worklist. Added `projector_eligible` + `projector-pins` op (eligible = 1 topic,
  no out-of-scope tag); minted `roxygen-sections.jsonl` (151/217); seeded 42 matching.
- **2026-06-22c:** Built the Phase 1 projector skeleton (`7473f2f`): section-level
  granularity (excluding roclet scaffolding, settled with the user), `block-to-sections`
  driver op, `src/roxygen/project_rd.rs`, pure-Rust pinned gate, curated `.rdtree` pins,
  tasks, docs reframed (projector = primary driver; fixed-point = secondary net).

- **2026-06-22b:** Grew the **harvested** backlog (user redirect: corpus-first,
  allowlist-gated like fatou). Cloned roxygen2 v7.3.3 → `roxygen2-ref/`;
  `scripts/harvest-roxygen-corpus.R` mines 217 slug-keyed blocks from roxygen2's tests;
  `trees-batch` driver op; `roxygen_harvested_{report,allowlist}`; tasks
  `roxygen-harvest{,-seed}`; seeded 212 PASS. (This built the broad fixed-point net — now
  reframed as a coverage backlog, *not* the parser driver; the projector is.)
- **2026-06-22a:** Planned the effort; built + committed Phase 0 (`acfd0b6`): R driver,
  seed curated corpus, strict fixed-point harness, `blocked.toml`, `task roxygen-oracle`,
  devenv R. Reframed soft → strict, adopted fatou's allowlist+blocked model (`b39e3d3`).
  Added the skill + recap; moved the report into the skill dir.
