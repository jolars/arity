# roxygen-parity recap

Rolling log. Read top-to-bottom: persistent traps → settled decisions → progress →
latest session → earlier log. Keep ≤ ~300 lines; demote "Latest session" to a
one-liner under "Earlier sessions" each new session. Traps are **terse by design** —
each is a rule + a source-of-truth pointer (usually a function name; go read it). The
`roxygen-parity` skill reads this first.

## Persistent traps & invariants

**Discipline**
- **Projector is faithful, never compensating.** A divergence means the CST (or the
  encoding translation) is wrong — fix the *parser*, never patch `project_rd.rs` to pass.
- **Strict only for the *curated* corpus** (every case allowlisted or `blocked` with a
  rationale). *Harvested* (JSONL, `rx-`+sha1 slugs): un-allowlisted = backlog, never
  `blocked`, never a build failure. Ratchet via `task roxygen-{harvest,projector}-seed`.
- **Cosmetic ≠ semantic.** The fixed-point check is layout-blind (a reflowed `\describe`
  renders identical Rd → passes); the structural *projector* gate is what catches it.
- **`format <file>` writes in place** — use `format < file` to avoid clobbering fixtures.
- **pre-commit `panache-format` reformats `.md`** and mangles long inline-code on wrap →
  put commands in fenced blocks.
- **R is for the oracle, not the gate.** The projector gate is pure-Rust (pinned
  `.rdtree`); only minting pins + the fixed-point net need `Rscript`.
- **Probe escape/bracket cases with exact-byte files**, never shell-quoted (`\\[` in a
  shell arg reaches R as two backslashes and masks the single-`\[` divergence).

**Oracle / serializer (`roxygen_oracle.R`)**
- **`parse_Rd` tags brace-group arg wrappers `TEXT` but they are *lists*.** Coalesce only
  genuine character TEXT leaves (`is_text_leaf`), or `\item{term}{def}` collapses to one atom.
- **`hardbreaks = TRUE`, yet soft-wrapped prose is safe** (no `\cr`) → coalesce TEXT runs.
  A real hard break (trailing `  `/`\\`) is a distinct node; preserve it.
- **`\examples` bodies are reformatted R** (Tenet 1) → serializer replaces them with `...`.
- **`roc_proc_text` needs the block on an object** (a function, or `@name` + `NULL`); a bare
  block errors. **`@md` must stand alone** — a prose value errors.

