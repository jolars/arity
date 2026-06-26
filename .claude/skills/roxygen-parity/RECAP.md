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
- **Block-macro openers, three forms.** Forms A/B are *line-start* (`is_block_macro_line`,
  section-level). Form A: a `RoxygenText` `\name{` unbalanced on its line
  (`\itemize`/`\describe`/`\name`) — necessarily a multi-line opener since the lexer extracts a
  *balanced* `\name{…}` as one inline token. Form B: a balanced structural `RoxygenRdMacro`
  (`\tabular{rl}`) then a `RoxygenText` opening the body `{`. `emit_block_macro` dispatches.
  **Form C: a *mid-prose* opener** (`So far so good. \preformatted{ …`). The lexer
  (`lex_roxygen_prose`, `is_block_macro_opener_at`) **always splits** an unbalanced `\name{`
  into its own to-EOL `RoxygenText` (at line start this reproduces the same whole-line token;
  mid-prose it splits the preceding run off). The grouper (`emit_prose_line`) promotes it to an
  **inline** `ROXYGEN_RD_MACRO` *inside the open paragraph* (sibling of the prose, folded into
  the same section by `section_body_parts`) — but **only if `block_macro_opener_closes`** (a
  later `}` balances it before a tag/block-end); an opener that never closes stays literal prose
  (parse_Rd rejects an unbalanced macro, so this is the conservative recovery —
  `roxygen_unbalanced_macro`, which is why a mid-prose unbalanced `\code{` splits into two prose
  tokens yet stays prose). `emit_block_macro_inline`/`emit_block_macro_from_opener` share the
  body-consume with line-start `emit_block_macro`. **Formatter:** `emit_block_macro` detects a
  markerless (mid-prose) opener (`first_token() != ROXYGEN_MARKER`) and **prepends `#' `** so the
  opener lands on its own line — lossless + idempotent (reparse makes it a line-start opener that
  re-emits identically). Unlike line-start, Form C does *not* verify-by-closure asymmetry is
  deliberate: a line-start `\name{` is unambiguously a block opener; a mid-prose one commits only
  when it actually forms a balanced block.
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
- **Emphasis is the real delimiter-stack inline pass** (slice 1 landed 2026-06-25d,
  paragraph-granularity slice 1.5 landed 2026-06-25e), NOT a local scanner. The lexer carves
  `*`/`_` as **neutral** `RoxygenMdDelim` leaves (no open/close decision);
  `src/parser/roxygen/inline.rs::resolve_emphasis` runs cmark's `process_emphasis` (full ASCII
  flanking, rule of 3, nesting) over each **paragraph-granularity** inline run, emitting
  `ROXYGEN_MD_EMPH`/`STRONG` **nodes** (kinds 90/91, now NODES) with `ROXYGEN_MD_DELIM` (kind
  101) opener/closer/leftover leaves. **Run = every paragraph-body `Event::Tok`** (content +
  the inter-line trivia — newline/`#'` marker/whitespace — a continuation folds in), bounded
  only by a structural `Start`/`Finish`/`Leaf` (paragraph/section/tag boundary, or an inline
  `ROXYGEN_RD_MACRO` which binds tighter). A span thus **crosses a soft line break**
  (`*foo`\n`bar*` → one `\emph` over `foo bar`); the trivia present as **whitespace** for
  flanking (`edge_char` maps the `#'` marker to a space; newline/whitespace bytes already are)
  and pass through verbatim, landing *inside* the node when the span crosses a line.
  **Interior unmatched delim ≠ the span's own delimiters:** the projector skips only the
  **first and last** `MD_DELIM` child (opener/closer); an interior `MD_DELIM` is literal text
  (`_foo_bar_baz_` → `\emph` over `foo_bar_baz`). Losslessness via `Event::Leaf` run-splitting.
  **Formatter:** `collect_logical_elements` **descends into a cross-line EMPH/STRONG node**
  (one threading a `ROXYGEN_MARKER`, `is_cross_line_emph` — mirrors `is_block_macro`) so its
  delimiter/text leaves distribute across the physical lines and prose reflow rejoins them
  (`*foo`\n`bar*` → `*foo bar*`); a single-line span (no marker) stays atomic (glues as one
  chunk). Idempotent (flanking is invariant under whitespace normalization). Backlog toward
  full parity: links onto the same stack (slice 2, yields cross-line links), markdown
  `\`-escapes (diagnostic-parity). (Empty-list-item interrupt cm-369 closed 2026-06-25f — a list
  fix, see the List-markers trap. **Unicode flanking/NBSP closed 2026-06-25j:** the parser's
  `char::is_whitespace` flanking already handled NBSP; the gap was the projector's `norm_ws`
  folding NBSP to a space — now ASCII-`[[:space:]]`-only, preserving Unicode whitespace. See the
  Sections/projection trap on `norm_ws`.)
- **The oracle is roxygen2, NOT the CommonMark spec** (settled 2026-06-25b). roxygen2 *parses*
  via `cmark` (so parsing is faithful CommonMark) but always processes *through roxygen2*, which
  adds a markdown-escaping pre-pass, the `rdComplete` brace/quote **validation**
  (`warn_roxy_tag "has mismatched braces or quotes"`), and a *subset* Rd translation. So
  roxygen2's behavior is truth wherever it diverges from raw `cmark` — both render and reject.
  Never "CommonMark says X → arity does X"; only "roxygen2 does Y → arity does Y." The spec test
  set is an **input corpus only**; roxygen2 supplies every answer.
- **Diagnostic parity is a SECOND oracle surface** (settled 2026-06-25b). roxygen2 validates and
  emits source-located warnings, then **drops** the bad content (`\*not emphasis\*` → `✖ <text>:3:
  @description has mismatched braces or quotes` + empty `\description{}`; `rdComplete` in
  `tag-parser.R`). arity should detect the same condition and emit a **side-channel diagnostic**
  (CST stays lossless) — high-value lint + LSP signal, aligned with the deferred linter/LSP phases.
  An oracle-*error* input is a **diagnostic-parity fixture**, NOT a silent `blocked`. Three test
  outcomes: render-parity (allowlist/backlog), diagnostic-parity (record the exact oracle message),
  out-of-scope (`blocked` with reason — small for emphasis).
- **END GOAL = full CommonMark parity, nothing less** (tenet, settled 2026-06-25). roxygen2
  delegates to `cmark`/`cmark-gfm`; a "pragmatic subset" is a parity *gap*, never acceptable.
  The early inline recognizers are local line-scoped span scanners in the lexer
  (`scan_md_emphasis` etc.) — the **wrong shape**: CommonMark inline is a non-local whole-block
  **delimiter-stack** pass (block→inline). Agreed direction: a real **block→inline pass**
  (`docs/design/roxygen-inline-pass.md`); **emphasis migrates first** (decided), then links/code.
  Do **not** widen a local scanner with heuristics to chase a tricky case — that entrenches the
  wrong shape; land it in the inline pass or record it as backlog. A bail-to-literal is a stopgap
  (structure never *wrong*), never a target.
- **Mode resolved per-block** by `resolve_roxygen_block` (scans the `#'` run for `@md`/`@noMd`,
  default off; loose-file default-ON deferred), threaded as `md: bool` and **baked into leaf
  kinds** — the lexer is the *single* mode source. **Never re-derive `@md` in the block builder.**
