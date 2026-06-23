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
- **The roxygen CST is logical, not line-based (since 2026-06-23).** `ROXYGEN_BLOCK` →
  `ROXYGEN_SECTION`* (intro + one per `@tag`) → `ROXYGEN_TAG` and/or `ROXYGEN_PARAGRAPH`*.
  `#'` markers, marker→content whitespace, and inter-line newlines are **trivia** threaded
  into the enclosing node. `ROXYGEN_LINE`/`RoxygenLine` no longer exist (reserved enum
  variant only). The **formatter** reconstructs physical lines from trivia (`physical_lines`);
  the **projector** walks `sections()`/`paragraphs()`. There is a committed **format-stability
  baseline** (`tests/roxygen-format-baseline.jsonl`, via `tests/roxygen_format_stability.rs`);
  any intended formatter change must re-bless it (`BLESS_ROXYGEN_FORMAT=1`) **with review**.
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

## Latest session (2026-06-23) — CST re-model (logical content, line node dissolved)

**The roxygen CST is no longer line-flat.** Approved plan:
`~/.claude/plans/cozy-swinging-patterson.md` (local). Motivated by panache's embedded-YAML
model (markers as `Trivia(LinePrefix)`, structure over the logical content stream) and the
fact that rowan/rust-analyzer trees are never line-based. `ROXYGEN_LINE` is **dissolved**
(kept as a reserved, unemitted enum variant for discriminant stability). A `ROXYGEN_BLOCK`
now owns **logical content**: children are `ROXYGEN_SECTION` (the intro, then one per
`@tag`), and a section's prose is grouped into `ROXYGEN_PARAGRAPH`s between blank lines.
`#'` markers, the marker→content whitespace, and inter-line newlines are **trivia threaded
into the enclosing node** (losslessness intact). syntax.rs +2 node kinds (`ROXYGEN_SECTION`
88, `ROXYGEN_PARAGRAPH` 89), COUNT bumped. AST: `RoxygenLine` → `RoxygenSection`
(`tag()`/`paragraphs()`) + `RoxygenParagraph`; `RoxygenBlock::sections()`. Parser
(`emit_roxygen_block`) rewritten to a section/paragraph state machine (`classify_line` →
Tag/Blank/Prose; `emit_tag_line`/`emit_line_tokens`). **No block-macro parsing yet** — a
multi-line `\itemize` is still flat prose here (Stage 2).

**Done as a staged, always-green migration:**
- **Stage 0** (`882889a`, `test(roxygen)`): a differential format-stability harness
  (`tests/roxygen_format_stability.rs`) pins `format(input)` byte-for-byte over the whole
  roxygen corpus (224 cases) to a committed baseline (`tests/oracle/roxygen-format-baseline.jsonl`).
  Re-bless with `BLESS_ROXYGEN_FORMAT=1`. This is the guardrail that proves the re-shape
  didn't move formatter output.
- **Stage 1** (`3a0846a`, `refactor(parser)`): the cutover above. **Pure re-shape, zero
  behavior change** — formatter output byte-identical (Stage-0 harness + 37 fixtures),
  projector output identical (**56 matching / 102 backlog unchanged**), losslessness +
  idempotence green. The **formatter** keeps its line-oriented reflow engine, fed by a
  `PhysicalLine` view *reconstructed from marker/newline trivia* (`physical_lines` /
  `collect_logical_elements` in `formatter/roxygen.rs`; `RoxygenLine` → `PhysicalLine`).
  The **projector** `project_block` walks sections/paragraphs directly (`paragraph_inlines`;
  the line-reassembly state machine gone). 22 parser CST snapshots regenerated;
  `ast_wrappers` test rewritten to `sections()`/`paragraphs()`. clippy + fmt clean.

**Next (ranked) — Stage 2: `\itemize`/`\enumerate` block macros.** Close `itemize_enumerate`
(the pin wants `(\itemize (\item) (TEXT …) (\item) (TEXT …))`). Now tractable because the
block grouping site is the new logical parser. Plan: a `ROXYGEN_RD_MACRO` node (reuse the
inline kind) that spans `#'` lines, with brace-less `\item` as a name-only
`ROXYGEN_RD_MACRO` child and the trailing text a sibling; markers/newlines threaded as
trivia. **Open design problem (handle first):** the lexer emits `\item First bullet…` as one
`RoxygenText` token, and `\itemize{`/`}` as `RoxygenText` too (unbalanced ⇒ `scan_rd_macro`
returns None). Splitting `\item` off its text, and carving `\itemize{`/`}` into NAME/DELIM,
needs a **tree-builder path generalizing `build_rd_macro` to a multi-token, trivia-interleaved
span** — the parser's `Event::Tok(idx)` references whole tokens and can't split them. Decide
that mechanism before coding. Then: projector grows a section-level block-macro arm
(`serialize_macro` already emits the nested shape); formatter routes block-macro nodes to
**atomic passthrough** (not reflow). **NB:** the current formatter *reflows* the multi-line
`\itemize` into a mangled run-on (a latent bug the projector gate exists to catch), so Stage 2
will legitimately change `itemize_enumerate`'s formatted output — **review + re-bless** the
Stage-0 baseline for the affected cases (it's a fix, not a regression). Then `\describe`
(`describe_format`, multi-arg `\item{term}{def}` → `GRP`), `\tabular`, markdown under `@md`.
**Fix the parser, never the projector.**

## Earlier sessions

- **2026-06-22e:** Inline Rd macros as structured `ROXYGEN_RD_MACRO` *nodes* (`be0521b`):
  tree builder (`build_rd_macro`/`build_rd_content`) expands the macro token into
  NAME/OPT/DELIM/VERB leaves + nested macros; projector emits nested Rd via the
  `Inline{Text,Macro}` sequence. 42→56 matching; closed `rd_macros`.
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
