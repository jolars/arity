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
- **Brace-less `\word` carves only when *unknown*.** `is_known_rd_macro`/`KNOWN_RD_MACROS`
  (parse_Rd's static table, R 4.5; excludes expanded user macros `\CRANpkg`/`\doi`). Unknown →
  `(UNKNOWN "\\word")`; known brace-less stays literal prose. A new known macro must go in the
  table or it silently becomes UNKNOWN.
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
- **Still backlog** (the `@md` `\`-escape *render* cluster — do NOT widen the lexer): a run **before a
  letter** (`\\y` — the lexer splits it into text `\` + an UNKNOWN macro node); brace-less known-macro
  decomposition under `@md` (`\emph z`→dropped, `\code z`→`\code{ z}`, `\dots`→kept — lexer leaves them
  plain text). These tie into the inline-pass migration.
- **HTML entities decode under `@md` only, projector-only.** cmark resolves every semicolon-terminated
  HTML5 named entity (`&amp;`/`&copy;`/`&hellip;`) + numeric ref (`&#65;`/`&#x41;`); U+0000/surrogate/
  out-of-range → U+FFFD; missing `;` or unknown name stays literal; single-pass (`&amp;amp;`→`&amp;`);
  **off in code spans** (separate verbatim leaves — nothing to do). Full 2125-entry table in generated
  `src/roxygen/entities.rs` (Python `html.entities.html5`, `;`-terminated, escaped non-ASCII, binary
  search); `decode_html_entities`/`decode_entity` (link-dest + prose). **Wired as the *last* transform in
  `prose_text_atom`'s `md` branch** (after `%`-swallow/backslash/bracket — an entity-produced `[`/`%`/`\`
  is inert text). CST stays lossless (raw `&amp;` prose); non-md keeps entities literal. Curated
  `md_entities`. Regenerate the table if the WHATWG list changes.
- **Images** (`scan_md_image`, inline `![…](…)` only): `mdxml_image` drops alt → `\figure{url}{title}`,
  wrapped per extension (`image_format`: svg→html, pdf→pdf, raster/unknown→bare). `\figure` = 2-arg
  verbatim macro.
- **Fenced code blocks** (`scan_md_fence`, carved whole *before* the list-marker carve; bails if a
  backtick follows). `emit_md_code_block` pairs opener↔closer into `ROXYGEN_MD_CODE_BLOCK`. Projector
  emits 3 atoms: `\if{html}{\out{<div…>}}` / `\preformatted{<code+\n>}` / `\if{html}{\out{</div>}}`.
  Out of scope: ` ```{r} ` knitr-eval blocks.
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
  **Backlog:** list-item **lazy continuation** (`- a` ⏎ non-list line glues into
  the item per cmark, probe e4 — arity ends the list; a *line-start* list has the same gap).
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
  eats the third `]`). **Backlog:** multi-line inline HTML (cmark inline spans cross a soft break;
  arity is line-scoped — faithful under-handling). Curated `md_html_inline_forms`.
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
  `prose_text_atom`/`strip_rd_comments` (`\%` survives). Inline-join sites carry breaks as `\n` so the
  comment is line-scoped. Under `@md`, `%` is escaped (`\%`) → strip is off. Formatter mode-gates
  reflow off for a non-md `%`-bearing line.
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
  `src/isComplete.cpp` — tracks only `{` `}` `\` `%`(line-comment) `\n`, **ignores quotes**). Projector
  port: `rd_complete` (verbatim) + `sexpr_to_rd` (rebuilds pre-parse Rd from S-expr atoms — imbalance
  comes only from leaf text, the trailing-`\` bug `*\**`→`\emph{\}`). **Critical:** re-escape `%`→`\%`
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
harvested. Full design: `~/.claude/plans/i-want-to-start-snoopy-haven.md`; roadmap: `TODO.md`.

## Progress

Phase 0 **done**. **Phase 1 skeleton done:** the projector + pinned projector-parity gate are the
**primary driver** (parser-first, structural, CI-safe). `src/roxygen/project_rd.rs` projects the CST
to parser-owned Rd section subtrees; `tests/roxygen_projector.rs` diffs against roxygen2 section pins —
pure Rust, **no R**, allowlist-gated (`tests/oracle/roxygen-projector-allowlist.txt`). **Three pin
sources:** curated dir corpus (`<stem>.rdtree`); the harvested corpus's projector-eligible subset
(`roxygen-sections.jsonl`, 151/217 single-topic self-contained blocks); the CommonMark spec emphasis
corpus (132 `cm-NNN` cases). **Current: 373 matching (all allowlisted), 18 divergent** of 391 pinned.
The 18 left are all roxygen2-*evaluation*/multi-block gaps (out of scope — knitr eval, RefClass
docstrings, cross-block `@name`/reexport). Tasks: `task roxygen-projector` (the gate),
`roxygen-projector-refresh`/`-pins`/`-seed`, `roxygen-spec-corpus`/`-pins`. Report:
`ROXYGEN_PROJECTOR.md`.

**Three checks, three roles** (don't conflate):
1. **Projector parity** (`tests/roxygen_projector.rs`, pure Rust) — the **primary parser-growth
   driver**. Compares Rd *structure*; sees block-structure gaps the fixed-point check is blind to.
   Curated + harvested + spec corpora (388 pinned). The 18 divergences are out-of-scope.
2. **Curated fixed-point** (`tests/roxygen_oracle.rs::roxygen_oracle_report`, needs R, `#[ignore]`d) —
   strict semantic preservation of the formatter; 108/108 preserving, 0 blocked. *Meaning, not layout.*
