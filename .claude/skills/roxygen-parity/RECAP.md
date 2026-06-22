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
`tests/roxygen_projector.rs` diffs that against per-case `<stem>.rdtree` pins
(minted by the R driver's `block-to-sections` op) — pure Rust, **no R, runs in plain
`cargo test`**. Allowlist-gated like the harvested corpus
(`tests/oracle/roxygen-projector-allowlist.txt`). Baseline on the curated corpus:
**2 matching (allowlisted: `examples`, `param_prose`), 5 divergent (backlog)**. The 5
divergences are now **structural/parser** gaps, not fixed-point cosmetics — exactly the
re-pointing. Tasks: `task roxygen-projector` (the gate) + `task roxygen-projector-refresh`
(re-mint pins). Report: `ROXYGEN_PROJECTOR.md` (this dir).

**Three checks, three roles** (don't conflate):
1. **Projector parity** (`tests/roxygen_projector.rs`, pure Rust) — the **primary,
   parser-growth driver**. Compares Rd *structure*, so it sees block-structure gaps
   the fixed-point check is blind to. Curated corpus.
2. **Curated fixed-point** (`tests/roxygen_oracle.rs::roxygen_oracle_report`, needs R,
   `#[ignore]`d) — strict semantic preservation of the formatter; 7/7 preserving, 0
   blocked. *Meaning, not layout.*
3. **Harvested fixed-point** (`tests/oracle/corpus/roxygen.jsonl`, 217 cases, needs R,
   `#[ignore]`d) — broad **opt-in** backlog gated by `roxygen-allowlist.txt`
   (212 preserving, 4 divergent, 1 skipped). A coverage net, **not** the parser driver
   (it's cosmetic-blind + R-dependent). Reports: `task roxygen-oracle` /
   `task roxygen-harvest`.

## Latest session (2026-06-22c)

**Built the Phase 1 projector skeleton — the parser-first structural gate** (user
redirect: the loop had drifted onto the fixed-point harvested check, which is
cosmetic-blind and R-dependent, so it could be satisfied by tuning formatter heuristics
instead of growing the CST). Decision locked with the user: project at the **section-body**
level, **excluding** roclet-*generated* scaffolding (`\name`/`\alias`/`\usage`/the
`\arguments` wrapper) — those are generation, not parsing, so reproducing them would make
the projector a roclet reimplementation rather than a faithful encoding translation.
Landed: (1) `block-to-sections` + `sections-batch` ops in the R driver (drop the
roclet-only macro set); (2) `src/roxygen/project_rd.rs` — faithful minimal projector
(intro→title/description; prose tags `@details`/`@return`→`\value`/`@seealso`/`@source`/
`@format`/`@section`/…; `@examples` placeholder; a section body is one coalesced `TEXT`,
so block structure and inline-macro/markdown translation diverge by construction);
(3) `tests/roxygen_projector.rs` — pure-Rust pinned gate, allowlist-gated; (4) curated
`.rdtree` pins + `roxygen-projector-allowlist.txt` (seeded `examples`, `param_prose`);
(5) tasks `roxygen-projector` / `roxygen-projector-refresh`. Baseline 2 match / 5 backlog.
All guardrails green.

**The 5 curated backlog cases** (now ranked structural/parser targets, not fixed-point):
- `rd_macros` — **inline Rd macros** (`\code{}`/`\emph{}`/`\strong{}`/`\url{}`/`\link[pkg]{}`)
  → nested nodes. Smallest first step: promote the single-line `ROXYGEN_RD_MACRO` leaf to a
  node (plan Phase 1's macro-promotion), project it faithfully. **Best first target.**
- `describe_format`, `itemize_enumerate`, `tabular` — **multi-line block Rd macros**
  (`\describe`/`\itemize`/`\enumerate`/`\tabular`); brace-balanced across `#'` lines (plan
  Phase 2). The motivating `\describe` reflow bug lives here.
- `markdown_list` — **markdown→Rd** (`*x*`→`\emph`, `` `x` ``→`\verb`, `- ` list→`\itemize`),
  needs `@md` mode-keyed parse (plan Phase 3).

**Next:** `rd_macros` (inline Rd macro promotion). Probe the target shape
(`… | Rscript tests/oracle/roxygen_oracle.R block-to-sections`), model the macro as a CST
node in `src/parser/roxygen.rs` (+ syntax/tree_builder/ast), grow a faithful projector arm,
confirm the case matches its pin (`task roxygen-projector`), add `rd_macros` to the
projector allowlist, guardrails, commit. **Fix the parser, never the projector.**

## Earlier sessions

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
