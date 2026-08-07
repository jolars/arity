# roxygen-parity recap

Rolling log. Read top-to-bottom: status → discipline → invariants → construct
inventory → settled decisions → latest session. Traps are **terse by design** —
each is a rule + a source-of-truth pointer (usually a function name; grep/LSP it,
the doc-comment there is the full story). Full mechanics live in the code and git
history; this file is the *map*, not the territory. The `roxygen-parity` skill
reads this first.

> **History note.** This recap was condensed on 2026-07-27 once the measured
> backlog closed. Pre-condensation detail (the per-construct blow-by-blow, the
> cm-NNN edge notes, the full session log) is in git — `git log --follow` this
> file, or read the function doc-comments the pointers name.

## Status

**Oracle = roxygen2 8.0.0** (pin bumped 2026-08-07; `tests/oracle/.roxygen2-source`,
`roxygen2-ref` checkout at `v8.0.0`). **Backlog closed at the new oracle.**
Projector gate **1021 matching (all allowlisted), 0 divergent, 12 blocked**.
The **whole CommonMark spec (655/655)** matches; the harvested corpus is fully
closed. Curated fixed-point **227/227** preserving (verified against 8.0.0).
The measured backlog is **exhausted** — no divergence currently drives parser
growth.

**Next growth comes from** either (a) harvesting a fresh/larger roxygen2 corpus
to surface new gaps, or (b) closing a documented trap-backlog item. Known open
items: **recovery-pass bails** (incomplete `@title`/`@format`/`@source` — tail
crosses generated `\usage`/`\arguments`; merged-topic members; `@section`
two-arg bodies; imbalance inside macro atoms — see `parse_rd_recovery`'s module
doc); loose-file default-`@md` ON; the block→inline delimiter-stack migration
for the remaining local scanners; same-`@name` static-scope object-topic
resolution (only explicit `@name`/`@rdname` is grouped today); kept-tag
(`@note`/…) code spans whose body holds a `%` or unbalanced braces (the
imbalance reaches parse_Rd's recovery — the drop side is modeled, the kept
output shape is not); `doi`/`CRANpkg`/`PR` `USERMACRO` expansions;
merged-topic member with repeated `@title` (within-block collapse keeps only the
member's first title value, so the *merged* title-as-description fallback joins
first-values only — roxygen2 joins every value of every member);
**`@md` block constructs inside a block-macro body**
(a body line lexing as a list marker/fence leaf is a non-Content token that
breaks `emit_block_macro_from_opener`'s consume loop — unterminated-macro
recovery kicks in; benign under md-off, unprobed under md-on).

