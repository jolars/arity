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
- **Block Rd macros span lines via threaded trivia (since 2026-06-23, Stage 2).** A
  `\name{` whose group is *unbalanced on its line* opens a multi-line `ROXYGEN_RD_MACRO`
  (the lexer extracts a *balanced* `\name{…}` as a single inline macro token, so a
  `RoxygenText` starting `\name{` is necessarily a multi-line opener — `is_block_macro_opener`).
  The node owns its opening marker **and** the inter-line `#'`/newline/indent as trivia;
  brace-less `\item`/`\cr`/… (a `\name` not followed by `{`) is a **name-only** child;
  closing `}` at depth 1 is the delim (greedy + lossless if unterminated). Built by
  splitting `RoxygenText` tokens into `Event::Leaf(kind,text)` synthetic leaves (new Event
  variant; tree builder + projector untouched otherwise). **Formatter = atomic passthrough**
  (not reflow), context-keyed: in a **prose** section `emit_block_macro` preserves the in-macro
  indentation (a `\itemize` list; splits `node.text()` on `\n`, drops only the inter-line
  indent before continuation markers); inside **`@examples`** a block macro wraps example R
  (`\dontrun{}`/`\donttest{}`/…) so `emit_block_macro_examples` emits it **flush**
  (marker-normalized) — example code is copy-pasted, so no list-indentation (user call, 2026-06-23).
  **Justification is Tenet 1, NOT air:** air does **not** format roxygen content at all (it leaves
  `#'` comments byte-for-byte untouched — verified), so it is **not an oracle** for any roxygen
  layout choice and the air-compat fixed point is satisfied by the roxygen portion no matter what
  we emit. The rule (don't reflow a block macro; prose-indent vs examples-flush) is arity's own,
  idempotent, preserving well-formed input. *(Open: a Tenet-1-pure **canonical** re-indent for
  prose lists — preserve-source means input indentation leaks into output; deferred.)*
  **`format <file>` writes in place — use `format < file` to avoid clobbering corpus fixtures.**
- **The roxygen CST is logical, not line-based (since 2026-06-23).** `ROXYGEN_BLOCK` →
  `ROXYGEN_SECTION`* (intro + one per `@tag`) → `ROXYGEN_TAG` and/or `ROXYGEN_PARAGRAPH`*.
  A **block macro is a direct `ROXYGEN_SECTION` child** (a sibling of paragraphs, not inside
  one); the projector's `section_body_parts` walks paragraphs+block-macros in document order.
  `#'` markers, marker→content whitespace, and inter-line newlines are **trivia** threaded
  into the enclosing node. `ROXYGEN_LINE`/`RoxygenLine` no longer exist (reserved enum
  variant only). The **formatter** reconstructs physical lines from trivia (`physical_lines`);
  the **projector** walks `sections()`/`paragraphs()`. There is a committed **format-stability
  baseline** (`tests/roxygen-format-baseline.jsonl`, via `tests/roxygen_format_stability.rs`);
  any intended formatter change must re-bless it (`BLESS_ROXYGEN_FORMAT=1`) **with review**.
- **Macro arity is per-macro, not greedy (since 2026-06-23, Stage 3).** `\code{x}{y}`
  is `\code{x}` + a literal `{y}` LIST (parse_Rd: `\code` takes one arg), but
  `\item{a}{b}` is a single two-arg `\item`. So the lexer (`scan_rd_macro`) consumes a
  second adjacent `{…}` group **only** for `is_two_arg_rd_macro` (`TWO_ARG_RD_MACROS` in
  `parser/roxygen.rs`: now `item`, `tabular`); the tree builder (`build_rd_macro`) loops
  over `{…}` groups emitting `{`/content/`}` per group. New 2-arg macro
  (`\href`/`\method`/`\section`-cell)? add it to that set, confirm the arity via
  `parse_Rd` (a trailing `{…}` tagged `LIST` = NOT consumed).
- **GRP wrapping is per-argument, keyed on `is_two_arg_rd_macro` (since 2026-06-23,
  Stage 4).** A *structural* macro models each `{…}` arg as a list, so the projector
  (`serialize_macro`) wraps a **multi-atom** argument in `(GRP …)` and **unwraps** a
  single-atom one: `\tabular{rl}{a \tab b}` → `(\tabular (TEXT "rl") (GRP (TEXT "a")
  (\tab) (TEXT "b")))`, `\item{a}{first}` → `(\item (TEXT "a") (TEXT "first"))`. A
  *latexlike* macro (`\code`/`\emph`/…) inlines its single argument's atoms directly,
  never GRP (`\code{a \emph{b} c}` → `(\code (TEXT "a") (\emph (TEXT "b")) (TEXT "c"))`).
  The projector segments atoms per group (finalize at each closing `}`); the
  structural/latexlike split is exactly `is_two_arg_rd_macro`. Confirm any new shape via
  `parse_Rd` `rd-to-tree`: a brace-group wrapper tagged `TEXT`-list with >1 child = GRP.
- **Two block-opener forms (since 2026-06-23, Stage 4).** `is_block_macro_line` admits
  **Form A** (a `RoxygenText` `\name{ …` whose group is unbalanced on its line —
  `\itemize`/`\describe`/`\name`) and **Form B** (a *balanced* `RoxygenRdMacro` for a
  structural macro, e.g. `\tabular{rl}`, immediately followed by a `RoxygenText` that
  `opens_unbalanced_brace` — the `{` body opener). `emit_block_macro` dispatches:
  Form B calls `emit_block_open_arg_macro` (decompose the macro token into NAME + format
  group leaves) then `emit_block_body_open` (open the body `{`, depth 1). The lexer eats
  the balanced `{rl}` as the inline macro token, which is why the trailing `{` is a
  separate `RoxygenText` and the body opener lives in Form B, not Form A.
- **Markdown *block* structure is mode-keyed via the lexer, not the block builder (since
  2026-06-23, Stage 6).** A `-`/`*`/`+` or `1.`/`1)` marker at a line's content start under
  `@md` is carved by the *lexer* as a `RoxygenMdListMarker` (punctuation only — the trailing
  space stays in the following text run, so a marker that does **not** form a list chunks for
  reflow identically to the plain text it stands in for → no format-baseline regression). The
  block builder (`emit_md_list`) forms `ROXYGEN_MD_LIST`/`ROXYGEN_MD_LIST_ITEM` from the token
  kind alone (the token's existence implies `@md`), applying the CommonMark **interrupt rule**
  (`is_md_list_start`/`md_list_marker_can_interrupt`: a bullet always interrupts an open
  paragraph, an ordered list only when start == 1; otherwise the marker stays inline prose and
  the projector renders its leaf as text). **Do NOT re-derive `@md` mode in the block builder**
  by scanning tokens — that is a second source of truth (a hack); the lexer (`resolve_roxygen_block`)
  is the single mode source, and mode reaches the block layer baked into token kinds. **Nested
  lists are not modeled** (in-list indentation is consumed as marker→content trivia and dropped),
  so a nested list projects *flat* — leave it un-allowlisted backlog, never patch it to a
  passing-but-wrong entry. Projector arm: `Inline::MdList` → `serialize_md_list`
  (`\itemize`/`\enumerate` from the first item's marker, name-only `(\item)` per item).
- **Mode-keyed parse.** Markdown structure exists in the CST only when `@md` is on;
  the CST (and projected Rd) differs by mode — pin both modes where relevant. **Mode is
  resolved per-block** by `resolve_roxygen_block` (lexer scans the `#'` run for `@md`/`@noMd`,
  default off; loose-file default-ON deferred) and threaded as `md: bool` into the prose
  lexer. **`@md` inline landed (Stage 5, 2026-06-23):** `*x*`/`**x**`/`` `x` `` →
  `ROXYGEN_MD_EMPH`/`STRONG`/`CODE` leaves; projector `\code`-vs-`\verb` = arity-parseability
  (one top-level expr, no diagnostics, or `SPECIAL_CODE`) mirroring roxygen2's `can_parse`.
- **A new roxygen line-body TokKind must be added to *every* line-body matcher** or tag/prose
  lines silently truncate at the unknown token (this bit Stage 5: a `@param` line's description
  vanished, its continuations became phantom intro paragraphs → extra `\title`/`\description`).
  The set: `classify_line`, `is_line_body_kind`, the block-macro consumer's inline-span arm
  (all `src/parser/roxygen.rs`), `expr.rs`'s atom-parser fallthrough, `tree_builder`'s
  `syntax_kind_for`, `lexer.rs`'s `is_comment_like`, `syntax.rs`'s `is_roxygen_token`, plus the
  formatter's `is_blank`/`is_tag_prose_kind`. Rust exhaustiveness catches the enum matches; the
  `matches!` lists are silent — grep an existing roxygen leaf kind to find them all.
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
- **A prose section whose value is the literal `"NULL"` is suppressed** (roxygen2's
  `rd_section()` sentinel, `R/field.R`; since 2026-06-23 Stage 8). Applies to the
  plain-string sections (`NULL_SUPPRESSIBLE` in `project_rd.rs`); `@section` (a (title,
  body) pair) is NOT suppressed. Value is trimmed first. A suppressed `@description NULL`
  re-fires the title-as-description fallback. Data-object auto-`\format` (roxygen2
  *evaluates* the object for class/dims) is **out of scope** — not statically derivable.
- **`\code` is the one latexlike macro whose plain text is `RCODE`, not `TEXT` (since
  2026-06-23, Stage 9).** parse_Rd tags a `\code{…}` body as verbatim R code: no
  `norm_ws`, and split at newlines (each `\n` stays on the atom it ends). `serialize_macro`'s
  `flush` keys on `head == "\\code"` → `rcode_atoms`. Every *other* latexlike text macro
  (`\emph`/`\strong`/`\command`/…) is `TEXT`; the verbatim macros (`\verb`/`\samp`/`\url`/…)
  are `VERB`. A *nested* macro inside `\code` still recurses (`\code{\link{x}}` →
  `(\code (\link …))`). Confirm a new macro's body tag via `block-to-sections`: a `{RCODE}`
  child = R-code body.
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
they stay in the R↔R fixed-point net, not false-positive backlog). Current (after
`\code`-body RCODE, 2026-06-23 Stage 9): **86 matching (all allowlisted),
73 divergent (backlog)** of 159 pinned cases. The
divergences are **structural/parser** gaps, not fixed-point cosmetics. Tasks:
`task roxygen-projector` (the gate),
`task roxygen-projector-refresh` (re-mint all pins), `task roxygen-projector-pins`
(harvested pins only), `task roxygen-projector-seed` (re-seed allowlist from matches).
Report: `ROXYGEN_PROJECTOR.md` (this dir).

**Three checks, three roles** (don't conflate):
1. **Projector parity** (`tests/roxygen_projector.rs`, pure Rust) — the **primary,
   parser-growth driver**. Compares Rd *structure*, so it sees block-structure gaps
   the fixed-point check is blind to. Curated corpus + harvested projector-eligible
   subset (151 cases). The 73 divergences are the worklist.
2. **Curated fixed-point** (`tests/roxygen_oracle.rs::roxygen_oracle_report`, needs R,
   `#[ignore]`d) — strict semantic preservation of the formatter; 8/8 preserving, 0
   blocked. *Meaning, not layout.*
3. **Harvested fixed-point** (`tests/oracle/corpus/roxygen.jsonl`, 217 cases, needs R,
   `#[ignore]`d) — broad **opt-in** backlog gated by `roxygen-allowlist.txt`
   (212 preserving, 4 divergent, 1 skipped). A coverage net, **not** the parser driver
   (it's cosmetic-blind + R-dependent). Reports: `task roxygen-oracle` /
   `task roxygen-harvest`.

## Latest session (2026-06-23) — Stage 9: `\code` body projects as `RCODE`

**Projector-only win, 3 cases.** parse_Rd tags a `\code{…}` body's *plain text* as
verbatim R code → `(\code (RCODE "…"))`, not the whitespace-normalized `(TEXT …)`
every other latexlike macro emits. (`\verb`/`\samp`/`\url`/… already produced VERB;
a nested macro inside `\code` still recurses, e.g. `\code{\link{x}}` →
`(\code (\link (TEXT "x")))`. Only `\code`'s *textual* segments are RCODE — confirmed
via `block-to-sections`: among `\code/\emph/\verb/\samp/\command/\preformatted`, only
`\code` yields RCODE; `\emph`/`\command` give TEXT, the rest VERB.)

- **Bucket: projector gap** (the CST is fine — `\code{code}` is already a
  `ROXYGEN_RD_MACRO` with a `ROXYGEN_TEXT` body; only the projection encoding was
  wrong). `src/roxygen/project_rd.rs`: `serialize_macro`'s `flush` closure now takes a
  `code: bool` (`head == "\\code"`); when set it emits `rcode_atoms(run)` (new helper)
  instead of `text_atom`. **RCODE is verbatim** (no `norm_ws`) and **splits at
  newlines**, attaching each `\n` to the atom it ends (`\code{a\nb}` →
  `(RCODE "a\n") (RCODE "b")`, mirroring parse_Rd; matches the `\verb` multiline split).
- **Why faithful, not compensating:** mirrors parse_Rd's documented `\code`→RCODE
  tagging; the markdown code-span path (`md_code_atom`) already emits the same
  `(\code (RCODE …))`, so this aligns the literal-Rd path with it. No pin anywhere uses
  `(\code (TEXT …))` (verified) — the only literal-`\code` curated case (`rd_macros`)
  has a *nested* `\link` body, never plain text, so it was unaffected.
- **Projector 83→86 matching** (all allowlisted via `task roxygen-projector-seed`),
  **73 divergent**; **0 regressions**; full `cargo test` green; clippy + fmt clean. Did
  not run the R fixed-point net (projector-only change, formatter untouched).
- **Test:** projector unit test `code_macro_body_projects_as_rcode` (TDD: failing
  first). No new parser fixture — no parser/CST change. Closed slugs: rx-41db756e
  (`\code{code}` + `\verb{…}`), rx-d58b849a, rx-db08ab08 (both `Inline \code{code} and
  \emph{emphasis}` alongside an `\itemize`).

**Next (ranked):** the largest remaining cluster is **markdown links** (≈10 cases:
rx-270b730c/rx-95dd50a4/rx-72858140/rx-2a68ab3f/rx-4adb1f22/rx-fd84eacf/rx-375ab9f1
`[text]`/`[text][dest]`/`[fn()]`/`[pkg::obj]` → `\link`/`\code{\link}`; rx-7743ba62/
rx-0605d020 `[text](url)` → `\href`; rx-1b4ef7c7 `[`code`]` → `\code{\link}`) — under
`@md` only; complex (CommonMark link parsing + roxygen2's `[x]`→`\link` resolution).
Cleaner small wins: **`\href{url}{text}`** two-arg (rx-d791aabe, rx-ac585ae8 — note the
*first* arg is VERB, second TEXT: a new per-arg encoding) and **@slot/@field →
`\section{Slots/Fields}{\describe …}`** (rx-853d2f8f, rx-d55651e1 — needs modeling those
tags). **Out of scope:** data-object auto-`\format` (rx-cbcc255c/rx-8f9c159b/rx-4d59d472/
rx-deb9d202 — roxygen2 evaluates the object) and ```{r}``` code blocks that evaluate R
(rx-2900ecd5/rx-a6ac1b4d — produce `#> [1] 2`). To re-triage, recreate the throwaway
`examples/rxdiff.rs` (dump input/projected/pin per divergent harvested case, sorted by
input length; removed at session end).

## Earlier sessions

- **2026-06-23 (Stage 8, `@tag NULL` suppression sentinel):** projector-only, 7 cases.
  roxygen2's `rd_section()` treats a section value of literal `"NULL"` as a sentinel that
  suppresses the section; a suppressed `@description NULL` re-fires the title-as-description
  fallback. New `NULL_SUPPRESSIBLE` set + `is_null_section` in `project_rd.rs`; `@section`
  excluded (value is a title/body pair). 76→83 matching. Data-object auto-`\format` is out
  of scope (roxygen2 evaluates the object).

- **2026-06-23 (Stage 7, title-as-description fallback):** projector-only, 11 cases.
  roxygen2 reuses the title as the description when there is no `@description` and no
  description paragraph — including an explicit `@title` with no intro prose. Gave the
  description-derivation an `else` branch falling back to the explicit `@title` body
  (`explicit_title` lookup replaced the old `has_explicit_title` bool). 65→76 matching.
- **2026-06-23 (Stage 6, `@md` block lists):** first markdown *block* structure. Under
  `@md`, `-`/`*`/`+` → `\itemize`, `1.`/`1)` → `\enumerate`, name-only `\item` per item.
  Mode-keyed lexing (new `RoxygenMdListMarker` TokKind, punctuation-only carve so a
  non-list marker chunks identically → no baseline regression); `emit_md_list` builds
  `ROXYGEN_MD_LIST`/`_ITEM` applying the CommonMark interrupt rule; projector
  `Inline::MdList` → `serialize_md_list`. Closed `markdown_list` (64→65).
- **2026-06-23 (Stage 5, `@md` inline foundation):** first markdown win —
  `*x*`→`\emph`, `**x**`→`\strong`, code span → `\code`/`\verb` (roxygen2's `can_parse`,
  replicated via arity-parseability). New `resolve_roxygen_block` mode infra (lexer is the
  single mode source, threads `md: bool`); `ROXYGEN_MD_EMPH`/`STRONG`/`CODE` leaves;
  `scan_md_emphasis` (CommonMark-flanking subset, bail-to-text). Projector 59→64; closed
  `markdown_inline` + 4 harvested. Fixtures `roxygen_md_inline`, `roxygen_md_inline_reflow`.

- **2026-06-23 (Stage 4, `\tabular{rl}{ … \tab … \cr }`):** closed `tabular` (58→59). Lexer
  eats the balanced `{rl}` as an inline macro token, so `is_block_macro_line` gained **Form B**
  (balanced structural `RoxygenRdMacro` + unbalanced-`{` `RoxygenText`); `emit_block_open_arg_macro`
  decomposes `\tabular{rl}` into NAME + format-group leaves. Projector `serialize_macro` segments
  per `{…}` group and GRP-wraps a structural macro's multi-atom arg. Fixtures `roxygen_tabular`,
  `multiline_tabular_projects_format_and_grp_body`.
- **2026-06-23 (Stage 3, `\describe` `\item{term}{def}`):** closed `describe_format` (57→58).
  New `TWO_ARG_RD_MACROS`/`is_two_arg_rd_macro` (then just `item`): `scan_rd_macro` pulls a
  second adjacent `{…}` into one token, `build_rd_macro` loops over groups, the projector
  flushed at each closing `}` so adjacent groups stayed separate atoms. Fixtures
  `roxygen_describe_item` + `multiline_describe_item_projects_two_args`.
- **2026-06-23 (Stage 2, `\itemize`/`\enumerate`):** first capability win on the logical CST.
  `is_block_macro_opener`/`emit_block_macro` build a multi-line `ROXYGEN_RD_MACRO` across `#'`
  lines (markers/newlines/indent threaded as trivia via new `Event::Leaf`); brace-less `\item`
  → name-only macro child. Projector `section_body_parts` walks block-macro section children;
  formatter passes the node through atomically (`is_block_macro`, prose-indent vs
  examples-flush), fixing a run-on-reflow bug for 7 prose cases (re-blessed format baseline).
  Closed `itemize_enumerate` (56→57). New fixture `roxygen_block_macro`.
- **2026-06-23 (CST re-model, Stage 1):** dissolved `ROXYGEN_LINE`; `ROXYGEN_BLOCK` →
  `ROXYGEN_SECTION`* → `ROXYGEN_TAG`/`ROXYGEN_PARAGRAPH`* with markers/newlines as trivia
  (`3a0846a`; Stage-0 baseline harness `882889a`). Pure re-shape, byte-identical formatter
  output (37 fixtures), projector unchanged (56/102), losslessness+idempotence green. Formatter
  reconstructs physical lines from trivia (`physical_lines`/`collect_logical_elements`);
  projector walks sections/paragraphs. Approved plan `~/.claude/plans/cozy-swinging-patterson.md`.
  This unblocked Stage 2.
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