**CST shape**
- **A prose tag's same-line value folds its plain-prose continuation into the `ROXYGEN_TAG`.**
  `emit_tag_line` (group.rs): when a tag has a same-line **Content** token *and* its name passes
  `tag_folds_prose_continuation` (lex.rs — prose classes only; code/examples/value/token-list/`@section`/
  rawRd are excluded, so `@examples`' `ExampleBody` etc. keep their per-line structure), it keeps the tag open
  and folds each contiguous plain-prose continuation line (`is_foldable_continuation`: `LineKind::Prose`, not a
  block-macro/list/fence/HTML) — trivia + tokens — **into** the tag. So opener and closer of an `@md` span live
  in one node and the emphasis pass (bounded by the tag) resolves across the soft break. A blank/new-tag/block
  ends the fold (stays a section sibling). Projector `tag_inlines` drops the folded `ROXYGEN_MARKER` and maps
  `NEWLINE`→`SOFT_BREAK` (mirrors `paragraph_inlines`; the EMPH-node path already did via `push_inline`).
  **Formatter:** the folded tag is **one `PhysicalLine`** (still pushed whole; no `physical_lines` change) —
  `chunk_elements`/`chunk_into` breaks on the threaded `NEWLINE`/`WHITESPACE`, skips the `MARKER`, and descends a
  cross-line EMPH/STRONG/LINK (shared `cur`); `tag_prose_chunks` passes `NEWLINE` through; the reflow-bail
  linkref/`%`-swallow check reads `tag_first_line_value` (first physical line only, not the glued whole); and a
  bailing multi-line tag re-splits its source via `emit_tag_passthrough`'s `is_multiline_tag` branch. Curated
  `tag_sameline_emph` + a plain-fold projector unit test (`sameline_tag_value_folds_plain_continuation`).
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
  (`BLESS_ROXYGEN_FORMAT=1`) **with review**. An atomic-span leaf that stops mid-construct
  reflow is a Tenet-1 win → re-bless the one affected case.
- **A new line-body TokKind must reach every classifier** or lines silently truncate at it.
  Single compiler-policed source: `TokKind::roxygen_role` (`lexer.rs`, wildcard-free). Still-
  explicit sites (grep an existing md leaf): `expr.rs` atom fallthrough,
  `tree_builder::syntax_kind_for`, `syntax.rs` `is_roxygen_token` + `is_roxygen_prose_content`,
  `kind_from_raw` + `COUNT`.

**Rd macros**
- **Name = `[A-Za-z][A-Za-z0-9]*`** (digits allowed, `\linkS4class`). One source for the name
  end: `rd_macro_name_end`; every name scan routes through it (else digit-truncation).
- **Arity is per-macro, not greedy.** `\code{x}{y}` = `\code{x}` + literal `{y}`; only
  `is_two_arg_rd_macro` (`TWO_ARG_RD_MACROS`: `item, tabular, href, figure`) consumes a 2nd
  `{…}`. Confirm via `parse_Rd`: a trailing `{…}` tagged `LIST` = NOT consumed.
- **GRP-wrap is per-argument, keyed on `is_two_arg_rd_macro`.** A *structural* macro wraps a
  multi-atom arg `(GRP …)`, unwraps a single-atom one. A *latexlike* macro (`\code`/`\emph`/…)
  inlines its arg's atoms, never GRP.
- **Verbatim is per-*argument*.** `is_verbatim_rd_arg(name, index)` drives `build_rd_macro`'s
  recurse decision (`VERBATIM_RD_MACROS`: `url, verb, samp, env, kbd, option`; plus `href` arg 0
  and `figure`). Projector emits `(VERB …)` for a `…_VERB` leaf.
- **`\code` body is `RCODE`, not `TEXT`** (verbatim R: no `norm_ws`, split at newlines).
  `serialize_macro` flush keys `head == "\\code"`. Other latexlike text macros are `TEXT`;
  fully-verbatim macros are `VERB`. Nested macros still recurse.
- **Brace-less `\word` carves when *unknown* or *zero-arg known*.** `is_known_rd_macro`/
  `KNOWN_RD_MACROS` (parse_Rd's static table, R 4.5; excludes expanded user macros
  `\CRANpkg`/`\doi`). Unknown → `(UNKNOWN "\\word")`; zero-arg known (`ZERO_ARG_RD_MACROS`:
  `cr, tab, dots, ldots, R`) → a name-only node (`\dots`→`(\dots)`, early return in
  `scan_rd_macro` — never consumes a following `{…}`). Any *other* known name brace-less stays
  literal prose in the **CST** (lossless); the *projector* renders parse_Rd's drop-recovery for
  the non-sticky names (see the brace-less-drop trap below). A new known macro must go in the
  table or it silently becomes UNKNOWN.
- **A `\` carve is parity-gated: parse_Rd pairs backslashes left-to-right.** A `\` preceded by
  an odd-length backslash run is consumed by its pair and never begins a macro (`\\y` = literal
  `\`+`y`; `\\\y` re-forms `\y`; `\\dots` literal, `\\\dots` → `(\dots)`). `rd_backslash_is_escaped`
  (lex.rs) gates the prose dispatch's `b'\\'` arm (inline carve + block-macro opener alike), both
  modes — the md pipeline (double→cmark) is a net no-op on a run before a letter, so parse_Rd sees
  the same k either way. **`build_rd_content` (in-arg sub-parse) IS now gated too (2026-07-07f)** —
  same `rd_backslash_is_escaped` guard on its `b'\\'` macro-carve arm, so `\emph{\\y}` = literal `\y`
  text (one TEXT leaf, no carve), `\emph{\\dots}` = literal `\dots`, `\emph{\\\dots}` = `(TEXT "\\")`
  + `(\dots)`; a genuine single `\dots`/`\strong` still carves. In-arg **text-escape** resolution was
  already handled projector-side by `resolve_rd_arg_escapes` (2026-07-07c), which collapses the
  now-literal `\\y`→`\y`.
- **Under `@md`, a non-fragile macro's ARG is markdown.** roxygen2's `escaped_for_md`
  (`markdown-escaping.R`) is the *fragile* protected set (`\code`/`\link`/`\verb`/`\url`/
  `\preformatted`/…) whose arg stays literal; **every other** macro keeps only its backslash-word
  literal while its arg **is** full markdown (`\emph{*x*}`→`\emph{\emph{x}}`). Ported as
  `is_fragile_for_md` (`src/parser/roxygen.rs`). **Projector-only** encoding translation (CST +
  formatter untouched — the projector's job): `serialize_macro` is `md`-threaded;
  `is_md_inline_text_macro` gates known + non-fragile + single-arg + non-block;
  `macro_single_arg_content` + `resolve_md_inline` (parser entry: `lex_roxygen_prose_fragment` +
  `resolve_emphasis` + `build_tree`) resolve via the **real arena**, projected by ordinary
  `push_inline`/`serialize_inlines`. Recursion re-checks fragility per macro.
- **Structural two-arg macro args md-process per-argument, whole-arg.** `\item`/`\tabular`/`\href`
  (non-fragile `is_two_arg_rd_macro`; `is_md_structural_macro`). `serialize_md_structural_macro`
  walks each pre-carved arg group into `MdArgPiece`s (prose → markdown-lexed text; every nested
  macro, braced or brace-less, → one opaque `Macro` carrying raw source) and resolves the **whole
  arg as one cmark run** via parser `resolve_md_inline_pieces`, so emphasis/link spans **cross a
  nested macro** (`\item{x}{*a \strong{y} b*}` → `(\item (TEXT "x") (\emph (TEXT "a") (\strong
  (TEXT "y")) (TEXT "b")))`, even `*a \tab b*`). Pieces not whole-string re-lex: the fragment lexer
  leaves brace-less known macros literal; emitting each carved macro as a synthetic `RoxygenRdMacro`
  token lets `build_rd_macro` re-expand a brace-less `\tab` to a name-only node. Verbatim args stay
  `(VERB …)`; multi-atom `(GRP …)`-wraps, single-atom stays bare.
  **Link-display drop (Case A):** `link_display_is_droppable` counts an `Inline::Macro` as plain
  only when `!macro_arg_has_active_markdown` (recursive), so `[a\emph{*x*}]` drops, `[a\emph{x}]`/
  `[a\code{*x*}]` keep. A **pure-macro display** uses `link_label_text` (= `inline_plain_text` +
  an `Inline::Macro(n) => n.text()` arm) at the link-reference sites only
  (`link_ref_label`/`linkref_skeleton_push`/`demoted_link_source`) so the label is non-empty and
  self-consistent and reaches the drop/keep decision.
  **Backlog:** `\value`/`\section` *inline*; cmark-active markdown inside a *fragile* arg (ties
  into the markdown `\`-escape backlog); `linkref_def_label` drops a macro *def* label.
- **Block-macro openers, three forms.** Forms A/B are *line-start* (`is_block_macro_line`). Form A:
  a `RoxygenText` `\name{` unbalanced on its line (`\itemize`/`\describe`) — a multi-line opener.
  Form B: a balanced structural `RoxygenRdMacro` (`\tabular{rl}`) then `RoxygenText` opening the
  body `{`. `emit_block_macro` dispatches. **Form C: a mid-prose opener** — the lexer
  (`lex_roxygen_prose`, `is_block_macro_opener_at`) always splits an unbalanced `\name{` into its
  own to-EOL `RoxygenText`; the grouper (`emit_prose_line`) promotes it to an **inline**
  `ROXYGEN_RD_MACRO` inside the open paragraph **only if `block_macro_opener_closes`** (else literal
  prose — the conservative recovery, parse_Rd rejects an unbalanced macro). Formatter detects a
  markerless opener (`first_token() != ROXYGEN_MARKER`) and **prepends `#' `** (lossless +
  idempotent).
- **Nested block macros are brace-driven, not indentation.** `emit_block_content` tracks open
  groups with a `Vec<BodyFrame>` stack (`Macro` = nested `\name{` → child `ROXYGEN_RD_MACRO`;
  `Plain` = bare prose `{`, literal both ends). A `}` at the *empty* stack terminates the enclosing
  macro. Only an **unbalanced** `\name{` triggers nesting.
- **Block Rd macro = atomic passthrough, context-keyed** (not reflow). Prose: `emit_block_macro`
  preserves in-macro indentation. `@examples`: `emit_block_macro_examples` emits **flush**. Air
  does **not** format roxygen content (verified) → not an oracle for any roxygen layout; the rule
  is arity's own (Tenet 1), idempotent. *(Open: canonical re-indent for prose lists; deferred.)*

**Markdown — mode-keyed**
- **Mode resolved per-block** by `resolve_roxygen_block` (scans the `#'` run for `@md`/`@noMd`,
  default off; loose-file default-ON deferred), threaded as `md: bool` and **baked into leaf
  kinds** — the lexer is the *single* mode source. **Never re-derive `@md` in the block builder.**
- **Every *inline* recognizer MUST be `if md`-gated** (`*`/`_`/`` ` ``/`[`-link/`<`-autolink/
  `<`-html/list-marker/fence/image) — else its leaf kind stops implying `@md` and the projector
  mis-fires in non-`@md` blocks. (The `[`-link slipped this once; audit every new recognizer.)
- **The oracle is roxygen2, NOT the CommonMark spec.** roxygen2 parses via `cmark` (faithful
  CommonMark) but processes *through roxygen2*: a markdown-escaping pre-pass, `rdComplete`
  brace/quote **validation**, and a *subset* Rd translation. Never "CommonMark says X → arity does
  X"; only "roxygen2 does Y → arity does Y." Spec test set = **input corpus only**.
- **END GOAL = full CommonMark parity** (tenet). CommonMark inline is a non-local whole-block
  **delimiter-stack** pass (block→inline), per `docs/design/roxygen-inline-pass.md`. Do **not**
  widen a local scanner with heuristics — land it in the inline pass or record it as backlog.
- **Diagnostic parity is a SECOND oracle surface.** roxygen2 validates, emits source-located
  warnings, then **drops** bad content (`rdComplete` in `tag-parser.R`). arity should detect the
  same condition and emit a **side-channel diagnostic** (CST stays lossless). An oracle-*error*
  input is a diagnostic-parity fixture, NOT a silent `blocked`.
- **Emphasis is the real delimiter-stack pass** (`inline.rs::resolve_emphasis`, cmark
  `process_emphasis`: full ASCII flanking, rule of 3, nesting), NOT a local scanner. The lexer
  carves `*`/`_` as **neutral** `RoxygenMdDelim` leaves; the pass emits `ROXYGEN_MD_EMPH`/`STRONG`
  **nodes** with `ROXYGEN_MD_DELIM` opener/closer/leftover leaves. **Run = every paragraph-body
  `Event::Tok`** (content + inter-line trivia), bounded by a structural `Start`/`Finish`/`Leaf`
  (paragraph/section/tag boundary) — so a span **crosses a soft line break**. A **single-line inline
  macro is a `RoxygenRdMacro` *token***, so it already joins the run as an opaque atom and a span
  crosses it; only a **multi-line** macro (`Start..Finish` events, `emit_block_macro_inline`) bounds
  the run (deferred backlog, contrived: multi-line inline macros are block/list macros). The
  projector skips only the **first and last** `MD_DELIM` child (opener/closer); interior `MD_DELIM`
  is literal text. **Formatter:** `collect_logical_elements` descends into a cross-line EMPH/STRONG
  node (`is_cross_line_emph`) so reflow rejoins it (a contained macro is one atomic reflow unit); a
  single-line span stays atomic. Idempotent.
- **An inline Rd macro flanks like roxygen2's placeholder, NOT its own punctuation** (`edge_char`).
  roxygen2 swaps a fragile tag for an **alphanumeric** placeholder suffixed `-<i>-` before cmark
  (`escape_rd_for_md`), so for flanking a `RoxygenRdMacro` token presents `'x'` (alnum) at its
  **leading** edge and `'-'` (hyphen) at its **trailing** edge — NOT its raw `\`/`}`. Asymmetry is
  load-bearing: an opener abutting a macro opens (`a*\code{x} y*`→`\emph{\code{x} y}`); a closer
  abutting one stays blocked by the `-` (`a*\code{z}*b` keeps both `*` literal). A delimiter not
  adjacent to a macro is unaffected (the macro is interior).
- **Cross-line emphasis works only when the tag value starts on the *next* `#'` line** (`@details`
  alone, prose below). A tag with a *same-line* value (`@details *a x`) splits continuation lines
  into separate `ROXYGEN_PARAGRAPH` siblings, so a span can't cross — a **separate grouping**
  divergence (independent of macros), still backlog.
- **Links: under `@md` *every* bracket-free `[…]` not followed by `[`/`{` is a link**
  (`get_md_linkrefs`; `is_shortcut_content` mirrors it). `resolve_md_link` ports `parse_link`
  (inline→`\href`, reference/shortcut→`\link`/`\linkS4class`, `\code`-wrapped per code-span/`()`).
  The section serializer **drops the topic option** (`\link[=dest]`); a `pkg::` prefix comes only
  from an explicit `::`.
- **Every same-line bracketed link span is a node; one carve rule.** `same_line_bracket_opener`
  (`lex.rs`) carves a `[` opening a balanced, bracket-free `[…]` (`is_shortcut_content`, **no `!`** —
  kept for images; `\` allowed) whose after-`]` ∉ `(`/`{` — covering shortcut display, reference
  display, and reference label. The arena's `classify_closer` reads a following `[ref]` via
  `neutral_ref_label` and folds it as `][ref]`. So plain references `[plain][ref]` are
  `ROXYGEN_MD_LINK` nodes (`MdRefLink`), not opaque leaves. `link_display_is_droppable` drops a
  non-plain display ("markdown links must contain plain text") **but counts `Inline::Macro` as
  plain**. A `\`-bearing display is on the arena too (`[a\b]`→`(\link (TEXT "a") (UNKNOWN "\\b"))`,
  `[a\emph{x}]` renders the macro via `display_has_macro`/`link_over_display`, `[a\*b\*]` drops).
  `scan_md_link`'s `[`-path survives ONLY for an `!`-bearing display; autolink `<url>` is on
  `scan_md_autolink`.
- **A `[…](…)` is an inline link only when the `(…)` is a VALID CommonMark destination** (2026-07-10b).
  A bare destination runs to the first ASCII whitespace (a `\` never escapes whitespace — `\ ` is a literal
  backslash then a terminating space), so `[t](a\ b)`/`[t](a b c)`/`[t](url ok)` are **not** links: `[t]` falls
  back to a shortcut `\link{t}` and the `(…)` stays literal prose. A `<…>` destination may contain spaces; after
  the destination only trailing whitespace — or whitespace then one `"…"`/`'…'`/`(…)` title then trailing
  whitespace — may follow (`[t]( url )`/`[t](url "t")` link, `[t](<a b>)` links). Single source: `valid_inline_dest_content`
  + `inline_dest_span` (`lex.rs`) replace the old raw `scan_balanced(…,'(',')')` at **all four** carve sites —
  `inline_link_span`, `same_line_bracket_opener`'s after-`]` `(` exclusion, the bare-`]` closer arm (**line ~704** —
  a `]` followed by an *invalid* `(` must still close so the shortcut pairs; missing this left `[t]` un-carved),
  and `scan_md_link`'s `(` arm (falls to the shortcut on an invalid dest); `cross_line_link_closer` too. Projector
  + formatter **unchanged** (lexer decides link-vs-not; the arena/projector follow). Curated `md_link_invalid_dest`,
  fixture `roxygen_md_link_invalid_dest`, unit `invalid_inline_dest_falls_back_to_shortcut`. **Reference images**
  (`![x](a\ b)` → `\figure{R:x}` via the synthesized linkref) — arity models inline images only, so `scan_md_image`
  is untouched (still backlog).
- **A trailing-backslash inline-link destination DROPS the section, projector-only (2026-07-10c).** `[t](foo\)bar)`:
  cmark's `double_escape_md` turns `\)` into `\\)` → a literal `\` then a closing `)`, so cmark's destination is
  `foo\` — a trailing backslash that escapes the `\href{…}` brace → `\href{foo\}{t}` is `rdComplete`-incomplete →
  roxygen2 **drops the whole section** (`(\details)`). arity's `scan_balanced` treats `\)` as an escaped paren, so
  the CST link node keeps the *wider* span `[t](foo\)bar)` (lossless; `\href{foo\)bar}{t}`) — **not** re-boundaried;
  the drop is detected projector-side. New `body_has_dropping_href`/`md_href_dest_drops` (project_rd.rs), gated into
  `section_rd_complete`'s md arm **before** the atom scan: find cmark's closer (first paren-depth-0 `)`, `\` literal),
  then the backslash run before it survives `double_escape`→cmark→`parse_Rd` as `r` backslashes, so an **odd** `r`
  escapes the brace (`foo\)bar` r=1 drops; `foo\\)bar` r=2 keeps). Recurses into emphasis/brace-group/list-item/display.
  Curated `md_link_dest_backslash_drop` (+pin `(\details)` +allowlist), fixture `roxygen_md_link_dest_backslash_drop`
  (CST link node spans the whole `[t](foo\)bar)`; lossless), unit `trailing_backslash_inline_dest_drops_the_section`,
  format baseline +1 (additive — the corpus key set). **Backlog:** the SURVIVING even-run case content still diverges
  (`\href{foo\\}{t}` vs roxygen2's `foo\` — the URL needs the `double_escape`→cmark→`parse_Rd` backslash-run collapse
  *and* the cmark link boundary, both untracked); an odd-run trailing `\` inside a *reference/shortcut* label or an image dest.
- **Arena does CommonMark opener deactivation; nested links resolve inner-first.** `match_brackets`
  (`inline.rs`): a stack pairs each `]` to the nearest *active* `[`, a formed link deactivates every
  opener below it, a lone `]` does the `][ref]` lookahead and is a shortcut only on a bracket-free
  interior. So `[a [b] c](url)` → inner `[b]` is an `MD_LINK` node, outer brackets stay literal. The
  lexer's `is_nested_bracket_opener` carves the outer `[`. A nested link is **no longer atomic** in
  the formatter (reflows within literal portions).
- **Arena resolves links OPTIMISTICALLY; poisoning is repaired in the projector.** The arena can't
  see the refmap, so the inner shortcut always wins. For a *poisoned* nested link,
  `relink_demoted_inline_links` (in `demote_poisoned_links`) re-forms the enclosing `[…](url)`
  `\href` from demoted bracket text, scoped by a **consecutive-`Inline::Text`** scan (a surviving
  inner link interrupts the run; an escaped `\[` keeps its backslash and never relinks).
- **The link-reference map is modeled; an undefined shortcut/ref stays literal.** roxygen's
  `get_md_linkrefs` `(?<!\])` lookbehind blocks reference-**definition** creation for a `[` after `]`
  (and `(?=[^\[{])` before `[`/`{`), but link **resolution** still uses the refmap. Projector:
  `linkref_keys(body)` builds the refmap from a faithful raw-source reconstruction
  (`linkref_source_skeleton` — re-exposes every link/image bracket, opaque leaf verbatim) scanned by
  `md_linkref_scan`; `demote_undefined_links` rewrites any shortcut/ref link whose normalized label
  (`normalize_linkref_label`) ∉ refmap to literal (`demoted_link_source`), before the positional
  poison demotion. **Full refmap = full candidate set** (so `md_ref_link_multiline`'s `a][b]` still
  links). **Open:** refmap is per-prose-body, not whole-field (a sibling-paragraph def is missed).
- **User link-reference definitions (`[ref]: url`) → `\href{url}{display}`, display KEPT.** A
  CommonMark def gives a destination → `\href` (not the R-topic `\link`, so the "must contain plain
  text" drop doesn't apply), and the def line is **consumed**. User def beats roxygen's synthesized
  `[ref]: R:ref`. Projector-only `resolve_user_linkrefs` (before `demote_undefined_links`, on the
  original body): `collect_user_linkrefs_tree` (whole-field, recursing into list items) +
  `apply_user_linkrefs`; a def run is consumed only at a **block start** (a def can't interrupt a
  paragraph). `parse_linkref_def_dest` handles bare/`<…>` dests, same-line title, entity-decode
  (`&amp;`→`&`), and multi-line dests (`match_linkref_def` gathers across soft breaks). `@section`'s
  arm runs the same shared `resolve_linkrefs` pipeline. **Formatter:** the prose-reflow bail fires
  under `@md` when a paragraph's first line (or a tag's prose value) is a link-ref def
  (`text_is_linkref_def`/`linkref_dest_is_clean`) so consecutive def lines stay unjoined.
  **Trailing content = not a def, physical-line-scoped.** `[foo]: url \emph{bar}` is not a definition
  (CommonMark forbids non-whitespace after the dest/optional-title *on its line*) → `foo` stays an
  undefined shortcut. `match_linkref_def`'s `Text` loop stops at the first non-`Text` inline, so a
  trailing macro/link was invisible; it now rejects a non-`Text` inline that follows **on the same
  physical line** (`parse_linkref_def_dest` returns `(url, line_closed)`; `line_closed` = a `SOFT_BREAK`
  follows the dest/title). Load-bearing: a *stacked* def's next `[r2]` label is also a non-`Text` inline
  but sits after a `SOFT_BREAK` (a new block) → allowed. `text` never has `\n` (loop breaks at a
  paragraph break), so a residual line boundary is always a soft wrap. **Backlog:**
  multi-line def *titles*, cross-list duplicate-label document order.
- **A shortcut/reference link with a non-plain display is DROPPED to empty.** roxygen2's `parse_link`:
  after unwrapping a *sole* `code` child (which links — `\code{\link{…}}`), any non-text display child
  → `warn` "markdown links must contain plain text" + `return("")` (link vanishes, prose stays
  contiguous). Drops: emphasis, a 2nd code span, text+code, autolink, image, HTML. Keeps: pure text
  (intraword `_` is *not* emphasis — needs real flanking), sole code span. Projector
  `link_display_is_droppable` drops the node in `serialize_inlines` via `continue` **without flushing
  the text run**. Inline `[text](url)` never drops (own dest → `\href`).
- **`get_md_linkrefs` leaks invalid synthesized defs, whole-field poisoning.** roxygen2's
  `add_linkrefs_to_md` appends `[label]: R:URLencode(label)` for **every** bracket-free `[…]`
  candidate as one cmark block, source order, parsed top-down. An **escaped-close** candidate
  `[text\]` yields a def whose label never closes → that def *and every def after it* fail, so cmark
  leaks the block **from the first invalid candidate to the end** (valid candidates included), and any
  shortcut/reference link in that tail is **de-linked**. `leaked_linkref_text` (projector, `@md`-only):
  `double_escape_md`→`md_linkref_labels`→take `[first_invalid..]`→`url_encode`→`cmark_unescape`.
  De-linking is upstream via `demote_poisoned_links` (`first_invalid_linkref_offset` — any trailing
  backslash = invalid, since `double_escape_md` makes a `k≥1` run odd). Whole-field: skeleton
  (`inline_skeleton_fragment`) + demotion walk (`demote_poisoned_walk`) descend into list items
  space-guarded per item. **Inline links/autolinks/code survive** (own destination); their candidate
  defs still **leak** (`inline_skeleton_fragment` exposes `[text] `/`[alt] `/inner `[b] ` via
  `opaque_inline_link_display`/`image_alt_text`) though the link is not demoted. **Backlog:** `@rawRd`
  leaks; `relink_demoted_inline_links` nested-in-list.
- **Escaped brackets are the ONLY honored punctuation escape.** roxygen2's `double_escape_md` doubles
  every `\` but **reverts** `\\[`→`\[`, `\\]`→`\]`, so only `[`/`]` keep a CommonMark escape: `\[`
  neither opens a link nor keeps its backslash, whereas `\*`/`` \` ``/`\%`/… keep their **single**
  backslash (do **not** add general escape handling). Lexer `bracket_is_escaped` guards the three
  `[`-openers; projector `unescape_md_brackets` drops one `\` before `[`/`]`. Escaped-*close* `[text\]`
  stays backlog.
- **A backslash *run* in `@md` prose text collapses `ceil(k/2)`.** `double_escape_md` doubles (`k`→`2k`),
  cmark resolves `\\` pairs (`2k`→`k`), parse_Rd collapses again (`k`→`ceil(k/2)`): `\\`→`\`, `\\\\`→`\\`;
  `k==1` is a **no-op** (so the single-escape trap above still holds). Projector-only
  `collapse_md_backslash_runs` (before `unescape_md_brackets`; **skips** runs abutting `[`/`]`).
- **The `@md` `%`-swallow is parity-keyed on the *original* backslash-run length.** `%` is the Rd comment
  char, so roxygen2's md→Rd pass escapes a rendered `%`→`\%`; when the markdown already places a literal
  `\` before the `%`, that escaping `\` collides and the `%` is left **bare** → comments to end of the
  physical line. **Odd** run before `%` (`\%`, `\\\%`): keep `ceil(k/2)` backslashes, drop `%`→EOL.
  **Even** (bare `%`, `\\%`): keep `ceil(k/2)` backslashes + literal `%`. Projector-only
  `md_percent_swallow` (per physical line; runs **before** `collapse_md_backslash_runs` so parity reads the
  raw run). Silent (no roxygen2 warning) → pure render-parity. Curated `md_percent_swallow`.
- **Soft-wrap physical-line boundary (RESOLVED 2026-07-02).** A `%`-swallow (and the non-md
  `strip_rd_comments`) ends at the **physical source line**; the projector had flattened a *soft-wrap*
  break to a **space**, so a `%` on a soft-wrapped line ate its continuation. Fix: a soft-wrap now carries
  a **`SOFT_BREAK` sentinel** (`'\u{c}'`, form feed) — is_posix_space so `norm_ws` collapses it, but not
  `\n` so the link-ref block machinery (`t.contains('\n')` in `collect_user_linkrefs`/`scan_linkref_run`)
  still reads only a paragraph break. `strip_rd_comments`/`md_percent_swallow` split on `physical_lines`
  (both `\n` and `SOFT_BREAK`). **All 5 NEWLINE→text sites** in `project_rd.rs` emit `SOFT_BREAK` now (not
  `" "`). Formatter got the `@md` analog of its non-md `%`-reflow gate: `line_has_md_percent_swallow`
  bails reflow so a `\%`-line is never joined with its continuation. Curated `rd_comment_softwrap` +
  `md_percent_softwrap`.
- **Backslash runs before a letter are LANDED (2026-07-06e), mode-independent** — the parity gate +
  zero-arg carve traps above, plus `resolve_rd_text_escapes` (non-md, below).
- **Brace-less brace-required known macros DROP in projection (2026-07-06f), mode-independent.**
  parse_Rd's "expecting `{`" recovery deletes the `\name` and the text continues (`\emph z` → ` z`,
  section headers like `\title z` too; at EOL the drop crosses the soft break). Projector-only (CST
  stays literal prose — lossless): `braceless_drop_name_end` (project_rd.rs) wired into the
  unpaired-`\` arm of `resolve_rd_text_escapes` (non-md) and the odd-run arm of
  `collapse_md_backslash_runs` (md keeps the paired `k/2` backslashes: `\\\link q` → `\ q`). Drop
  set = `is_rd_braceless_drop_macro` (roxygen.rs) = known ∧ ¬zero-arg ∧ ∉
  `STICKY_BRACELESS_RD_MACROS` — probed exhaustively (R 4.5, every KNOWN name as
  `before \name z after`). The **sticky** names don't drop: parse_Rd's lexer is already in the arg
  mode when the error fires, so recovery leaves **RCODE** (`code, donttest, dontshow, testonly`) or
  **VERB** (`verb, url, samp, env, kbd, option, out, eqn, deqn, href, figure, preformatted,
  dontrun, newcommand, renewcommand`) state and every line to *section end* (crossing paragraph
  breaks) becomes a per-line atom; `\item` instead becomes an `(UNKNOWN "\item")` node mid-text.
  The code/verb sticky swallow (explicit prose tag, single-paragraph plain-text tail) is now projected
  (2026-07-08d trap below); brace-less `\item` too; an **even-run braced macro** too (2026-07-08e:
  `\\emph{x}` → literal `\emph` + `(LIST (TEXT "x"))`, both modes — this was the "needs a
  bare-brace-group model" item, and it fell out **for free** once that model landed, no code change).
  **Escape-cluster remainder (backlog):** the sticky-mode flips (a brace-less `\code`/`\verb` mid-cluster);
  cross-paragraph sticky tails. (In-arg parity/escapes landed 2026-07-07f; md `\{`/`\}` landed 2026-07-07b.)
- **Brace-less `\item` → `(UNKNOWN "\item")` node, projector-only, BOTH modes landed (2026-07-08c).** Of
  `STICKY_BRACELESS_RD_MACROS`, `\item` is the one name whose brace-less misuse is neither a clean drop
  nor a code/verb swallow: out of list context parse_Rd tags it `(UNKNOWN "\item")` and the surrounding
  text continues (`a \item b` → `(TEXT "a") (UNKNOWN "\item") (TEXT "b")`; item can start/end a line,
  splits repeatedly). CST keeps it literal prose (lossless); projector `split_braceless_items`/
  `split_item_text` (project_rd.rs) is a pre-pass in `serialize_prose`, run **after** `group_brace_lists`
  and gated `group=true` (output path only — the `group=false` md `rdComplete` scan reads the raw `\item`
  text, which counts no braces either way). Emits `Inline::BracelessItem`; `serialize_inlines` →
  `(UNKNOWN "\item")` (`encode_text`). Parity-gated (odd `\`-run only via a local backslash-run count,
  `\\item` stays literal `\item`), name-exact (`rd_macro_name_end` == `"item"`; `\itemize`/`\itemx` are a
  different macro), recurses into `BraceGroup`/`MdEmphasis` children. Running **after** grouping means a
  following `{…}` is already a `LIST` (`\item{x}` → `(UNKNOWN "\item") (LIST (TEXT "x"))`, matching
  parse_Rd, which never binds a top-level `\item` to its group). Curated `rd_braceless_item`+
  `md_braceless_item`, fixture `roxygen_braceless_item` (CST literal), unit
  `braceless_item_projects_as_unknown_node`. **Backlog:** a *braced* `\item{x}` in prose that arity's
  lexer CARVES as a real `\item` macro node (→ `(\item (TEXT "x"))` vs roxygen2's `(UNKNOWN "\item")
  (LIST …)`) — a parser carve-suppression (don't carve `\item` outside a list body); contrived.
- **Sticky brace-less RCODE/VERB swallow — explicit prose tag, plain-text tail, BOTH modes landed (2026-07-08d), projector-only.**
  Of `STICKY_BRACELESS_RD_MACROS` minus `item`, a brace-less `\code z`/`\verb z`/… drops parse_Rd's
  lexer into R-code (`code, donttest, dontshow, testonly` → `RCODE`) or verbatim (the rest → `VERB`)
  mode; the `\name` is deleted and everything from there to *section end* becomes **one atom per physical
  source line** (`before \code z here` line-wrapped → `(TEXT "before") (RCODE " z here\n") (RCODE
  "continued\n")`). New parser `sticky_braceless_code_mode(name)->Option<bool>` (roxygen.rs, Some=RCODE)
  is the RCODE/VERB split. Projector-only (CST keeps `\code` literal — lossless): `split_sticky_braceless_swallow`
  (project_rd.rs), a pre-pass at `project_tag_section` entry (explicit prose tags only — excludes `@rawRd`/`@section`;
  the **intro is out of scope**, its swallow crosses roxygen2's generated field braces, and it never reaches
  `project_tag_section`). Finds the first parity-gated (odd `\`-run) brace-less trigger via `find_sticky_trigger`,
  splits the containing `Inline::Text` at the trigger, and emits `Inline::StickyVerbatim{code,lines}` →
  per-line `(RCODE …)`/`(VERB …)` in `serialize_inlines`. `sticky_swallow_lines` splits the tail on `SOFT_BREAK`;
  the trigger line keeps its leading ws verbatim, continuation lines strip **one** `#'`-marker space (non-md) or
  **all** leading ws (md — cmark strips paragraph-continuation indent, engine-probed). **Withhold (leave `\code`
  literal, its prior projection) for anything but a single-paragraph plain-text tail:** a following non-Text inline
  (a real macro/list/emphasis still parses *inside* the swallow, splitting the RCODE — `\code z \emph{x}`, and md
  `*b*` → an `MdEmphasis` node), any `\n` (crosses a paragraph break — arity's model collapses blank-line *counts*,
  unreconstructable), or a raw `{`/`}` (breaks the section's field braces → leaks a stray `(TEXT …)`), `%` (an Rd
  line comment), or `\` (nested carve) in the tail; also a same-line `%` *before* the trigger (a non-md comment
  eats the `\name`). **Formatter (Tenet 1):** a swallowed tail is verbatim-per-line, so reflow — joining a
  soft-wrap or splitting an overlong line — changes the atom count. `line_has_sticky_swallow` (formatter/roxygen.rs,
  mirrors `find_sticky_trigger`, mode-independent, conservative: fires even where the projector withholds) bails
  reflow in both `prose_bails_reflow` and `TagUnit::flush`, keeping physical lines (like the `%`-swallow bail).
  Curated `rd_braceless_sticky`+`md_braceless_sticky`, fixture `roxygen_braceless_sticky` (CST literal), units
  `braceless_sticky_{swallows_tail_per_line,md_strips_continuation_indent,withholds_impure_tail}`, format baseline
  +2 (re-blessed; `@note` continuation stays 2 lines). **Backlog:** cross-paragraph tail (blank-line counts lost);
  `%`/`{`/`}`/`\`/macro-bearing tails; the **intro-paragraph** swallow (crosses generated field braces —
  unmodelable at section granularity); `@rawRd`/`@section` swallow.
- **Bare `{…}` prose groups are Rd `LIST`s, BOTH modes landed (non-md 2026-07-07g, md 2026-07-07h), projector-only.**
  parse_Rd models an *unescaped* brace pair in prose text as a `(LIST …)` node over its parsed contents (a
  macro's own braces live inside its CST node — only *bare* text braces reach here): `a {b c} d` →
  `(TEXT "a") (LIST (TEXT "b c")) (TEXT "d")`. Groups **nest** (`{a {b} c}` → nested `LIST`), **span
  macros/emphasis** (`{k \emph{x} l}`, `{k *x* l}` → `(LIST (TEXT "k") (\emph …) (TEXT "l"))`), cross
  **soft breaks**, and an empty `{}` → `(LIST)`. Projector-only (CST keeps flat TEXT — lossless):
  `group_brace_lists(body, md)` (project_rd.rs), a group-stack pre-pass over the inline run in
  `serialize_prose(body, md, group=true)`, producing an `Inline::BraceGroup(Vec<Inline>)` variant;
  `serialize_inlines`' arm emits `(LIST …)`/`(LIST)`. **Brace parity is mode-independent** (mirrors
  `resolve_rd_text_escapes`): an odd backslash run escapes the brace (`\{`/`\}` literal, no group), an
  even run opens it (`\\{` groups; md source `\\{y}` = cmark's `\`+bare-brace, so `s \\{t}` → `(TEXT
  "s \\") (LIST (TEXT "t"))`). **The `%`-comment trigger is INVERTED by mode** (mirrors
  `md_percent_swallow`): non-md a bare `%` hides braces to the physical line end (an escaped `\%` was
  consumed by the backslash arm); **md a bare/even-preceded `%` stays literal** (roxygen2 escapes it →
  `\%`, does NOT hide — `v % {w}` groups) and only an **odd-preceded `\%`** renders bare + swallows.
  Implemented in `group_brace_lists`: the backslash arm skips escape-consuming a `%` under md; the `%`
  arm looks back at the preceding backslash-run parity (md) vs always-comment (non-md). Text pieces keep
  raw form (escapes/comments resolved later by `process_prose`). **Drop scan reads UNGROUPED atoms under
  md** (`section_rd_complete` → `serialize_prose(body, md, group=false)`): roxygen2 decides the drop on
  `markdown(text)` whose braces are flat; grouping a balanced pair into a `LIST` loses the brace-abutting
  backslash parity `rd_complete` weighs (an even run that opened a group collapses `\\`→`\` once the brace
  leaves the text, and the trailing `\` would spuriously escape the `LIST`'s `{` in the reconstruction).
  **Balanced-only**: an unbalanced run returns unchanged (section drops via `rdComplete` first, reading
  the *raw*/ungrouped body). Curated `rd_brace_group`+`md_brace_group`, fixtures
  `roxygen_{rd,md}_brace_group`, unit `md_bare_brace_groups_project_as_lists`. (The pre-existing
  `%`-swallow trailing-`\` false-drop below is now RESOLVED, 2026-07-07i.)
- **Bare `{…}` groups inside a *prose macro arg* are `LIST`s too, both modes landed (2026-07-08), projector-only.**
  parse_Rd lexes a braced arg with the same bare-group rule as prose (`\emph{a {b} c}` → `(\emph (TEXT "a")
  (LIST (TEXT "b")) (TEXT "c"))`); groups nest, span a nested macro (`\emph{i {j \strong{k} l} m}`), and an
  empty `{}` → `(LIST)`. **Non-md:** `serialize_macro` no longer accumulates a raw `run: String`; it collects
  per-argument `Vec<ArgPiece>` (`Text(raw)` | `Atom(serialized nested macro/VERB)`), and `finalize_macro_arg`
  folds bare groups via `group_arg_pieces` (a stack pass mirroring `group_brace_lists` but on pieces; a nested
  `Atom` is opaque, lands inside the group). Returns `None` when no group / unbalanced → byte-identical
  ungrouped atomization (drop via `rdComplete`). **Verbatim never groups**: `\code` (RCODE, `code==true`) and
  VERB-leaf macros (`\verb`/`\url`/…) skip grouping — braces literal (`\code{v {w} x}` → `(RCODE "v {w} x")`).
  Brace parity = `resolve_rd_arg_escapes` (odd `\`-run escapes `\{`, no group); **`%` is literal in an arg**
  (no comment — an in-arg `%` actually drops the section via mismatched braces, out of scope). **Structural**
  (`\href`/`\item`/`\tabular`) GRP-wraps a multi-atom arg with the group counted as one atom
  (`\href{u}{s {t} u}` → `(GRP (TEXT "s") (LIST (TEXT "t")) (TEXT "u"))`). **Md:** the `is_md_inline_text_macro`
  branch and `serialize_md_structural_macro` now run `group_brace_lists` on the resolved arg inlines before
  `serialize_inlines` (fragile macros route through the non-md loop → `code`-guard covers `\code`). Curated
  `rd_macro_arg_brace_group`+`md_macro_arg_brace_group`, fixture `roxygen_macro_arg_brace_group`, unit
  `macro_arg_bare_groups_project_as_lists`.
- **Bare `{…}` groups inside a *markdown heading title* are `LIST`s too, landed (2026-07-08b), projector-only.**
  Under `@md` a hoisted ATX/setext heading's title runs the same bare-group rule (`# H {a b}` →
  `(\section (GRP (TEXT "H") (LIST (TEXT "a b"))) …)`); groups nest and span emphasis
  (`# H {k *x* l}` → `(GRP (TEXT "H") (LIST (TEXT "k") (\emph (TEXT "x")) (TEXT "l")))`). One-line fix:
  `render_heading_frame` (project_rd.rs) now folds the title via `group_brace_lists(&f.title, md)` before
  `serialize_inlines` (the frame's `title` is `resolve_macro_arg_inlines(title_text)`; grouping mirrors the
  prose + macro-arg entries — headings only exist under `@md`, so `md` is always true here). CST unchanged
  (bare braces stay flat text in `ROXYGEN_MD_HEADING` — grouping is projector-only). Curated
  `md_heading_brace_group`, fixture `roxygen_md_heading_brace_group`, unit
  `heading_title_bare_groups_project_as_lists`. **Backlog:** a bare group inside a heading *body* is already
  covered (it's ordinary prose); the group pass now runs at prose-section + macro-arg + heading-title entries.
- **`%`-swallow trailing-`\` false-drop RESOLVED (2026-07-07i), projector-only.** `@details y \% {z} end.`
  (odd-run `\%` swallows `{z} end.` → renders `y \`) — arity **used to drop** the md section (`rd_complete`
  saw the output atom `y \` ending mid-escape → `RdEscape` → incomplete), but roxygen2 KEEPS it: it scans
  `markdown(text)` = `y \\% {z} end.` (the `\\` pairs, the bare `%` comments to EOL, braces balanced →
  complete). Root cause: the md drop scan reconstructed from the **output** atoms, which had run
  `md_percent_swallow` (keeps `ceil(k/2)` backslashes — parse_Rd's rendered text — and drops the comment
  tail), leaving an odd trailing `\` at the section end. **Fix:** `section_rd_complete`'s md arm now
  pre-strips each odd-run `\%` comment region from the body's top-level `TEXT` leaves
  (`strip_scan_percent_comments`/`strip_scan_percent_comment`/`scan_line_before_odd_percent`) **before**
  serializing the scan atoms — dropping the backslash run *and* the `%` *and* the line tail, so the scan
  matches `markdown(text)`'s even-run+comment (no dangling escape). Only an **odd** run is stripped; an
  even-run `%` (a literal percent roxygen2 escapes to `\%`) is left for render-time re-escaping. Bug bit
  only at a **physical line end** (a soft-wrap continuation resolved the escape via the following `\n`) —
  which is why `md_brace_group` (2026-07-07h) sidestepped it. Curated `md_percent_trailing`, unit
  `trailing_percent_swallow_does_not_false_drop`. **Backlog:** an odd-run `\%` inside a *macro arg*
  (only top-level prose is stripped; a balanced-brace arg contributes a balanced pair regardless).
- **HTML entities decode under `@md` only, projector-only.** cmark resolves every semicolon-terminated
  HTML5 named entity (`&amp;`/`&copy;`/`&hellip;`) + numeric ref (`&#65;`/`&#x41;`); U+0000/surrogate/
  out-of-range → U+FFFD; missing `;` or unknown name stays literal; single-pass (`&amp;amp;`→`&amp;`);
  **off in code spans** (separate verbatim leaves — nothing to do). Full 2125-entry table in generated
  `src/roxygen/entities.rs` (Python `html.entities.html5`, `;`-terminated, escaped non-ASCII, binary
  search); `decode_html_entities`/`decode_entity` (link-dest + prose). **Wired as the *last* transform in
  `prose_text_atom`'s `md` branch** (after `%`-swallow/backslash/bracket — an entity-produced `[`/`%`/`\`
  is inert text). CST stays lossless (raw `&amp;` prose); non-md keeps entities literal. Curated
  `md_entities`. Regenerate the table if the WHATWG list changes.
- **Images** (`scan_md_image`, **inline + shortcut + reference**): `mdxml_image` drops alt →
  `\figure{url}{title}`, wrapped per extension (`image_format`: svg→html, pdf→pdf, raster/unknown→bare).
  `\figure` = 2-arg verbatim macro. **Three forms (2026-07-10d):** inline `![alt](dest)` needs a *valid*
  `inline_dest_span` (else it falls back to the shortcut `![alt]`, leaving the `(…)` literal —
  `![z](a\ b)` → `\figure{R:z}` + `(TEXT "(a\\ b)")`); **shortcut** `![alt]` and **reference**
  `![alt][ref]` (both alt+ref bracket-free non-empty, `is_shortcut_content`) resolve against roxygen2's
  synthesized `[label]: R:URLencode(label)` def → `\figure{R:label}` (shortcut keys on alt, reference on
  ref). A collapsed `![alt][]` / empty `![]` / `![alt]{` are **not** carved (literal). Projector
  `resolve_md_image` matches on the char after `![alt]`: `(`→inline, `[`→`R:url_encode(ref)`,
  end→`R:url_encode(alt)` (`synthesized_image_dest`). Images are `@md`-only (lexer `b'!' if md`).
  **Backlog:** a **user-def** `[ref]: url` override (`![alt][ref]` uses `R:ref`, not the url — needs the
  refmap threaded into image resolution, à la `apply_user_linkrefs`); a **URL-unsafe label**
  (`![see this]`→`R:see%20this`, `![a\b]`→`R:a%5Cb`) where the `%` comments out the `\figure` `}` and
  roxygen2 **drops the whole section** — arity's fragile-macro neutralizer keeps it (a `%`-in-`\figure`
  drop gap); **reference images** in poisoning skeletons; cross-line images.
- **Fenced code blocks** (`scan_md_fence`, carved whole *before* the list-marker carve; bails if a
  backtick follows). `emit_md_code_block` pairs opener↔closer into `ROXYGEN_MD_CODE_BLOCK`. Projector
  emits 3 atoms: `\if{html}{\out{<div…>}}` / `\preformatted{<code+\n>}` / `\if{html}{\out{</div>}}`.
  Out of scope: ` ```{r} ` knitr-eval blocks. **Fence-indent strip is CommonMark, not just marker
  strip** (`md_code_block_parts`, 2026-07-08h): a fence indented past `#' ` (top-level 1–3 cols, or to a
  list item's content column) removes its own indentation from the info string *and* up to that many
  leading cols of each body line — surviving leading spaces = `max(0, body_col − fence_col)`, computable
  from the node text alone (the item content column cancels). Was latently wrong for any indented fence.
- **A fenced code block at a list item's content column folds INTO the item (2026-07-08h).** In
  `emit_md_list_level_inner`'s item-body loop, after the prose/blank/nested-list arms, a
  `next_content_line` at indent `>= content_indent` that `is_md_code_block_start` is emitted **inside** the
  `ROXYGEN_MD_LIST_ITEM` via `emit_md_code_block` (blanks threaded as trivia). Folds **with or without** an
  intervening blank (a fence interrupts the item paragraph; a blank only loosens) and into an **empty** item
  (no `item_has_content` gate — unlike the prose arms); a **below**-content-column fence ends the list (a
  section sibling). Projector: `push_inline` maps a `ROXYGEN_MD_CODE_BLOCK` list-item child →
  `Inline::MdCodeBlock` (serialize_inlines already handled it). **Formatter unchanged** — the whole
  `ROXYGEN_MD_LIST` is atomic passthrough (`emit_md_list`), and `normalize_list_marker_text` is byte-idempotent
  on the fence/code/blank lines. Curated `md_list_item_code_block`, fixture, 3 units, baseline +1.
- **An indented code block at a list item's content column folds INTO the item too (2026-07-09).** A
  **blank-separated** line indented `content_indent + 4` (four columns past the item's content column) is an
  indented code block inside the item — but the **blank is required** (indented code can't interrupt a
  paragraph, so a *no-blank* over-indented line is a lazy continuation → prose `a code`), and an **empty** item
  does **not** fold it (ends the list — engine-probed → gate on `item_has_content`, unlike the fence). Parser: a
  new arm in `emit_md_list_level_inner`'s item-body loop **before** the loose-prose arm (which else claims the
  over-indented line as prose), using `next_content_line_across_blanks` (like `next_content_line` but requires a
  blank) + `is_indent_code_line_min(m, content_indent+4)` + `emit_md_indented_code_min` (threads the
  content-relative threshold into the continuation gather via `finish_md_indented_code`'s new `min_ws` param;
  the old top-level callers pass 5). **Unlike the fence, the content column does NOT cancel** — an indented code
  block has no marker line at the content column to measure against — so the projector strips `content_col + 4`,
  not 4: `serialize_md_indented_code` adds `md_indented_code_extra_strip(node)` (the parent item's content
  column = `marker_col + marker_width + content_leading` in markdown coords; `marker_col` = ws-before-marker − 1
  via `prev_token`, `content_leading` clamped 1..=4) to its `.take(4)`, so surplus indentation survives. `push_inline`
  gained an `MdIndentedCode` arm (was missing — an in-item node would else hit the text fallback). **Formatter
  unchanged** (atomic `ROXYGEN_MD_LIST` passthrough, byte-identical). Curated `md_list_item_indented_code`,
  fixture, 3 units, baseline +1.
- **A GFM table at a list item's content column folds INTO the item too (2026-07-09b).** A table header
  line (a `is_md_table_start` two-line look-ahead) indented `>= content_indent` folds in as a
  `ROXYGEN_MD_TABLE` child → `\tabular` between the two `\item`s — **with or without** an intervening blank
  (a table interrupts the item's paragraph at the content column; a blank only loosens). An **unindented**
  header (below the content column) stays a **lazy paragraph continuation** flattened as prose
  (`a | x | y | ...`) — `is_md_item_lazy_continuation` already folds table headers there. Both engine-probed;
  no `item_has_content` gate (like the fence). Parser: a new arm at the **top** of the item-body loop (before
  the lazy-continuation and blank-prose arms, either of which would else claim a content-column header as
  text), mirroring the fence: `next_content_line` (crosses blanks, none required) + `>= content_indent` +
  `is_md_table_start` → `emit_md_table`. `finish_md_table`'s `is_table_row_line` excludes list starts, so a
  sibling `- b` ends the table cleanly. Projector: a `push_inline` `ROXYGEN_MD_TABLE` arm (was missing) →
  `Inline::MdTable`; `serialize_md_table`'s per-line `strip_marker` already strips the content-column
  indentation (no extra-strip needed — cells split on `|` and trim). **Formatter unchanged** (atomic
  `ROXYGEN_MD_LIST` passthrough, byte-identical + idempotent). Curated `md_list_item_table`, fixture
  `roxygen_md_list_item_table`, 3 units (`table_at_content_column_folds_into_item`,
  `table_without_blank_folds_into_item`, `unindented_table_is_lazy_continuation`), baseline +1.
- **A block Rd macro after a list item folds INTO the item too, but by the PARAGRAPH-CONTINUATION rule, not the
  block-interrupt gate (2026-07-09c).** A `\itemize{…}`/`\describe{…}`/`\tabular{…}{…}` following a list item
  folds in as a `ROXYGEN_RD_MACRO` child → nested `\itemize`/… inside the `\item`, `- b` a sibling. **A raw
  `\name{…}` is NOT a markdown block** — to cmark the backslash-word is literal text, so it folds as the item's
  paragraph continuation and parse_Rd reads the macro only afterward. So the gate is CommonMark paragraph
  continuation, **not** the fence/table `>= content_indent` block-interrupt gate: **no blank → folds at *any*
  indent** (lazy, even below the content column), **blank → folds only at content column** (loose paragraph). The
  **only** non-fold is *blank + below-column* → a section-level macro that ends the list, splitting into three
  `\itemize`s (arity's prior behavior — no change). Parser: **two arms at the top** of the item-body loop (before
  every other — a block-macro line matches no other predicate, `is_md_item_lazy_continuation` even excludes it,
  but its multi-line opener must be consumed whole by `emit_block_macro`). Arm 1 (`following_line_marker`, no
  blank) gated `item_has_content || indent >= content_indent`; arm 2 (`next_content_line_across_blanks`, blank)
  gated `(content_indent..content_indent+4).contains(indent)` (cedes a 4+-indent macro to the indented-code arm).
  **Projector + formatter unchanged** — `push_inline` already maps `ROXYGEN_RD_MACRO` → `Inline::Macro`, the list
  is atomic passthrough. Curated `md_list_item_block_macro`, fixture `roxygen_md_list_item_block_macro`, 3 units
  (`block_macro_at_content_column_folds_into_item`, `block_macro_below_content_column_folds_lazily`,
  `blank_separated_below_column_block_macro_ends_list`), baseline +1.
  **In-item fold series COMPLETE** — fence + indented-code + table + block-macro land. A **heading / block-quote /
  thematic-break / HTML block** at the content column still ends the list, but all are roxygen2-**unsupported**
  in-item (probed 2026-07-09: block-quote "block quotes are not currently supported"; thematic-break internal
  "unknown xml node thematic_break"; ATX heading "mismatched braces" / hoists to `\section`) → out of scope.
- **Indented code blocks** (`ROXYGEN_MD_INDENTED_CODE` node): a line indented **>= 5 space columns** past
  the marker (roxygen2 strips `#'` + one space, cmark needs 4). **No mode-carrying leaf** — the content
  lexes as ordinary tokens, so the block builder re-derives `@md` via `block_md` (reusing
  `roxygen_md_directive`), the one sanctioned exception to "never re-derive `@md` in the block builder"
  (there is no leaf). `is_indent_code_line` reads the ordinary `Whitespace` length (do NOT add a
  whitespace-variant token — it fights every `== Whitespace` loop). Checked in the group main loop
  **before `classify_line`** (a column-5 `@param`/`\item` is code, not a tag/macro), gated `md && !para_open`
  (interrupt rule). `emit_md_indented_code` gathers code lines + interior blanks (trailing blanks
  dropped). Projector `serialize_md_indented_code`: strip marker + up to 5 columns/line, one VERB per
  line, bare `sourceCode` class — same 3-atom shape as fenced. Formatter uses `normalize_list_marker_text`
  (indent is semantic; trimming destroys the block). Tabs → prose (backlog). Curated `md_indented_code`.
- **HTML blocks** (`ROXYGEN_MD_HTML_BLOCK` node): `scan_md_html_block` carves a line-start opener
  (before the fence carve) for **conditions 1–6**. Each carves the **whole line** as one leaf; they differ
  only in the **terminator**, re-derived from the opener text by `html_block_closers` (build.rs) — the leaf
  already implies `@md`; re-deriving the *condition* is not re-deriving the mode. **Terminator-based
  (cond 1–5):** gather until a line **containing** a closer (`html_line_contains_closer`, case-insensitive,
  **inclusive**) — **through blank lines**; a new tag (section boundary) or non-roxygen/EOF also ends it; the
  opener line can self-close. Closer sets: cond 1 `HTML_VERBATIM_TAGS` (`pre`/`script`/`style`/`textarea`,
  opening form only — a close tag never starts cond 1, `/>` is cond 7 not 1) → `</pre>` etc. (need not match
  the opener); **cond 2** `<!--`→`-->`; **cond 3** `<?`→`?>`; **cond 4** `<!`+**uppercase**-letter→`>` (the
  engine keeps the pre-0.31 uppercase-only rule — `<!doctype` stays prose); **cond 5**
  `<![CDATA[`→`]]>` (the `CDATA` keyword is **case-insensitive**, `is_html_cdata_opener` — cmark spells it
  as a re2c case-insensitive literal; the four forms are disjoint — `<!--`/`<![` fail the cond-4 letter
  test). **Cond 6**
  (a `BLOCK_TAGS` tag, open or close form; `html_block_closers` returns `None`): gather opener + following
  Prose lines until **blank**/tag/non-roxygen. **Cond 7** (a complete standalone tag — open/closing/
  self-closing, only ws to EOL): **not lexer-carved** — the builder recognizes the line structurally
  (`is_md_html_block7_line`: content = exactly one inline `RoxygenMdHtml` *tag* leaf + trailing ws; the
  leaf kind is the mode signal), then the same `emit_md_html_block` blank-terminated path as cond 6. **No
  tag-name exclusion** in the closing/self-closing forms (`</pre>` and `<pre/>` both open, engine-probed) —
  so `is_html_verbatim_opener` must NOT treat `/` as a name boundary, else `<pre/>` runs as a
  closer-terminated cond-1 block to section end. **Cond 7 cannot interrupt a paragraph, and the engine's
  gate is positional** (cmark blocks it only when the deepest *matched* container is an open paragraph): a
  direct prose continuation folds (`@details value`⏎`<span>` stays inline), but the same line where a
  container match already failed opens a block — after a `>` quote line (NOT a lazy continuation), after a
  table row (ends the table), after a list item (ends the list). Wired: `!para_open` gate in the group
  loop; exclusions in `emit_md_block_quote`'s lazy gather and `is_table_row_line` (the list gather ends on
  any non-list line already). Projector `serialize_md_html_block` (unchanged, walks
  `node.text()`) → one `(\if (TEXT "html") (\out (VERB "\n") <verb-per-line>))` for all. **Formatter**
  `emit_md_html_block` uses `normalize_list_marker_text` (preserves content indentation — roxygen2 renders
  each line verbatim into `\out`, so a trimmed indent is a fixed-point violation).
  Curated `md_html_verbatim`/`_oneline`/`md_html_conditions`/`md_html_cond7`/`md_html_cond7_edges`.
  **From-value form (landed 2026-07-05b, generalized 2026-07-05c):** a prose tag's same-line value IS
  its md-doc start, so a value-leading block start opens that block — HTML block (all conditions),
  **fence, ATX heading, list, GFM table, indented code** (`@details <span>` / `` @details ```r `` /
  `@details # T` / `@details - item` / `@details | a | b |` / `@details      x`). roxygen2 strips only
  the **single separator space** after the tag head — further indent renders (`@details   <span>` →
  `(VERB "  <span>\n")`), and >= 5 columns past it is an **indented code block** (probes p1–p3:
  5 cols → `"x\n"`, 6 → `" x\n"`, 4 → prose; merges with following >= 5-col `#'` lines). Lexer:
  `lex_roxygen_tag` carves fence/HTML-block/ATX/list-marker at the value
  (`md && tag_folds_prose_continuation && ws_len <= 4` — the gate keeps a deeper-indented value's
  content lexing as ordinary tokens for the code block); cond 7 is builder-structural
  (`is_md_html_block7_at`); the table needs **no value carve** (header = generic prose; the gate
  `is_md_table_value` reads the *next* line's delim leaf + cell counts); indented code has no leaf at
  all (`is_md_indented_code_value` reads the head→value ws run, `md` threaded into `emit_tag_line`).
  Grouper: `emit_tag_line` dispatch, all **before the setext branch** — order: indented code (pre-empts
  everything), HTML block, fence, list, **ATX before table** (a heading line is never a table header),
  table, setext (a fence/HTML block swallows a following `===`; a list value + `---` is item +
  thematic break, not setext — engine-probed e3/e4). Tag closes **empty** via `close_tag_at_value`;
  value + head→value ws become the sibling block node (shared `finish_md_*` gathers; list reuses
  `emit_md_list_level_inner` with a marker-less first item — the head→value ws has the same one-based
  indent semantics as marker→content ws, so nesting/sibling columns work unchanged, `@details - a` ⏎
  `#'   - nested` nests). The **value position is always fresh**: any list marker opens a list (even an
  empty `-` → `(\itemize (\item))`, ordered `1.` → `\enumerate`). Marker-less first line: projector +
  formatter strip exactly **one** leading ws char (`strip_marker` would eat the semantic indent — or a
  heading's own `#` run / a header cell's `#`; `parse_md_heading`/`serialize_md_indented_code`/
  `serialize_md_table` have explicit from-value branches; fence/list/quote arms need none); formatter
  normalizes to the next-line form (`#' @details`⏎`#' <span>`, shared `push_value_opener_line`,
  `keep_indent` for indented-code/list/HTML), which projects identically — idempotent. Curated
  `md_html_block_value`/`_edges`, `md_{atx_heading,fence,indented_code,table,list}_value`,
  `md_block_value_edges`.
  **From-value quote + break (landed 2026-07-05d):** the value carve also covers a **block quote**
  (`@details > q` → sibling `ROXYGEN_MD_BLOCK_QUOTE`, flattened; following `>` lines + lazy
  continuations gather via shared `finish_md_block_quote`) and a **thematic break** (`@details ***`/
  `---`/`- - -` → sibling `ROXYGEN_MD_THEMATIC_BREAK`, renders empty). `is_thematic_break` carves
  directly at the value — the value position is fresh, so a contiguous `---` is never a setext
  underline there (`===` stays prose, engine-probed q9/q10); the carve sits **before the list-marker
  carve** (spaced `- - -`/`* * *` starts with a valid bullet). An all-break tag renders an **empty
  section atom** (`@seealso ---` → `(\seealso)`), which the projector already emits. No projector
  change (marker-less first line: `strip_marker` is a no-op, `block_quote_flat_text` handles it;
  the break node is dropped wholesale). Formatter: both emitters got the `is_from_value` →
  `push_value_opener_line(…, false)` branch.
- **Inline raw HTML** (`scan_md_html_inline`, chained after autolink at `b'<'`): every form →
  `(\if (TEXT "html") (\out (VERB …)))`, all **line-scoped**. Dispatch on `bytes[i+1]`: `!` →
  comment/CDATA/declaration (`scan_md_html_inline_bang`), `?` → PI, else tag. **Probe the engine, not
  a spec version** — commonmark 2.0.0 mixes rules: **comments** are the relaxed 0.31 form (`<!-->`/
  `<!--->` empty forms; else the closer is the **first `-->` not preceded by a text `-`** — interior
  `--`/`>`/`->`/dash-blocked `-->` are all text, so `<!-- x --->` is literal but `<!-- x ---> b -->`
  closes late; the opener's own dashes don't count, `<!---->` is fine), while **declarations** keep the
  old rule (**uppercase** letters + **required** whitespace + `[^>]*>` — `<!doctype …>`/`<!DOCTYPE>`
  literal, `<!D x>` fine). **PI** `<?`…first `?>` (empty `<??>` fine). **CDATA** `<![CDATA[` (keyword
  case-insensitive) + `("]" [^\]] | "]]" [^>] | [^\]])*` + `]]>` — `]]]>` never closes (the `]]` pair
  eats the third `]`). Curated `md_html_inline_forms`.
- **Multi-line inline HTML resolves in the inline pass** (`resolve_multiline_spans`, inline.rs). Every
  form crosses soft breaks (comment/PI/CDATA/declaration text, tag ws, quoted attr values — even a
  newline as a declaration's *required* ws; `skip_html_ws` accepts `\n`, engine-probed). The pass scans
  each run's **logical text** — content tokens verbatim, NEWLINE→`\n`, MARKER/WS→ε (roxygen2 strips
  `#' `, cmark strips remaining continuation indent: `#'    y -->` renders `y -->`),
  `RoxygenRdMacro`→`"x-"` (the escape-placeholder shape: leading alnum + trailing `-`, which
  dash-blocks an abutting `-->` exactly like roxygen2's real `-<i>-` placeholder) — with the *same*
  `scan_md_html_inline`, starting only at a `<` in a `RoxygenText` token. A match becomes a
  `ROXYGEN_MD_HTML` **node** (leaf+node coexistence, the `ROXYGEN_MD_LINK` precedent): covered tokens
  (inter-line trivia included) move inside; a partially covered token splits into `ROXYGEN_TEXT`
  `Event::Leaf` pieces tiling it exactly. The node enters the emphasis arena as an opaque
  `RunItem::Html` (run = `Vec<RunItem>` now: `Tok`/`Text`/`Html`; edge chars `<`/`>`), so an emphasis
  span **wraps** it (`*a <!-- x`⏎`y --> b*` → `\emph` around the `\if`). An unmatched opener stays
  untouched tokens — later emphasis still resolves, as cmark's re-scan would. **Gating:** `md` threaded
  from `block_md` at the group.rs call site (the `<`-in-prose candidate has no mode-carrying leaf — the
  sanctioned-exception shape) + linear rawRd-scope suppression in the event walk (tag-name token seen
  in-run; reset at `Start(ROXYGEN_SECTION)`). **Projector:** `push_inline` node arm →
  `md_html_node_text` (MARKER/WS→ε, NEWLINE→real `\n`, else raw — a covered macro/leaf renders its raw
  source, roxygen2 restores placeholders) → `html_inline_atom` emits **one VERB per line**
  (`split_inclusive('\n')`, matching parse_Rd: `(VERB "<!-- x\n") (VERB "y -->")`). **Formatter:** the
  cross-line node descends in `collect_logical_elements` (physical lines re-form) and any
  paragraph/tag/section touching one **bails reflow** (`line_has_cross_line_md_html`: element parent ==
  the node, or a folded `ROXYGEN_TAG` element carrying a cross-line MD_HTML descendant — the from-value
  case) — the span's `\n` is verbatim `\out` content, so joining changes rendered Rd. **Backlog:** a
  split piece cut from an *opaque* leaf may hide markdown cmark would re-scan (contrived); non-fragile
  macro raw text carrying a closer char (`\emph{>}`) — the all-macros-placeholder simplification shared
  with `edge_char`; the
  single-line lexer's raw-vs-placeholder divergences (`\code{}-->` closes for the lexer, not roxygen2).
  Curated `md_html_inline_multiline`/`_edges`/`_value`.
- **Multi-line code spans resolve in the same pass, one leftmost-first scan with HTML.**
  `resolve_multiline_spans` scans the run's logical text for `<` *and* `` ` `` candidates in one
  left-to-right pass (cmark's equal-precedence scan: the earlier successful match consumes — a code
  opener eats a would-be `<!--`). A backtick opener is eligible in plain prose **or inside a carved
  `RoxygenMdCode` leaf**: an unterminated prose opener on line 1 must re-split a later line's
  line-scoped carve exactly as cmark's whole-paragraph scan does (``a `open`` ⏎ ``b` and `closed` ``
  → `\verb{open b}` + `\code{closed}`), and a split leaf's leftover backticks are re-scanned. A match
  that coincides **exactly** with one carved leaf is *skipped* (the leaf already models it) — so an
  all-single-line paragraph rebuilds byte-identically, zero churn. An unmatched opener run is literal;
  the scan advances past the whole run (cmark treats a backtick string as a unit). The match becomes a
  `ROXYGEN_MD_CODE` **node** (leaf+node coexistence; a covered carved leaf rides *inside* whole).
  Flanking edges now come from the span's own text ends (backticks; `<`/`>` for HTML — same as the old
  hardcode). **Projector:** node arm = `md_html_node_text` (marker/ws→ε, NEWLINE→`\n`) →
  `strip_code_span` (already does the CommonMark newline→space + single-space trim); everything
  downstream consumes the same `Inline::MdCode`. Continuation-line indent inside a span is
  block-stripped before cmark's inline pass (engine-probed: `#'    span` renders `code span`), so
  marker normalization is render-safe. **Formatter:** descends the cross-line node
  (`is_cross_line_inline`) and bails reflow (`line_has_cross_line_verbatim_span`, shared with HTML) —
  a rejoin *would* preserve Rd (soft break renders as a space), but re-wrapping could land a 3+
  backtick run at a line start where it reparses as a **fence opener**; conservative bail, upgrade =
  backlog. **Backlog:** a cross-line code span as a *link display* (the arena handles it; the
  projector's display checks see `Inline::MdCode` — untested edge). Curated
  `md_code_multiline`/`_value`.
- **Reflow must never move an HTML-block opener to a line start** (`is_unsafe_line_start` →
  `starts_md_html_block`, the lexer's block scanner as a `pub(crate)` predicate). Blocks 1–6
  **interrupt a paragraph**, so an *inline* comment/PI/CDATA/declaration/block-tag atom (or literal
  prose that merely looks like an opener — an unterminated `<!--`, a `<div>`; `<span>` is safe) that
  migrates to a wrapped-line start reparses as a block and changes the rendered Rd. The wrap loop glues
  such a chunk to its predecessor, accepting overflow. The hazard predates the inline recognizers (as
  plain prose the words could migrate too); formatter fixture `roxygen_md_html_inline_forms` pins it.
  **Cond 7 must NOT join that guard** (it can't interrupt, so a mid-paragraph wrapped-line start is
  safe); its hazard is a complete standalone tag **alone on a line that reparses at a fresh position**:
  the paragraph's *first* line (`wrap_chunks` glues the next chunk when `lines.is_empty()` and the line
  so far is one complete tag) and the first continuation line under a bare form-2 `@tag` header
  (`wrap_chunks_hanging`'s not-fit-beside-header path keeps a standalone-tag chunk beside the header).
  Predicate `is_md_standalone_html_tag` (lex.rs). Residual corner (unguarded, contrived): a sole
  overlong standalone-tag value forced to form-2 emits the tag alone with no break decision to guard.
  Fixture `roxygen_md_html_cond7`.
- **Block quotes** (`ROXYGEN_MD_BLOCK_QUOTE` node): `is_block_quote_marker` carves a line-start `>` leaf
  (`RoxygenMdBlockQuote`→`ROXYGEN_TEXT`); `emit_md_block_quote` gathers **consecutive** `>` lines (no lazy
  continuation). roxygen2 **has no block-quote support** — `mdxml_unsupported` warns then renders
  `escape_comment(xml_text)`: the **flattened plain text**, `>` + inner markdown dropped, lines glued with
  **no separator**. Projector `block_quote_flat_text` strips each `>`, flattens per line via
  `inline_plain_text` with **SOFT_BREAK removed** (glue, not a space), concatenates → **un-normalized**
  raw text. **Glue onto adjacent prose (RESOLVED 2026-07-02l):** roxygen2 emits no `\n\n` around a quote, so
  its text runs straight onto the neighbor with no separator — `before`+`> q`→`beforeq`, even across a blank
  line, and `> q1`⏎blank⏎`> q2`→`q1q2`; a following *paragraph* keeps its own leading space (`beforeq after`).
  Modeled by pushing the quote as a **`RunSeg::Final`** (pre-flattened) segment into `serialize_inlines`'
  segmented `run` (now `Vec<RunSeg>` = `Raw`(md-pipeline-pending) | `Final`(verbatim); `flush_run` processes
  each `Raw` via `process_prose` **without** norm_ws, concatenates, norm_ws's **once**). `trim_trailing_run_ws`
  drops the preceding node's trailing break first (cmark strips a paragraph's trailing ws before the quote
  appends). Separators suppressed before a quote in **both** join sites: `section_body_parts` (same-part, the
  `" "`) and `project_block`'s part loop (cross-part, the `\n`). Formatter = marker-normalized passthrough.
  **Indent is `#'` marker-ws trivia** → a 4+-space `>` over-recognizes vs roxygen2's indented code block
  (shared fence/heading gap).
  **Lazy continuation (RESOLVED 2026-07-02m):** CommonMark folds a non-`>` **paragraph-continuation** line
  (no intervening blank) into the quote's open paragraph, so it flattens **into** the quote with no separator
  (`> quoted line one`⏎`lazy continuation`→`quoted line onelazy continuation`). Parser-only: `emit_md_block_quote`'s
  gather loop now continues on a line that is a `>` block-quote start **or** an `is_foldable_continuation` (a plain
  prose line opening no new block — same guard the same-line-tag fold uses); a blank/tag/new-block still ends it.
  Projector/formatter unchanged (`block_quote_flat_text` strips a missing `>` to a no-op; the folded line rides in
  the node text). **Backlog:** a lazy line after a non-paragraph quote body (`> ---`⏎`x` — arity over-folds since
  it flattens the body as prose; contrived); inner Rd macros; the diagnostic side-channel.
- **GFM tables** (`ROXYGEN_MD_TABLE` node) are the **only** table kind (cmark-gfm `table` ext; pandoc
  simple/grid tables stay prose). Recognition is **two-line**, so the `@md` signal is a mode-gated leaf on
  the **delimiter** row (the header is generic prose — no header leaf): `RoxygenMdTableDelim`
  (`is_table_delim_row`: ≥1 unescaped `|` so bare `---` stays setext; cells `:?-+:?`) → **`ROXYGEN_TEXT`**
  in `syntax_kind_for` so an *unmatched* delim row is literal prose for free. Gate `is_md_table_start`
  (two-line look-ahead: next line is the delim leaf **and** header cell-count == delim cell-count).
  Cell counting/splitting (`split_table_row_cells`, `parser/roxygen.rs`) honors `\|` but **not** code
  spans — a code-span pipe breaks the count (matches cmark-gfm). `emit_md_table` gathers header+delim+
  greedy body rows (a pipeless line is a single-cell row; stop at blank/tag/new-block/EOF) verbatim
  (like the HTML block). Projector `serialize_md_table` reparses `node.text()`: delim→align
  (`:--`→l/`:-:`→c/`--:`→r/none→l), each cell an **independent** `resolve_macro_arg_inlines` run
  (emphasis never crosses `|`), `\|`→`|` per-cell, ragged rows pad(empty cell, no atom)/truncate to
  ncol; header+body fill one `GRP` with `\tab`/`\cr`. Formatter = atomic passthrough (Tenet 1). CST is
  **flat verbatim** (no row/cell nodes) — the right shape (HTML-block precedent; structure would only
  help column alignment, which the formatter deliberately doesn't do). Curated `md_table`/`_cells`/`_prose`.
- **List markers** (`scan_md_list_marker`): punctuation only (trailing space stays in text).
  `is_md_list_start` applies the CommonMark interrupt rule (mid-paragraph): a bullet interrupts
  **unless the item is empty** (`md_list_item_is_empty`, cm-369); an ordered list interrupts only when
  start == 1. A fresh-position empty bullet still opens a list. Lexer always carves; emptiness is a
  block-level decision.
- **Markdown nested lists are indentation-driven** (`emit_md_list` recurses: a following list line
  indented ≥ an item's content column opens a child `ROXYGEN_MD_LIST` inside that item; a line at the
  marker column is a sibling; shallower ends it). Content indentation is **semantic** → the formatter
  preserves it (`normalize_list_marker_text`). Projector: `push_inline` maps a nested list node →
  `Inline::MdList`; `md_list_is_ordered` reads **direct-child** item markers only.
- **List-item lazy continuation folds into the item, keyed on an item-specific predicate.** A
  plain line after an item (any indent — unindented "lazy" or past the content column) is paragraph
  continuation text and folds **into** the `ROXYGEN_MD_LIST_ITEM` (`emit_md_list_level_inner`'s fold
  loop, **before** the nested-list gather; a post-nested lazy line belongs to the innermost item,
  which the recursion folds itself). Guard `is_md_item_lazy_continuation` (build.rs) =
  `is_foldable_continuation` with three engine-probed differences: **any marker line is the next
  item** (even an empty `-`, which mid-paragraph could not *start* a list — sibling, not text); a
  **setext underline folds** (`- a` ⏎ `===` → `a ===`; the underline can't apply across the container
  boundary — `---` stays excluded, it's a thematic break here and interrupts); a **table header
  folds** (GFM tables can't interrupt a paragraph). Cond-7 HTML stays excluded (positional gate: it
  opens a block after an item). An **empty item never folds** (`item_has_content` gate — an item
  starting blank needs indented content). Projector/formatter unchanged (`md_list_item_inlines`
  already drops markers + NEWLINE→SOFT_BREAK; the list emitter is per-line passthrough). **Backlog:**
  an Rd **block macro** after an item folds into it per roxygen2 (`\itemize{…}` lands *inside*
  the `\item` via placeholder-escaping — arity ends the list, kept: needs block-macro-in-item
  machinery); the block-*quote* lazy gather still excludes setext (`> a` ⏎ `===` likely folds per
  cm-64 — unprobed).
- **Blank lines don't end a list — they only make it *loose*, which Rd rendering ignores.** Both
  gather sites in `emit_md_list_level_inner` cross blank roxygen lines via
  `next_list_line_across_blanks` (build.rs; disjoint from `next_list_line`, so blanks are consumed
  as trivia only when a list line actually follows — a trailing blank stays with the section): the
  **nested** gather takes any deeper-than-content-column marker after blanks; the **sibling** gather
  takes a same-indent marker only when `md_list_marker_type` matches (the bullet char itself, or the
  ordered delimiter `.`/`)` — start numbers are irrelevant, `1.`…`5.` is one list; `-`…`*` and
  `1.`…`2)` split, engine-probed). **The marker-type check now gates BOTH the no-blank and
  blank-separated sibling gathers (landed 2026-07-08f)** — hoisted out of the `else` branch in
  `emit_md_list_level_inner` so a type change ends the list whether or not a blank intervenes
  (`- a` ⏎ `2. b` → `\itemize`+`\enumerate`, `- a` ⏎ `* b` → two `\itemize`, `1.` ⏎ `2)` → two
  `\enumerate`); the caller's block loop re-classifies the sibling line and starts the fresh list.
  Projector + formatter unchanged (the item filter
  skips blank trivia; the list emitter is per-line passthrough — a split changes *structure* only, so
  the formatted bytes are byte-identical; format baseline +1 for the new curated case, no existing
  case re-blessed).
- **Blank + content-column prose folds into the item (LANDED 2026-07-08g).** A blank line closes an
  item's paragraph, but the item continues: a following prose line indented to (or past) the item's
  **content column** opens a new paragraph *inside the same item* (a loose item), which Rd rendering
  flattens into the item text (`- a` ⏎ blank ⏎ `  more` → item text `a more`; multiple such paragraphs
  all fold — `a more even more`). A **below-content-column** line after a blank ends the list instead
  (`- a` ⏎ blank ⏎ `more` at col 1 → item `a` + sibling prose `more`, engine-probed b/c). Parser-only,
  in `emit_md_list_level_inner`: the item-body gather (previously a lazy-continuation loop *then* a
  nested-list loop) is now **one loop** trying, in source order: no-blank lazy prose (`following_line_marker`
  + `is_md_item_lazy_continuation`), blank-separated content-column prose (new `next_prose_line_across_blanks`
  — mirrors `next_list_line_across_blanks` but requires a following `is_md_item_lazy_continuation` line, gated
  `list_line_indent >= content_indent` by the caller), then a nested list. `is_md_item_lazy_continuation` is
  the correct guard for the blank case too (a `===` folds `a ===`, engine-probed h; a `---`/block opener does
  not). Projector + formatter **unchanged** (the item filter already drops blank trivia + NEWLINE→SOFT_BREAK,
  norm_ws coalesces the folded paragraphs; the list emitter is per-line passthrough, so formatted bytes are
  byte-identical). Curated `md_list_item_para` (+pin +allowlist), fixture `roxygen_md_list_item_para` (CST:
  three paragraphs in one `ROXYGEN_MD_LIST_ITEM`, blanks as trivia), units
  `blank_separated_content_column_prose_folds_into_item` + `blank_then_underindented_prose_ends_the_list`,
  format baseline +1 (new case only). **Backlog:** a heading/fence/etc block (not prose) at the content
  column after a blank is a block *within* the item (arity ends the list — needs the item-block model); a
  block macro folding into an item (below).
- **Code span `\code`-vs-`\verb` per arity-parseability** (roxygen2 `can_parse`). A `_`-leading code
  span renders `\verb` (R's lexer rejects a `_`-leading name; `has_invalid_underscore_name` in
  `code_span_is_r`, but a lone `_` stays valid as the native-pipe placeholder, gated on `|>`).

**Sections / projection**
- **`norm_ws` is ASCII-`[[:space:]]`-only, never Unicode-aware.** The R driver's `norm_ws`
  (`gsub("[[:space:]]+", " ")` + `trimws`) collapses *ASCII* whitespace only; NBSP/NEL/`Zs` pass
  through. `project_rd::norm_ws` mirrors via `is_posix_space`; **do not** revert to
  `split_whitespace`/`char::is_whitespace` (folds NBSP→space, breaking flanking-rejected emphasis,
  cm-355). Flanking itself (`inline.rs`) *is* Unicode-aware.
- **Non-md prose is literal Rd; an unescaped `%` is a comment to EOL.** The projector re-derives `@md`
  (`block_md`, mirrors `resolve_roxygen_block` — a separate, necessary re-derivation, NOT the
  block-builder anti-pattern) and, md off, strips `%` line comments per physical line in
  `prose_text_atom`/`strip_rd_comments` (`\%` survives the strip — its pairing `escaped` flip-flop
  already handles `\\%` = escaped `\` + comment). Inline-join sites carry breaks as `\n` so the
  comment is line-scoped. Under `@md`, `%` is escaped (`\%`) → strip is off. Formatter mode-gates
  reflow off for a non-md `%`-bearing line.
- **Non-md TEXT resolves parse_Rd's literal escapes** (`resolve_rd_text_escapes`, after
  `strip_rd_comments` in `process_prose`'s else-branch — the non-md sibling of
  `collapse_md_backslash_runs`): backslashes pair left-to-right (`\\`→`\`); an unpaired trailing `\`
  before `%`/`{`/`}` consumes the escape (`\%`→`%`, `\{x\}`→`{x}` — parse_Rd renders the escaped char
  bare in TEXT, NOT the `\%` form arity used to keep); before a brace-less drop-set macro name it
  drops the `\name` (the brace-less-drop trap above); before anything else it stays literal
  (`a \ b` keeps its backslash). Runs before a letter are even here *except* the drop-set case (odd
  runs otherwise carved a macro in the lexer). Curated `rd_backslash_parity`.
- **Intro prose splits by *roxygen2 paragraph*, not CST node.** `parse_description` splits intro on
  `\n\n`: 1st = `\title`, 2nd = `\description`, rest = `\details` (folded with explicit `@details` only
  when leftover intro paras exist). Explicit `@title`/`@description` claims its slot.
  `section_body_parts` groups by paragraph (a block macro abutting prose = same para; a section-level
  blank-`#'` `ROXYGEN_MARKER` = break). **Title-as-description fallback is post-hoc**
  (`topics_add_default_description` runs after all tags): an explicit `@description` whose content
  hoisted entirely into `\section`s (a leading `# heading`) leaves no `\description` → fallback fires
  (`project_block` scans `out` for a `(\description` head after the tag loop). A *dropped*
  (`rdComplete`) description still emits an empty `(\description)` atom → correctly suppresses it.
- **Section pins sort in byte order, not locale collation.** The driver uses `sort(secs, method =
  "radix")` (C-locale) to match the Rust projector's `sections.sort()`. Latent until a section heads
  with a bare top-level `(TEXT …)`/`(GRP …)` (from `@rawRd`): any new bare-headed section ⇒ confirm the
  pin is byte-sorted.
- **`@rawRd` is bare top-level Rd, never markdown.** roxygen2 injects content verbatim (`tag_value`,
  not `tag_markdown`); parse_Rd splits into top-level nodes (each a "section"). Projector arm:
  `serialize_inlines(body)` pushed atom-by-atom, no `(\macro …)` wrap. The lexer keys it per-tag
  (`roxygen_line_tag` + `is_raw_rd_tag`, `"rawRd"` only, reset per block) so the body carries no md
  leaves. (`@evalRd`/`@usage` share the non-markdown semantics, out of scope.)
- **A prose section whose trimmed value is literal `"NULL"` is suppressed** (`rd_section()` sentinel;
  `NULL_SUPPRESSIBLE`). `@section` (title+body pair) is NOT suppressed; a suppressed `@description NULL`
  re-fires the title fallback. Data-object auto-`\format` (roxygen2 *evaluates*) is out of scope.
- **`rdComplete` brace-balance drop, mode-dependent.** roxygen2's `markdown_if_active` runs
  `rdComplete(rendered)` and replaces the body with `""` on a brace imbalance (`R/markdown.R`,
  `src/isComplete.cpp` — tracks only `{` `}` `\` `%`(line-comment) `\n`, **ignores quotes**). **Which
  text is scanned is mode-dependent** (`section_rd_complete` dispatches): md **on** →
  `rdComplete(markdown(text))`, the cmark-*rendered* Rd, whose structural braces (incl. the trailing-`\`
  bug `*\**`→`\emph{\}`) are reconstructed by `section_atoms_rd_complete` + `sexpr_to_rd` (rebuilds
  pre-parse Rd from S-expr atoms — imbalance comes only from leaf text). md **off** → `rdComplete(x$raw)`
  on the **raw tag value**, scanned by `section_raw_rd` (concatenates raw `Inline::Text` + verbatim
  `Inline::Macro` source, `SOFT_BREAK`→`\n`) — NOT the atoms, whose escape resolution has already
  collapsed `\{`→bare `{` (see the escaped-brace note below). **Critical (md path):** re-escape `%`→`\%`
  for every leaf in `@md` (`escape_percent = md`) or a `%20` URL false-drops; literal `{`/`}` in md
  text are *not* escaped. **The drop rule is mode-dependent** (`push_section`'s `check_drop = if md
  { drop_on_incomplete } else { true }`): md **on** → only the `sections=TRUE` tags
  (`@description`/`@details`) drop; md **off** → `rdComplete(text)` runs **unconditionally** so *every*
  prose section (title included) drops to empty.
  - **`@section` (md-OFF):** uses plain `tag_markdown` (`sections=FALSE`), so the per-section drop never
    fires under `@md`; md-off, the else-branch runs `rdComplete(x$raw)` on `title: body` → `""` →
    `str_split` gives `title=""`, `content=NA` → `\section{}{NA}` → `(\section (TEXT "NA"))`. Guard in
    `project_block`'s `"section"` arm (`!md && !rd_complete(source)`).
  - **`@field`/`@slot` (mode-independent):** parsed via `tag_two_part` → `rdComplete(x$raw,
    is_code=FALSE)` returns `NULL` on imbalance → the whole tag drops (a bad slot/field contributes no
    `\describe` item; all-dropped → no Slots/Fields section). `continue`-on-incomplete guard in the
    `"slot" | "field"` arm; raw section text scans identically to `x$raw` (no `{}\%` in scaffolding,
    quotes ignored).
  - **rdComplete scans raw text md-off (LANDED 2026-07-07).** roxygen2's md-off `rdComplete(x$raw)`
    reads where `\{`/`\}` is still `\`-escaped and thus **not counted**, so an unbalanced *escaped* brace
    does **not** drop (`a \{ b` renders "a { b"). The old atoms path false-dropped it: escape resolution
    (`resolve_rd_text_escapes`) had already collapsed `\{`→bare `{`, which the reconstruction counted.
    Fix (projector-only): `section_rd_complete` routes md-off through `rd_complete(section_raw_rd(body))`
    (raw leaf source, `SOFT_BREAK`→`\n` so `%` comments stay line-scoped); md-off never synthesizes the
    cmark-derived braces that make the atoms path necessary, so raw ≈ rendered for the chars
    `rd_complete` weighs (`{}`,`\`,`%`). Curated `rd_brace_escape_unbalanced` (title/desc/seealso with a
    single escaped brace kept; details with a balanced `\{…\}` pair kept).
- **md `\{`/`\}` render bare in TEXT, drop decision untouched (LANDED 2026-07-07b).** Under `@md` a lone
  prose escape `\{`/`\}` renders a **bare** brace (`a \{ b \} c`→`a { b } c`, unbalanced `a \{ b`→`a { b`
  still kept). The `double_escape_md`→cmark round trip is a **net no-op on a backslash-brace run** (raw
  source ≈ rendered Rd for `{}`,`\`), so arity's `@md` drop decision was **already correct** on the
  `\{`-preserving atoms (it keeps `a \{ b`, drops genuine bare `a { b`); only the rendered TEXT was wrong.
  Fix (projector-only, drop decision UNCHANGED): a `@md`-gated post-pass in `project_block` runs **after**
  every section (and its `rdComplete` drop) is built — `resolve_md_text_braces` walks the block's section
  strings and applies `resolve_md_brace_runs` (see the multi-backslash trap below) to **`TEXT` leaves only**.
  **VERB keeps `\{`** (verbatim `\verb`/`\url`/fenced code — engine-probed), and the resolution is
  quote-state-aware so a literal `(TEXT "` inside a code span is data, not a leaf opener. Doing it in
  `process_prose` would feed the resolved bare brace to `section_atoms_rd_complete` and **false-drop**
  `a \{ b` — the post-pass sidesteps that (drop scan reads the escaped brace, output reads bare). Curated
  `md_brace_escape`.
- **Multi-backslash before a brace resolves at the right stage (LANDED 2026-07-07e; even-`k` group landed 2026-07-07h).**
  A run of `k` source backslashes before a `@md` prose `{`/`}` renders `floor(k/2)` literal backslashes + a
  **bare** brace for **odd** `k` (`\{`→`{`, `\\\{`→`\{`, `\\\\\{`→`\\{`; matches roxygen2 exactly), and an
  **even** `k` opens an Rd `(LIST …)` group with `k/2` backslashes before it (`\\{y}` → `(TEXT "\\") (LIST
  (TEXT "y"))`) — now modeled by `group_brace_lists` under md (see the bare-group trap above). The old code
  mis-collapsed: `collapse_md_backslash_runs` halved the run
  to `ceil(k/2)` (destroying the parity the post-pass needs — a latent wrong-drop for `k >= 2`), and the
  post-pass only unescaped a **lone** `\{`. Fix: `collapse_md_backslash_runs` now leaves a run abutting `{`/`}`
  **verbatim** (like a bracket run) at cmark's `k`-backslash stage — which is *exactly* what roxygen2's
  `rdComplete(markdown(text))` scans, so the drop decision reads the right braces (an unbalanced *even* run
  like `a \\{ b` now correctly **drops**, as roxygen2 does) — and the parity-dependent parse_Rd resolution is
  deferred to the post-pass `resolve_md_brace_runs` (pairs the run → `floor(k/2)` backslashes + bare brace,
  odd or even). CST/formatter untouched (projector-only). Curated `md_brace_escape_runs`. **Backlog:**
  multi-backslash before a brace inside a *fragile* VERB/RCODE macro arg under `@md` (a separate path —
  `resolve_rd_arg_escapes`).
- **Literal Rd macro args resolve parse_Rd's Rd-string escapes (LANDED 2026-07-07c), mode-independent.**
  parse_Rd lexes every braced arg (`\code{…}`, `\verb{…}`, `\emph{…}`, `\link{…}`, `\url{…}`) with the same
  escape rules: `\{`→`{`, `\}`→`}`, `\%`→`%`, `\\`→`\` (backslashes pair left-to-right), for verbatim
  `RCODE`/`VERB` and prose `TEXT` alike, **on** and **off** `@md` (a fragile macro's arg resolves the same
  under `@md`). Projector-only (`resolve_rd_arg_escapes` — no braceless-drop + no `%`-comment strip: an
  in-arg `\word` is a carved nested macro, `%` in a braced arg is literal), wired into `serialize_macro`'s
  `flush` (RCODE/TEXT) and its `ROXYGEN_RD_MACRO_VERB` arm. **Discriminator: literal macro vs markdown
  construct.** A markdown code span/fence keeps its `\{` — it projects through a *different* path
  (`md_code_atom`/`serialize_md_code_block`/`verb_atoms`), never `serialize_macro`. Under `@md` the TEXT-only
  post-pass (`resolve_md_text_braces`) already resolved braces in a fragile macro's *TEXT* arg (`\link`), so
  the new arm is redundant-but-harmless there (post-pass sees a bare `{`, no-ops) and additionally reaches
  the fragile-macro *RCODE*/*VERB* the post-pass skipped. Curated `rd_macro_arg_escapes` (non-md, full set) +
  `md_macro_arg_escapes` (md, braces only).
- **The md rdComplete scan neutralizes a fragile macro's interior braces (LANDED 2026-07-07d).** Under
  `@md` an odd escaped brace in a *fragile* macro arg (`\verb{d \{ e}`/`\code`/`\url`/`\link`/`\preformatted`)
  used to **false-drop** the section: `section_atoms_rd_complete` reconstructs the md `rdComplete(markdown(text))`
  scan from the projected atoms, and the 2026-07-07c escape resolution had already collapsed the arg's `\{`→
  bare `{`, so the reconstruction counted an unbalanced brace. **Key fact:** every Rd macro node in the CST is
  brace-balanced *by construction*, and markdown() keeps a **fragile** macro's arg **raw** (escapes preserved),
  so a fragile macro contributes exactly **one balanced `{…}` pair** to the scan regardless of its interior —
  the interior must not count. Fix (`render_sexpr`/`append_leaf_text`, scan-path only, output untouched): thread
  a `verbatim` flag set when entering a fragile head (`is_fragile_for_md`) and propagated to the whole subtree
  (nested `\code` inside `\href` too); a `verbatim` leaf **drops every `rd_complete`-significant char**
  (`{`/`}`/`\`/`%`) so the interior is inert. Dropping (not re-escaping) is load-bearing: the atom may hold
  **resolved** braces (a literal macro → bare `{`) *or* **escaped** ones (a markdown code span → `\{`, kept
  verbatim via a different path — `markdown_code_span_keeps_its_backslash_brace`), so re-escaping would
  double-escape the latter into `\\{` and unbalance it. Cmark-*derived* `\emph{\}` (`*\**`) is outside any
  fragile macro → still counts → still drops (correct). **Also:** `preformatted_atoms` now runs
  `resolve_rd_arg_escapes` on its body (it skipped it, keeping `\{` unresolved in output) so `\preformatted`
  matches the other fragile macros; this made the scan-fix a prerequisite (resolved `\preformatted` braces would
  otherwise false-drop too). Curated `md_fragile_odd_brace` (verb/code/url/link, inline) + `md_preformatted_arg_escape`.

## Settled decisions (don't relitigate without reason)

Mode-keyed parse (one `markdown_default` salsa input; `@md`/`@noMd` per-block override; loose-file
default ON). CommonMark reference-spec two-pass (block tree → inlines); **no crate dependency**.
Projector is the **primary conformance engine** — `pub` (the gate lives in an integration-test crate)
but a **test-only faithful diagnostic**, never patched to pass. **Projection granularity: section-body
subtrees, excluding roclet-generated scaffolding** (`\name`/`\alias`/`\usage`/`\arguments`) — settled
2026-06-22c; `block-to-sections` drops the same set. Markdown = CommonMark core + GFM `table`,
`hardbreaks = TRUE`; **full CommonMark parity is the end goal** (a subset is a gap, not an end state).
The local lexer span-scanners are the **wrong shape** — the path is the block→inline delimiter-stack
pass (`docs/design/roxygen-inline-pass.md`). **Driver = the real CommonMark spec test set** (inputs
only; roxygen2 supplies every answer), vendored as a **third corpus source** alongside curated +
harvested. **The WHOLE spec is adopted at once as a measured backlog** (not section-by-section) with a
per-section burndown + an allowlist floor + a `blocked` bucket — panache's `commonmark`/`pandoc`
conformance model (2026-07-10f; superseded the emphasis-only slice). A fix now shows its blast radius
(cases drop in clusters), and already-passing cases are visible immediately. Full design:
`~/.claude/plans/i-want-to-start-snoopy-haven.md`; roadmap: `TODO.md`.

## Progress

Phase 0 **done**. **Phase 1 skeleton done:** the projector + pinned projector-parity gate are the
**primary driver** (parser-first, structural, CI-safe). `src/roxygen/project_rd.rs` projects the CST
to parser-owned Rd section subtrees; `tests/roxygen_projector.rs` diffs against roxygen2 section pins —
pure Rust, **no R**, allowlist-gated (`tests/oracle/roxygen-projector-allowlist.txt`). **Three pin
sources:** curated dir corpus (`<stem>.rdtree`); the harvested corpus's projector-eligible subset
(`roxygen-sections.jsonl`, 151/217 single-topic self-contained blocks); the **whole CommonMark spec**
(`commonmark-spec*.jsonl`, all 655 `cm-NNN` examples, per-section burndown in `ROXYGEN_PROJECTOR.md`).
**Current: 808 matching (all allowlisted), 153 divergent** of 961 pinned. The divergent 153 are the
per-section backlog (Fenced code blocks 22, Links 18, List items 18, harvested 18, Block quotes 11,
Images 11, …). Tasks: `task roxygen-projector` (the gate),
`roxygen-projector-refresh`/`-pins`/`-seed`, `roxygen-spec-corpus`/`-pins`. Report:
`ROXYGEN_PROJECTOR.md`. Blocked bucket: `roxygen-projector-blocked.txt` (empty for now).

**Three checks, three roles** (don't conflate):
1. **Projector parity** (`tests/roxygen_projector.rs`, pure Rust) — the **primary parser-growth
   driver**. Compares Rd *structure*; sees block-structure gaps the fixed-point check is blind to.
   Curated + harvested + whole-spec corpora (961 pinned); 153 divergent backlog.
2. **Curated fixed-point** (`tests/roxygen_oracle.rs::roxygen_oracle_report`, needs R, `#[ignore]`d) —
   strict semantic preservation of the formatter; 155/155 preserving, 0 blocked. *Meaning, not layout.*
3. **Harvested fixed-point** (`tests/oracle/corpus/roxygen.jsonl`, 217 cases, needs R, `#[ignore]`d) —
   broad opt-in backlog gated by `roxygen-allowlist.txt` (216 preserving, 1 skipped). A coverage net,
   not the parser driver. Reports: `task roxygen-oracle`/`roxygen-harvest`.

## Latest session (2026-07-10f) — adopt the WHOLE CommonMark spec as measured backlog

Switched the spec corpus from an emphasis-only slice (132 `cm-NNN`) to the **entire spec at once** (655
examples), adopting panache's `commonmark`/`pandoc` conformance model: a measured backlog with a
per-section burndown, an allowlist floor, and a `blocked` bucket. The section-by-section approach was
hiding a huge latent pass rate — **just measuring** what already worked jumped the projector from
**420→808 matching** (+388, all ratcheted into the allowlist), 153 divergent. Per-section coverage now
visible (`ROXYGEN_PROJECTOR.md`): Autolinks 19/19, Emphasis 132/132, Indented code 12/12, HTML blocks
43/46, Links 72/90, Code spans 19/22, …; biggest gaps Fenced code blocks (7/29), Links (18), List items
(18). This is the "a fix drops many cases / the report makes the gap visible" property the user wanted.

**Build + gate changes.** `build-commonmark-corpus.R` gained an `ALL` sentinel (every example-bearing
section) and emits a `section` field, tracked at the top level so example-content headings never leak in
(`task roxygen-spec-corpus` now mints `commonmark-spec.jsonl` + `-sections.jsonl`, the emphasis files
removed). `roxygen_projector.rs`: `HarvestInput` gained `section`; `Report` gained a `group` (spec
section, or `curated`/`harvested`); the report writes a **Coverage by section** table (sorted by biggest
remaining gap) + a section-grouped backlog; a new `roxygen-projector-blocked.txt` (read like the
allowlist, inline `# reason`) is excluded from the backlog and asserted disjoint from the allowlist
(empty for now — the fresh spec backlog is almost all reachable parser work).

**Surfaced + fixed a real lexer bug (the corpus's first payoff).** The broad corpus panicked the R
lexer: its two-/three-char operator lookahead sliced `&input[i..i+2/3]` **inside a multibyte UTF-8 char**
(a U+00A0 non-breaking space in 4 spec examples). Fixed with byte-safe `two_bytes`/`three_bytes` helpers
(16 sites), and the unknown-char catch-all — which emitted `bytes[i] as char` per byte, Latin-1-mangling
a nbsp into `Â`+nbsp and leaving `i` mid-char — now reads the whole UTF-8 char and advances by its width
(lossless). Committed **separately** as `fix(parser)` with a lexer unit test (`lexes_multibyte_char_before_two_char_operator_without_panicking`).

**Result:** projector **808 matching** (all 808 allowlisted, 0 regressions), 153 divergent, 0 blocked, of
961 pinned. `cargo test` green, clippy + fmt clean. The 153 divergent are the parser-growth backlog,
now navigable by section — that is the next-target menu (biggest levers: Fenced code blocks, Links,
List items).

**Ranked next target:** pick the **biggest-lever section** from the burndown — **Fenced code blocks**
(7/29, 22 remaining) or **Links** (72/90, 18) or **List items** (30/48, 18) — and close a construct
cluster, ratcheting the section toward 100%. The old image-title / space-paren-URL picks (2026-07-10e)
remain, now visible as specific `cm-NNN` divergences in Images (11/22). The harvested 18 stay out-of-scope.

## Earlier sessions

- **2026-07-10e** — user-defined image reference override (`![alt][ref]`/`![alt]` whose label has a user `[label]: url` def resolves to that URL, not synthesized `R:label`; projector-only `apply_user_linkrefs` arm rewrites the image to inline `![alt](url)` so `resolve_md_image`/`figure_atom` render it, image-format wrapping for free; undefined label unchanged). Curated `md_image_user_def`, fixture, unit, baseline +1, fixed-point 155/155. 419→420. (Superseded numerically by the whole-spec adoption above.)

- **2026-07-10d** — shortcut + reference markdown images (all three CommonMark image forms; `scan_md_image` rewritten to mirror `scan_md_link`, `resolve_md_image` dispatches on the char after `![alt]`; shortcut `![alt]`/reference `![alt][ref]` → synthesized `\figure{R:label}`, inline `![alt](dest)` validates its dest via `inline_dest_span`; collapsed/empty/`{`-followed stay literal). Curated `md_ref_image` + `md_image_invalid_dest`, fixtures, unit, baseline +2. 417→419.

- **2026-07-10c** — a trailing-backslash inline-link destination drops the section (`[t](foo\)bar)` → `(\details)`; `double_escape_md` makes `\)` a literal `\`+`)`, so cmark's dest is `foo\` and `\href{foo\}{t}` is `rdComplete`-incomplete → drop; projector-only, CST keeps the wider span; `body_has_dropping_href`/`md_href_dest_drops` gate into `section_rd_complete`'s md arm before the atom scan, counting the pre-closer backslash run — odd `r` drops, even keeps; recurses into emphasis/brace-group/list-item/display). Curated `md_link_dest_backslash_drop`, fixture, unit, baseline +1. 416→417.
- **2026-07-10b** — an invalid inline `(…)` destination is not a link (`valid_inline_dest_content` + `inline_dest_span` replace raw `scan_balanced(…,'(',')')` at all four carve sites; a bare dest runs to the first ASCII whitespace, `<…>` may hold spaces, an optional `"…"`/`'…'`/`(…)` title after ws; the bare-`]` closer arm relaxed to still close on an *invalid* `(` so the shortcut pairs; projector + formatter unchanged). Curated `md_link_invalid_dest`, fixture `roxygen_md_link_invalid_dest`, unit `invalid_inline_dest_falls_back_to_shortcut`. 415→416.
- **2026-07-10** — inline link titles dropped from the `\href` destination (projector-only; `inline_link_destination` mirrors cmark's dest parse — trim ws, `<…>` or bare-run-to-first-ws, entity-decode, discard the title; wired into `inline_link_dest` node path + `resolve_md_link` leaf arm; `[t](url"x")` no-ws keeps the quote in the dest, `<url with space>` keeps spaces). Curated `md_link_title`, fixture `roxygen_md_link_title`, unit `inline_link_title_is_dropped_from_href`, baseline +1. 414→415.

- **2026-07-09d** — setext underline folds into a block quote lazily (a `===`/`--`-too-short-for-a-thematic-break line after `> quote` folds into `ROXYGEN_MD_BLOCK_QUOTE` as lazy paragraph text — a setext underline can't be a lazy continuation *underline* in a quote, so it never promotes; roxygen2 flattens the whole quote to one `(TEXT …)`, `> foo`/`===`→`foo===`; `finish_md_block_quote`'s break gains an `is_lazy_setext` exception, block-quote-local, NOT in `is_foldable_continuation` which excludes underlines because a tag-value underline *does* promote; projector/formatter unchanged; `---` thematic break still ends the quote). Curated `md_blockquote_setext`, fixture, 2 units, baseline +1. 413→414.
- **2026-07-09c** — block Rd macro folds into a list item (closes the block-within-a-list-item series; a nested `\itemize{…}`/`\describe{…}`/`\tabular{…}{…}` after a `- a` item folds **into** the `\item` as an `Inline::Macro` child, `- b` resuming the same `\itemize`; a raw `\name{…}` is not a markdown block so it folds by CommonMark's *paragraph-continuation* rule — no blank → any indent, blank → content column only; two arms at the top of `emit_md_list_level_inner` calling `emit_block_macro`; projector/formatter unchanged). Curated `md_list_item_block_macro`, fixture, 3 units, baseline +1. 412→413.
- **2026-07-09b** — GFM table folds into a list item (third in-item construct; a `is_md_table_start` header at the item's content column folds inside the `\item` → `\tabular` between the `\item`s, with/without a blank, `- b` sibling; unindented header is lazy prose; new arm at the top of `emit_md_list_level_inner` mirroring the fence + a `push_inline` `ROXYGEN_MD_TABLE` arm that was missing; formatter unchanged). Curated `md_list_item_table`, fixture, 3 units, baseline +1. 411→412.

- **2026-07-09** — indented code block folds into a list item (blank-separated line indented `content_indent + 4` → indented code inside the `\item`, three-atom `\if…\preformatted…\if`; **blank required** — no-blank over-indented is a lazy continuation; **empty** item doesn't fold — `item_has_content` gate; new arm before the loose-prose arm using `next_content_line_across_blanks` + `is_indent_code_line_min` + `emit_md_indented_code_min`; projector `push_inline` `MdIndentedCode` arm + `md_indented_code_extra_strip` strips `content_col + 4` since the content column does NOT cancel; formatter unchanged). Curated `md_list_item_indented_code`, fixture, 3 units, baseline +1. 410→411.
- **2026-07-08h** — fenced code block folds into a list item (first block-within-a-list-item construct; `next_content_line` + a fence arm in `emit_md_list_level_inner` at `>= content_indent`, no `item_has_content` gate — folds with/without a blank and into an empty item; projector `push_inline` `MdCodeBlock` arm + a latent `md_code_block_parts` indent-strip fix so an indented fence's info string/body don't leak indent, surviving = `max(0, body_col−fence_col)`). Curated `md_list_item_code_block`, fixture, 3 units, baseline +1. 409→410.

- **2026-07-08g** — blank + content-column prose folds into a list item (single item-body loop in `emit_md_list_level_inner`: no-blank lazy prose, blank-separated content-column prose via new `next_prose_line_across_blanks`, nested list — interleaved in source order; projector/formatter unchanged, per-line passthrough byte-identical). Curated `md_list_item_para`, fixture, units `blank_separated_content_column_prose_folds_into_item` + `blank_then_underindented_prose_ends_the_list`. 408→409.
- **2026-07-08f** — no-blank list marker-type split, both modes (hoisted the `md_list_marker_type` check out of the blank-path `else` branch in `emit_md_list_level_inner`'s sibling gather so a type change ends the list with or without a blank; `- a` ⏎ `* b` → two `\itemize`, `1.` ⏎ `2)` → two `\enumerate`; projector/formatter unchanged, structural). Curated `md_list_marker_split`, fixture `roxygen_md_list_marker_split`. 407→408.
- **2026-07-08e** — even-run braced macro → literal `\name` + `(LIST …)`, both modes (no code change; falls out from the backslash-parity gate + `group_brace_lists` + escape resolution; `\\emph{x}` → `(TEXT "\emph") (LIST (TEXT "x"))`, `\link` spared). Curated `{rd,md}_even_braced_macro`, fixture `roxygen_even_braced_macro`, unit `even_run_braced_macro_projects_as_literal_plus_list`. 405→407.

- **2026-07-08d** — sticky brace-less RCODE/VERB swallow (explicit prose tag, single-paragraph plain-text tail; `sticky_braceless_code_mode` + projector `split_sticky_braceless_swallow` at `project_tag_section` entry → per-line `(RCODE …)`/`(VERB …)`; withhold on impure tails; formatter `line_has_sticky_swallow` reflow bail). Curated `{rd,md}_braceless_sticky`, fixture `roxygen_braceless_sticky`, 3 units, baseline +2. 403→405.
- **2026-07-08c** — brace-less `\item` → `(UNKNOWN "\item")` node, both modes (`split_braceless_items`/`split_item_text` pre-pass in `serialize_prose` after `group_brace_lists`, gated `group=true`; `Inline::BracelessItem`; parity-gated, name-exact, recurses into `BraceGroup`/`MdEmphasis`; `\item{x}` → `(UNKNOWN "\item") (LIST …)` falls out). Curated `{rd,md}_braceless_item`, fixture `roxygen_braceless_item`, unit `braceless_item_projects_as_unknown_node`. 401→403.

- **2026-07-08b** — bare `{…}` groups inside a *markdown heading title* → Rd `(LIST …)`, `@md` (`render_heading_frame` folds the frame `title` via `group_brace_lists` before `serialize_inlines`; CST unchanged, projector-only). Curated `md_heading_brace_group`, fixture `roxygen_md_heading_brace_group`, unit `heading_title_bare_groups_project_as_lists`. 400→401.
- **2026-07-08** — bare `{…}` groups inside a *prose macro arg* → Rd `(LIST …)`, both modes (`serialize_macro` collects per-arg `Vec<ArgPiece>` + `finalize_macro_arg`/`group_arg_pieces`; verbatim never groups, structural GRP-wraps; md branch + `serialize_md_structural_macro` run `group_brace_lists` on resolved arg inlines). Curated `{rd,md}_macro_arg_brace_group`, fixture `roxygen_macro_arg_brace_group`, unit `macro_arg_bare_groups_project_as_lists`. 398→400.
- **2026-07-07i** — `%`-swallow trailing-`\` false-drop fixed (md drop scan; `section_rd_complete`'s md arm runs `strip_scan_percent_comments` over top-level `TEXT` leaves before serializing scan atoms, dropping each odd-run `\%` region whole so the scan matches `markdown(text)`; only odd runs stripped). Curated `md_percent_trailing`, unit `trailing_percent_swallow_does_not_false_drop`. 397→398.
- **2026-07-07h** — bare `{…}` prose brace groups → Rd `(LIST …)`, md (`group_brace_lists` threaded `md`, runs both modes; brace parity mode-independent, `%`-comment trigger inverted by mode; md `rdComplete` scan reads ungrouped atoms). Curated `md_brace_group` + fixture `roxygen_md_brace_group`. 396→397.
- **2026-07-07g** — bare `{…}` prose brace groups → Rd `(LIST …)`, non-md (`group_brace_lists`, a group-stack pre-pass in `serialize_prose_with_linkrefs`; `Inline::BraceGroup`; escape/comment parity mirrors `resolve_rd_text_escapes`; balanced-only, unbalanced drops via `rdComplete` on the ungrouped body). Curated `rd_brace_group` + fixture `roxygen_rd_brace_group`. 395→396.

- **2026-07-07f** — in-arg backslash parity in `build_rd_content` (`rd_backslash_is_escaped` made `pub(crate)` + added to the `b'\\'` macro-carve guard so `\emph{\\y}`→`(\emph (TEXT "\\y"))`; text-escape half already handled by `resolve_rd_arg_escapes`; CST-level, baseline +1 additive). Curated `rd_arg_backslash_parity`. 394→395.
- **2026-07-07e** — multi-backslash before a brace resolves at the right stage (`collapse_md_backslash_runs` leaves a run abutting `{`/`}` verbatim at cmark's `k`-backslash stage so the md `rdComplete` scan reads true braces and even runs drop correctly; parity-dependent pairing → `floor(k/2)` backslashes + bare brace moved to post-pass `resolve_md_brace_runs`; odd `k` exact, even `k` still `(LIST)` backlog). Curated `md_brace_escape_runs`. 393→394.
- **2026-07-07d** — md rdComplete scan neutralizes fragile-macro interior braces (`render_sexpr` threads a `verbatim` flag from a fragile head via `is_fragile_for_md`; a verbatim leaf drops every `rd_complete`-significant char so a fragile arg contributes one balanced pair; also `preformatted_atoms` now runs `resolve_rd_arg_escapes`). Curated `md_fragile_odd_brace` + `md_preformatted_arg_escape`. 391→393.
- **2026-07-07c** — literal Rd macro args resolve parse_Rd's Rd-string escapes (`resolve_rd_arg_escapes`: `\{`→`{`/`\}`→`}`/`\%`→`%`/`\\`→`\`, left-to-right pairing, mode-independent; wired into `serialize_macro`'s flush + VERB arm; a markdown code span keeps its `\{` via a different path). Curated `rd_macro_arg_escapes` + `md_macro_arg_escapes`. 391 (389→391).
- **2026-07-07b** — md `\{`/`\}` render bare in TEXT (projector-only, drop decision UNCHANGED; `@md`-gated post-pass `resolve_md_text_braces` applies `unescape_lone_rd_brace` to TEXT leaves only, after each section's `rdComplete` drop — the `double_escape_md`→cmark round trip is a net no-op on a backslash-brace run, so the drop was already correct on the `\{`-preserving atoms). Curated `md_brace_escape`. 388→389.
- **2026-07-07** — rdComplete scans raw text md-off (`section_rd_complete` dispatch: md-off routes through `rd_complete(section_raw_rd(body))` on the raw tag value, so an unbalanced *escaped* brace keeps the section; the atoms path already collapsed `\{`→`{` and false-dropped). Curated `rd_brace_escape_unbalanced`. 387→388.
- **2026-07-06f** — brace-less known-macro drop, projector-only (parse_Rd's "expecting `{`" recovery deletes a brace-required known `\name`; `is_rd_braceless_drop_macro` = known ∧ ¬zero-arg ∧ ∉ `STICKY_BRACELESS_RD_MACROS`; `braceless_drop_name_end` wired into `resolve_rd_text_escapes` + `collapse_md_backslash_runs`). 385→387.
- **2026-07-06e** — backslash-run parity gate + zero-arg name-only carves (`rd_backslash_is_escaped` gating the prose `\` dispatch; `ZERO_ARG_RD_MACROS` early return in `scan_rd_macro`; projector `resolve_rd_text_escapes` for non-md `\\`/`\%`/`\{` resolution). 382→385.

- **2026-07-06d** — multi-line code spans in the inline pass (`resolve_multiline_spans`, one leftmost-first `<`+`` ` `` scan; recarve of later line-scoped leaves, exact-coincidence skip; `ROXYGEN_MD_CODE` node, `RunItem::Span`; shared reflow bail). 380→382.
- **2026-07-06c** — multi-line inline HTML in the inline pass (`resolve_multiline_html`, run→`Vec<RunItem>`, `ROXYGEN_MD_HTML` node + lossless token tiling; projector per-line VERBs; formatter descend + reflow bail). 377→380.
- **2026-07-06b** — blank-separated loose-list merge (`next_list_line_across_blanks` + `md_list_marker_type`, both gather sites of `emit_md_list_level_inner`; a blank only loosens, type change splits, deeper marker nests). 375→377.
- **2026-07-06** — list-item lazy continuation (`is_md_item_lazy_continuation` + fold loop in `emit_md_list_level_inner`, before the nested gather, gated on `item_has_content`; `===`/table headers fold, any marker line is a sibling, `---`/cond-7 interrupt). 373→375.
- **2026-07-05d** — from-value block quote + thematic break (`@details > q` / `@details ***`; shared `finish_md_block_quote`, `is_thematic_break` at the value; projector zero changes). 370→373.
- **2026-07-05c** — all remaining from-value block starts (indented code / fence / ATX / table / list from a tag's same-line value; `close_tag_at_value` dispatch, `push_value_opener_line`; post-hoc title-as-description fallback fix). 364→370.
- **2026-07-05b** — from-value HTML block (`@details <span>` / `<!-- note`, all conditions; tag closes empty + sibling node with marker-less first line; formatter next-line normalization). 362→364.
- **2026-07-05** — HTML block condition 7 (standalone complete tag on its own line → blank-terminated block; builder-structural `is_md_html_block7_line`, positional can't-interrupt gate, `<pre/>` cond-1 bug fix, formatter `is_md_standalone_html_tag` fresh-position guard). 360→362.
- **2026-07-04b** — inline raw-HTML comment/PI/declaration/CDATA (`RoxygenMdHtml` leaves, `\if{html}{\out{…}}`; block cond 5 `CDATA` case-insensitive, cond 4 uppercase-only) + the conds-1–6 reflow line-start guard (`is_unsafe_line_start` → `starts_md_html_block`). 358→360.

One-liners (date — what landed; projector matching delta). Mechanics live in the traps above and git.

- **2026-07-04** — markdown HTML block conditions 2–5 (line-start comment/PI/declaration/CDATA → `ROXYGEN_MD_HTML_BLOCK` running to a line containing `-->`/`?>`/`>`/`]]>`, through blanks; no new token — terminator re-derived by `html_block_closers`). 357→358.
- **2026-07-03** — markdown HTML block condition 1 (verbatim `<pre>`/`<script>`/`<style>`/`<textarea>` → run to a line containing `</tag>`, inclusive, through blanks; no new token — terminator re-derived from the opener text via `is_html_verbatim_opener`; also fixed a latent cond-6 formatter indent-trim bug by switching to `normalize_list_marker_text`). 355→357.
- **2026-07-02o** — markdown indented code blocks (a line >= 5 space columns past the `#'` marker → same `\if{html}{\out{<div>}}`/`\preformatted`/`</div>` shape as a fenced block; no new token — the block builder re-derives `@md` via `block_md`; `is_indent_code_line`, `emit_md_indented_code`; formatter `normalize_list_marker_text` preserves the indent). Also fixed a reflow-as-prose formatter bug. 354→355.

- **2026-07-02n** — setext heading whose title begins as a tag's same-line value (`@details Big Title`⏎`===` → sibling `ROXYGEN_MD_HEADING`, empty tag; `emit_tag_line` pre-scan + `emit_md_setext_heading_from_value`; formatter `mid_prose` prefix; projector unchanged). 353→354.
- **2026-07-02m** — block-quote lazy continuation (a non-`>` paragraph-continuation line folds into the quote's open paragraph, no separator; `emit_md_block_quote` gather loop continues on `is_md_block_quote_start` OR `is_foldable_continuation`; projector/formatter unchanged). 352→353.
- **2026-07-02l** — block-quote glue onto adjacent prose (no paragraph separator before a quote; projector-only, `RunSeg::Final` segment + `trim_trailing_run_ws`, separators suppressed in both join sites). 350→352.
- **2026-07-02k** — single-dash setext H2 underlines (`-`/`- ` after a paragraph → level-2 setext; an empty list item can't interrupt a paragraph); build.rs `is_md_setext_dash_underline`/`is_md_setext_underline_or_dash` wired into the two setext functions only, list-check-first disambiguates the fresh-position empty bullet. Parser-only. 349→350.
- **2026-07-02j** — markdown thematic breaks (`***`/`---`/`___`) → render **empty**, neighbors coalesce (roxygen2 has no support: `mdxml_unknown`→`escape_comment`=""); `is_thematic_break` leaf + block-level `setext_underline_is_thematic` for a bare `---`; projector flushes a part with no atom. 347→349.
- **2026-07-02i** — markdown block quotes (`> quoted`) → flattened plain text (`>` + inner markdown dropped, lines glue with no separator); `is_block_quote_marker`/`emit_md_block_quote`, projector `serialize_md_block_quote`. 345→347.
- **2026-07-02h** — setext headings (`Title`/`===`|`---`) → hoisted Rd `\section`/`\subsection` like ATX; block-level **look-back** promotes the whole preceding paragraph (`is_md_setext_heading_start`/`emit_md_setext_heading`, multi-line `ROXYGEN_MD_HEADING`; `is_foldable_continuation` excludes underlines). Single `-`/`- ` deferred. 343→345.

- **2026-07-02g** — ATX headings `# Title` (levels 1-6) → hoisted Rd `\section`/`\subsection` (single-line `RoxygenMdHeading` leaf; projector `emit_section_with_headings`/`HeadingFrame` outline; trap: md mode is block-wide, so a "not a heading" fixture needs its own no-`@md` block). 340→343.
- **2026-07-02f** — GFM pipe tables `| a | b |` + `|---|:--:|` → `\tabular{<align>}{… \tab … \cr}` (`is_table_delim_row` carve, `emit_md_table`; projector `serialize_md_table` per-cell inline runs). 337→340.
- **2026-07-02e** — CommonMark email autolinks `<addr>` → `\href{mailto:addr}{addr}` (`scan_md_email_autolink`; projector `autolink_has_uri_scheme` splits URI→`\url` vs email→mailto). 336→337.
- **2026-07-02d** — link-ref def with a trailing macro on the dest line is not a definition (`match_linkref_def` rejects a non-`Text` inline without a `SOFT_BREAK`; `parse_linkref_def_dest`→`(url, line_closed)`). 335→336.
- **2026-07-02c** — HTML character references decode under `@md` (full 2125-entry HTML5 table, `decode_html_entities` wired into `prose_text_atom`). 334→335.
- **2026-07-02b** — same-line tag-value continuation folds into the `ROXYGEN_TAG` (emphasis/link spans cross the soft break). 333→334.
- **2026-07-02** — soft-wrap physical-line boundary for `%`-swallow/comment (`SOFT_BREAK` sentinel). 331→333 (no delta; format-only).
- **2026-07-01b** — `@md` `%`-swallow (parity-keyed on the source backslash run); projector `md_percent_swallow`. 330→331.
- **2026-07-01** — `@md` backslash-run collapse (`\\`→`\`, `ceil(k/2)`); projector `collapse_md_backslash_runs`. 329→330.
- **2026-06-30g** — emphasis span crosses an inline Rd macro; faithful placeholder flanking in `edge_char`. 328→329.
- **2026-06-30f** — emphasis/link span crosses a nested macro in a *structural* arg (`resolve_md_inline_pieces`). 327→328.
- **2026-06-30e** — md in structural two-arg macro args (each arg processed); per-run `md_structural`. 326→327.
- **2026-06-30d** — pure-macro link displays drop/keep via `link_label_text` (not literal `[]`). 325→326.
- **2026-06-30c** — md inside non-fragile inline macro args; `is_fragile_for_md`, `is_md_inline_text_macro`. 323→325.
- **2026-06-30b** — `\`-bearing same-line link display on the arena; `display_has_macro`/`link_over_display`. 321→323.
- **2026-06-30** — Slice B remainder: whole-field poisoning lift, multi-line/entity defs, references on the arena. 317→321.
- **2026-06-29q** — whole-field refmap + undefined-label demotion, recursing list items. 315→317.
- **2026-06-29p** — user link-refs resolve across list items (`collect_user_linkrefs_tree`/`apply_user_linkrefs`). 313→315.
- **2026-06-29o** — `@section` runs the full link-ref pipeline (`resolve_linkrefs` shared). 312→313.
- **2026-06-29n** — formatter: link-ref-definition lines stay unjoined (third reflow-bail). 311→312.
- **2026-06-29m** — URL-defined reference links `[ref]: url` → `\href` (display kept, def consumed). 310→311.
- **2026-06-29l** — same-line non-plain *reference* `[*foo*][ref]` drop (`same_line_ref_opener`). 309→310.
- **2026-06-29k** — non-plain shortcut/reference link drop ("must contain plain text"). 308→309.
- **2026-06-29j** — link-reference map; undefined shortcut/ref stays literal (`demote_undefined_links`). 306→308.
- **2026-06-29i** — `@section` md-OFF drop to `(\section (TEXT "NA"))`. 305→306.
- **2026-06-29h** — `@field`/`@slot` whole-tag drop on raw brace imbalance (`tag_two_part`). 303→305.
- **2026-06-29g** — markdown-OFF rdComplete brace-balance drop (every section, title included). 301→303.
- **2026-06-29f** — opener-deactivation slice B core (`match_brackets`, nested links inner-first). 299→301.
- **2026-06-29e** — same-line plain-text shortcut `[text]` onto the arena node path. 298→299.
- **2026-06-29d** — `rdComplete` brace-balance drop (`*\**`→`\emph{\}`); `rd_complete`/`sexpr_to_rd`. 294→298.
- **2026-06-29c** — `@rawRd` body is verbatim Rd, never markdown (per-tag `rox_raw` flag). 293→294.

Clustered rollups (mechanics all in the traps above):
- **2026-06-26..29b — `get_md_linkrefs` poisoning suite (slices 1–6):** all-invalid → mixed → outside
  `push_section` → inline-link / image / nested-bracket candidate-def leaks; cross-line *shortcut*
  `[text]` links. 284→293.
- **2026-06-25d..l — the emphasis inline-pass + same-line/cross-line links:** the real delimiter stack
  (`resolve_emphasis`), paragraph-granularity runs, cross-line emphasis + inline/reference links, the
  spec emphasis corpus as 3rd source, cm-369/355/481 edges, escaped `\[`/`\]`. 147→284.
- **2026-06-25/25b — CommonMark-parity tenet + inline-pass design; `%`-reflow follow-on.** 147.
- **2026-06-24e..u — markdown block/inline coverage + Rd verbatim blocks:** intro split
  (`parse_description`), images/`\figure`, fenced code, autolinks, raw HTML (inline + block), nested md
  + Rd lists, `\preformatted` (line-start + Form C), brace-less unknown macros, `@section`/`@examples`
  aggregation, digit-bearing macro names, non-md `%` comments, `@rawRd` bare top-level. 105→147.
- **2026-06-24b..d — `@md` links land + refactors:** inline/reference/shortcut links → `\href`/`\link`;
  split `roxygen.rs` into lex/group/build; `RoxygenRole`. 93→105.
- **2026-06-23 (Stages 1–11):** CST re-model (`ROXYGEN_LINE` dissolved); `\itemize`/`\enumerate`,
  `\describe`/`\item`, `\tabular`, `@md` inline + block lists, title-as-description fallback,
  `@tag NULL` suppression, `\code`→RCODE, `\href` per-arg verbatim, `@slot`/`@field`→`\section`. 56→93.
- **2026-06-22 (Phase 0 + Phase 1 skeleton):** R driver, curated corpus, fixed-point harness,
  `blocked.toml`, devenv R; harvested backlog (217 blocks); projector skeleton (`project_rd.rs`,
  pure-Rust pinned gate); filtered bulk-pin; inline Rd macros as `ROXYGEN_RD_MACRO` nodes. →56.
