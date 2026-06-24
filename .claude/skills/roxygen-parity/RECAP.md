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
  **Every markdown *inline* recognizer in the lexer MUST be `if md`-gated** (`*`/`_`/`` ` ``/
  the `[`-link/`<`-autolink/list-markers) — else its leaf kind stops implying `@md`, and the projector
  (which keys structure off leaf kind and never re-derives mode) mis-fires in non-`@md`
  blocks where roxygen2 keeps the markup literal. The `[`-link recognizer slipped this and
  was fixed 2026-06-24c (it carved `ROXYGEN_MD_LINK` even with markdown off); audit any
  *new* inline recognizer the same way. **`@md` inline link `[text](url)` landed (Stage 12):**
  → `\href` via projector `Inline::MdLink`.
- **Under `@md`, roxygen2 treats *every* bracket-free `[…]` as a link (since Stage 13,
  2026-06-24d).** `get_md_linkrefs` (`markdown-link.R`) injects a reference definition for
  any `[content]` (no nested brackets) **not followed by `[` or `{`**, so `[note]`/`[1]`/
  `[see this]` all resolve to `\link`s — the lexer's `is_shortcut_content` mirrors this
  (the `Some(&b'{') => None` arm is the followed-by-`{` exclusion). The projector's
  `resolve_md_link` ports `parse_link`'s three forms (inline→`\href`, reference→`\link`,
  shortcut→`\link`/`\linkS4class`/`\code{\link}`). **Two static-faithfulness facts:** (1) the
  section serializer **drops the `\link[=dest]`/`[pkg:file]` topic option**, so the projector
  only emits macro head + display text + `\code`-wrap (it never resolves a topic *file*);
  (2) `resolve_link_package` is non-static, so a `pkg::` prefix in the display comes **only**
  from an explicit `::` — faithful to roxygen2 run with `current_package == ""` (the corpus
  context). A real package would gain a `pkg::` display prefix via topic resolution; that is
  correctly **not** modeled (no installed-package introspection in a static projector).
- **Markdown image format is extension-keyed (since 2026-06-24f).** roxygen2's
  `mdxml_image` drops the alt text and emits `\figure{url}{title}`, *conditionally
  wrapped* via `get_image_format` (`markdown.R`): two regexes
  (`[.](jpg|jpeg|gif|png|svg)$` html, `[.](jpg|jpeg|gif|png|pdf)$` pdf). A raster
  ext (jpg/jpeg/gif/png) matches **both** → "all" → **bare** `\figure`; svg matches
  html-only → `\if{html}{\figure}`; pdf matches pdf-only → `\if{pdf}{\figure}`;
  unknown ext matches neither → "all" → bare. `image_format` in `project_rd.rs`
  ports this (case-insensitive, `.ext$`). The image is an **inline-form-only** leaf
  (`ROXYGEN_MD_IMAGE`, `scan_md_image` requires `![…](…)`); reference/shortcut
  images stay backlog. The Rd `\figure{path}{caption}` route is a separate
  **two-arg verbatim** macro (`TWO_ARG_RD_MACROS` + `is_verbatim_rd_arg`) — same
  output shape via the generic `serialize_macro`.
- **Rd macro names allow digits** (`[A-Za-z][A-Za-z0-9]*`, since 2026-06-24g) — e.g.
  `\linkS4class`. The leading char is a letter, the rest letters *or* digits. There is
  **one** source of truth for where a `\name` ends: `rd_macro_name_end(bytes, start)` in
  the `roxygen` parent module. The lexer (`scan_rd_macro`), the tree builder
  (`build_rd_macro`), and the four block builders in `build.rs` all route through it — a
  *new* name scan must too, or a digit-bearing macro silently truncates (name cut at the
  digit → macro unrecognized → falls through to literal `TEXT`).
- **A new roxygen line-body TokKind must be added to *every* line-body matcher** or tag/prose
  lines silently truncate at the unknown token (this bit Stage 5: a `@param` line's description
  vanished, its continuations became phantom intro paragraphs → extra `\title`/`\description`).
  The set: `classify_line`, `is_line_body_kind`, the block-macro consumer's inline-span arm
  (all `src/parser/roxygen.rs`), `expr.rs`'s atom-parser fallthrough, `tree_builder`'s
  `syntax_kind_for`, `lexer.rs`'s `is_comment_like`, `syntax.rs`'s `is_roxygen_token`, plus the
  formatter's `is_blank`/`is_tag_prose_kind`. Rust exhaustiveness catches the enum matches; the
  `matches!` lists are silent — grep an existing roxygen leaf kind to find them all.
- **Intro prose splits title/description/details by *roxygen2 paragraph*, not CST node
  (since 2026-06-24e).** roxygen2's `parse_description` (`R/block.R`) splits the intro on
  `\n\n`: 1st paragraph = `\title`, 2nd = `\description`, the rest = `\details` (folded
  with explicit `@details` *only when leftover intro paras exist*; else `@details` stands
  alone). An explicit `@title`/`@description` claims its slot and shifts the paras down.
  The projector's `section_body_parts` groups body parts into these paragraphs: a block
  macro / md-list abutting a prose line (no blank `#'` line) is the **same** paragraph; a
  **section-level `ROXYGEN_MARKER`** (a blank `#'` line — per-line markers live nested
  inside each node) starts a new one. Don't revert to per-node parts (that folds a
  trailing list into the wrong section). Title-as-description fallback still fires when no
  description exists anywhere.
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
- **Verbatim is per-*argument*, not per-macro (since 2026-06-23, Stage 10).** `\href`
  is a two-arg structural macro (`is_two_arg_rd_macro`) whose *first* arg (the URL) is
  verbatim `VERB` but whose *second* (the link text) is sub-parsed latexlike. New
  `is_verbatim_rd_arg(name, index)` (in `parser/roxygen.rs`) drives the tree builder's
  per-group recurse decision (`build_rd_macro` tracks `arg_index`); the projector needs
  **no change** — it already emits `(VERB …)` for a `ROXYGEN_RD_MACRO_VERB` leaf and
  GRP-wraps a structural macro's multi-atom arg, so `\href{url}{a \emph{b} c}` →
  `(\href (VERB "url") (GRP (TEXT "a") (\emph (TEXT "b")) (TEXT "c")))`. Confirm a new
  macro's per-arg encoding via `block-to-sections`: a `{VERB}`-tagged arg = verbatim.
  **Lexing a balanced `\href{…}{…}` as one atomic macro token also stops the formatter
  from reflowing *inside* it** (it had split a multi-line link text mid-macro) — a Tenet-1
  improvement; re-blessed the format baseline for the one affected case (rx-2e54a81b).
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
- **Brace-less `\word` is recognized *only when unknown* (since 2026-06-24j).** parse_Rd
  tags any unrecognized `\word` `UNKNOWN` even brace-less; a *known* brace-less name is
  either a zero-arg macro (`\cr`→`(\cr)`) or arg-requiring misuse (messy/expanded) — both
  **left as literal prose** (backlog), so existing tokenization/format is untouched. The
  single source of truth is `is_known_rd_macro`/`KNOWN_RD_MACROS` in the `roxygen` parent
  (parse_Rd's static keyword table, verified vs R 4.5; **excludes** user macros `\CRANpkg`/
  `\doi`/… which parse_Rd *expands* — out of scope). `scan_rd_macro` carves a brace-less
  `\word` iff `!is_known_rd_macro`; the projector's `serialize_macro` empty-`out_atoms`
  branch keys on the same table — a name-only node is `(\name)` if known (a block list child
  like `\item`/`\cr`), else `(UNKNOWN "\\name")`. **A new known macro must go in
  `KNOWN_RD_MACROS`** or it silently becomes UNKNOWN. Zero-arg name-only *rendering* in prose
  (`\cr`→`(\cr)`) is still deferred (those only appear in excluded `@param`/code-span/block
  contexts in the corpus, never in-scope).
- **Markdown fenced code blocks are mode-keyed via the lexer, like md-lists (since
  2026-06-24k).** Under `@md`, a line whose content opens a fence (3+ backticks) is
  carved *whole* by the lexer (`scan_md_fence` in `lex.rs`) as a `RoxygenMdFence`
  leaf — the opener (with its info string) and the bare closer alike. The block
  builder (`emit_md_code_block`) pairs an opener with its closer into a
  `ROXYGEN_MD_CODE_BLOCK` (a direct `ROXYGEN_SECTION` child, sibling of paragraphs),
  threading `#'`/newline/indent as trivia; the verbatim code lines pass through as
  their body tokens. **The leaf's existence implies `@md`** (lexer = single mode
  source; the builder never re-derives mode), like `RoxygenMdListMarker`. The fence
  carve sits **before** the list-marker carve in `lex_roxygen_prose` and bails when a
  backtick follows the opening run (CommonMark forbids a backtick in a backtick
  fence's info string → an inline ` ```code``` ` span at line start is *not* a fence).
  Projector arm: `Inline::MdCodeBlock` → `serialize_md_code_block` emits roxygen2's
  **three** atoms (`mdxml_code_block`): `(\if (TEXT "html") (\out (VERB "<div
  class=\"sourceCode[ <info>]\">")))`, `(\preformatted (VERB <code+\n>))`, `(\if (TEXT
  "html") (\out (VERB "</div>")))` — info "" → bare `sourceCode`, info `r` →
  `sourceCode r`. Code = each content line's `#'`-and-one-space-stripped text, joined
  with `\n` plus a trailing `\n` (commonmark `xml_text`); `%`/`{`/`}` stay raw (parse_Rd
  decodes `escape_verb`). **Formatter:** `ROXYGEN_MD_CODE_BLOCK` is atomic passthrough
  in `physical_lines`/`emit_md_code_block` (marker-normalized per line) —
  **byte-identical to the pre-node textual `is_fence`/`emit_normalized` path**, so the
  format baseline is unchanged (no re-bless). **Code indentation beyond the marker is
  dropped** (matches that prior behavior; canonical re-indent deferred). **Out of
  scope:** ` ```{r} ` knitr eval blocks (roxygen2 *evaluates* them) stay divergent;
  ` ```{verbatim} ` is *not* evaluated and renders as a plain fenced block (bonus
  case rx-0d100638 — info `{verbatim}` → class `sourceCode {verbatim}`).

- **Inline raw HTML is a precise lexer recognizer, gated `if md` (since 2026-06-24n).**
  Under `@md`, a `<tag>` (open or close) carves as a `RoxygenMdHtml` leaf
  (`scan_md_html_inline` in `lex.rs`, chained *after* `scan_md_autolink` at the
  `b'<'` arm — autolink wins a `<scheme:…>`, raw HTML the rest). The recognizer
  mirrors the **CommonMark "Raw HTML" grammar precisely** (tag name `[A-Za-z][A-Za-z0-9-]*`;
  attributes `name(=value)?` with quoted/unquoted values) because **over-recognition is a
  real bug**: carving a span commonmark would keep literal makes the projector emit a
  spurious `\out`. Comment/PI/declaration/CDATA forms are **not** recognized (faithful
  under-handling, backlog). Projector: `Inline::MdHtml` → `html_inline_atom` →
  `(\if (TEXT "html") (\out (VERB <tag>)))` (`mdxml_html_inline`; `}`-escape decoded by
  parse_Rd, raw tag in the pin). **The atomic leaf stops the formatter reflowing
  *inside* a tag** (the old baseline split `<img\n#' src=…>` at the space) — a Tenet-1
  improvement, re-blessed the one affected case (rx-299f50fb), same family as the `\href`
  re-bless. **Block HTML** (`<p>…</p>` at line start, multi-line, `mdxml_html_block`,
  rx-daf9322f) is still backlog — it needs a line-start block recognizer (the 7 CommonMark
  start conditions), a different shape than this inline span.

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
inline raw HTML → `\if{html}{\out}`, 2026-06-24n): **135 matching (all
allowlisted), 26 divergent (backlog)** of 161 pinned cases. The
divergences are now almost all roxygen2-*evaluation* gaps (out of scope); the
in-scope remainder is block raw HTML (`<p>…</p>`) plus `@format %`. Tasks:
`task roxygen-projector` (the gate),
`task roxygen-projector-refresh` (re-mint all pins), `task roxygen-projector-pins`
(harvested pins only), `task roxygen-projector-seed` (re-seed allowlist from matches).
Report: `ROXYGEN_PROJECTOR.md` (this dir).

**Three checks, three roles** (don't conflate):
1. **Projector parity** (`tests/roxygen_projector.rs`, pure Rust) — the **primary,
   parser-growth driver**. Compares Rd *structure*, so it sees block-structure gaps
   the fixed-point check is blind to. Curated corpus + harvested projector-eligible
   subset (161 pinned cases). The 26 divergences are the worklist.
2. **Curated fixed-point** (`tests/roxygen_oracle.rs::roxygen_oracle_report`, needs R,
   `#[ignore]`d) — strict semantic preservation of the formatter; 10/10 preserving, 0
   blocked. *Meaning, not layout.*
3. **Harvested fixed-point** (`tests/oracle/corpus/roxygen.jsonl`, 217 cases, needs R,
   `#[ignore]`d) — broad **opt-in** backlog gated by `roxygen-allowlist.txt`
   (212 preserving, 4 divergent, 1 skipped). A coverage net, **not** the parser driver
   (it's cosmetic-blind + R-dependent). Reports: `task roxygen-oracle` /
   `task roxygen-harvest`.

## Latest session (2026-06-24n) — inline raw HTML `<tag>` → `\if{html}{\out{<tag>}}`

**Construct:** under `@md`, a raw inline-HTML tag (`before-<img src='foo.png'>-after`)
renders to `(\if (TEXT "html") (\out (VERB "<img src='foo.png'>")))` per roxygen2's
`mdxml_html_inline`, tiling around the surrounding prose. Closed **1 harvested case**
(rx-299f50fb).

**Bucket: parser gap (+ faithful projector arm).** New `RoxygenMdHtml` leaf
(`ROXYGEN_MD_HTML`, SyntaxKind 99). Lexer `scan_md_html_inline` (`lex.rs`) is chained
**after** `scan_md_autolink` at the `b'<' if md` arm — autolink claims `<scheme:…>`,
raw HTML the rest. The recognizer mirrors the **CommonMark "Raw HTML" grammar
precisely** (open tag with `name`/quoted+unquoted attrs, close tag) because
over-recognition would emit a spurious `\out` where roxygen2 keeps literal;
comment/PI/declaration/CDATA forms stay literal (faithful under-handling, backlog).
Projector: collect `ROXYGEN_MD_HTML` → `Inline::MdHtml(raw)` → `html_inline_atom`.
Wired the new TokKind through `roxygen_role` (single source) + the two explicit
matchers (`expr.rs`, `tree_builder.rs`) + the two `is_roxygen_*` lists (`syntax.rs`).

**Tenet-1 note:** the atomic leaf stops the formatter reflowing **inside** a tag
(the old baseline split `…image before-<img\n#' src='foo.png'>-after` at the space
in the tag); the new output breaks *before* `before-<img…>`, keeping the tag intact.
Re-blessed the one affected format-baseline case (rx-299f50fb) — same justification
as the `\href` atomic-token re-bless.

**Result:** projector **134→135 matching** (135 allowlisted), 27→26 divergent, 0
regressions, 161 pinned. `cargo test` fully green (incl. parser snapshots +
re-blessed format-stability baseline), clippy + fmt clean, curated fixed-point
**10/10** preserving. Files: `src/syntax.rs`, `src/parser/lexer.rs`,
`src/parser/tree_builder.rs`, `src/parser/expr.rs`, `src/parser/roxygen/lex.rs`
(`scan_md_html_inline` + 3 unit tests), `src/roxygen/project_rd.rs`
(`Inline::MdHtml` + `html_inline_atom`), new fixture
`tests/fixtures/parser/roxygen_md_html_inline/` (+ snapshots), re-blessed
`roxygen-format-baseline.jsonl`, allowlist (+1 via re-seed), TODO, RECAP.

**Next (ranked):** down to 26 divergent, almost all roxygen2-*evaluation* gaps
(out of scope: ` ```{r} ` eval blocks rx-2900ecd5/24b3bfd6/24ef0d37/a6ac1b4d/
e0e631c5/55b6980b, inline `` `r …` `` rx-21fd7c2f/8770c410/cc0ae196, data-object
auto-`\format` rx-4d59d472/cbcc255c/deb9d202, RefClass docstrings rx-e02bf95c/
f5812049). The remaining **in-scope** targets: **block raw HTML** under `@md` —
`<p>…</p>` at line start, multi-line, → `(\if (TEXT "html") (\out (VERB "\n")
(VERB "<p>…</p>\n") …))` (rx-daf9322f, `mdxml_html_block`); needs a *line-start*
block recognizer (the 7 CommonMark start conditions, ends at blank line) — a
different shape than this session's inline span (carve whole lines like the fence,
pair in the block builder). **`@format %`** (rx-f6927028, `(\format)` empty) needs Rd
`%`-comment handling but a *broad* non-md lexer carve fires in ~6 excluded roclet
fields (`@name %a%`, `@usage %\`, `@importFrom …%>%`) and risks the reflow-merge
hazard (a `%`-to-EOL leaf could absorb the next prose line on reflow) — scope it
tightly or defer. **Nested lists** (rx-91e67e79 md, rx-959fc227 Rd) and
**links broken across lines** (rx-383f2ca3, rx-eb12b6b6) are parser gaps
(in-list indentation dropped / line-scoped lexer), deferred. `\preformatted{}`
mid-line block macro (rx-0a1710c0) needs a non-line-start block-macro opener.

## Earlier sessions

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
- **2026-06-24j (brace-less unknown macros → `(UNKNOWN …)`):** parser+projector, +2
  (rx-16f78b2f non-md, rx-b8082617 md). A brace-less `\word` not in the built-in Rd
  keyword table (new `is_known_rd_macro`/`KNOWN_RD_MACROS`, verified vs R 4.5)
  projects to `(UNKNOWN "\\word")`; `scan_rd_macro` carves it only when unknown (a
  known brace-less name stays literal prose). 124→126.
- **2026-06-24i (`@section` body inline macros + GRP-wrap):** projector-only, +2
  (rx-41e06b64 non-md, rx-1b26c2a4 md). `@section Title: body` → `\section{Title}{body}`;
  body sub-parses inline macros, two-arg structural GRP-wrap of a multi-atom arg.
  `split_section_title` + `grp_arg`, −`inlines_raw_text`; also routed `describe_section`'s
  `\item` def through `grp_arg`. 122→124.
- **2026-06-24h (multiple `@examples` aggregate into one `\examples`):** projector-only,
  +3 (2 harvested rx-5ac40b37/rx-73a5b650 + curated `examples_merge`). `@examples`/
  `@examplesIf` is an aggregating field; moved the examples arm out of per-tag dispatch
  into `project_block` (a `has_examples` flag → one `(\examples ...)`). 119→122.
- **2026-06-24g (digit-bearing Rd macro names, `\linkS4class`):** +1 case
  (rx-852ee490). Rd command names are `[A-Za-z][A-Za-z0-9]*`; six duplicated name
  scans truncated at a digit. Replaced all with one shared `rd_macro_name_end`
  helper (the single source of truth for where a `\name` ends). Projector unchanged.
  118→119.
- **2026-06-24f (images `![](…)` + Rd `\figure` → `\figure`):** +3 cases. The Rd
  `\figure{path}{caption}` is a two-arg verbatim macro; a markdown image
  `![alt](url "title")` lexes to `ROXYGEN_MD_IMAGE` (`scan_md_image`, inline form
  only). `resolve_md_image` ports `mdxml_image`: alt dropped, `\figure{url}{title}`,
  wrapped per the extension-keyed `get_image_format` (svg→html, pdf→pdf,
  raster/unknown→bare). 115→118.
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
- **2026-06-24b (Refactor #2, split `roxygen.rs` along phase boundaries):** carved the
  1686-line `src/parser/roxygen.rs` into a thin parent (113, shared macro-classification +
  `scan_balanced`/`utf8_len` + re-exports) + 3 phase submodules under `src/parser/roxygen/`:
  `lex.rs` (996, sub-lexing + lexer tests), `group.rs` (200, token→event grouping),
  `build.rs` (430, Rd-macro/markdown building). Sibling-internal items `pub(super)`,
  externally-reached `pub(crate)`. Pure refactor, byte-identical (projector 93/66 unmoved).
  The "shared infra over `cursor.rs`/`recovery.rs`" rewrite is a **NON-GOAL** (cursor = same
  index-threading idiom; recovery builds ERROR nodes roxygen rejects). Form A/B block-opener
  split is a watch item (a *third* form ⇒ reconsider lex-time greediness).
- **2026-06-24 (Refactor #1, unify `TokKind`/`SyntaxKind` classification):** collapsed 8
  silent `matches!` lists onto a compiler-policed source. New `RoxygenRole` +
  wildcard-free `TokKind::roxygen_role` (`lexer.rs`) for the lexer/parser side;
  `SyntaxKind::is_roxygen_prose_content` (`syntax.rs`) for the formatter side. Pure
  refactor, byte-identical (projector 93/66 unmoved). +78/−78 across 4 source files.
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