- **Every *inline* recognizer MUST be `if md`-gated** (`*`/`_`/`` ` ``/`[`-link/`<`-autolink/
  `<`-html/list-marker/fence/image) — else its leaf kind stops implying `@md` and the projector
  mis-fires in non-`@md` blocks. (The `[`-link slipped this once; audit every new recognizer.)
- **Inline landed:** emphasis/strong (now the **real delimiter-stack pass**, see above),
  code (`\code`-vs-`\verb` per arity-parseability = roxygen2 `can_parse`), links, images, raw
  HTML. **Block landed:** lists
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
- **Inline `[text](url)` links are on the stack (2026-06-25h); ref/shortcut/autolink stay opaque
  leaves.** The lexer (`inline_link_span`, bracket-free text) splits an inline link into neutral
  `RoxygenMdBracket` leaves (`[` / `](url)`) and **recursively lexes the link text**; the inline pass
  (`Arena::build`) **collapses** the matched pair into an opaque `ROXYGEN_MD_LINK` **node** whose
  display children are resolved by a recursive `resolve_run` (bounded by the bracket chars for
  flanking) — so inner emphasis resolves *and* an outer span wraps the link (the Link node is opaque
  to the outer stack, like a Token). Projector: a `ROXYGEN_MD_LINK` **node** → `Inline::MdInlineLink`
  (skip first/last bracket child, recurse the middle), `inline_link_node_atom` GRP-wraps a multi-atom
  display (`\href` two-arg structural), `\url` on empty/equal dest. A `ROXYGEN_MD_LINK` **leaf** is
  still an autolink/ref/shortcut (`resolve_md_link`) — node-vs-leaf dispatch coexist. Bracket-free gate
  keeps the opaque path for nested-bracket text (no deactivation modeled yet).
- **Cross-line inline `[text](url)` links landed (2026-06-25i).** The lexer's same-line
  `inline_link_span` split is line-scoped, but `Arena::build` **already** pairs bracket opener/closer
  leaves over the whole **paragraph-granularity** run (cross-line trivia included) — so cross-line just
  needed the lexer to emit the **lone** brackets: `is_cross_line_link_opener` (a `[` with a
  bracket-free line-remainder, i.e. no same-line `]`) carves a `[` leaf; `cross_line_link_closer` (a
  bare `]` then a balanced `(url)`) carves a `](url)` leaf. The pair collapses into a line-spanning
  `ROXYGEN_MD_LINK` node; unmatched brackets fall back to literal text (cmark/roxygen2 faithful). **No
  new TokKind / inline-pass / projector change.** Formatter: `is_cross_line_emph`→`is_cross_line_inline`
  also matches a marker-threading `ROXYGEN_MD_LINK` node so reflow rejoins it (output byte-identical —
  structure-only; idempotent).
- **Cross-line *reference* `[text][ref]` links landed (2026-06-25k).** A reference link whose `[`
  opens on an earlier `#'` line collapses the same way as the inline form — `Arena::build` pairs the
  bracket leaves over the paragraph run — but the closer is `][ref]`, not `](url)`, and the lexer
  cannot disambiguate a cross-line closer from a stray `]`+shortcut (`a][b]`) **line-locally** (the
  lexer is line-scoped, no cross-line state). Fix is **correct-by-construction in the arena**: the
  lexer carves only the **lone `]`** as a neutral bracket leaf (`cross_line_ref_closer`: `]`
  immediately followed by a clean bracket-free `[ref]` shortcut, not followed by `(`/`[`/`{`), leaving
  the `[ref]` to `scan_md_link` as a normal shortcut `MD_LINK` leaf. `find_link_closer` then either
  pairs the `]` with an earlier `[` opener — folding the following `[ref]` label into the closer text
  as `][ref]` (consumed as the **dropped** topic) — or, **with no opener**, leaves the `]` literal
  (re-emitted `Delim`) and the `[ref]` a standalone shortcut. So `a][b]` stays `a]` + `\link{b}`
  with zero special-casing. Projector: a `ROXYGEN_MD_LINK` node whose closer is `][ref]` →
  `Inline::MdRefLink` → `ref_link_node_atom` (`\link{display}`, `\code`-wrapped iff display is a
  single code span, shortcut fallback when display==label; mirrors the opaque `ref_link_atom`). No new
  TokKind / SyntaxKind; formatter unchanged (already matches any marker-threading `ROXYGEN_MD_LINK`).
  Fixture `roxygen_md_ref_link_multiline` + curated `md_ref_link_multiline`. rx-eb12b6b6 closed.
- **Cross-line *shortcut* `[text]` links landed (2026-06-26).** A `[text]` whose `[` opens on an
  earlier `#'` line resolves into one `\link{text}` over the coalesced text. Line-locally every `]`
  is ambiguous (no cross-line state), so the disambiguation is **correct-by-construction in the
  arena**, exactly like the ref-link form: the lexer carves a lone `]` as a neutral bracket leaf
  whenever it is **not** an inline (`](url)`) or reference (`][ref]`) closer and **not** a non-link
  `]{…}` lookahead (`!matches!(bytes.get(i+1), Some(b'(' | b'[' | b'{'))`) — so it now carves
  *every* bare `]` in `@md` prose. `find_link_closer` pairs the lone `]` (no following `[ref]` label)
  with an earlier `[` opener as a **shortcut** closer (closer text just `]`), or — with no opener —
  the bare `]` re-emits as a literal `Delim` (a truly stray `]` is unchanged; `a]` stays `a]`). A
  *same-line* shortcut is still consumed whole by `scan_md_link`, so a `]` reaching the carve has no
  same-line opener. Projector: a `ROXYGEN_MD_LINK` node whose closer is `]` → `Inline::MdShortcutLink`
  → `shortcut_link_node_atom` (the display *is* the destination, so it mirrors `shortcut_link_atom` —
  `\link`/`\linkS4class` per `-class`/`pkg::`/`()`, `\code`-wrapped for a single code-span display).
  **No new TokKind / SyntaxKind; formatter unchanged** (`is_cross_line_inline` already matches any
  marker-threading `ROXYGEN_MD_LINK` node, so reflow rejoins it byte-identically). Note: carving
  every bare `]` changed the CST of `roxygen_md_escaped_bracket` (the `]` in `\[shortcut]` is now a
  standalone unmatched `Delim`) — projection unchanged, snapshot re-accepted. Fixture
  `roxygen_md_shortcut_link_multiline` + curated `md_shortcut_link_multiline`.
  **Still backlog:** escaped-*close*-bracket `[text\]` (roxygen2's synthesized-linkref quirk) and the
  `get_md_linkrefs` pre-pass / opener-deactivation full migration that would retire the opaque
  same-line `scan_md_link`.
- **Escaped brackets are the ONLY honored punctuation escape (2026-06-25l).** roxygen2's
  `double_escape_md` doubles every `\` but **reverts** `\\[`→`\[`, `\\]`→`\]`, so only `[`/`]` keep a
  CommonMark escape through cmark: `\[` neither opens a link **nor keeps its backslash** (`\[x](u)`→
  literal `[x](u)`), whereas `\*`/`` \` ``/`\%`/… **keep** their backslash (the doubling neutralizes
  the escape — and arity already matched those, so do **not** add general escape handling). Two-part,
  both faithful: lexer `bracket_is_escaped` (a `[` with preceding `\`) guards all three `[`-openers
  (`inline_link_span`/`is_cross_line_link_opener`/`scan_md_link`); projector `unescape_md_brackets`
  drops one `\` before `[`/`]` in `@md` text (`prose_text_atom` md branch). A *single* adjacent `\`
  already suppresses the link (oracle-verified 1–3 backslashes); deeper runs + escaped-*close*
  `[text\]` (roxygen2's synthesized-linkref quirk) stay backlog. **Probe escape cases with
  exact bytes (write the source to a file), never shell-quoted — `\\[` in a shell arg reaches R as
  two backslashes and masks the single-`\[` divergence.** Curated `md_escaped_bracket`.
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
  non-list marker reflows like plain text). `is_md_list_start` applies the CommonMark interrupt
  rule (mid-paragraph only): a bullet interrupts **unless the item is empty**
  (`md_list_item_is_empty`, cm-369 — a lone `*`/`-`/`+` after prose stays paragraph text, not a
  spurious `\itemize`); an ordered list interrupts only when start == 1; else stays inline prose →
  projector renders the leaf as text. **A fresh-position empty bullet still opens a list** (the
  empty-item gate is `para_open`-only). The lexer always carves the marker; emptiness is a
  block-level decision.

**Sections / projection**
- **`norm_ws` is ASCII-`[[:space:]]`-only, never Unicode-aware.** The R driver's `norm_ws`
  (`gsub("[[:space:]]+", " ")` + `trimws`) collapses *ASCII* whitespace only — NBSP `U+00A0`,
  NEL `U+0085`, and the `Zs` separators pass through verbatim. `project_rd::norm_ws` mirrors this
  via `is_posix_space` (` \t\n\x0b\x0c\r`); **do not** revert to Rust's Unicode-aware
  `split_whitespace`/`char::is_whitespace` (it folds NBSP→space, breaking flanking-rejected
  emphasis like `*\u{a0}a\u{a0}*`, cm-355). Flanking itself (`inline.rs`) *is* Unicode-aware
  (`char::is_whitespace`), so a NBSP correctly can't open/close a span — only the projection's
  text-coalesce needed the ASCII fix.
- **Non-md prose is literal Rd; an unescaped `%` is a comment to EOL.** The projector
  re-derives `@md` (`block_md`, mirrors `resolve_roxygen_block` — plain-text leaves carry
  *no* mode, so this is a separate, necessary re-derivation, **not** the block-builder
  anti-pattern the traps warn about) and, with md off, strips `%` line comments per
  *physical line* in `prose_text_atom`/`strip_rd_comments`. The inline-join sites
  (`paragraph_inlines` NEWLINE, tag→continuation, `section_body_parts`, `join_paras`) now
  carry breaks as `\n` (norm_ws-equivalent for non-`%` text) so the comment is line-scoped.
  `\%` survives (escape). Under `@md`, `%` is escaped (`\%`) → survives, so the strip is
  mode-gated off. **Formatter follow-on (open):** reflowing multi-line non-md prose joins
  lines across a `%`, changing rendered Rd (Tenet-1 bug) → curated cases stay single-line.
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

**Markdown = full CommonMark parity (end goal, settled 2026-06-25).** The markdown layer
targets *complete* CommonMark fidelity (roxygen2 delegates to `cmark`/`cmark-gfm`); a subset
is a gap, not an end state. The local lexer span-scanners (`scan_md_emphasis` etc.) are the
**wrong shape** — CommonMark inline is a whole-block **delimiter-stack** pass. The agreed
path is a real **block→inline pass** (`docs/design/roxygen-inline-pass.md`): a paragraph-level
inline pass inside `parse()` (salsa/incremental untouched) where the lexer emits *raw*
`RoxygenMdDelim` runs and the pass resolves them into `ROXYGEN_MD_EMPH`/`STRONG` **nodes** via
the delimiter-stack algorithm (full flanking, rule of 3, `process_emphasis`). **Slice 1 =
emphasis only** (links/code stay opaque local tokens, correct per CommonMark precedence);
**flanking = ASCII-class first** with a noted Unicode backlog. Then links move onto the same
stack — which also yields cross-line links for free.
**Driver = the real CommonMark spec test set** (settled 2026-06-25), adapted: panache compares
parser→HTML vs `expected_html`, but arity's target is the *Rd roxygen2 renders*, so we take the
spec's markdown **inputs only** and keep **roxygen2 as the oracle**. The spec becomes a **third
corpus source** for the existing projector gate (alongside curated + harvested): vendor
`spec.txt`, scope per slice (slice 1 = the ~132 "Emphasis and strong emphasis" examples), wrap
each into an `@md` block, mint Rd pins once (no R at test time), same allowlist/**blocked** (with
reason) discipline. roxygen2 models only a CommonMark *subset*→Rd and errors on some inputs →
those are `blocked`, never silenced. The user's hand-written "Complex Cases" list stays as
*curated* legible fixtures; the spec corpus is the breadth net.

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
they stay in the R↔R fixed-point net, not false-positive backlog). **A third source landed
2026-06-25c: the CommonMark spec emphasis corpus** (132 `cm-NNN` cases, the inline-pass
driver). Current (post cross-line shortcut links, 2026-06-26): **285 matching (all allowlisted), 22 divergent
(backlog)** of 307 pinned. Of the 22, 4 are remaining `cm-` cases (`\`-escapes-in-emphasis cm-439/442/451/454
= **diagnostic-parity**); the other 18 are roxygen2-*evaluation*/multi-block gaps (out of scope —
knitr `` `r …` ``/` ```{r} ` eval, RefClass docstrings, cross-block `@name`/reexport association).
Tasks:
`task roxygen-projector` (the gate), `task roxygen-projector-refresh` (re-mint all pins),
`task roxygen-projector-pins` (harvested pins), `task roxygen-spec-corpus`/`roxygen-spec-pins`
(spec corpus + pins), `task roxygen-projector-seed` (re-seed allowlist from matches).
Report: `ROXYGEN_PROJECTOR.md` (this dir).

**Three checks, three roles** (don't conflate):
1. **Projector parity** (`tests/roxygen_projector.rs`, pure Rust) — the **primary,
   parser-growth driver**. Compares Rd *structure*, so it sees block-structure gaps
   the fixed-point check is blind to. Curated + harvested + CommonMark-spec corpora
   (307 pinned cases). The 22 divergences are the worklist (4 = remaining `cm-`).
2. **Curated fixed-point** (`tests/roxygen_oracle.rs::roxygen_oracle_report`, needs R,
   `#[ignore]`d) — strict semantic preservation of the formatter; 24/24 preserving, 0
   blocked. *Meaning, not layout.*
3. **Harvested fixed-point** (`tests/oracle/corpus/roxygen.jsonl`, 217 cases, needs R,
   `#[ignore]`d) — broad **opt-in** backlog gated by `roxygen-allowlist.txt`
   (216 preserving, 0 divergent, 1 skipped). A coverage net, **not** the parser driver
   (it's cosmetic-blind + R-dependent). Reports: `task roxygen-oracle` /
   `task roxygen-harvest`.

## Latest session (2026-06-26) — Cross-line *shortcut* `[text]` links

**Under `@md`, a shortcut link `[text]` whose `[` opens on an earlier `#'` line now resolves into one
`\link{text}` over the coalesced text** (roxygen2 joins it across the soft break, exactly like the
same-line shortcut). This was the last cross-line link form; the inline `[text](url)` and reference
`[text][ref]` forms landed earlier (2026-06-25i/k). The hard part — flagged in the prior session's
ranked target — is that line-locally **every `]` is ambiguous** (the lexer is line-scoped, no
cross-line state). The disambiguation is **correct-by-construction in the arena**, mirroring the
ref-link fix:

- **Lexer (`lex.rs`):** carve a lone `]` as a neutral `RoxygenMdBracket` leaf whenever it is **not**
  an inline (`](url)`) or reference (`][ref]`) closer (handled by the preceding carves) and **not**
  a non-link `]{…}` lookahead — i.e. `bytes[i]==']' && !matches!(bytes.get(i+1), Some(b'(' | b'[' | b'{'))`.
  This now carves **every** bare `]` in `@md` prose. Safe because a *same-line* shortcut is consumed
  whole by `scan_md_link` first, so a `]` reaching the carve has no same-line opener.
- **Arena (`inline.rs::find_link_closer`):** a lone `]` with **no** following `[ref]` label now pairs
  with an earlier `[` opener as a **shortcut** closer (closer text just `]`); previously it returned
  `None`. With no opener the bare `]` re-emits as a literal `Delim`, so a truly stray `]` is
  unchanged (`a]` stays `a]`).
- **Projector (`project_rd.rs`):** a `ROXYGEN_MD_LINK` node whose closer is `]` → new
  `Inline::MdShortcutLink { display }` → `shortcut_link_node_atom` (the display *is* the destination,
  so it mirrors `shortcut_link_atom` — `\link`/`\linkS4class` per `-class`/`pkg::`/`()`, `\code`-wrapped
  for a single code-span display). Node-closer dispatch is now three-way (`](url)`/`][ref]`/`]`).

**No new TokKind / SyntaxKind. Formatter unchanged** — `is_cross_line_inline` already matches any
marker-threading `ROXYGEN_MD_LINK` node, so reflow rejoins the cross-line shortcut byte-identically
(idempotent, verified by the format-stability baseline: +1 new case, **no existing case drifted**).
**Side effect:** carving every bare `]` changed the CST of the existing `roxygen_md_escaped_bracket`
fixture (the `]` in `\[shortcut]` is now a standalone unmatched `Delim` instead of buried in a text
run) — **projection unchanged** (still literal `[shortcut]`), snapshot re-accepted.

**TDD:** parser fixture `roxygen_md_shortcut_link_multiline` (cross-line shortcut + a contrasting
stray `a]`; lossless, CST shows the `ROXYGEN_MD_LINK` node + a standalone `Delim` `]`) + curated
projector case `md_shortcut_link_multiline` (pinned, ratcheted in) + 2 projector unit tests.

**Result:** projector **284→285 matching (all allowlisted), 22 divergent unchanged** (1 new curated
pin; 307 pinned). The 22 backlog is untouched (18 out-of-scope `rx-` + 4 `cm-` escape-in-emphasis).
Curated fixed-point **24/24** preserving, 0 blocked; harvested 216 preserving, 0 divergent. `cargo
test` green, clippy + fmt clean.

**Next (ranked):** **(1)** Escaped-*close*-bracket `[text\]` (still backlog — trips roxygen2's
synthesized-linkref quirk `[text\]: R:…`, so probe the exact oracle shape with exact-byte files
first). **(2)** The `\`-escapes-in-emphasis diagnostic-parity surface (cm-439/442/451/454): roxygen2
runs `rdComplete` on the markdown-rendered Rd and **drops** the section on failure — meaty,
design-level, needs a side-channel diagnostic (**user check before starting**). **(3)** The full
`get_md_linkrefs`/opener-deactivation migration retiring the opaque same-line `scan_md_link` (would
also subsume the remaining shortcut/escape edges into the unified bracket-stack).

## Earlier sessions

- **2026-06-25l (Escaped square brackets `\[`/`\]` are literal):** under `@md` a backslash-escaped
  `\[`/`\]` no longer opens a link, and the projected literal drops one backslash (`\[text](url)` →
  `[text](url)`). Brackets are the only punctuation whose CommonMark escape roxygen2 honors
  (`double_escape_md` reverts `\\[`→`\[`). Lexer `bracket_is_escaped` guards the three `[`-openers;
  projector `unescape_md_brackets` drops one `\` before `[`/`]`. **Trap:** probe escape cases with
  exact-byte files, never shell-quoted. Fixture `roxygen_md_escaped_bracket` + curated. 283→284.
- **2026-06-25k (Cross-line *reference* `[text][ref]` links, rx-eb12b6b6):** a `[text][ref]`
  whose `[` opens on an earlier `#'` line resolves into one cross-line `\link{text}` (topic
  dropped). The `][ref]` closer is byte-identical to a stray `]`+shortcut line-locally, so
  disambiguation lives in the **arena**: the lexer carves the lone `]` (`cross_line_ref_closer`),
  `find_link_closer` pairs it with an earlier opener (folding `[ref]` as the dropped topic) or
  leaves both literal (`a][b]`→`a]`+`\link{b}`). Projector node arm branches on the closer
  (`MdRefLink`/`ref_link_node_atom`). 281→283; cm 128/132.

- **2026-06-25j (Unicode NBSP flanking, `norm_ws` ASCII-only, cm-355):** a NBSP (`U+00A0`) is Unicode
  whitespace, so `*\u{a0}a\u{a0}*` can't flank (parser already right — leftover `MD_DELIM`); the
  divergence was the projector's `norm_ws` folding NBSP→space via Rust's `split_whitespace`. Rewrote
  `norm_ws` to the C-locale POSIX `[[:space:]]` set (`is_posix_space`, ASCII-only), preserving every
  non-ASCII Unicode whitespace — faithful to the R driver. Fixture `roxygen_md_emphasis_nbsp`. 280→281.
- **2026-06-25i (Slice 2.5 — cross-line inline links `[text](url)`, rx-383f2ca3):** an inline
  `[text](url)` whose `[`…`](url)` spans a soft line break resolves into one cross-line
  `ROXYGEN_MD_LINK` node (whitespace coalesced, inner emphasis crosses too). `Arena::build`
  **already** pairs bracket opener/closer over the paragraph-granularity run; the only gap was the
  lexer never emitting the lone brackets. Two `@md` lexer carves after `inline_link_span`: a
  cross-line opener (`is_cross_line_link_opener`, bracket-free line-remainder → lone `[` leaf) and a
  cross-line closer (`cross_line_link_closer`, bare `]` + balanced `(url)` → `](url)` leaf). No new
  TokKind / inline-pass / projector change; formatter `is_cross_line_emph`→`is_cross_line_inline`
  also matches a marker-threading Link node (byte-identical output). Fixture
  `roxygen_md_link_multiline` + curated `md_link_multiline`. 278→280; cm 127/132.

- **2026-06-25h (Slice 2 — inline `[text](url)` on the stack, cm-421/435):** the lexer splits a
  **same-line** bracket-free inline link into neutral `RoxygenMdBracket` leaves (`[` / `](url)`) and
  recursively lexes the link text; the inline pass collapses the matched pair into an opaque
  `ROXYGEN_MD_LINK` **node** whose display resolves recursively, so inner emphasis resolves *and* an
  outer span wraps the link. Projector node arm GRP-wraps a multi-atom display (`\href` two-arg),
  `\url` on empty/equal dest. Fixture `roxygen_md_link_emphasis` + curated `md_link_emphasis`.
  278 matching; cm 127/132.

- **2026-06-25g (`_`-leading code span is `\verb`, not `\code`, cm-481):** a markdown code span whose
  content begins with `_` renders `\verb` (R's lexer rejects a `_`-leading name; arity's is lenient).
  Pure **projector** nuance: `has_invalid_underscore_name` screens it out in `code_span_is_r` (a lone
  `_` stays valid as the native-pipe placeholder, gated on a `|>` present). 274→275; cm 124→125/132.

- **2026-06-25f (Empty list item can't interrupt a paragraph, cm-369):** a lone `*`/`-`/`+` with no
  content no longer opens a spurious one-item list mid-paragraph (CommonMark: an empty item can't
  interrupt). New `md_list_item_is_empty` in `build.rs` gates `is_md_list_start`; the `*` folds into
  the paragraph as a literal `ROXYGEN_MD_LIST_MARKER` and the emphasis pass leaves `*foo bar *`
  literal (trailing `*` preceded by a soft break can't close). Parser-only; fixture
  `roxygen_md_empty_list_item` + curated `md_empty_list_item`. 272→274; cm 123→124/132.

- **2026-06-25e (Inline-pass slice 1.5 — paragraph-granularity runs, cross-line emphasis):**
  widened the emphasis delimiter stack from line-scoped to paragraph-scoped. `inline.rs::resolve_emphasis`
  now pushes **every** paragraph-body `Event::Tok` (was: only `Content`), so inter-line trivia joins
  the run and a `*`/`**` span resolves across a soft line break (`*foo`\n`bar*` → one `\emph`). New
  `edge_char` maps a `#'` marker neighbor to whitespace for flanking. Formatter `collect_logical_elements`
  descends into a cross-line EMPH/STRONG node (`is_cross_line_emph`) so reflow rejoins it; single-line
  spans stay atomic. Projector unchanged. Fixtures `roxygen_md_emphasis_multiline` (+ reflow). 267→272;
  cm 119→123/132.

- **2026-06-25d (Inline-pass slice 1 — the real emphasis delimiter stack, parser + projector):**
  replaced the local `scan_md_emphasis` forward-scan with a faithful cmark `process_emphasis`. Lexer
  carves `*`/`_` as neutral `RoxygenMdDelim` runs; new `inline.rs::resolve_emphasis` builds an arena +
  delimiter stack (full ASCII flanking, rule of 3, nesting, partial-run consumption) → `ROXYGEN_MD_EMPH`/
  `STRONG` **nodes** (90/91, now nodes) with `ROXYGEN_MD_DELIM` (101) opener/closer/leftover leaves.
  Projector recurses, skipping only first/last `MD_DELIM` (interior = literal). Formatter unchanged
  (line-scoped ⇒ single-line nodes glue atomically). Projector 205→267, cm 58→119/132. Curated fixture
  `roxygen_md_emphasis`.

- **2026-06-25c (CommonMark spec emphasis corpus wired as 3rd projector source, test-infra only):**
  vendored `spec.txt` (0.31.2), `scripts/build-commonmark-corpus.R` extracts the 132 "Emphasis"
  examples → `commonmark-emphasis.jsonl`, `evaluate_jsonl_corpus` generalized to drive it; pins via
  the reused `projector-pins` op. 58/132 passed the interim scanner (seeded). Projector 147→205
  matching, 20→94 divergent (the 74-case `cm-` worklist this slice-1 session closed to 13).

- **2026-06-25b (markdown CommonMark-parity tenet + inline-pass design, no parser change):** the
  direction-setting session. Diagnosed `scan_md_emphasis` as the **wrong shape** (atomic local
  forward-scan → nesting unrepresentable, rule-of-3/flanking wrong, line-scoped). Settled the tenet
  (full CommonMark parity, nothing less), authored `docs/design/roxygen-inline-pass.md` (real
  block→inline delimiter-stack pass), and clarified the two oracle surfaces (render parity +
  diagnostic parity; roxygen2 is THE oracle, spec is inputs only). Decisions: slice 1 = emphasis
  only, flanking ASCII-first. Docs/framing only; projector held 147/167.

- **2026-06-25 (formatter `%`-reflow follow-on, formatter-only):** the paired Tenet-1 bug to
  2026-06-24u — reflowing multi-line **non-md** prose joined text across a live `%` comment,
  changing rendered Rd. Fixed by mode-gating reflow: `ir_roxygen_block` re-derives `@md`
  (`block_md`, the formatter's own copy) and a non-md `Paragraph`/`TagUnit` carrying a live `%`
  (`line_has_live_rd_comment`, escape-aware) bails to verbatim marker-normalized lines (same
  shape as `is_unsafe_line_start`). Under `@md` the `%` is escaped → reflow proceeds. Fixtures
  `roxygen_bail_rd_comment`, `roxygen_tag_bail_rd_comment`, `roxygen_rd_comment_md_reflows`.
  Curated 16/16 + harvested 216 still preserving, 0 regressions. Projector unchanged (147/167).

- **2026-06-24u (non-md Rd `%` line comments, projector + encoding):** in non-markdown
  prose the value is literal Rd, so an unescaped `%` is a comment to EOL (`@format %` →
  empty `\format`). Projector re-derives `@md` (`block_md`) and, md off, strips `%` per
  physical line (`strip_rd_comments`/`strip_rd_line_comment`, `\%` survives); the four
  inline-join sites emit `\n` (norm_ws-equivalent) to line-scope the comment. +2
  (rx-f6927028 + curated `rd_comment`) + 4 unit tests. 145→147; curated 15→16/16. Its
  formatter follow-on is this 2026-06-25 session.
- **2026-06-24t (mid-prose `\preformatted` opener, block-opener Form C):** a
  `\preformatted{ … }` opener appearing **mid-prose** (`So far so good. \preformatted{`).
  Lexer always splits an unbalanced `\name{` to its own to-EOL token; grouper
  (`emit_prose_line`) promotes it to an **inline** `ROXYGEN_RD_MACRO` *only if it closes*
  (`block_macro_opener_closes`), else stays prose. Formatter prepends `#' ` to a markerless
  opener (lossless + idempotent), fixing a Tenet-1 reflow violation. Parser + formatter;
  projector unchanged. + fixture `roxygen_preformatted_midline`. rx-0a1710c0. 144→145.
- **2026-06-24s (verbatim `\preformatted` block → per-line VERB):** a *line-start*
  multi-line `\preformatted{ … }`. Pure **projector** gap — `serialize_macro` early arm
  `head == \preformatted` → `preformatted_atoms` (verbatim per-line `VERB`, not norm_ws-
  collapsed prose). + curated `preformatted`. 143→144.
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