3. **Harvested fixed-point** (`tests/oracle/corpus/roxygen.jsonl`, 217 cases, needs R, `#[ignore]`d) —
   broad opt-in backlog gated by `roxygen-allowlist.txt` (216 preserving, 1 skipped). A coverage net,
   not the parser driver. Reports: `task roxygen-oracle`/`roxygen-harvest`.

## Latest session (2026-07-05d) — from-value block quote + thematic break

Closed the last two from-value block starts: a prose tag's same-line value now opens a **block
quote** (`@details > quoted` — roxygen2 has no support: warns, flattens to plain text, glues with no
separator; following `>` lines and lazy continuations join) and a **thematic break** (`@details ***`/
`---`/`- - -` — renders empty, following prose survives; `@seealso ---` → empty `(\seealso)` atom).
Lexer: `is_thematic_break` + `is_block_quote_marker` carves at the value (thematic **before** the
list-marker carve — spaced `- - -` starts with a valid bullet; a value `---` is a break, never setext,
the value position is fresh; `===` stays prose). Grouper: two leaf-keyed dispatch branches; builder:
`emit_md_thematic_break_from_value` + `emit_md_block_quote_from_value` (gather factored into shared
`finish_md_block_quote`). Formatter: `is_from_value` → `push_value_opener_line` in both emitters
(next-line normalization). **Projector: zero changes** — `strip_marker` no-ops on the marker-less
first line, `block_quote_flat_text` strips the `>` regardless, the break node drops wholesale, and
the empty-section atom already emits. Full mechanics in the HTML-blocks trap's from-value paragraph.

**Result:** projector **370→373 matching** (all allowlisted, seeded), 18 divergent (unchanged,
out-of-scope). `cargo test` green (863), clippy + fmt clean; curated fixed-point **108/108
preserving**, 0 blocked; format baseline **+3 additive** (re-blessed, reviewed). New fixtures: parser
`roxygen_md_{blockquote,thematic_break}_value` (lazy continuation, 4-space indent, spaced `- - -`,
`===`-stays-prose, non-md negatives); formatter `roxygen_md_quote_break_value`; curated
`md_blockquote_value` + `md_thematic_break_value` + `md_thematic_break_value_edges` (+pins
+allowlist).

**Ranked next target:** list-item **lazy continuation** (`- a` ⏎ plain line glues into the item per
cmark, probe e4 — arity ends the list; line-start lists share the gap); then multi-line **inline**
HTML (cmark inline spans cross a soft break; arity is line-scoped — ties into the inline-pass
migration); then the `@md` `\`-escape *render* cluster (do NOT widen the lexer). The 18 projector
divergences remain out-of-scope (roxygen2 evaluation / multi-block).

## Earlier sessions

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
