# roxygen-parity recap

Rolling log. Read top-to-bottom: persistent traps → settled decisions → progress →
latest session → earlier log. Keep ≤ ~300 lines; demote "Latest session" to a
one-liner under "Earlier sessions" each new session. The `roxygen-parity` skill
reads this first.

## Persistent traps & invariants

Terse by design — each line is a rule + a source-of-truth pointer; build history
lives in git/TODO and the demoted session log. Most cite a function name; go read it.

**Discipline**
- **Projector is faithful, never compensating.** A divergence means the CST (or the
  encoding translation) is wrong — fix the *parser*, never patch `project_rd.rs` to pass.
- **Strict only for the *curated* corpus** (every case allowlisted or `blocked` with a
  rationale). *Harvested* (JSONL, `rx-`+sha1 slugs): un-allowlisted = backlog, never
  `blocked`, never a build failure. Ratchet via `task roxygen-{harvest,projector}-seed`.
- **Cosmetic ≠ semantic.** The fixed-point check is blind to layout (a reflowed `\describe`
  renders identical Rd → passes); the structural *projector* gate is what catches it.
- **`format <file>` writes in place** — use `format < file` to avoid clobbering fixtures.
- **pre-commit `panache-format` reformats `.md`** and mangles long inline-code on wrap →
  put commands in fenced blocks.
- **R is for the oracle, not the gate.** The projector gate is pure-Rust (pinned
  `.rdtree`); only minting pins + the fixed-point net need `Rscript`.

**Oracle / serializer (`roxygen_oracle.R`)**
- **`parse_Rd` tags brace-group arg wrappers `TEXT` but they are *lists*.** Coalesce only
  genuine character TEXT leaves (`is_text_leaf`), or `\item{term}{def}` collapses to one atom.
- **`hardbreaks = TRUE`, yet soft-wrapped prose is safe** (no `\cr`) → coalesce TEXT runs.
  A real hard break (trailing `  `/`\\`) is a distinct node; preserve it.
- **`\examples` bodies are reformatted R** (Tenet 1) → serializer replaces them with `...`.
- **`roc_proc_text` needs the block on an object** (a function, or `@name` + `NULL`); a bare
  block errors. **`@md` must stand alone** — a prose value errors.

**CST shape**
- **Logical, not line-based.** `ROXYGEN_BLOCK` → `ROXYGEN_SECTION`* (intro + one per `@tag`)
  → `ROXYGEN_TAG`/`ROXYGEN_PARAGRAPH`*. A **block macro / md-list / md-code-block is a direct
  `ROXYGEN_SECTION` child** (sibling of paragraphs). `#'` markers, marker→content WS, and
  inter-line newlines are **trivia** threaded into the enclosing node (`ROXYGEN_LINE` gone).
  Formatter rebuilds lines via `physical_lines`; projector walks `sections()`/`paragraphs()`/
  `section_body_parts`.
- **`ROXYGEN_RD_MACRO` is a NODE, not a leaf.** Classify with `el.kind()`, never `as_token()`.
  Lexed atomically; the tree builder (`build_rd_macro`) expands it.
- **Format-stability baseline** (`tests/oracle/roxygen-format-baseline.jsonl`,
  `roxygen_format_stability.rs`): any intended formatter change re-blesses it
  (`BLESS_ROXYGEN_FORMAT=1`) **with review**. An atomic-span leaf (`\href`, inline HTML) that
  stops mid-construct reflow is a Tenet-1 win → re-bless the one affected case.
- **A new line-body TokKind must reach every classifier** or lines silently truncate at it.
  Single compiler-policed source: `TokKind::roxygen_role` (`lexer.rs`, wildcard-free → adding
  a kind is a compile error). Still-explicit sites (grep an existing md leaf): `expr.rs` atom
  fallthrough, `tree_builder::syntax_kind_for`, `syntax.rs` `is_roxygen_token` +
  `is_roxygen_prose_content`, and `kind_from_raw` + `COUNT`.

