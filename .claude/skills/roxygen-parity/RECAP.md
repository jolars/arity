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
- **Same-line plain-text *shortcut* `[text]` is now a node too (2026-06-29e, opener-deactivation slice A).**
  `same_line_shortcut_opener` (`lex.rs`) carves a `[` opening a balanced, **bracket-free, plain-text** (no
  `* _ ` ` ` < ! \\`) `[…]` whose after-`]` ∉ `(`/`[`/`{` and which is **not preceded by `]`** as a neutral
  bracket opener → arena pairs it (bare-`]` closer) → `MdShortcutLink`. **Behavior-preserving:** plain
  interior = one `Inline::Text` whose text == raw interior, so node `shortcut_link_node_atom →
  shortcut_link_atom(text)` == leaf `resolve_md_link → shortcut_link_atom(text)` (same fn/string); poisoning
  arms already cover the node form. **Marked-up displays now also nodeify (2026-06-29k, see below)** — the
  gate carves `* _ ` ` ` <` displays too (only `!`/`\` plain-text displays stay opaque), so the inline pass
  resolves their children and `link_display_is_droppable` drops the non-plain ones. The `!preceded-by-]`
  guard keeps a cross-line `[ref]` *label* on `scan_md_link` so the arena's `][ref]` fold still sees its
  token. **Same-line markup *reference* `[*foo*][ref]` now also nodeifies (2026-06-29l)** — the sibling
  `same_line_ref_opener` (`lex.rs`) carves a `[` whose markup display (`* _ ` ` ` <`, not `!`/`\`) is followed
  by a clean `[ref]` (`cross_line_ref_closer(close-1)`); the opener is the **only** new carve — the existing
  line-agnostic `cross_line_ref_closer` (lone `]`) + opaque `scan_md_link` (`[ref]`) + arena `classify_closer`
  (`][ref]` fold) + `link_display_is_droppable` do the rest. **Plain** `[plain][ref]` stays opaque
  (byte-identical), so the markup-only gate keeps the diff tiny. **Slice B (remaining):**
  plain same-line `[t]`/`[t][r]` + nested-inline still opaque (the `][ref]` fold couples to the opaque
  `[ref]` token); fully retiring `scan_md_link` needs the arena `look_for_link_or_image` rewrite to move
  references onto the lookahead. **Still opaque on purpose:** a URL-defined ref `[*foo*][r]`+`[r]: url` →
  `\href{url}{markup}` (markup kept) — arity models ref *labels*, not *destinations*, so this is the next
  target, not yet handled. See `~/.claude/plans/luminous-zooming-toast.md`.
- **Arena now does CommonMark opener deactivation; nested links resolve inner-first (2026-06-29f, slice B core).**
  `match_brackets` (`inline.rs`) replaced the forward `find_link_closer`: a **stack** pairs each `]` to the
  nearest *active* `[`, a formed link **deactivates every opener below it**, a lone `]` does the `][ref]`
  label lookahead and is a shortcut only on a **bracket-free** interior. So `[a [b] c](url)` → inner `[b]`
  is an `MD_LINK` node, outer `[`/`](url)` stay literal `MD_DELIM` (CommonMark-faithful). The lexer's
  `is_nested_bracket_opener` carves the outer `[` of a nested group (so its brackets reach the arena);
  non-nested still routes through the existing recognizers + opaque `scan_md_link`. A matched link's body
  provably has **no nested matched link** (it would have deactivated this opener), so the recursive
  body-resolve only sees emphasis/literal-brackets. **Formatter:** a nested link is **no longer atomic** —
  the formatter reflows within its literal portions (`md_linkref_poisoning_nested_link` re-blessed); still
  Tenet-1 faithful (fixed-point 36/36).
- **Arena resolves links OPTIMISTICALLY (all shortcuts live); poisoning is repaired in the projector.** The
  arena can't see the `get_md_linkrefs` refmap, so it always makes the inner shortcut win. For a *poisoned*
  nested link (inner de-linked), `relink_demoted_inline_links` (inside `demote_poisoned_links`) re-forms the
  enclosing `[…](url)` `\href` from the **demoted bracket text**. The **consecutive-`Inline::Text`** scan is
  the scoping trick: a *surviving* inner link node interrupts the run, so only the poisoned case (inner
  demoted to text) re-links; an escaped `\[` keeps its backslash and is skipped (never relinks). Knife-edge:
  poisoned `[a [b] c](url)` → outer `\href` + leaked `[b]: R:b`; non-poisoned → inner `\link{b}`.
- **The link-reference map is modeled; an undefined shortcut/ref stays literal (2026-06-29j).** roxygen's
  `get_md_linkrefs` `(?<!\])` lookbehind blocks reference-**definition** creation for a `[` immediately after
  `]` (and `(?=[^\[{])` for one before `[`/`{`), but link **resolution** still needs the refmap — so `a][b]` /
  `[a [b] c][ref]` *standalone* render **all literal** (no def for `b`/`ref`), yet link when the label is
  defined elsewhere (`md_ref_link_multiline`'s `a][b]` works because a later `[b]` defines it). The arena still
  links optimistically; the **projector** now demotes it: `linkref_keys(body)` builds the refmap from a faithful
  raw-source reconstruction (`linkref_source_skeleton` — re-exposes every link/image bracket, recursing emphasis
  between space guards, the opaque leaf verbatim; unlike `inline_source_skeleton` which hides resolved
  shortcuts) scanned by the existing `md_linkref_scan`; `demote_undefined_links` rewrites any shortcut/ref link
  whose **normalized** label (`normalize_linkref_label`: trim + ws-collapse + lowercase) ∉ refmap to its literal
  bracket source (`demoted_link_source`), running **before** the positional poison demotion in
  `serialize_prose_with_linkrefs`. The two compose (refmap gap = never a candidate; poison = valid candidate
  whose def leaks), both monotonic. **Full refmap = full candidate set** (not poison-boundary-limited): that is
  what keeps `md_ref_link_multiline`'s `a][b]` linking. **Distinct, still-open:** (1) the refmap is per-*prose-body*,
  not whole-*field* — a label defined in a sibling paragraph/list of the same tag is missed (backlog); (2) a
  shortcut inside a code span / right-after-`]`-inside-emphasis is a reconstruction edge (MdCode → space, emphasis
  markers dropped); (3) the **non-plain shortcut** `[*foo*]` is a *separate* mechanism (landed 2026-06-29k, below)
  — roxygen synthesizes the def, links the shortcut, then `parse_link` rejects "links must contain plain text" and
  drops **the link** (not the section) to empty. Curated `md_undefined_shortcut` (all-literal `a][b]`),
  `md_undefined_ref` (`[a [b] c][ref]` → inner `\link{b}`, outer literal).
- **User link-reference definitions (`[ref]: url`) → `\href{url}{display}`, display KEPT (2026-06-29m).** A
  CommonMark def gives a referencing shortcut/reference link a real destination, so it renders `\href` (not the
  R-topic `\link`, so the "must contain plain text" drop does **not** apply — markup display survives), and the
  def line is **consumed** (renders nothing). User def beats roxygen's synthesized `[ref]: R:ref` (cmark keeps
  the first def). Projector-only: `resolve_user_linkrefs` (in `serialize_prose_with_linkrefs`, before
  `demote_undefined_links`, on the **original** body so the refmap is unaffected) builds a label→url map via
  `collect_user_linkrefs`/`scan_linkref_run` (a def run is consumed only at a **block start** — body start or
  after a `Text` containing `\n` — since a def *cannot interrupt a paragraph*; leading-indent + soft-break-
  separated stacked defs are tolerated and dropped), then **rewrites** each defined-label link to
  `Inline::MdInlineLink{url, display}` (reusing the `\href` rendering). `parse_linkref_def_dest` handles bare or
  `<…>` dests + optional same-line title (trailing non-title content ⇒ not a def). Returns `None` (no change)
  when no def → existing cases byte-identical. **Backlog:** multi-line defs, URL normalization, whole-*field*
  refmap (sibling list-item / `@section`-body defs), and `@section`'s own arm (no `serialize_prose_with_linkrefs`
  call). **Formatter follow-on (deferred, RECAP next #1):** the formatter joins consecutive `#'` def lines into
  one → invalidates them as CommonMark defs → changes rendered Rd. Curated `md_url_reference` dodges it with
  **blank-line-separated** defs (format-stable, like the `%`-comment cases); the consecutive-def projector path
  is unit-tested (no formatter). Curated `md_url_reference`.
