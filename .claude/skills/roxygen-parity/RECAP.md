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
inlines); **no crate dependency** (panache secondary). Projector test-only
`pub(crate)` but the **primary conformance engine**. Markdown = CommonMark core + GFM
`table`, `hardbreaks = TRUE`. Full design rationale:
`~/.claude/plans/i-want-to-start-snoopy-haven.md` (local); roadmap: `TODO.md` roxygen
section.

## Progress

Phase 0 **done**. **Two corpora now:** (1) the small **curated** dir corpus
(`tests/oracle/corpus/roxygen/*.R`, 7 cases) — strict, allowlist-or-`blocked`, 100%
Rd-preserving, 0 blocked; (2) the large **harvested** corpus
(`tests/oracle/corpus/roxygen.jsonl`, 217 cases mined from roxygen2's own tests) — gated
**opt-in** by `tests/oracle/roxygen-allowlist.txt`, fatou-style: **212 preserving
(allowlisted), 4 divergent (the backlog), 1 skipped**. The 4 divergent slugs are the
concrete pick-off list. Projector + pinned projector-parity gate: **still not built**
(Phase 1) — but the harvested fixed-point gate already drives parser/formatter growth
without it. Reports: `task roxygen-oracle` → `ROXYGEN_ORACLE.md`; `task roxygen-harvest`
→ `ROXYGEN_HARVEST.md` (both in this dir).

## Latest session (2026-06-22b)

Grew the corpus into a real backlog (user redirect: corpus-first, allowlist-gated like
fatou's `parser-parity`, not block-everything). Cloned roxygen2 v7.3.3 to `roxygen2-ref/`
(gitignored reference, like `air/`); pin in `tests/oracle/.roxygen2-source`. New
`scripts/harvest-roxygen-corpus.R` walks the roxygen2 test suite's ASTs for
`roc_proc_text(rd_roclet(), "…")` source strings, dedents, drops unrenderable, dedups,
and emits slug-keyed JSONL (slug = `rx-`+sha1 prefix, stable across re-harvest → allowlist
survives). Added a `trees-batch` op to the driver (one R process for the whole corpus →
5 s, not ~10 min). Extended `tests/roxygen_oracle.rs`: `roxygen_harvested_report` (triage
+ greppable `PASS <slug>` + writes `ROXYGEN_HARVEST.md`) and `roxygen_harvested_allowlist`
(regression guard: only allowlisted slugs must stay preserving; backlog ≠ failure). Tasks
`roxygen-harvest` + `roxygen-harvest-seed`. Seeded 212 PASS slugs. All guardrails green;
curated corpus untouched (still strict 7/7).

**The 4 divergent backlog slugs** (all `@md` block-structure — the "full markdown/Rd block
parser" TODO item; arity reflows what should stay atomic/nested):
- `rx-91e67e79` — **nested markdown lists** (ordered w/ itemized sublist, mixed) — best
  first target (high value, pure structure).
- `rx-0a1710c0` — multi-line `\preformatted{}` whose body looks like markdown (must stay
  verbatim).
- `rx-daf9322f` — raw HTML **block** (`<p>…</p>` lines).
- `rx-299f50fb` — inline raw HTML (`<img …>`) mid-paragraph.

**Next:** pick `rx-91e67e79` (nested lists). Inspect `format()` output vs raw, find where
the reflow breaks nesting, fix in the formatter (or parser if structural), confirm the
slug flips to PASS, re-seed the allowlist (`task roxygen-harvest-seed`), guardrails,
commit. The projector (Phase 1) is still the eventual CI-safe engine, but the harvested
gate is the active driver now.

## Earlier sessions

- **2026-06-22a:** Planned the effort; built + committed Phase 0 (`acfd0b6`): R driver,
  seed curated corpus, strict fixed-point harness, `blocked.toml`, `task roxygen-oracle`,
  devenv R. Reframed soft → strict, adopted fatou's allowlist+blocked model (`b39e3d3`).
  Added the skill + recap; moved the report into the skill dir.