**Rd macros**
- **Name = `[A-Za-z][A-Za-z0-9]*`** (digits allowed, `\linkS4class`). One source for where a
  name ends: `rd_macro_name_end`; every name scan must route through it (else digit-truncation).
- **Arity is per-macro, not greedy.** `\code{x}{y}` = `\code{x}` + literal `{y}`; only
  `is_two_arg_rd_macro` (`TWO_ARG_RD_MACROS`: `item, tabular, href, figure`) consumes a 2nd
  `{…}`. Confirm via `parse_Rd`: a trailing `{…}` tagged `LIST` = NOT consumed.
- **GRP-wrap is per-argument, keyed on `is_two_arg_rd_macro`.** A *structural* macro wraps a
  multi-atom arg `(GRP …)`, unwraps a single-atom one (`\item{a}{first}` → `(\item (TEXT "a")
  (TEXT "first"))`). A *latexlike* macro (`\code`/`\emph`/…) inlines its arg's atoms, never GRP.
- **Verbatim is per-*argument*.** `is_verbatim_rd_arg(name, index)` drives `build_rd_macro`'s
  recurse decision (`VERBATIM_RD_MACROS`: `url, verb, samp, env, kbd, option`; plus `href` arg
  0 and `figure`). Projector needs no change (emits `(VERB …)` for a `…_VERB` leaf). Confirm: a
  `{VERB}`-tagged arg in `block-to-sections`.
- **`\code` body is `RCODE`, not `TEXT`** (verbatim R: no `norm_ws`, split at newlines).
  `serialize_macro` flush keys `head == "\\code"`. Other latexlike text macros are `TEXT`;
  fully-verbatim macros are `VERB`. Nested macros still recurse. Confirm via `block-to-sections`
  (`{RCODE}`/`{VERB}`).