- **A shortcut/reference link with a non-plain display is DROPPED to empty (2026-06-29k).** roxygen2's
  `parse_link` (`markdown-link.R`): after unwrapping a *sole* `code` child (which links — `\code{\link{…}}`), if
  any display child is not text/softbreak/linebreak it `warn`s "markdown links must contain plain text" and
  `return("")` — the link vanishes, surrounding prose stays contiguous (`x [*foo*] y` → one `(TEXT "x y")`). Drops:
  emphasis (`[*foo*]`/`[_a_]`), a *second* code span (`` [`x` `y`] ``), text+code (`` [a `b`] ``), an autolink
  (`[<url>]`), image, HTML. Keeps: pure text (`[a_b]`: intraword `_` is *not* emphasis — needs real flanking, so a
  char-scan won't do), sole code span. Two-part fix: (a) lexer `same_line_shortcut_opener` now carves `* _ ` ` ` <`
  displays as arena bracket pairs (only `!`/`\` — always plain text — stay on the opaque `scan_md_link` leaf), so
  the inline pass resolves the display *children*; (b) projector `link_display_is_droppable(display)` (sole-code
  unwrap, then all-`Inline::Text`) drops the `MdShortcutLink`/`MdRefLink` node in `serialize_inlines` via
  `continue` **without flushing the text run** — the dropped link is transparent so the prose coalesces (mirrors
  roxygen2's `""` concatenation). Inline `[text](url)` never drops (own dest → `\href`). Refmap-safe: the node's
  candidate label (`linkref_source_skeleton`) and resolution label (`link_ref_label`) both use
  `inline_plain_text(display)`, so it stays self-consistently *in* the refmap (never spuriously demoted) and
  reaches serialize to be dropped. **Same-line *reference* `[*foo*][ref]` stays opaque → backlog** (needs the
  `][ref]` same-line carve, slice B). Fixture `roxygen_md_shortcut_emphasis` + curated `md_shortcut_emphasis`.
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
- **`get_md_linkrefs` leaks invalid synthesized link-ref defs, whole-field poisoning (2026-06-26c).**
  roxygen2's `add_linkrefs_to_md` (`markdown-link.R`) appends `[label]: R:URLencode(label)` for **every**
  bracket-free `[…]` shortcut candidate, as **one cmark block** (`paste0(text,"\n\n",refs,"\n")`, one
  line per candidate, source order), parsed top-down. A *valid* def is consumed (shortcut → link, arity
  makes the `\link` directly); an **escaped-close** candidate `[text\]` yields a def whose label never
  closes (its `\]` doesn't close, the label runs into the **next** line's `[` which is illegal inside a
  label) → that def *and every def after it* fail, so cmark leaks the block **from the first invalid
  candidate to the end** (valid candidates included), and any shortcut/reference link in that tail is
  **de-linked**. `leaked_linkref_text` (projector, `@md`-only, from `push_section`): `double_escape_md`
  →`md_linkref_labels`→take `[first_invalid..]` (was: filter `!closes`)→`url_encode`→`cmark_unescape`.
  De-linking is done **upstream** by `demote_poisoned_links`: it locates the poison boundary on the body
  skeleton (`first_invalid_linkref_offset` — **any trailing backslash = invalid**, since `double_escape_md`
  makes a `k≥1` run odd `2k-1`) and rewrites the tail's shortcut/reference link nodes (`MdShortcutLink`/
  `MdRefLink`/opaque-shortcut/ref `MdLink`) to literal bracket text *before* the skeleton is rebuilt — so
  they reappear as candidates and their now-leaked defs surface naturally. **Inline links/autolinks/code
  survive** (own destination, no def needed; `demoted_link_source` returns None). **Probe with exact-byte
  files** (`\]` in a shell arg masks the case). Curated `md_escaped_close_bracket` (all-invalid),
  `md_linkref_poisoning` (mixed). **Inline-link candidate defs now leak too (2026-06-26e):** roxygen2's
  `get_md_linkrefs` also synthesizes `[text]: R:text` for an inline `[text](url)` link (its `[text]` is a
  bracket-free candidate followed by `(`, lookahead-allowed), so in a poisoned tail that def leaks even
  though the `\href` survives. The skeleton exposes the link via `inline_skeleton_fragment` (single source
  shared by `inline_source_skeleton` + `skeleton_len`): an `MdInlineLink` contributes `[text] `; the link
  is **not** demoted. Curated `md_linkref_poisoning_inline_link`. **Image alt-text defs now leak too
  (2026-06-29):** an image `![alt](url)`'s `[alt]` is a bracket-free candidate as well (`[` preceded by
  `!`, allowed; followed by `(`, lookahead-allowed), so its `[alt]: R:alt` def leaks in a poisoned tail
  even though the `\figure` survives. `inline_skeleton_fragment`'s `MdImage` arm contributes `[alt] `
  (`image_alt_text` extracts the literal alt span via `scan_delimited`); the image is **not** demoted.
  Curated `md_linkref_poisoning_image`. **Opaque nested-bracket inline-link inner candidates now leak too
  (2026-06-29b):** a nested-bracket display `[a [b] c](url)` keeps the inline link an **opaque** `MdLink`
  leaf (the lexer only nodes a *bracket-free* display), yet `get_md_linkrefs` still finds the *inner*
  bracket-free `[b]` candidate (the outer `[a [b] c]` is not one — its content has brackets). New
  `opaque_inline_link_display` (display verbatim iff a balanced `[…]` is followed by `(`, else None for
  shortcut/ref/autolink) drives an `MdLink` arm in `inline_skeleton_fragment` → `[a [b] c] ` (space for the
  consumed `(url)`); the link is **not** demoted. **Autolink-adjacent was already correct** (`<url>` carries
  no `[…]` candidate → a single space is faithful; confirmed by curated `md_linkref_poisoning_autolink`,
  which passed unchanged). Curated `md_linkref_poisoning_nested_link`. **Still backlog:** `@rawRd` leaks
  (never markdown today — a parser-side gap).
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
  curate such a case). **`@rawRd` is never markdown (closed 2026-06-29c):** roxygen2
  uses `tag_value`, not `tag_markdown`, so a `[bracket]`/`*star*` in the body is
  literal Rd even under `@md`. The lexer keys this per-tag — the `lex()` driver tracks
  a raw-Rd region (`roxygen_line_tag` + `is_raw_rd_tag`, `"rawRd"` only; reset per
  block, re-keyed when a line opens a tag) and lexes those lines with `md=false`, so
  the body carries no md leaves and `serialize_inlines` projects literal text. (`@evalRd`/
  `@usage` share the non-markdown semantics but are out of the projector's scope.)
- **A prose section whose trimmed value is literal `"NULL"` is suppressed** (`rd_section()`
  sentinel; `NULL_SUPPRESSIBLE`). `@section` (title+body pair) is NOT suppressed; a suppressed
  `@description NULL` re-fires the title fallback. Data-object auto-`\format` (roxygen2 *evaluates*
  the object) is **out of scope**.
- **`@description`/`@details` drop to empty on a brace-incomplete render** (2026-06-29d). roxygen2's
  `markdown_if_active` runs `rdComplete(rendered, is_code=FALSE)` per section for the `sections=TRUE`
  tags (`@description`/`@details`, incl. intro paras re-emitted by `parse_description`) and replaces
  the body with `""` on a brace imbalance (`R/markdown.R`, `src/isComplete.cpp`). Projector replicates
  it (`push_section(drop_on_incomplete=true)` → `section_atoms_rd_complete`): `sexpr_to_rd` rebuilds
  the **pre-parse** Rd from the S-expr atoms (node = `\m{c}…` balanced by construction, so imbalance
  comes only from leaf text — the trailing-`\` bug `*\**`→`\emph{\}`), then `rd_complete` (verbatim
  port). **Critical:** re-escape `%`→`\%` for **every** leaf in `@md` (`escape_percent = md`) — roxygen2
  escapes `%` everywhere in md render, so a `%20` URL must not comment-out the closing braces (else a
  false drop). Literal `{`/`}` in md text are **not** escaped (they count). **The drop rule is
  mode-dependent** (`push_section`'s `check_drop = if md { drop_on_incomplete } else { true }`,
  2026-06-29g): with md **on**, only the `sections=TRUE` tags (`drop_on_incomplete`) drop, others
  (`tag_markdown`, `sections=FALSE`: `@title`/`@seealso`/`@note`/…) don't; with md **off**,
  `markdown_if_active`'s else-branch runs `rdComplete(text)` **unconditionally**, so *every* prose
  section `push_section` emits (title included) drops to empty on imbalance regardless of
  `drop_on_incomplete`. `section_atoms_rd_complete` works for md-off too (the `%`-comment strip on
  non-md prose removes only comment-state chars, so brace balance is preserved; `escape_percent=md`=false
  is correct). **`@field`/`@slot` whole-tag drop landed 2026-06-29h**; **`@section` md-off drop landed
  2026-06-29i** (bullets below).
- **`@section` drops to `(\section (TEXT "NA"))` on a md-OFF raw brace imbalance (2026-06-29i).**
  `@section` uses plain `tag_markdown` (`sections = FALSE`) — NOT `tag_markdown_with_sections` — so under
  `@md` the per-section `rdComplete` drop **never fires** (an imbalanced md-on `@section` renders the
  content as-is, producing broken Rd → not curatable). But with md **off**, `markdown_if_active`'s
  else-branch runs `rdComplete(x$raw)` **unconditionally** on the whole `title: body` value and replaces
  it with `""` on imbalance; `roxy_tag_rd` then `str_split("", ":", n=2)` → `title=""`, `content=NA`,
  rendering `\section{}{NA}` → `(\section (TEXT "NA"))` (the literal R "NA" from `paste0(…, NA, …)`; the
  empty `{}` title coalesces away). Same raw-source `rd_complete` as `@field`/`@slot` (no `{}\%` in the
  scaffolding, quotes ignored), so the guard sits in `project_block`'s `"section"` arm
  (`if !md && !rd_complete(section source)` → push the NA section). `%` is an active comment in the raw
  (`body %{` survives, `body{ %x` drops). Curated `rdcomplete_section_drop`.
- **`@field`/`@slot` drop the WHOLE tag on a raw brace imbalance (2026-06-29h), mode-independent.**
  roxygen2 parses them via `tag_two_part`, which runs `rdComplete(x$raw, is_code = FALSE)` on the
  **raw** tag value (`name + description`, *before* markdown) and returns `NULL` on a brace imbalance —
  so a bad `@slot`/`@field` contributes **no** `\describe` item, and an all-dropped Slots/Fields
  aggregate emits no section at all (vs `push_section`'s empty-`(\macro)` for `@description`/`@details`).
  **Key simplification:** `rdComplete(is_code=FALSE)` tracks only `{` `}` `\` `%`(line-comment) `\n` —
  it **ignores quotes** (`"` `'` `` ` ``), and none of `{}\%` appear in the `#'`/`@slot` scaffolding,
  so `rd_complete(section.syntax().text())` scans **identically** to roxygen2's `x$raw`. No source
  reconstruction needed; reuse the existing `rd_complete` port on the raw section text. The raw is
  pre-markdown, so `%` is an active comment **regardless of `@md`** (`@slot a a %{` survives — `%`
  comments the `{`; `@slot a a{ %20` drops — `{` precedes the `%`). The gate sits in `project_block`'s
  `"slot" | "field"` arm (`continue` on incomplete). Curated `rdcomplete_slot_drop` (partial: one bad
  slot dropped, one survives), `rdcomplete_field_drop` (all-bad → no Fields section).

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
driver). Current (post URL-defined reference links, 2026-06-29m): **311 matching (all allowlisted), 18
divergent (backlog)** of 329 pinned. The 18 left are all roxygen2-*evaluation*/multi-block gaps (out
of scope — knitr `` `r …` ``/` ```{r} ` eval, RefClass docstrings, cross-block `@name`/reexport association).
Tasks:
`task roxygen-projector` (the gate), `task roxygen-projector-refresh` (re-mint all pins),
`task roxygen-projector-pins` (harvested pins), `task roxygen-spec-corpus`/`roxygen-spec-pins`
(spec corpus + pins), `task roxygen-projector-seed` (re-seed allowlist from matches).
Report: `ROXYGEN_PROJECTOR.md` (this dir).

**Three checks, three roles** (don't conflate):
1. **Projector parity** (`tests/roxygen_projector.rs`, pure Rust) — the **primary,
   parser-growth driver**. Compares Rd *structure*, so it sees block-structure gaps
   the fixed-point check is blind to. Curated + harvested + CommonMark-spec corpora
   (329 pinned cases). The 18 divergences are the worklist (all roxygen2-eval/multi-block, out of scope).
2. **Curated fixed-point** (`tests/roxygen_oracle.rs::roxygen_oracle_report`, needs R,
   `#[ignore]`d) — strict semantic preservation of the formatter; 46/46 preserving, 0
   blocked. *Meaning, not layout.*
3. **Harvested fixed-point** (`tests/oracle/corpus/roxygen.jsonl`, 217 cases, needs R,
   `#[ignore]`d) — broad **opt-in** backlog gated by `roxygen-allowlist.txt`
   (216 preserving, 0 divergent, 1 skipped). A coverage net, **not** the parser driver
   (it's cosmetic-blind + R-dependent). Reports: `task roxygen-oracle` /
   `task roxygen-harvest`.

## Latest session (2026-06-29m) — URL-defined reference links (`[ref]: url` → `\href`)

**RECAP ranked next #1.** A user CommonMark link-reference definition (`[ref]: url`) gives a referencing
shortcut/reference link a real destination, so roxygen2 renders `\href{url}{display}` with the **display
kept** (the "must contain plain text" drop is `\link`-only — `\href` carries its own dest). The definition
lines are **consumed** by cmark (render nothing). The user def wins over roxygen2's synthesized `[ref]: R:ref`
(cmark keeps the first def; the synthesized block is appended last). Probed boundaries: markup display
`[*foo*][r1]`→`\href{url}{\emph{foo}}`, plain `[plain][r2]`→`\href{url}{plain}`, code `` [`x`][r3] ``→
`\href{url}{\code{x}}`, bare shortcut `[r1]`→`\href{url}{r1}`; `<url>` brackets stripped, title ignored;
**a definition cannot interrupt a paragraph** (a `[r1]: url` line with no preceding blank stays literal
prose + R-topic `\link`).

**Fix (projector-only, ~120 lines).** New `resolve_user_linkrefs` (wired into `serialize_prose_with_linkrefs`
*before* `demote_undefined_links`, on the original body so the refmap still sees every bracket): builds a
label→url map via `collect_user_linkrefs`/`scan_linkref_run` (consumes a definition run only at a **block
start** — body start or after a `Text` containing `\n` — tolerating leading-indent + soft-break-separated
stacked defs, all dropped), drops the def inlines, and **rewrites** each referencing link with a defined
label to `Inline::MdInlineLink{url, display}` (reusing the existing `\href` rendering — `inline_link_node_atom`
GRP-wraps a multi-atom display, code-span sub-renders). `match_linkref_def`/`linkref_def_label`/
`parse_linkref_def_dest`/`link_display_inlines` are the helpers. Returns `None` (no change) when the field has
no def → existing cases byte-identical. Refmap-safe: a label with a user def is already a synthesized
candidate (its `[ref]` bracket), so never spuriously demoted; the poisoning machinery stays inert on clean
cases.

**TDD/tests:** 3 unit tests (consecutive-def render, bare-shortcut, **interrupt-rule regression guard**),
curated `md_url_reference` (blank-separated defs, emph/plain/code displays). **Formatter follow-on (deferred):**
the formatter **joins consecutive `#'` def lines** into one line, which invalidates them as CommonMark defs
(multiple defs need separate lines) → changes rendered Rd (Tenet-1 fixed-point break). Dodged exactly like the
`%`-comment reflow: the curated case uses **blank-line-separated** defs (each its own paragraph; the formatter
preserves the blanks → format-stable). The consecutive-def projector path is still covered by the unit test
(no formatter involved). Re-blessed the format baseline (+1 key).

**Result:** projector **310→311 matching (all allowlisted), 18 divergent** (unchanged — all out of scope).
`cargo test` 529 lib green, clippy + fmt clean, curated fixed-point 45→46/46.

**Next (ranked):** **(1)** **Formatter: keep link-ref-definition lines unjoined** — the deferred follow-on above;
the formatter should recognize a `[label]: dest` def line (markdown-block-level) and not reflow it into the
previous/next line, so consecutive defs stay format-stable. Same family as the (since-fixed) `%`-comment reflow
bail. **(2)** **Whole-*field* refmap** — `collect_user_linkrefs`/`linkref_keys` are per-prose-body, so a def in
the **description** intro paragraph used in **details** (a *separate* markdown doc in roxygen) is correctly
isolated, but a def in a sibling **list item / `@section` body** of the same field is missed; also `@section`
has its own projector arm that doesn't call `serialize_prose_with_linkrefs` (URL refs there are unhandled).
**(3)** Slice B **remainder** — fully retire `scan_md_link` (still serves plain same-line `[t]`/`[t][r]`,
autolink `<url>`): carve every bracket, move references onto the arena lookahead. The 18 projector divergences
stay roxygen2-eval/multi-block, **out of scope**.

## Earlier sessions

- **2026-06-29l (same-line non-plain *reference* `[*foo*][ref]` drop):** the reference analog of 29k. A new
  `same_line_ref_opener` (lex.rs) carves the `[` of a markup-display (`* _ ` ` ` <`, not `!`/`\`) reference
  followed by a clean `[ref]`; the existing `cross_line_ref_closer` + `scan_md_link` + arena fold + 29k's
  `link_display_is_droppable` do the rest with zero new code. Plain `[plain][ref]` stays opaque. Curated
  `md_ref_emphasis`. 309→310.

- **2026-06-29k (non-plain shortcut/reference link drop):** a shortcut/reference link (`R:` dest) whose
  display (after unwrapping a *sole* `code` child) has any non-text child is **dropped to empty** by
  `parse_link` ("markdown links must contain plain text"); the link vanishes, prose stays contiguous.
  `same_line_shortcut_opener` carves `* _ ` ` ` <` displays as arena pairs (`!`/`\` stay opaque); projector
  `link_display_is_droppable` drops the `MdShortcutLink`/`MdRefLink` node via `continue` (no run flush).
  Fixture `roxygen_md_shortcut_emphasis` + curated `md_shortcut_emphasis`. 308→309.

- **2026-06-29j (link-reference map; undefined shortcut/ref stays literal):** arity linked every bracket-free
  shortcut optimistically, but roxygen's `get_md_linkrefs` only defines a label for a `[` not preceded by `]`
  and not followed by `[`/`{`. New `linkref_keys` builds the refmap from `linkref_source_skeleton` (re-exposes
  every link/image bracket) scanned by `md_linkref_scan`; `demote_undefined_links` rewrites a shortcut/ref link
  whose normalized label ∉ refmap to literal, before the positional poison demotion (full candidate set, so
  `md_ref_link_multiline`'s `a][b]` still links). Projector-only. Curated `md_undefined_shortcut` +
  `md_undefined_ref`. 306→308.
- **2026-06-29i (`@section` md-OFF drop to `(\section (TEXT "NA"))`):** `@section` uses plain `tag_markdown`
  (`sections=FALSE`), so the per-section rdComplete drop never fires under `@md`; but md-OFF
  `markdown_if_active`'s else-branch runs `rdComplete(x$raw)` unconditionally on the `title: body` value →
  `""` on imbalance → `str_split` gives `title=""`, `content=NA` → `\section{}{NA}` → `(\section (TEXT "NA"))`.
  Projector guard in `project_block`'s `"section"` arm (`!md && !rd_complete(source)`); raw scans identically
  to `x$raw`. Curated `rdcomplete_section_drop`. 305→306.
- **2026-06-29h (`@field`/`@slot` whole-tag drop on raw brace imbalance):** roxygen2 parses them via
  `tag_two_part` → `rdComplete(x$raw, is_code=FALSE)`, dropping the whole tag (mode-independent) on
  imbalance — a bad slot/field contributes no `\describe` item, all-dropped → no Slots/Fields section.
  `continue`-on-incomplete guard in `project_block`'s `"slot" | "field"` arm; raw source scans identically
  to `x$raw` (no `{}\%` in scaffolding, quotes ignored). Curated `rdcomplete_slot_drop` + `rdcomplete_field_drop`.
  303→305.
- **2026-06-29g (markdown-OFF rdComplete brace-balance drop):** extended the per-section `rdComplete`
  drop to md-off mode, where `markdown_if_active`'s else-branch runs `rdComplete(text)`
  **unconditionally** → *every* prose section (title included) drops to empty on imbalance, not just
  the `sections=TRUE` `@description`/`@details`. One-line `push_section` gate
  (`check_drop = if md { drop_on_incomplete } else { true }`). Curated `rdcomplete_off_description` +
  `rdcomplete_off_seealso`. 301→303.

- **2026-06-29f (opener-deactivation slice B core):** the arena now implements CommonMark
  `look_for_link_or_image` (backward matching + **opener deactivation**), fixing the latent non-poisoned
  nested-bracket bug (`[a [b] c](url)` standalone → inner `\link{b}`, outer brackets literal). Three
  changes: arena `match_brackets` (stack pairing + deactivation, `BracketRole`), lexer
  `is_nested_bracket_opener` (carves the outer `[` so every bracket reaches the arena), projector
  `relink_demoted_inline_links` (re-forms the enclosing `\href` for the *poisoned* case, scoped by the
  consecutive-`Inline::Text` run). Curated `md_nested_link` + `_chain`; nested link no longer atomic
  (format baseline re-blessed, fixed-point 36/36). 299→301.

- **2026-06-29e (opener-deactivation slice A):** moved same-line plain-text *shortcut* `[text]` off the
  opaque `scan_md_link` leaf onto the arena node path (`same_line_shortcut_opener` → `MdShortcutLink`).
  Behavior-preserving (plain interior coalesces to the same text); plain-text gate keeps marked-up shortcuts
  opaque (roxygen2 rejects them); `!preceded-by-]` guard kept cross-line `[ref]` labels on `scan_md_link`.
  Curated `md_shortcut_link`; 298→299.

- **2026-06-29d (`rdComplete` brace-balance drop, cm-439/442/451/454 closed):** an escaped emphasis
  delimiter (`*\**`) resolves to `\emph{\}` whose trailing `\` escapes its own `}`; roxygen2's
  `markdown_if_active` runs `rdComplete` on the rendered section and **drops** `@description`/`@details`
  (`sections = TRUE`) to empty. Projector-only faithful drop: `rd_complete` (verbatim port of
  `src/isComplete.cpp`) + `sexpr_to_rd`/`render_sexpr` (pre-parse Rd reconstruction) + `push_section`'s
  `drop_on_incomplete`. Trap: `@md` escapes `%`→`\%` everywhere, so re-escape every leaf (`escape_percent`)
  or a `%20` URL false-drops. 294→298. (Also re-blessed the format baseline missed in 2026-06-29c.)

- **2026-06-29c (`@rawRd` body is verbatim Rd, never markdown):** roxygen2's `@rawRd` uses `tag_value`, not
  `tag_markdown`, so its body is never markdown-processed; arity's block-keyed lexer wrongly carved md leaves
  (`[bracket]`/`*star*`) inside it under `@md`. Fixed parser-side with a per-tag `rox_raw` flag in the
  `lex()` driver (`roxygen_line_tag`/`is_raw_rd_tag`, `"rawRd"` only, reset per block). Fixture
  `roxygen_rawrd_no_markdown` + curated `rawrd_md_literal`. 293→294. (NB: missed re-blessing the format
  baseline — fixed 2026-06-29d.)
- **2026-06-29b (opaque nested-bracket inline-link inner candidates in a poisoned tail, slice 6):**
  roxygen2's `get_md_linkrefs` synthesizes a `[label]: R:label` def for every bracket-free `[…]` candidate
  scanning the **raw** field text; a nested-bracket inline link `[a [b] c](url)` stays an **opaque** `MdLink`
  leaf yet the raw scan still finds the inner `[b]` candidate, so in a poisoned tail `[b]: R:b` leaks though
  `\href` survives. New `opaque_inline_link_display` drives an `MdLink` arm in `inline_skeleton_fragment`
  (`[a [b] c] `); link not demoted. Autolink-adjacent was already correct (`md_linkref_poisoning_autolink`
  passed on first mint). Projector-only. Curated `md_linkref_poisoning_nested_link` + `_autolink`. 291→293.

- **2026-06-29 (image alt-text candidate defs in a poisoned tail, slice 5):** an image `![alt](url)`'s
  `[alt]` is a bracket-free candidate, so its `[alt]: R:alt` def leaks in a poisoned tail even though the
  `\figure` survives. Added an `MdImage` arm to `inline_skeleton_fragment` (`[alt] `; `image_alt_text`
  extracts the literal alt via `scan_delimited`); image not demoted. Projector-only. Curated
  `md_linkref_poisoning_image` + unit test. 290→291.

- **2026-06-26e (inline-link candidate defs in a poisoned tail, slice 4):** roxygen2's `get_md_linkrefs`
  synthesizes a `[text]: R:text` def for an inline `[text](url)` link too (its `[text]` is a bracket-free
  candidate followed by `(`), so in a poisoned tail that def leaks even though the `\href` survives.
  Extracted `inline_skeleton_fragment` as the single source for `inline_source_skeleton` + `skeleton_len`;
  its `MdInlineLink` arm contributes `[text] `. Projector-only. Curated `md_linkref_poisoning_inline_link`
  + a unit test. 289→290.

- **2026-06-26d (`get_md_linkrefs` leaks outside `push_section`, slice 3):** the demote+leak pair was
  extracted into `serialize_prose_with_linkrefs` and wired into the two other `markdown_if_active`
  builders — `@field`/`@slot` item defs (`describe_section`, the description half of `tag_two_part`) and
  the `@section` body (roxygen2 markdown-processes the whole `title: body` then splits on `:`, so demote
  runs on the whole body and the leaked defs land in the content after the colon). Projector-only. Curated
  `md_linkref_poisoning_field` + `_section`. 287→289.
- **2026-06-26c (`get_md_linkrefs` mixed valid+invalid poisoning, slice 2):** a field mixing valid
  shortcuts and escaped-close candidates. The defs append as one cmark block parsed top-down; the
  **first invalid** (escaped-close) candidate's label runs into the next line's `[` (illegal in a
  label), failing that def *and every def after it* → cmark leaks from the first invalid to the end
  (valid candidates included), **de-linking** every shortcut/reference link in that tail. Projector-only,
  **demote-then-leak**: `demote_poisoned_links` finds the boundary (`first_invalid_linkref_offset`, any
  trailing backslash = invalid) and rewrites the tail's shortcut/ref link nodes to literal bracket text
  *before* the skeleton is rebuilt, so they reappear as candidates and their now-dead defs leak;
  `leaked_linkref_text` changed to "from the first invalid onward". Inline links/autolinks/code survive.
  Curated `md_linkref_poisoning` + 4 unit tests. 286→287.
- **2026-06-26b (`get_md_linkrefs` leaked defs, migration slice 1):** escaped-close `[text\]` yields a
  synthesized def whose label never closes → cmark leaks it as literal trailing prose. Projector-only
  (`leaked_linkref_text`: `double_escape_md`→`md_linkref_labels`→filter `!closes`→`url_encode`→
  `cmark_unescape`, appended via `append_rendered_text`). Modeled all-invalid fields only; mixed was
  deferred (closed this session). Curated `md_escaped_close_bracket`. 285→286.
- **2026-06-26 (Cross-line *shortcut* `[text]` links, bare-`]` closer):** a `[text]` whose `[` opens on
  an earlier `#'` line resolves into one `\link{text}` over the coalesced text. Line-locally every `]`
  is ambiguous, so the lexer carves **every** bare `]` not part of a `](url)`/`][ref]`/`]{…}` shape as a
  neutral bracket; `find_link_closer` pairs a lone `]` (no following label) with an earlier `[` as a
  shortcut closer, else re-emits it literal (`a]` stays `a]`). Projector node arm closer `]` →
  `MdShortcutLink`/`shortcut_link_node_atom`. No new TokKind/SyntaxKind; formatter unchanged. Side
  effect: the `]` in `\[shortcut]` is now a standalone `Delim` (projection unchanged). Curated
  `md_shortcut_link_multiline`. 284→285.
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