**Three checks, three roles** (don't conflate):
1. **Projector parity** (`tests/roxygen_projector.rs`, pure Rust, no R) — the
   **primary parser-growth driver**. `project_to_rd(parse(x))` vs pinned
   `<stem>.rdtree`; compares Rd *structure*, allowlist-gated. Writes
   `ROXYGEN_PROJECTOR.md`. Tasks: `task roxygen-projector`,
   `roxygen-projector-{refresh,pins,seed}`, `roxygen-spec-corpus`.
2. **Curated fixed-point** (`tests/roxygen_oracle.rs::roxygen_oracle_report`, needs
   R, `#[ignore]`d) — strict semantic preservation of the formatter
   (`roxygen2(format(x)) == roxygen2(x)`). *Meaning, not layout.*
3. **Harvested fixed-point** (`roxygen.jsonl`, needs R, `#[ignore]`d) — broad
   opt-in coverage net gated by `roxygen-allowlist.txt`.

## Discipline

- **Projector is faithful, never compensating.** A divergence means the CST (or the
  encoding translation) is wrong — fix the *parser*, never patch `project_rd.rs` to pass.
- **`project_rd.rs` is a facade** over `src/roxygen/project_rd/` submodules (`section`,
  `serialize`, `linkrefs`, `md_blocks`, `md_links`, `escapes`, `collect`, `sexpr`,
  `text`, `tests`). Any "(project_rd.rs)" pointer means the module — find the function
  by name, not by file.
- **Strict only for the *curated* corpus** (every case allowlisted or `blocked` with a
  rationale). *Harvested*/spec (JSONL): un-allowlisted = backlog, never `blocked`, never
  a build failure. Ratchet via `task roxygen-{harvest,projector}-seed`.
- **Cosmetic ≠ semantic.** The fixed-point check is layout-blind (a reflowed `\describe`
  renders identical Rd → passes); the structural *projector* gate is what catches it.
- **The oracle is roxygen2, NOT the CommonMark spec.** roxygen2 parses via `cmark` but
  processes *through* itself (md-escaping pre-pass, `rdComplete` validation, subset Rd
  translation). Never "CommonMark says X → arity does X"; only "roxygen2 does Y". Spec
  test set = **input corpus only**; roxygen2 supplies every answer.
- **R is for the oracle, not the gate.** The projector gate is pure-Rust (pinned
  `.rdtree`); only minting pins + the fixed-point net need `Rscript`.
- **Probe escape/bracket cases with exact-byte files**, never shell-quoted (`\\[` in a
  shell arg reaches R as two backslashes and masks the single-`\[` divergence). The
  driver ops: `block-to-sections`/`sections-batch` (projector pins),
  `block-to-tree`/`rd-to-tree` (fixed-point). `roc_proc_text` needs the block on an
  object (a function, or `@name` + `NULL`); `@md` must stand alone.
- **`format <file>` writes in place** — use `format < file` to avoid clobbering fixtures.
- **pre-commit `panache-format` reformats `.md`** and mangles long inline-code on wrap →
  put commands in fenced blocks. Never `--no-verify`.
- **END GOAL = full CommonMark parity** (tenet). CommonMark inline is a non-local
  whole-block **delimiter-stack** pass; do **not** widen a local scanner with heuristics —
  land it in the inline pass or record it as backlog. **Diagnostic parity is a second
  oracle surface** (roxygen2 warns then drops; arity should emit a side-channel diagnostic,
  CST stays lossless).

## Guardrail invariants (violate these and you silently regress)

- **A new line-body `TokKind` must reach EVERY classifier** or lines truncate at it.
  Single compiler-policed source: `TokKind::roxygen_role` (`lexer.rs`, wildcard-free).
  Still-explicit sites to update (grep an existing md leaf): `expr.rs` atom fallthrough,
  `tree_builder::syntax_kind_for`, `syntax.rs` `is_roxygen_token` +
  `is_roxygen_prose_content`, `kind_from_raw` + `COUNT`.
- **Mode is resolved per-block** (`resolve_roxygen_block`: `@md`/`@noMd`, default off) and
  **baked into leaf kinds** — the lexer is the *single* mode source. **Never re-derive
  `@md` in the block builder.** The one sanctioned exception is indented code (no
  mode-carrying leaf) via `block_md`; the projector's own `block_md` re-derivation is a
  separate, necessary mirror.
- **Every *inline* recognizer MUST be `if md`-gated** (`*`/`_`/`` ` ``/`[`/`<`/`!`/list/
  fence/…) — else its leaf kind stops implying `@md` and the projector misfires in a
  non-`@md` block. Audit every new recognizer.
- **`ROXYGEN_RD_MACRO` is a NODE, not a leaf** — classify with `el.kind()`, never
  `as_token()`. Lexed atomically; `build_rd_macro` expands it.
- **Logical CST, not line-based.** `ROXYGEN_BLOCK` → `ROXYGEN_SECTION`* (intro + one per
  `@tag`) → `ROXYGEN_TAG`/`ROXYGEN_PARAGRAPH`*. A block macro / md-list / md-code-block is
  a direct `ROXYGEN_SECTION` child. `#'` markers, marker→content WS, inter-line newlines
  are **trivia** threaded into the enclosing node. Projector walks `sections()`/
  `paragraphs()`; formatter rebuilds lines via `physical_lines`.
- **`\` carve is parity-gated** (`rd_backslash_is_escaped`, both modes + in-arg
  `build_rd_content`): parse_Rd pairs backslashes left-to-right, so a `\` after an odd run
  is consumed by its pair and never starts a macro (`\\y` literal, `\\\y` → `\y`).
- **`norm_ws` is ASCII-`[[:space:]]`-only** (`is_posix_space`), never Unicode. Do **not**
  revert to `split_whitespace`/`char::is_whitespace` (folds NBSP→space, breaks
  flanking-rejected emphasis). Flanking itself (`inline.rs`) *is* Unicode-aware.
- **Format-stability baseline** (`roxygen-format-baseline.jsonl`,
  `roxygen_format_stability.rs`): any intended formatter change re-blesses with
  `BLESS_ROXYGEN_FORMAT=1` **and review**. A new curated case adds one key.
- **SOFT_BREAK sentinel** (`'\u{c}'`): a soft-wrap carries it, not a space, so `%`-comment/
  linkref-block machinery reads paragraph breaks (`\n`) correctly. All NEWLINE→text sites
  in the projector emit it.

## Oracle serializer footguns (`roxygen_oracle.R`)

- **parse_Rd tags brace-group arg wrappers `TEXT` but they are *lists*.** Coalesce only
  genuine character TEXT leaves (`is_text_leaf`), or `\item{term}{def}` collapses to one atom.
- **`hardbreaks = TRUE`, yet soft-wrapped prose is safe** (no `\cr`) → coalesce TEXT runs.
  A real hard break (trailing `  `/`\\`) is a distinct node; preserve it.
- **`\examples` bodies are reformatted R** (Tenet 1) → serializer replaces them with `...`.
- **Section pins sort in byte order** (`sort(method="radix")`, matching Rust `.sort()`).

## Emphasis / inline pass

- **Emphasis is the real delimiter-stack pass** (`inline.rs::resolve_emphasis`, cmark
  `process_emphasis`), NOT a local scanner. The lexer carves `*`/`_` as neutral
  `RoxygenMdDelim` leaves; the pass emits `ROXYGEN_MD_EMPH`/`STRONG` **nodes**. Run =
  every paragraph-body token, bounded by a structural boundary → a span **crosses a soft
  break**. A single-line inline macro is a `RoxygenRdMacro` *token* (opaque atom a span
  crosses); a fragile macro presents alnum-leading, `-`-trailing placeholder edges for
  flanking (`edge_char`). Projector skips only first+last `MD_DELIM`.
- **Whole-run rescan** (`resolve_multiline_spans`) is cmark's precedence repair — extend it,
  never the `[`-carve. Optimistic lexer carve; the rescan runs cmark's leftmost scan
  (code spans, autolink/email/raw-HTML at `<` in `handle_pointy_brace` order) and a match
  can cover an already-carved `](url)` token, demoting the `[`. A failed backtick opener
  run is literal WHOLE (advance by `run_len`, both lexer and rescan).
- **Multi-line inline HTML + code spans resolve in the same pass** (leaf+node coexistence,
  lossless token tiling). Formatter descends cross-line nodes and bails reflow.

## Rd macro encoding (projector, faithful translation)

- **Name = `[A-Za-z][A-Za-z0-9]*`** (`rd_macro_name_end`, one source; digits allowed).
- **Arity is per-macro** — only `is_two_arg_rd_macro` (`TWO_ARG_RD_MACROS`: `item, tabular,
  href, figure`) consumes a 2nd `{…}`. GRP-wrap is per-arg, keyed on structural-vs-latexlike.
  **Verbatim is per-arg** (`is_verbatim_rd_arg`, `VERBATIM_RD_MACROS` + `href` arg 0 + `figure`).
- **`\code` body is `RCODE`** (verbatim R, no norm_ws); other latexlike text macros are TEXT;
  fully-verbatim are VERB. Nested macros recurse.
- **Brace-less `\word` carves only when unknown or zero-arg-known** (`is_known_rd_macro`/
  `KNOWN_RD_MACROS`; `ZERO_ARG_RD_MACROS`: `cr,tab,dots,ldots,R`). Any other known name
  brace-less stays literal prose in the CST; the *projector* renders parse_Rd's
  drop-recovery: `is_rd_braceless_drop_macro` = known ∧ ¬zero-arg ∧ ∉
  `STICKY_BRACELESS_RD_MACROS`. Sticky names leave RCODE/VERB swallow to section end
  (`split_sticky_braceless_swallow`, single-paragraph plain-text tails only) or an
  `(UNKNOWN "\item")` node (`split_braceless_items`).
- **Literal macro args resolve parse_Rd's Rd-string escapes** (`resolve_rd_arg_escapes`:
  `\{`→`{`, `\%`→`%`, `\\`→`\`, both modes). Non-md TEXT: `resolve_rd_text_escapes`.
- **Under `@md`, a non-fragile macro's ARG is markdown** (`is_fragile_for_md` = roxygen2's
  `escaped_for_md` protected set; `is_md_inline_text_macro`/`serialize_md_structural_macro`
  resolve via the real arena). Backslash runs pair even ACROSS the rendered text/macro
  boundary (`run_ends_odd_backslash_run`/`push_demoted_macro`).
- **Bare `{…}` prose groups are Rd `(LIST …)`** (`group_brace_lists`, both modes + in macro
  args + heading titles; brace parity mode-independent, `%`-trigger mode-inverted).

## Sections / projection

- **Same-topic blocks MERGE** (`@name`/`@rdname`). `project_to_rd` groups by `topic_name`;
  single-block groups keep the untouched `project_block` path, multi-block →
  `project_merged_topic`: members projected with `apply_title_fallback=false`, sections
  bucketed by head, `\title`→first (`format_first`), `COLLAPSE_HEADS`→one macro
  (`collapse_sections`/`coalesce_text_atoms`), other heads kept (dedup). **Title-as-desc
  fallback runs ONCE on the merged title vector** (only if no `\description` survives).
- **Comment-scan range** (`roxygen_scan_end`) tiles `[byte 1, end of last top-level
  expression]` — a `#'` line past it (trailing, or in an expression-less file) never
  renders; an in-body `#'` line joins its enclosing expression's block.
- **Intro splits by roxygen2 *paragraph*, not CST node** (`parse_description`): 1st=`\title`,
  2nd=`\description`, rest=`\details`. Title-as-description fallback is post-hoc
  (`topics_add_default_description`). `@rawRd` is bare top-level Rd, never markdown. A
  trimmed-`"NULL"` prose section is suppressed.
- **Field edges `str_trim` with Unicode White_Space** (`trim_field_atoms`; `@section` trims
  `title: content` as one string). Wider than `norm_ws`'s ASCII set; interior NBSPs survive.
- **`rdComplete` brace-balance drop is mode-dependent** (`section_rd_complete`): md-on scans
  `rdComplete(markdown(text))` (only `@description`/`@details` drop); md-off scans
  `rdComplete(x$raw)` unconditionally (every prose section, title included). A md-on
  **kept**-incomplete section's imbalance reaches parse_Rd, whose error recovery
  restructures the Rd file's affected tail — modeled by `parse_rd_recovery`
  (recovery.rs), a bounded bracket machine in roxygen2's physical emission order
  (`\value` renders BEFORE `\description`). `@field`/`@slot`
  drop the whole tag; `@section` md-off → `(\section (TEXT "NA"))`. A fragile macro's interior
  braces are neutralized in the scan.
- **`@md` text transforms** (order in `prose_text_atom`): `%`-swallow (parity-keyed on the
  source backslash run) → backslash-run collapse (`ceil(k/2)`, skips runs abutting `[`/`]`/
  `{`/`}`) → bracket unescape → HTML entity decode (`decode_html_entities`, 2125-entry
  `entities.rs`). Escaped brackets are the ONLY honored punctuation escape.
- **Link-reference map is modeled** (`get_md_linkrefs`, `normalize_linkref_label` =
  cmark's `normalize_reference`: ASCII-ws collapse + full Unicode case fold via
  `casefold.rs`). User `[ref]: url` defs parse at block level (`match_linkref_def`,
  consumed whole-line). `get_md_linkrefs` leaks invalid synthesized defs (whole-field
  poisoning: `leaked_linkref_text`/`append_leaked_defs`/`demote_poisoned_links`).

## Markdown constructs modeled (all COMPLETE — mechanics in the named functions)

- **Links** — inline/reference/shortcut/collapsed/autolink → `\href`/`\link`/`\url`;
  destination parity is cmark-after-`double_escape_md` (`inline_dest_span`); cross-line
  destinations; non-plain display drop; refmap-aware chain re-pairing
  (`repair_ref_link_chains`); nested links inner-first (`match_brackets`). `resolve_md_link`,
  `scan_md_link`, `same_line_bracket_opener`, `cross_line_*`.
- **Images** — inline/shortcut/reference → `\figure{url}{title}` (`resolve_md_image`,
  `scan_md_image`); extension-keyed `\if{html/pdf}` wrap; user-def override; demotion after
  odd backslash run.
- **Code** — spans (`\code` vs `\verb` per `code_span_is_r`/`has_invalid_name`); fenced
  (`scan_md_fence`, tilde + CommonMark closer `md_fence_run_closes`, indent strip
  `md_code_block_parts`, raw info-string drop `md_fence_info_drops`, knitr `{lang}` class
  `knitr_chunk_language`); indented (`ROXYGEN_MD_INDENTED_CODE`, ≥5-col, `block_md`).
- **HTML** — inline (`scan_md_html_inline`, all forms, multi-line via the inline pass);
  block conditions 1–7 (`scan_md_html_block`, `html_block_closers`; cond 7 builder-structural;
  reflow line-start guard `is_unsafe_line_start`).
- **Block quotes** — flatten = strip one `>` level + REPARSE as a synthesized `@md` fragment
  (`block_quote_flat_text`, `finish_md_block_quote`); glue onto neighbors (no separator);
  lazy continuation folds.
- **Headings** — ATX + setext → hoisted `\section`/`\subsection` (`emit_section_with_headings`,
  `HeadingFrame`); def-strip before promotion; per-piece `rdComplete` drop
  (`heading_piece_complete`); from-tag-value form.
- **Thematic breaks** — render empty (`is_md_thematic_break_line`; block-level column gate;
  same-line-in-list carve).
- **Lists** — nested (indent window `container_indent..min(content, +4)`), lazy/loose,
  marker-type split, same-line nested markers (`carve_md_list_markers`), all in-item block
  folds (fence/indented-code/table/block-macro/quote/heading — `emit_md_list_level_inner`).
- **Tables** — GFM `table` ext only (`is_md_table_start`, `serialize_md_table`); flat
  verbatim CST, per-cell independent inline runs.
- **Tabs** — tab-stop expansion in value coordinates (`advance_md_col`/`md_ws_gauge`); never
  count ws chars for block structure.
- **Block Rd macros** — three opener forms (`is_block_macro_line`, mid-prose `Form C`);
  brace-driven nesting (`BodyFrame` stack); atomic passthrough, context-keyed (`@examples`
  flush). Air does not format roxygen content → arity's own rule (Tenet 1).

## Settled decisions (don't relitigate without reason)

Mode-keyed parse (one `markdown_default` salsa input; `@md`/`@noMd` per-block override;
loose-file default ON deferred). CommonMark reference-spec two-pass (block tree → inlines);
**no crate dependency**. Projector is the **primary conformance engine** — `pub` but a
**test-only faithful diagnostic**, never patched to pass. **Projection granularity:
section-body subtrees, excluding roclet-generated scaffolding** (`\name`/`\alias`/`\usage`/
`\arguments`). Markdown = CommonMark core + GFM `table`, `hardbreaks = TRUE`; full parity is
the end goal (a subset is a gap). The local lexer span-scanners are the **wrong shape** — the
path is the block→inline delimiter-stack pass. The
WHOLE CommonMark spec is adopted as a measured backlog (panache's conformance model). Full
design: `~/.claude/plans/i-want-to-start-snoopy-haven.md`; roadmap: `TODO.md`. Phase 0 done;
Phase 1 (projector + pinned gate) is the driver.

## Latest session (2026-08-07f) — within-block same-head collapse + per-topic fallback scope

Closed the "within-block same-head collapse" backlog item. roxygen2 merges
repeated same-type sections inside ONE block exactly as across blocks
(`RoxyTopic$add` vector-appends each `rd_section`'s value; the per-type
`format` renders): repeated `COLLAPSE_HEADS` tags join into one macro
(`format_collapse` = `paste(collapse="\n\n")` → space after TEXT coalescing),
repeated `@title`s keep the first (`format_first`). Two regimes, probed and
modeled separately: with **leftover intro paragraphs**, every explicit
`@details` raw-joins into ONE tag *before* markdown (`parse_description` — a
trailing intro heading swallows the tag bodies; the existing `merge_details`
path); with **no intro leftovers**, markdown runs per tag value *first*
(headings hoist their own `\section` without swallowing the next value), then
the rendered values join.

Projector-processing only (no CST change): `collapse_same_head_sections`
(section.rs) runs post-tag-loop over the block's rendered section strings —
first occurrence anchors the merged section, `\title` keeps first — and
returns every title inner for the fallback. Two fallback fixes rode along:
(1) repeated `@title` with no description — `topics_add_default_description`
reuses the WHOLE title value vector, so the early Inline-level fallback site
defers (`explicit_titles > 1`) to a string-level collapse at the post-hoc
site; (2) the post-hoc site's `\description` presence check was scanning ALL
of `out` — now scoped to `out[block_start..]`, so an earlier topic's
description no longer suppresses a later topic's fallback. Trap re-learned:
the title-as-description fallback has **two sites** (the early `description`
match and the post-hoc check) — change both or the early one masks the fix.

Curated +1 (`same_head_collapse`, 7 units: details/seealso+note+source
collapse, title first-wins, multi-title fallback, md-on per-value heading
hoist, per-topic fallback scope, intro raw-join non-interference); baseline
+1 key (reviewed: only TagClass reflow). Projector 1020→**1021** matching
(all allowlisted), 0 divergent, 12 blocked. Fixed-point **227/227**. Full
workspace suite + clippy + fmt green. New recorded sub-edge: merged-topic
member with repeated `@title` (see Status).

## Earlier sessions (condensed)

### 2026-08-07e — block-form verbatim macro bodies + tail placement

Multi-line `\eqn{`/`\deqn{`/`\out{` bodies stay verbatim (per-line VERB atoms,
GRP per two-arg rule) with an adjacent second group consumed after the close;
escape regime per-macro (`eqn`/`deqn` raw, others resolve `\%`). Parser: block
macro tails placed *outside* the node (`emit_block_macro_from_opener` returns
`(index, tail)`; `emit_prose_rest`). Projector: `serialize_verbatim_block`.
Formatter: marker-less closing-line remainder kept as its own `PhysicalLine`.
Curated +1 (`verbatim_block_macro`, 10 units). 1019→1020. Fixed-point 226/226.

### 2026-08-07d — `\eqn`/`\deqn` optional second argument

parse_Rd gives `eqn`/`deqn` an optional second (ASCII fallback) `{…}` group,
both args verbatim, same-line-adjacent only. One-line parser fix:
`TWO_ARG_RD_MACROS` += `eqn`/`deqn` (lexer consumption already
present-and-adjacent-only; per-arg verbatim + md fragility already covered).
Curated +1 (`eqn_two_arg`). 1018→1019. Fixed-point 225/225.

### 2026-08-07c — demoted code spans + fragile gating in `` `Rd …` `` bodies

A code span's content reaches rendered Rd through roxygen2's fragile-tag
protection (`find_fragile_rd_tags`/`findEndOfTag`): inside a span body only a
**fragile** `\word{…}` survives as a parseable macro; non-fragile resolves to
literal `\` + prose (braces bare in `Sexpr`/`code` branches → `LIST`, escaped in
`verb`). Parser: `VERBATIM_RD_MACROS` += `eqn`/`deqn`/`out`, new
`resolve_rd_inline`. Projector: fragile-gated `sexpr_atom` via the real fragment
pipeline, `demoted_md_code_parts`, one-brace-pair `sexpr_to_rd` LIST rendering,
`defer_md_text_braces` (gated on `rendered_braces_balanced`),
`md_sexpr_span_drops`. Curated +5. 1013→1018. Fixed-point 224/224.

### 2026-08-07b — parse_Rd brace recovery for kept-incomplete `@md` sections

Kept-incomplete md-on prose tags (`markdown_if_active` skips `rdComplete`) reach
parse_Rd, whose recovery restructures the Rd tail — modeled by
`parse_rd_recovery` (recovery.rs), a bracket machine over projected section
strings in roxygen2's physical emission order (`\value` BEFORE `\description`),
detection via `text_brace_disturbance`, bounded by `SAFE_RECOVERY_TAGS` bails.
Curated +4 (`rdcomplete_keep_*`). 1009→1013. Fixed-point 219/219.

### 2026-08-07 — oracle bump to roxygen2 8.0.0 (churn absorbed) + grammar additions

Every pin re-minted at 8.0.0; 24 allowlisted regressions closed in five clusters,
all folded into the invariant sections above: trim narrowed to base `trimws`
(`is_trimws_space`); md-off incomplete `@section` → childless `(\section)`;
`\linkS4class` gone (`[s4-class]` → `\link[=s4-class]{s4}`); empty-bodied trailing
headings render (7.x splicer-crash fallback removed — unmasked the cm-070 lazy-
continuation parser gap, gated in `group.rs`); non-plain link displays KEPT
(`link_over_display`, with the `\emph{b\}` whole-field drop via
`link_display_render_drops` and the heading-title brace swallow via
`extend_escaped_list_closer`). Same-day grammar additions: `` `Rd expr` `` →
`(\Sexpr …)` (`sexpr_atom`), backtick-quoted two-part names (`split_two_part`),
`@prop`/`@R6method` classification. 1003→1009 matching, 15→12 blocked. Full
detail: `git log --follow` this file.

### 2026-07-27 — same-`@name` topic merge (rx-aef0e809) — 7.3.3 backlog closed

The last measured divergence. Blocks sharing a topic are one Rd file, so roxygen2 merges
them (`RoxyTopic$add`): the merge combines each section type's value vector, then the
per-type `format` renders — `\title` → `format_first` (first wins), the prose sections
(`COLLAPSE_HEADS`: description/details/seealso/value/note/references/author/format/source) →
`format_collapse` (values joined by a paragraph break → `norm_ws` space). The
title-as-description fallback runs ONCE on the *merged* title vector (a `@description` in any
block suppresses it). `@rdname`-alone errors in `roc_proc_text`, so `@name` is the practical
key.

**Projector processing** (not a CST change — all blocks stay lossless): `project_to_rd`
groups by `topic_name`; single-block groups keep the untouched `project_block` path,
multi-block → `project_merged_topic` (section.rs) with `project_block_impl(..,
apply_title_fallback=false)`. Curated `topic_merge`, 4 units, baseline +1. Projector
1001→1003, 1→0 divergent. Fixed-point 212/212. Full suite + clippy + fmt green.

### Older highlights

Condensed — full per-session detail is in `git log`. Recent highlights:

- **2026-07-26n** — block-to-object attachment (rx-93452c15): `roxygen_scan_end` skips a block starting at/past the last top-level expression. Curated `block_attachment`, 3 units. 999→1001.
- **2026-07-26 (a–m)** — whole-spec completion sweep: entities, setext/thematic-break/ATX block edges, code-span backtick runs, raw fence-info drop, cross-line inline-link dest, image backslash collision, HTML attr newline, knitr chunk info, harvested-backlog triage (15 blocked). **Whole spec 655/655 COMPLETE.** ~965→999.
- **2026-07-10f..25d** — adopted the whole CommonMark spec as a measured backlog (+388 latent); links/images/lists/quotes/tables/tabs/linkref coverage to completion. 808→965.
- **2026-06-22..07-10** — Phase 0 + Phase 1 skeleton; CST re-model (`ROXYGEN_LINE` dissolved); the emphasis inline-pass delimiter stack; markdown block/inline coverage; Rd macro + `rdComplete` + brace-group + escape machinery. →808.