- **Brace-less `\word` carves only when *unknown*.** `is_known_rd_macro`/`KNOWN_RD_MACROS`
  (parse_Rd's static table, R 4.5; excludes expanded user macros `\CRANpkg`/`\doi`). Unknown →
  `(UNKNOWN "\\word")`; known brace-less stays literal prose (zero-arg `\cr` rendering deferred).
  A new known macro must go in the table or it silently becomes UNKNOWN.
- **Block-macro openers, two forms** (`is_block_macro_line`). Form A: a `RoxygenText` `\name{`
  unbalanced on its line (`\itemize`/`\describe`/`\name`) — necessarily a multi-line opener
  since the lexer extracts a *balanced* `\name{…}` as one inline token. Form B: a balanced
  structural `RoxygenRdMacro` (`\tabular{rl}`) then a `RoxygenText` opening the body `{`.
  `emit_block_macro` dispatches. (A *third* form ⇒ reconsider lexer greediness.)
- **Nested block macros are brace-driven, not indentation.** `emit_block_content`
  tracks open groups with a `Vec<BodyFrame>` stack (`Macro` = a nested `\name{` we
  opened → child `ROXYGEN_RD_MACRO`; `Plain` = bare prose `{`, literal both ends). A
  `}` at the *empty* stack terminates the enclosing macro; the parent body is that
  empty-stack baseline. Only an **unbalanced** `\name{` in `RoxygenText` triggers
  nesting — balanced `\item{a}{b}` is its own `RoxygenRdMacro` token (passed through),
  brace-less `\item`/`\cr` a separate branch. (Markdown nested lists are a *separate*,
  still-deferred indentation problem in `emit_md_list`.)
- **Block Rd macro = atomic passthrough, context-keyed** (not reflow). Prose section:
  `emit_block_macro` preserves in-macro indentation. `@examples`: `emit_block_macro_examples`
  emits **flush** (example code is copy-pasted). Air does **not** format roxygen content
  (verified) → not an oracle for any roxygen layout; the rule is arity's own (Tenet 1),
  idempotent. *(Open: canonical re-indent for prose lists; deferred.)*

**Markdown — mode-keyed**
- **Mode resolved per-block** by `resolve_roxygen_block` (scans the `#'` run for `@md`/`@noMd`,
  default off; loose-file default-ON deferred), threaded as `md: bool` and **baked into leaf
  kinds** — the lexer is the *single* mode source. **Never re-derive `@md` in the block builder.**
- **Every *inline* recognizer MUST be `if md`-gated** (`*`/`_`/`` ` ``/`[`-link/`<`-autolink/
  `<`-html/list-marker/fence/image) — else its leaf kind stops implying `@md` and the projector
  mis-fires in non-`@md` blocks. (The `[`-link slipped this once; audit every new recognizer.)
- **Inline landed:** emphasis/strong/code (`\emph`/`\strong`/`\code`-vs-`\verb` per
  arity-parseability = roxygen2 `can_parse`), links, images, raw HTML. **Block landed:** lists
  (incl. **nested**), fenced code blocks, HTML blocks.
- **Markdown nested lists are indentation-driven** (`emit_md_list` recurses: a following list
  line indented ≥ an item's content column = `marker_indent + marker_width + 1..=4` opens a
  child `ROXYGEN_MD_LIST` *inside* that item; a line back at the list's marker column is a
  sibling; shallower ends it). The content indentation is **semantic** → the formatter must
  preserve it (`normalize_list_marker_text` keeps content indent, normalizing only the `#'` +
  one conventional space); flattening it = a behavior change. Projector: `push_inline` maps a
  nested `ROXYGEN_MD_LIST` node → `Inline::MdList`; `md_list_is_ordered` reads **direct-child**
  item markers only (a nested ordered sublist must not flip the parent's `\itemize`/`\enumerate`).
- **Links: under `@md` *every* bracket-free `[…]` not followed by `[`/`{` is a link**
  (`get_md_linkrefs`; `is_shortcut_content` mirrors it). `resolve_md_link` ports `parse_link`
  (inline→`\href`, reference/shortcut→`\link`/`\linkS4class`, `\code`-wrapped per code-span/`()`).
  Static-faithfulness: the section serializer **drops the topic option** (`\link[=dest]`), and a
  `pkg::` display prefix comes only from an explicit `::` (no installed-package introspection).
  An inline `[`code`](url)` sub-renders its code-span text (`link_display_atom`).
- **Images** (`scan_md_image`, inline `![…](…)` only): `mdxml_image` drops alt → `\figure{url}
  {title}`, wrapped per extension (`image_format`: svg→html, pdf→pdf, raster/unknown→bare). The
  Rd `\figure` route is a 2-arg verbatim macro.
- **Fenced code blocks** (`scan_md_fence`, carved whole, *before* the list-marker carve; bails
  if a backtick follows the run). `emit_md_code_block` pairs opener↔closer into
  `ROXYGEN_MD_CODE_BLOCK`. Projector emits 3 atoms: `\if{html}{\out{<div class="sourceCode[
  <info>]">}}` / `\preformatted{<code+\n>}` / `\if{html}{\out{</div>}}` (`%`/`{`/`}` raw,
  parse_Rd decodes). Atomic passthrough, baseline unchanged. Out of scope: ` ```{r} `
  knitr-eval blocks (roxygen2 evaluates).
- **HTML blocks** (`ROXYGEN_MD_HTML_BLOCK` node, the block analog of the fenced code block):
  `scan_md_html_block` carves a line-start opener (CommonMark start **condition 6** only — a
  block-level tag from `BLOCK_TAGS`, before the fence carve) as a `RoxygenMdHtmlBlock`→`TEXT`
  leaf; `emit_md_html_block` gathers the opener + following **Prose** lines until a blank line
  /tag/non-roxygen (the block swallows plain prose, per condition 6). Projector
  `serialize_md_html_block` → ONE `(\if (TEXT "html") (\out <verb-per-line>))` with a leading
  `(VERB "\n")` (`verb_atoms` splits at newlines, the VERB analog of `rcode_atoms`). Atomic
  passthrough; idempotent. Conditions 1–5/7 stay literal/inline (faithful under-handling, backlog).
- **Inline raw HTML** (`scan_md_html_inline`, chained after autolink at `b'<'`): `<tag>` →
  `(\if (TEXT "html") (\out (VERB <tag>)))` (`mdxml_html_inline`). Mirrors CommonMark Raw-HTML
  grammar **precisely** — over-recognition emits a spurious `\out`; comment/PI/declaration/CDATA
  forms stay literal (backlog). A line-start block-level tag is the **block** path above (carved
  first); a non-block tag (`<span>`) or a tag mid-prose stays inline.
- **List markers** (`scan_md_list_marker`): punctuation only (trailing space stays in text → a
  non-list marker reflows like plain text). `emit_md_list` applies the CommonMark interrupt rule
  (a bullet always interrupts; an ordered list only when start == 1; else stays inline prose →
  projector renders the leaf as text).

**Sections / projection**
- **Intro prose splits by *roxygen2 paragraph*, not CST node.** `parse_description` splits intro
  on `\n\n`: 1st = `\title`, 2nd = `\description`, rest = `\details` (folded with explicit
  `@details` only when leftover intro paras exist). Explicit `@title`/`@description` claims its
  slot. `section_body_parts` groups by paragraph (a block macro abutting prose = same para; a
  section-level blank-`#'` `ROXYGEN_MARKER` = break). Title-as-description fallback when no
  description exists. Don't revert to per-node parts (folds a trailing list into the wrong section).
- **Section pins sort in byte order, not locale collation.** The driver's
  `block_sections`/`projector_eligible` use `sort(secs, method = "radix")` (C-locale
  byte order) to match the Rust projector's `sections.sort()`. Latent until a section
  heads with something other than `(\…)` — a bare top-level `(TEXT …)`/`(GRP …)` from
  `@rawRd` sorts before `(\…)` by byte (`T`=0x54 < `\`=0x5C) but *after* under most
  UTF-8 locales. Any new bare-headed section ⇒ confirm the pin is byte-sorted.
- **`@rawRd` is bare top-level Rd, not a wrapped section.** roxygen2 injects the
  content verbatim; parse_Rd splits it into top-level nodes (each a "section").
  Projector arm: `serialize_inlines(body)` pushed atom-by-atom, no `(\macro …)` wrap.
  arity already lexes inline Rd macros in prose, so valid top-level Rd projects
  faithfully; invalid top-level Rd (an inline `\emph` at top level) makes parse_Rd
  error-recover (flatten to TEXT) while arity keeps the macro → divergence (don't
  curate such a case). Under `@md` the body would carry md leaves (rawRd is never
  markdown) → mis-projects; parser-side gap, deferred.
- **A prose section whose trimmed value is literal `"NULL"` is suppressed** (`rd_section()`
  sentinel; `NULL_SUPPRESSIBLE`). `@section` (title+body pair) is NOT suppressed; a suppressed
  `@description NULL` re-fires the title fallback. Data-object auto-`\format` (roxygen2 *evaluates*
  the object) is **out of scope**.

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
they stay in the R↔R fixed-point net, not false-positive backlog). Current (after verbatim
`\preformatted`, 2026-06-24s): **144 matching (all
allowlisted), 22 divergent (backlog)** of 166 pinned cases. The
divergences are now almost all roxygen2-*evaluation*/multi-block gaps (out of
scope); the in-scope remainder is links-across-lines, mid-line `\preformatted`
(rx-0a1710c0, body now handled — only its mid-line opener remains), and `@format %`
(all hard tail). Tasks:
`task roxygen-projector` (the gate),
`task roxygen-projector-refresh` (re-mint all pins), `task roxygen-projector-pins`
(harvested pins only), `task roxygen-projector-seed` (re-seed allowlist from matches).
Report: `ROXYGEN_PROJECTOR.md` (this dir).

**Three checks, three roles** (don't conflate):
1. **Projector parity** (`tests/roxygen_projector.rs`, pure Rust) — the **primary,
   parser-growth driver**. Compares Rd *structure*, so it sees block-structure gaps
   the fixed-point check is blind to. Curated corpus + harvested projector-eligible
   subset (166 pinned cases). The 22 divergences are the worklist.
2. **Curated fixed-point** (`tests/roxygen_oracle.rs::roxygen_oracle_report`, needs R,
   `#[ignore]`d) — strict semantic preservation of the formatter; 15/15 preserving, 0
   blocked. *Meaning, not layout.*
3. **Harvested fixed-point** (`tests/oracle/corpus/roxygen.jsonl`, 217 cases, needs R,
   `#[ignore]`d) — broad **opt-in** backlog gated by `roxygen-allowlist.txt`
   (215 preserving, 1 divergent, 1 skipped). A coverage net, **not** the parser driver
   (it's cosmetic-blind + R-dependent). Reports: `task roxygen-oracle` /
   `task roxygen-harvest`.

## Latest session (2026-06-24s) — verbatim `\preformatted` block → per-line VERB

**Construct:** a line-start multi-line `\preformatted{ … }` block macro. The CST
already modeled it as a `ROXYGEN_RD_MACRO` (line-start unbalanced `\name{` = Form
A); the gap was purely the **projector**: it whitespace-collapsed the body into one
`(TEXT …)` instead of the verbatim, per-line `(VERB …)` parse_Rd emits for a
verbatim macro. Closed via a curated `preformatted` case (no harvested case is
line-start — see Next).

**Bucket: projector gap (faithful encoding, not parser).** `\preformatted` is a
verbatim Rd macro, so its body must encode like `\out`/`\code`, not like prose.
- `serialize_macro` gains an early arm: `head == \preformatted` → `preformatted_atoms`,
  bypassing the run/flush prose model (which norm_ws-collapses).
- `preformatted_atoms(node)`: body = text between the opening `{` and closing `}`;
  the opener-line remainder keeps its leading space verbatim, each continuation line
  drops only its `#'` marker + one space (`strip_marker`), lines rejoin with `\n`,
  then `verb_atoms` splits at newlines (the established `\out`/HTML-block pattern).
- New `macro_head(node)` helper (peek the `ROXYGEN_RD_MACRO_NAME`).
- No parser change; the `*…*` inside `\preformatted` already stayed literal
  `ROXYGEN_TEXT` (it spans `#'` lines, so the line-scoped md emphasis recognizer
  never matched), which is exactly the verbatim-body behavior we want.

**Result:** projector **143→144 matching** (144 allowlisted), 22 divergent
(unchanged — the only `\preformatted` harvested case, rx-0a1710c0, is *mid-line*,
still backlog), 0 regressions, 166 pinned (+1 curated `preformatted`). `cargo test`
green (479). Curated fixed-point **15/15** preserving (was 14/14). One
format-baseline re-bless (intended: the new curated case; the formatter passes the
block through verbatim, idempotent). Files: `src/roxygen/project_rd.rs` (projector
arm + 2 helpers), new fixture `tests/fixtures/parser/roxygen_preformatted/` (+2
snapshots), new curated `tests/oracle/corpus/roxygen/preformatted.{R,rdtree}`,
projector allowlist (+re-seed), `roxygen-format-baseline.jsonl`,
`tests/parser_snapshots.rs`, TODO, RECAP.

**Next (ranked):** **rx-0a1710c0 (mid-line `\preformatted`)** is now *one gap from
done* — the projector arm landed this session handles its body; what remains is the
**mid-line opener** (`So far so good. \preformatted{ …`): a *third* block-opener
form where the verbatim `\name{` is not at line-content-start. Needs the **lexer**
to split a prose run at a mid-line unbalanced verbatim opener (flush the prefix
`RoxygenText`, start the opener) AND the **grouper** (`is_block_macro_line` /
`emit_block_macro` in `build.rs`, the `LineKind::Prose` arm in `group.rs`) to split
the open paragraph so the prefix prose stays a paragraph sibling and the
`\preformatted` block follows. Invasive (touches the line/paragraph state machine) —
scope it on its own. After that, the in-scope tail is **links broken across lines**
rx-383f2ca3/eb12b6b6 (line-scoped lexer can't span a `[…](…)` across `#'` lines) and
**`@format %`** rx-f6927028 (`%`-to-EOL Rd comment in non-md prose; coupled to the
formatter's prose reflow — defer). The other ~17 divergences are roxygen2-evaluation
or cross-block (out of scope; see 2026-06-24r below).

## Earlier sessions

- **2026-06-24r (markdown nested lists, `\itemize` in `\enumerate`, indent-driven):**
  an `@md` list whose items carry sub-lists by **indentation**. `emit_md_list` recurses
  (`emit_md_list_level` keyed on `list_indent`: a following line indented ≥ the item's
  content column opens a nested `ROXYGEN_MD_LIST` inside it, a line back at the marker
  column is a sibling); projector `push_inline` maps the nested node → `Inline::MdList`
  and `md_list_is_ordered` reads direct-child markers only; formatter
  `normalize_list_marker_text` **preserves** the now-semantic content indent (flattening
  = a behavior change). +1 (rx-91e67e79) + curated `md_nested_list`. 141→143.
- **2026-06-24q (nested *Rd* block macros, `\itemize` in `\enumerate`):** an unbalanced
  nested `\name{` opener inside a block macro's body now opens a child `ROXYGEN_RD_MACRO`
  via a `Vec<BodyFrame>` stack in `emit_block_content` (`Macro`/`Plain` frames; `}` at the
  empty stack terminates the enclosing macro) — replacing the flat `depth` counter. Projector
  already recursed (`serialize_macro` on a child `ROXYGEN_RD_MACRO`), so parser-only. Brace-
  driven, indentation-independent. +1 (rx-959fc227) + curated `rd_nested_list`. 139→141.
  *Triaged out-of-scope (confirmed via oracle):* rx-49a38f56 reexports, rx-8f9c159b/cbcc255c/
  deb9d202/4d59d472 data-object auto-`\format` (roxygen2 evaluates), rx-aef0e809 `@name` merge,
  rx-93452c15 block→object association — all cross-block or evaluation, arity is per-block+static.

- **2026-06-24p (`@rawRd` → bare top-level Rd nodes):** projector arm in
  `project_tag_section` (`rawRd` pushes each `serialize_inlines` atom as a *bare*
  top-level section, unwrapped). Exposed + fixed a latent locale-collation bug:
  switched the driver's two `sort(secs)` calls to `method = "radix"` (C-locale byte
  order) to match the Rust projector's `.sort()`, so pins are locale-independent.
  +1 (rx-3d22b1a9) + curated `rawrd`. 137→139.
- **2026-06-24o (block raw HTML `<p>…</p>` → `\if{html}{\out{…}}`):** parser+projector,
  +1 (rx-daf9322f) + curated `markdown_html_block`. New `ROXYGEN_MD_HTML_BLOCK` node
  (SyntaxKind 100, block analog of `ROXYGEN_MD_CODE_BLOCK`); `scan_md_html_block` carves
  a line-start CommonMark **start-condition-6** opener under `@md`, `emit_md_html_block`
  swallows following prose to the next blank line; projector `serialize_md_html_block`
  → one `\if{html}{\out{<verb-per-line>}}`. Atomic block re-blessed the one baseline. 137.
- **2026-06-24n (inline raw HTML `<tag>` → `\if{html}{\out{<tag>}}`):** parser+projector,
  +1 (rx-299f50fb). New `RoxygenMdHtml` leaf (`ROXYGEN_MD_HTML`, SyntaxKind 99);
  `scan_md_html_inline` chained after `scan_md_autolink` at `b'<' if md` (autolink claims
  `<scheme:…>`, raw HTML the rest), mirroring the CommonMark Raw-HTML grammar precisely
  (comment/PI/declaration/CDATA stay literal). Projector `Inline::MdHtml` → `html_inline_atom`.
  Atomic leaf stops mid-tag reflow (re-blessed baseline). 134→135.
- **2026-06-24m (inline-link-text code-span sub-render → `\verb`/`\code`):** projector-only,
  +1 (rx-3c528f59). An **inline** `[`code`](url)` carries the rendered code span as its
  `\href` text arg (`(\href (VERB url) (\verb …))`); new `link_display_atom` routes
  `href_atom`'s text through `md_code_atom` for a whole-text single span. 133→134.
- **2026-06-24l (URL autolinks `<url>` + empty-dest links → `\url`):** parser+projector,
  +1 (rx-f97e8917) + curated `markdown_url`. `scan_md_autolink` carves a CommonMark
  absolute-URI autolink `<scheme:body>` under `@md` (reusing `ROXYGEN_MD_LINK`; raw HTML
  has no scheme `:` → stays literal); projector `resolve_md_link` autolink branch +
  `inline_link_atom` (empty/equal dest → `url_atom`, else `href_atom`). 131→133.
- **2026-06-24k (markdown fenced code blocks → `\preformatted` triple):**
  parser+projector, +5 (rx-59e70a3d, rx-8c9662d6, rx-fb5d2ad5, rx-dd2506bf,
  bonus rx-0d100638 `{verbatim}`). Mode-keyed `RoxygenMdFence`/`scan_md_fence`
  carve before the list-marker carve; `emit_md_code_block` pairs opener↔closer into
  a `ROXYGEN_MD_CODE_BLOCK` section child; projector `Inline::MdCodeBlock` emits the
  3-atom `\if{html}{\out{<div…>}}`/`\preformatted`/`\if{html}{\out{</div>}}`
  (`mdxml_code_block`). Formatter atomic passthrough, baseline unchanged. 126→131.
- **2026-06-24j (brace-less unknown macros → `(UNKNOWN …)`):** +2. A brace-less `\word`
  not in `KNOWN_RD_MACROS` (R 4.5 table) → `(UNKNOWN "\\word")`; `scan_rd_macro` carves
  only when unknown (known brace-less stays literal). 124→126.
- **2026-06-24i (`@section` body inline macros + GRP-wrap):** projector-only, +2.
  `@section Title: body` → `\section{Title}{body}`, body sub-parses inline macros, 2-arg
  structural GRP-wrap. `split_section_title` + `grp_arg`. 122→124.
- **2026-06-24h (multiple `@examples` aggregate into one `\examples`):** projector-only,
  +3 (+ curated `examples_merge`). Aggregating field → `has_examples` flag in
  `project_block` → one `(\examples ...)`. 119→122.
- **2026-06-24g (digit-bearing Rd macro names, `\linkS4class`):** +1. Rd names are
  `[A-Za-z][A-Za-z0-9]*`; six duplicated scans truncated at a digit → one shared
  `rd_macro_name_end`. Projector unchanged. 118→119.
- **2026-06-24f (images `![](…)` + Rd `\figure` → `\figure`):** +3. `\figure{path}{cap}`
  is 2-arg verbatim; `![alt](url "title")` → `ROXYGEN_MD_IMAGE` (`scan_md_image`, inline
  only); `resolve_md_image` ports `mdxml_image` (alt dropped, extension-keyed wrap). 115→118.
- **2026-06-24e (intro paragraph split, title/description/details):** roxygen2's
  `parse_description` splits the intro on `\n\n` — 1st para = title, 2nd =
  description, rest = details (folded with explicit `@details` only when leftover
  intro paras exist). Pure projector gap: `section_body_parts` now groups by
  roxygen2 paragraph (section-level `ROXYGEN_MARKER` = blank `#'` = para break, a
  block macro abutting prose is the same para); `project_block` reimplements
  `parse_description`. Projector 105→115, +10. `Inline` now `#[derive(Clone)]`.
- **2026-06-24d (`@md` reference + shortcut links → `\link`):** the rest of the
  markdown-link cluster. roxygen2 treats *every* bracket-free `[…]` not followed by
  `[`/`{` as a link (`get_md_linkrefs`); `is_shortcut_content` widened the lexer,
  `resolve_md_link` ported `parse_link` (inline→`\href`, reference/shortcut→`\link`/
  `\linkS4class`, `\code`-wrapped per code-span/`()`). Projector 95→105, +10.
- **2026-06-24c (`@md` inline links `[text](url)` → `\href`):** projector +2
  (rx-7743ba62/rx-0605d020). Fixed a lexer mode-gating bug (the `[`-recognizer fired
  even without `@md`, mislabeling literal Rd brackets); gated it `b'[' if md`. New
  `Inline::MdLink` arm; `serialize_md_link` → `(\href (VERB url) (TEXT text))`.
  93→95 matching. **Trap:** every md *inline* recognizer must be `if md`-gated.
- **2026-06-24b/24 (Refactors, byte-identical, projector 93/66 unmoved):** #2 split the
  1686-line `src/parser/roxygen.rs` into a thin parent + 3 phase submodules
  (`lex.rs`/`group.rs`/`build.rs`); shared-infra-over-`cursor`/`recovery` is a **NON-GOAL**.
  #1 collapsed 8 silent `matches!` lists onto a compiler-policed source (`RoxygenRole` +
  wildcard-free `TokKind::roxygen_role`; `SyntaxKind::is_roxygen_prose_content`).
- **2026-06-23 (Stages 1–11, condensed; mechanics in traps + TODO):** Stage 1 CST
  re-model (`ROXYGEN_LINE` dissolved → `ROXYGEN_SECTION`/`ROXYGEN_PARAGRAPH`, trivia
  threading; byte-identical); Stage 2 `\itemize`/`\enumerate` (multi-line `ROXYGEN_RD_MACRO`
  via `Event::Leaf`); Stage 3 `\describe` `\item{term}{def}` (`TWO_ARG_RD_MACROS`,
  per-group flush); Stage 4 `\tabular` (Form B opener, GRP-wrap); Stage 5 `@md` inline
  (`*`→`\emph`, `**`→`\strong`, code→`\code`/`\verb`; mode infra); Stage 6 `@md` block
  lists (`RoxygenMdListMarker`, interrupt rule); Stage 7 title-as-description fallback;
  Stage 8 `@tag NULL` suppression (`NULL_SUPPRESSIBLE`); Stage 9 `\code` body→`RCODE`;
  Stage 10 `\href` per-arg verbatim (`is_verbatim_rd_arg`); Stage 11 `@slot`/`@field`
  aggregate→`\section{Slots/Fields}` (`describe_section`). 56→93 matching.
- **2026-06-22 (Phase 0 + Phase 1 skeleton, condensed):** Phase 0 (`acfd0b6`): R driver,
  curated corpus, strict fixed-point harness, `blocked.toml`, devenv R; soft→strict reframe.
  Then the harvested backlog (`harvest-roxygen-corpus.R`, 217 slug-keyed blocks, 212 PASS);
  the Phase 1 projector skeleton (`7473f2f`: section-level granularity, `block-to-sections`,
  `project_rd.rs`, pure-Rust pinned gate); filtered bulk-pin (`58ad5e4`: `projector_eligible`,
  `roxygen-sections.jsonl` 151/217, seeded 42); inline Rd macros as `ROXYGEN_RD_MACRO` nodes
  (`be0521b`: NAME/OPT/DELIM/VERB leaves + nesting; 42→56, closed `rd_macros`).
