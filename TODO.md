# TODOs

## Parser

- [ ] Comment-aware suppression placement edge cases: directives inside
  `R_CALL_ARGUMENTS` between `( ... , <directive> , next_arg )` need special
  handling so they attach to `next_arg` instead of the argument list. (Jarl
  solved this by overriding biome's `place_comment`; arity's
  next-non-trivia-sibling walk already handles most cases.)

- [x] Incremental reparse (token/block) beneath `parsed_document`
  (`src/incremental.rs`): rowan-style `reparse_token` → `reparse_block` →
  full-reparse fallback (cf. rust-analyzer `reparsing.rs`), splicing reused
  green subtrees (`src/parser/reparse.rs`). `parsed_document` recovers the
  edit from the old/new text via a prefix/suffix diff and splices off a
  non-salsa per-file previous-parse cache (a pure perf hint --- a successful
  reparse is byte-identical to a full parse, so it never changes query
  output). Correctness is pinned by an oracle property test
  (`tests/incremental_reparse.rs`: `reparse == parse(new)` in tree *and*
  diagnostics across the corpus) plus a salsa-level test
  (`body_edit_uses_incremental_reparse_and_stays_correct`). On a \~100 KB
  file reparse is \~200× faster than a full parse (`benches/parse.rs`).
  Serves Tenet 2. No `SyntaxNodePtr`/`AstPtr` added (no feature needs a
  stable cross-edit reference yet). See `ARCHITECTURE_AUDIT.md` §3.4.

  - [ ] Follow-up: top-level-statement reparse (non-braced). v1 reparses
    only brace blocks + single tokens; edits elsewhere fall back to a
    full parse (correct, just not incremental). Could also use the LSP's
    precise edit ranges instead of the prefix/suffix text diff.

## Formatter

- [ ] Tribbles

- [ ] `line-ending` config (landed for whole-document `format_node`) is **not**
  applied by `format_range` (`src/formatter/core.rs`), which always emits `\n`.
  Range/on-type formatting in a CRLF document via the LSP can therefore splice LF
  into a CRLF buffer (mixed endings). Thread the source/ending into `format_range`
  like `format_node` does. Low urgency: the CLI `format` path (whole-document) is
  correct; this only affects LSP range edits in CRLF files.

- [ ] `exclude`/`extend-exclude` (landed for the CLI `format`/`lint` walk via
  `ExcludeFilter` in `src/file_discovery.rs`) is **not** consulted by LSP
  workspace seeding (`src/lsp/lint_thread.rs` `seed_workspace`), salsa sibling
  discovery (`src/linter/check.rs`), or `arity index` package discovery
  (`src/rindex/discover.rs`) — all pass `ExcludeFilter::none()`. Wire the resolved
  config's excludes through those paths so excluded files don't get indexed/linted
  in-editor either.

- [ ] Trailing comments are not line suffixes (width-counted). A same-line
  trailing comment counts toward its line's width for fit measurement, so a long
  comment can force an otherwise-fitting group to break (e.g.
  `isFALSE(getOption("dplyr.show_progress", default = TRUE)) || # ...` breaks the
  `getOption(...)` call). air treats trailing comments as zero-width line
  suffixes and leaves the call inline. Pre-existing (visible without any `if`);
  surfaced while fixing #37 (condition-level comments). Fix is a printer-level
  line-suffix concept; broad, so deferred. Fixtures
  `if_condition_trailing_comment` (records the divergence) and
  `if_condition_comment_forms` (matches air).

- [ ] Roxygen syntax formatting

  - [x] **Parsing foundation (CST-native).** Roxygen lines (`#'`, per roxygen2's
    `^#+'`) are lexed into sub-tokens (`ROXYGEN_MARKER`/`ROXYGEN_AT`/
    `ROXYGEN_TAG_NAME`/`ROXYGEN_TAG_ARG`/`ROXYGEN_TEXT`). **CST re-model (2026-06-23):**
    a block is no longer line-flat — the parser builds **logical content**
    `ROXYGEN_BLOCK` → `ROXYGEN_SECTION` (intro + one per `@tag`) →
    `ROXYGEN_TAG`/`ROXYGEN_PARAGRAPH`, with `#'` markers and inter-line newlines as
    trivia (rowan/rust-analyzer style), so block macros spanning `#'` lines become
    representable (`src/parser/roxygen.rs`, hooked into the `core.rs` root loop and
    `expr.rs` `parse_block_expr`). `ROXYGEN_LINE`/`RoxygenLine` dissolved (reserved
    enum variant). Arg-bearing tags (`@param`, `@field`, …) split out the name as
    `ROXYGEN_TAG_ARG`. Typed wrappers `RoxygenBlock`/`Section`/`Paragraph`/`Tag`
    (`src/ast/nodes.rs`). Losslessness holds (clean CRLF/EOF handling); the
    formatter reconstructs physical lines from trivia and emits blocks
    byte-identically (pinned by `tests/roxygen_format_stability.rs`).
    Edits inside a roxygen line fall back to block/full reparse (roxygen tokens
    only arise from the `#'` lexer path, so token reparse can't relex them in
    isolation). Air leaves roxygen untouched, so the eventual transforms are a
    conscious divergence under Tenet 1.
    - *Known degradation:* a `#'` line inside an expression (e.g. call args) is
      emitted as loose tokens and may draw a parse diagnostic; real roxygen only
      sits at statement level. Pinned by `roxygen_loose_in_call`.
  - [ ] **Transforms (future rounds), consuming the CST above:**
    - [x] (1) normalize the marker + a single space (`normalize_roxygen_line`
      in `src/formatter/roxygen.rs`): one space after the marker before content,
      trailing whitespace trimmed, blank lines collapse to the bare marker. The
      marker bytes are kept verbatim (`##'` is *not* collapsed) and tag-internal
      spacing is left for transform (3). Fixtures `roxygen_normalize_space`,
      `roxygen_trailing_space`, `roxygen_blank_trailing`,
      `roxygen_tag_marker_space`, `roxygen_multi_hash_kept`. No air-compat
      allowlist entry needed: air leaves roxygen untouched, so it preserves
      arity's normalized output and the fixed-point gauge sees no divergence.
    - [x] (2) reflow prose (`ROXYGEN_TEXT`) to line width (`ir_roxygen_block`
      in `src/formatter/roxygen.rs`): consecutive plain-prose lines are grouped
      into a paragraph (bounded by blank lines, tag lines, and structured lines)
      and greedily width-filled (`wrap_chunks`) to `line_width`, accounting for
      the marker + nesting-indent prefix consumed per line. On by default, the
      natural continuation of transform (1). **CST enrichment (parser layer):**
      to keep markup atomic *by construction* (Tenet 3), the prose lexer now
      carves protected spans out of `ROXYGEN_TEXT` runs into three new leaf
      kinds — `ROXYGEN_CODE` (`` `…` ``), `ROXYGEN_RD_MACRO`
      (`\code{…}`/`\link[pkg]{…}`), `ROXYGEN_MD_LINK` (`[t](u)`/`[func()]`) —
      via conservative, line-scoped, byte-exact recognizers in
      `lex_roxygen_prose` (`src/parser/roxygen.rs`); malformed/unterminated
      markup stays prose, so losslessness holds by construction (fuzz +
      `tests/fixtures/parser/roxygen_{inline_code,rd_link,rd_link_pkg,rd_code,md_link,md_autolink,mixed_inline,nested_braces,unterminated_code,unbalanced_macro,backtick_in_macro}`).
      Reflow builds **breakable chunks** (a chunk = maximal run with no breakable
      whitespace; spans glued in, so `[g()].` stays one chunk) and treats each
      span as atomic. **Passthrough** (marker-normalized, not reflowed): tag
      lines, blank separators, `@examples`/`@examplesIf` bodies, fenced code
      blocks, and structured lines (lists, tables, ATX headers, blockquotes). A
      paragraph is kept verbatim when a chunk could migrate to a line start and
      reparse as a list/header marker, preserving idempotence. Fixtures
      `roxygen_reflow_*` (basic, join_short_lines, indented_in_function,
      multi_paragraph, blank_boundaries, atomic_{inline_code,rd_macro,md_link},
      long_word, idempotent) and `roxygen_bail_{list,code_fence,examples_body}` +
      `roxygen_tag_line_prose_unchanged` (locks the transform-3 boundary).
      No air-compat allowlist entry: air leaves roxygen untouched, so it
      preserves arity's reflowed output and the fixed-point gauge sees no
      divergence. Incremental path unchanged (roxygen edits already fall back to
      block/full reparse; oracle corpus extended with markup-edit cases). Tag
      prose (`@param x <prose>`) is intentionally *not* flowed yet (transform 3).
      *Future (linter):* the protected-span leaf tokens have the same byte spans
      as the richer nodes a future roxygen-code-reference lint would need
      (resolve `\link{f}`/`[func()]`), so promoting tokens→nodes is additive.
    - [x] (3) hanging-indent reflow of tag prose (`TagUnit` in
      `src/formatter/roxygen.rs`). A tag line carrying inline prose
      (`@param x <prose>`, `@return <prose>`, `@seealso <prose>`, …) plus the
      plain-prose lines that follow it form **one reflow unit**: the normalized
      header (`@tag [arg]`, single-spaced — the "normalize tag-internal spacing"
      half) stays on the first line, and continuation lines hang-indent **two
      extra spaces** under `#'` (the tidyverse rule, `style/documentation.qmd`
      "Indents and line breaks"; applies to all description tags, not just
      arg-bearing ones). Absorbing the following lines is **forced by
      idempotence**: a `#'   …` continuation reparses as a separate plain-prose
      line whose leading whitespace the formatter drops, so without re-joining,
      `format(format(x))` would detach and de-indent it. `wrap_chunks_hanging`
      wraps with a narrower first-line budget (room beside the header) and the
      hanging-indent budget for continuations; protected spans stay atomic
      (reuses the transform-2 chunker via `chunk_elements`). **Passthrough**
      (header spacing normalized, never reflowed): `@examples`/`@examplesIf`
      bodies (transform 4), code tags (`@usage`/`@eval`/`@evalRd`), the
      `@section Title:` heading shape, namespace/identifier directives
      (`NON_PROSE_TAGS`), bare tags (`@export`), and tags written form-2 (tag
      alone on its line, body unindented). The transform-2 `is_unsafe_line_start`
      guard carries over: a prose chunk that could migrate to a continuation-line
      start and reparse as a list/header marker bails the unit to verbatim,
      marker-normalized lines. Fixtures `roxygen_tag_reflow_{param,return,seealso,
      absorb,idempotent}`, `roxygen_tag_normalize_spacing`,
      `roxygen_tag_alone_passthrough`, `roxygen_tag_examples_unchanged` (and
      `roxygen_tag_marker_space` now asserts the normalized spacing). No
      air-compat allowlist entry: air leaves roxygen untouched, so the
      fixed-point gauge sees no divergence.
    - [x] (4) run arity's own formatter on embedded R in
      `@examples`/`@examplesIf` (`ExampleBody` in `src/formatter/roxygen.rs`).
      The body lines are collected, stripped of their markers (reusing
      `content_text`), formatted as one R source unit via `format_with_style`,
      and re-prefixed. The body line-width budget is reduced by the marker prefix
      and indentation so the `#'`-prefixed output respects the line width
      (Tenet 1). **Conservative fallback**: a body that does not parse cleanly as
      R falls back to the current marker-normalized passthrough, byte-for-byte —
      this covers Rd-macro wrappers (`\dontrun{}`/`\donttest{}`/`\dontshow{}`,
      not valid R: `\` lexes as the lambda token, so a following identifier is a
      parse error) and any other unparseable example. Idempotent by construction
      (extraction inverts prefixing; the embedded formatter is itself idempotent
      at a fixed width). Fixtures `roxygen_examples_{format,multiline,idempotent,
      dontrun_passthrough}` and `roxygen_examplesif_format`. No air-compat
      allowlist entry: air leaves roxygen untouched, so it preserves arity's
      formatted output and the fixed-point gauge sees no divergence.

    LSP follow-ups: fold/semantic-token/completion awareness for roxygen
    (folding already preserved; completion may trigger inside `#'` lines).

  - [ ] **Full roxygen2 + markdown parser (CST-native block structure).** Evolve
    the parsing foundation above from "tags + inline protected spans, with block
    structure re-derived by the formatter" into a *complete* roxygen2 parser whose
    CST models block structure too: paragraphs, ordered/unordered lists (and
    nesting), fenced/indented code blocks, ATX/setext headings, block quotes,
    tables, **block-level Rd macros** (see the Rd bullet below), and the
    `@examples`/`@usage`/etc. bodies. The formatter (and a future linter/LSP) then
    consumes one structural model instead of re-deriving it from text with string
    heuristics.
    - *A roxygen block is Rd-first, markdown-second.* The block's native language
      is **Rd** (R documentation markup — the same syntax as `.Rd` files);
      markdown (when enabled) is a convenience layer roxygen2 *translates into*
      Rd, not a replacement for it. So Rd macros are always in scope, independent
      of `@md`. The parser needs a real Rd sub-grammar, not just markdown:
      - *Inline macros* (`\code{}`, `\emph{}`, `\strong{}`, `\link[pkg]{name}`,
        `\url{}`, `\href{}{}`) — **done** (single-arg, line-scoped):
        `ROXYGEN_RD_MACRO` is now a *node* whose content is sub-parsed
        (`ROXYGEN_RD_MACRO_{NAME,OPT,DELIM,VERB}` leaves, nesting, verbatim
        bodies for `\url`/`\verb`/`\samp`/…), and the projector emits the
        faithful nested Rd (`\code{\link{x}}` → `(\code (\link (TEXT "x")))`,
        `[pkg]` dropped, `\url` → `(VERB …)`). A `\code` body's *plain text*
        projects as verbatim `(RCODE …)`, not `(TEXT …)` (Stage 9, 2026-06-23):
        parse_Rd tags `\code` content as R code (whitespace preserved, split at
        newlines).
      - *Multi-argument macros* (`\item{term}{def}`, `\link[pkg]{name}`,
        `\method{generic}{class}`, `\href{url}{text}`) — multiple adjacent brace
        groups, not one. **`\item{term}{def}` landed (Stage 3, 2026-06-23):**
        `is_two_arg_rd_macro` (currently `item`) makes the lexer pull a second
        adjacent `{…}` into one macro token, the tree builder emits both groups as
        `\item` children, and the projector flushes at each closing `}` so the two
        groups stay separate atoms (`(\item (TEXT "a") (TEXT "first"))`);
        `describe_format` matches its pin. **`\href{url}{text}` landed (Stage 10,
        2026-06-23):** added to `is_two_arg_rd_macro`, but with a *per-argument*
        encoding — new `is_verbatim_rd_arg` makes the first arg (the URL) verbatim
        `(VERB …)` while the second (the link text) is sub-parsed like any latexlike
        body (`(\href (VERB "url") (GRP …))`); +5 projector cases. `\method`/`\section`
        extend the set when needed.
      - *Aggregating section tags* (`@slot`, `@field`) — **done (Stage 11,
        2026-06-23, projector-only):** every `@slot` (S4) / `@field` (reference
        class) of a topic aggregates into one `\section{Slots|Fields}{\describe{…}}`,
        each tag a `\item{\code{name}}{def}` (the name verbatim `RCODE`). The CST
        already modeled the tag arg + prose; `describe_section` in `project_rd.rs`
        synthesizes the aggregate. +2 projector cases (rx-853d2f8f, rx-d55651e1).
        **`@examples`/`@examplesIf` aggregation landed (2026-06-24h,
        projector-only):** every examples tag of a topic concatenates into a single
        `\examples`; `project_block` records a `has_examples` flag and emits one
        `(\examples ...)` (body is reformatted R → placeholder). +2 projector cases
        (rx-5ac40b37, rx-73a5b650). **`@section Title: body` body inlines landed
        (2026-06-24i, projector-only):** parse_Rd models `\section` as a two-arg
        structural macro, so the `section` arm now splits on the first `:`
        (`split_section_title`) and sub-parses both sides as inlines, GRP-wrapping a
        multi-atom argument (`grp_arg`) while a single-atom title stays bare
        (`(\section (TEXT "Foobar") (GRP (TEXT "With some") (\strong …) (TEXT ".")))`);
        the same `grp_arg` now wraps the `@slot`/`@field` `\item` definition when its
        prose carries a macro. +2 projector cases (rx-41e06b64, rx-1b26c2a4).
      - *Block macros that span many `#'` lines with nested content.*
        **`\itemize`/`\enumerate` landed (Stage 2, 2026-06-23):** a `\name{` whose
        group is unbalanced on its line opens a `ROXYGEN_RD_MACRO` node spanning
        `#'` lines (markers/newlines threaded as trivia), with brace-less `\item`
        as a name-only child; the projector reuses `serialize_macro`, and the
        formatter passes the node through verbatim (it had reflowed it into a
        run-on — that bug is fixed; `itemize_enumerate` matches its pin).
        **`\describe{ \item{term}{def} … }` landed (Stage 3, 2026-06-23):** the
        multi-arg `\item` (above) closes it; `describe_format` matches its pin.
        **`\tabular{rl}{ … \tab … \cr }` landed (Stage 4, 2026-06-23):** a balanced
        `RoxygenRdMacro` for a structural macro (`is_two_arg_rd_macro`, now also
        `tabular`) immediately followed by an unbalanced `{` body opens the block
        (`emit_block_open_arg_macro` decomposes `\tabular{rl}` into name + format
        group leaves, `emit_block_body_open` opens the body), with `\tab`/`\cr` as
        name-only children. The projector GRP-wraps a structural macro's multi-atom
        argument (`(\tabular (TEXT "rl") (GRP …))`); `tabular` matches its pin.
      - *Verbatim / non-prose content* (`\deqn{}`/`\eqn{}` carry LaTeX-ish math,
        `\preformatted{}`/`\verb{}` carry literal text, `\tabular` cells use
        `\tab`/`\cr` separators) — never reflow or markdown-interpret the interior.
      - *Comments and escapes* (`%` begins an Rd comment to end of line; `\%`,
        `\{`, `\}`, `\\` are escapes) — relevant to both losslessness and where a
        macro argument actually ends.
    - *Motivation — single source of truth.* Block structure is currently
      reconstructed twice in `src/formatter/roxygen.rs`: a classifier
      (`is_structured`/`starts_ordered_list_item`) decides what a *line* is, and a
      migration guard (`is_unsafe_line_start`) decides what a reflowed *chunk* may
      become — and the two must be kept in lockstep by hand or idempotence breaks.
      That coupling is exactly what produced the "`2008.` mid-sentence treated as
      an ordered-list marker, so the `@source` paragraph bailed to verbatim" bug
      (fixed by collapsing both onto a shared `ordered_marker` predicate + the
      CommonMark "a non-`1` ordered list can't interrupt a paragraph" rule;
      fixtures `roxygen_{tag_reflow_year_in_prose,reflow_year_not_list_item,bail_ordered_list}`).
      A real block CST makes that class of bug structurally impossible: the
      formatter reflows *prose nodes* and never guesses whether a chunk might
      reparse as a marker.
    - *Markdown flavor — confirm before implementing.* When markdown is on,
      roxygen2's markdown is **not** hand-rolled: it delegates to the `commonmark`
      R package (a binding to `cmark`/`cmark-gfm`), then translates the result
      into Rd (`` `code` `` → `\code{}`, `[fn()]`/`[text][dest]` → `\link`, etc.).
      So the markdown layer's base grammar is CommonMark, but the exact extension
      set roxygen enables (tables, autolinks, strikethrough, …) must be pinned
      against roxygen2's actual `commonmark::markdown_*` call. Settle the precise
      flavor as an early step, because — together with the Rd grammar above — it
      decides what the parser accepts. Note markdown and Rd **coexist** in one
      block (markdown prose can contain inline `\emph{}`; an Rd `\item` body can
      contain markdown), so the grammar is genuinely the union, not a mode switch
      between two disjoint languages.
    - *Markdown mode is opt-in.* roxygen markdown is only active under
      `@md`/`@noMd` or `Roxygen: list(markdown = TRUE)` in `DESCRIPTION`.
      **Mode resolution landed (Stage 5, 2026-06-23):** `resolve_roxygen_block`
      (`src/parser/roxygen.rs`) scans the contiguous `#'` block for an `@md`/`@noMd`
      directive (last wins; default off), the lexer caches it per block and threads
      `md: bool` into the prose lexer, so lexing is mode-keyed. **Inline markdown
      landed:** under `@md`, `*x*`→`\emph`, `**x**`→`\strong`, and a code span →
      `\code`/`\verb` per roxygen2's `can_parse` (arity-parseability) rule; new
      `ROXYGEN_MD_EMPH`/`STRONG`/`CODE` leaves; `markdown_inline` matches its pin.
      **Block lists landed (Stage 6, 2026-06-23):** under `@md`, a `-`/`*`/`+`
      list → `\itemize` and a `1.`/`1)` list → `\enumerate`, each item a name-only
      `\item` ahead of its content. The lexer carves a line-start `RoxygenMdListMarker`
      (mode-keyed, punctuation only), `emit_md_list` builds a `ROXYGEN_MD_LIST` of
      `ROXYGEN_MD_LIST_ITEM`s (markers/newlines threaded as trivia), applying the
      CommonMark interrupt rule (an ordered list ≠ 1 can't interrupt a paragraph —
      its marker stays inline text); the projector adds an `Inline::MdList` arm and
      the formatter passes the list through marker-normalized. `markdown_list`
      matches its pin. **Inline links landed (Stage 12, 2026-06-24c):** under `@md`,
      an inline `[text](url)` link → `\href{url}{text}` (`(\href (VERB url) (TEXT
      text))`, URL verbatim). The lexer's `[`-recognizer is now **mode-gated** (it
      was firing in non-`@md` mode too, mislabeling literal Rd brackets as
      `ROXYGEN_MD_LINK` — fixed to match `*`/`` ` ``/list-markers, so a link's
      existence implies `@md`); the projector adds an `Inline::MdLink` arm.
      rx-7743ba62/rx-0605d020 match their pins. **Reference + shortcut links landed
      (Stage 13, 2026-06-24d):** the lexer now carves *any* bracket-free `[…]` not
      followed by `{` (mirroring roxygen2's `get_md_linkrefs` regex), and the
      projector's `resolve_md_link` replicates `parse_link` (`markdown-link.R`):
      shortcut `[obj]`→`\link{obj}`, `[func()]`/`` [`code`] ``→`\code{\link{…}}`,
      `[name-class]`→`\linkS4class{name}` (`\link{pkg::name}` with a package),
      reference `[text][dest]`→`\link{text}` (the dropped `\link[…]` topic option
      means only head + display + `\code`-wrap survive). +10 projector cases.
      Static-context faithful: package resolution is non-static, so a `pkg::` prefix
      comes only from an explicit `::` (the corpus's `current_package == ""`). Still
      **deferred:** the settled loose-file/`DESCRIPTION`
      **default-ON** (only an explicit per-block `@md` enables markdown today —
      flipping the default reinterprets every block, so it needs its own re-bless
      pass). **Verbatim `\preformatted` block landed (2026-06-24s):** `\preformatted`
      is now projected as a verbatim block macro — its body becomes one `(VERB …)`
      per line (parse_Rd's verbatim split), not a whitespace-collapsed `(TEXT …)`.
      Projector-only (`serialize_macro` early-arm + `preformatted_atoms`, mirroring
      `serialize_md_html_block`/`verb_atoms`); the line-start block was already a
      `ROXYGEN_RD_MACRO`, so no parser change. + curated `preformatted`. *Still
      backlog:* the mid-line opener (a prose prefix before `\preformatted{`,
      `rx-0a1710c0`) — a third block-opener form needing the lexer to split a prose
      run at a mid-line verbatim opener and the grouper to split the paragraph.
      **Markdown nested lists landed (2026-06-24r):** `emit_md_list` now
      recurses by CommonMark indentation (a following list line indented to an
      item's content column opens a nested `ROXYGEN_MD_LIST` inside that item, a
      line back at the list's marker column is a sibling), the projector handles a
      nested `ROXYGEN_MD_LIST` child (new `push_inline` arm; `md_list_is_ordered`
      now reads direct-child markers only, so a nested ordered sublist can't flip
      the parent's head), and the formatter **preserves** the content indentation
      (`normalize_list_marker_text`) because it now sets the nesting depth —
      flattening it would change the rendered Rd (a behavior change). +1
      (rx-91e67e79) + curated `md_nested_list`. **Nested *Rd* block macros landed
      (2026-06-24q):** an unbalanced nested `\name{` opener inside a block macro's
      body (`\itemize{` inside `\enumerate{`) now opens a child `ROXYGEN_RD_MACRO`
      via a `BodyFrame` stack in `emit_block_content` (replacing the flat brace-depth
      counter); the projector already recursed, so +1 (rx-959fc227) + curated
      `rd_nested_list`. Rd nesting is brace-driven (indentation-independent), so this
      is distinct from the markdown nested list above.
      **Images + `\figure` landed (Stage 14, 2026-06-24f):** the Rd `\figure{path}{caption}`
      macro is now a two-arg macro with both args verbatim (`TWO_ARG_RD_MACROS` +
      `is_verbatim_rd_arg`), and a markdown image `![alt](url "title")` lexes to a new
      `ROXYGEN_MD_IMAGE` leaf (`scan_md_image`, mode-gated). The projector's
      `resolve_md_image` mirrors roxygen2's `mdxml_image`: alt dropped, `\figure{url}{title}`,
      wrapped in `\if{html}{…}`/`\if{pdf}{…}` per the extension-keyed `get_image_format`
      rule (svg→html, pdf→pdf, raster/unknown→bare). +3 projector cases (rx-29d590cf,
      rx-44eb4ad9, rx-561b9e7d). Reference/shortcut images (`![alt][ref]`/`![alt]`) are
      backlog (inline form only). **Digit-bearing macro names landed (Stage 15,
      2026-06-24g):** an Rd command name is `[A-Za-z][A-Za-z0-9]*`, so `\linkS4class`
      now lexes as one macro (the name scan stopped at the digit `4`, dropping the
      macro to literal text). New shared `rd_macro_name_end` helper unifies the six
      duplicated name scans (lexer + tree builder + block builders). +1 projector
      case (rx-852ee490). **Brace-less unknown macros landed (Stage 16,
      2026-06-24j):** a brace-less `\word` not in the built-in Rd keyword table
      (new `is_known_rd_macro`/`KNOWN_RD_MACROS`, verified vs R 4.5) projects to
      `(UNKNOWN "\\word")`; `scan_rd_macro` carves it only when unknown (a known
      brace-less name stays literal prose — zero-arg name-only/arg-misuse is
      backlog), and the projector's name-only branch keys on the same table. +2
      projector cases (rx-16f78b2f non-md, rx-b8082617 md). Zero-arg name-only
      *rendering* in prose (`\cr`→`(\cr)`) is deferred (never in-scope today).
      **URL autolinks + empty-dest links landed (Stage 18, 2026-06-24l):** the
      lexer now carves a CommonMark absolute-URI autolink `<scheme:…>`
      (`scan_md_autolink`, mode-gated, reusing the `ROXYGEN_MD_LINK` kind; raw
      HTML `<p>`/`<img …>` has no scheme `:` so it stays literal), and the
      projector mirrors roxygen2's `mdxml_link`: a destination that is empty *or*
      equal to the link text → `\url{text}`, else `\href` (`<url>` and `[url]()`
      both → `\url`). +1 projector case (rx-f97e8917) + curated `markdown_url`.
      **Inline-link-text code-span sub-render landed (Stage 19, 2026-06-24m):**
      roxygen2 renders a link's markdown *children*, so an inline `[`code`](url)`
      now carries the rendered span as its `\href` text arg
      (`(\href (VERB url) (\verb (VERB "code")))`) via the new `link_display_atom`
      (a single code span → `md_code_atom`'s `\verb`/`\code`, else plain `(TEXT …)`);
      the reference path's always-`\code` wrap was already correct. +1 projector
      case (rx-3c528f59). General *mixed* inline sub-rendering in link text
      (emphasis/strong) and email autolinks remain backlog.
    - *Hard constraints (the reason this is non-trivial).* Must preserve
      losslessness (Tenet 4: `reconstruct(text) == text`) against CommonMark's
      context-sensitive, whitespace-significant grammar (lazy continuation lines,
      tight/loose lists, setext underlines, trailing-space hard breaks); must fit
      the salsa incremental pipeline (Tenet 2) — today roxygen edits already fall
      back to block/full reparse, which a richer grammar can keep but should not
      regress; and the inline protected-span leaves
      (`ROXYGEN_CODE`/`ROXYGEN_RD_MACRO`/`ROXYGEN_MD_LINK`) already carved by
      `lex_roxygen_prose` are the precedent and the inline layer to build the
      block layer *over* (promoting tokens → nodes is additive, as already noted
      under transform 2).
    - *Scope note.* This subsumes the formatter's `is_structured` family and the
      `is_unsafe_line_start` guard; it does **not** require a separate Markdown
      renderer — arity still only needs the structure to decide reflow boundaries
      and embedded-R extents, not to emit HTML.
    - *Primary driver — projector parity (Phase 1 skeleton landed).* This is
      **parser work**: the gate compares the *CST's structure* to roxygen2, so it
      forces the structure into the parser rather than rewarding formatter
      heuristics. `src/roxygen/project_rd.rs` projects the CST to the parser-owned
      Rd **section subtrees** (excluding roclet-*generated* scaffolding —
      `\name`/`\alias`/`\usage`/the `\arguments` wrapper); `tests/roxygen_projector.rs`
      diffs that against pinned roxygen2 section trees — **pure Rust, no R, runs in
      plain `cargo test`**, allowlist-gated (`roxygen-projector-allowlist.txt`). Two
      pin sources: the curated dir corpus (`<stem>.rdtree`) and the **harvested
      corpus's projector-eligible subset** (`roxygen-sections.jsonl` — 151/217
      single-topic, self-contained blocks; `@inherit`/`@template`/`@eval`/… filtered
      out as resolve-from-elsewhere, kept in the fixed-point net instead). Progress:
      **133 matching / 28 divergent** of 161 pinned (was 42; `rd_macros`,
      `itemize_enumerate`, `describe_format`, `tabular`, `@md` inline + block lists,
      the title-as-description fallback, the `@tag NULL` suppression sentinel,
      `\code`-body RCODE, `\href` per-arg verbatim, `@slot`/`@field` aggregation,
      `@md` inline + reference + shortcut links, the **intro paragraph split**
      (roxygen2's `parse_description`: 1st intro paragraph = title, 2nd =
      description, the rest = details merged with explicit `@details`; body parts
      grouped by blank `#'` lines), markdown images + Rd `\figure`, digit-bearing
      Rd macro names (`\linkS4class`), **multiple `@examples`/`@examplesIf`
      aggregating into one `\examples`**, `@section` body inline macros, brace-less
      unknown macros, then **markdown fenced code blocks** (` ``` `/` ```r ` →
      `\if{html}{\out{<div…>}} \preformatted{…} \if{html}{\out{</div>}}`, info →
      `sourceCode` class), then **URL autolinks + empty-dest links** (`<url>`/`[url]()`
      → `\url`), then **inline raw HTML** (`<img …>`/`</span>` → `\if{html}{\out{<tag>}}`,
      `mdxml_html_inline`), then **block raw HTML** (`<p>…</p>` line-start, CommonMark
      start condition 6 → one `\if{html}{\out{<verb-per-line>}}`, `mdxml_html_block`),
      then **`@rawRd`** (content injected verbatim as bare top-level Rd nodes, no
      wrapping section macro; also switched the driver's section sort to
      `method = "radix"` so pins are byte-order/locale-independent, matching the
      Rust projector — the first non-`(\…)`-headed section exposed the gap)
      — closed). Now **139 matching / 24 divergent** of 163 pinned.
      The remaining divergences are the worklist. Run
      `task roxygen-projector`; re-mint with
      `task roxygen-projector-refresh`; re-seed with `task roxygen-projector-seed`.
      Use the `roxygen-parity` skill.
    - *Coverage net — harvested fixed-point (landed, secondary).* A harvested oracle
      corpus (`tests/oracle/corpus/roxygen.jsonl`, 217 standalone blocks mined from
      roxygen2's own tests by `scripts/harvest-roxygen-corpus.R`) measures the fixed
      point `roxygen2(format(x)) == roxygen2(x)` per case, gated opt-in by
      `tests/oracle/roxygen-allowlist.txt`. Baseline **215 preserving / 1 divergent /
      1 skipped**. This is a broad *semantic*-preservation net for the formatter, **not**
      the parser-growth driver: it is cosmetic-blind (a reflowed `\describe` renders
      identical Rd, so it passes here) and R-dependent (`#[ignore]`d). Its remaining
      divergent `@md` slug (`\preformatted{}` `rx-0a1710c0`; nested lists `rx-91e67e79`,
      inline raw HTML `rx-299f50fb`, and block raw HTML `rx-daf9322f` now **closed**) is
      downstream of the same block-structure work. Run `task roxygen-harvest`; ratchet via
      `task roxygen-harvest-seed`.
    - *Parser architecture — refactor BEFORE the next markdown push (links/tables/
      nested lists).* The roxygen parser is sound but its phase discipline has eroded
      as it grew; `src/parser/roxygen.rs` is now ~1700 lines, the largest file in the
      parser. Do these while it's that size, not after it doubles. Ranked:
      1. **~~Unify the `TokKind` line-body classification (highest correctness ROI).~~
         DONE (2026-06-24).** New `RoxygenRole` enum + wildcard-free
         `TokKind::roxygen_role` (`lexer.rs`) is the single, compiler-policed source for
         the lexer/parser side: `is_comment_like`, `classify_line`, `is_line_body_kind`,
         and the block-macro inline-span arm all derive from it; adding a `TokKind` is
         now a compile error in the one match. The `SyntaxKind` side (rowan-flat, can't
         be policed) collapsed onto a single `SyntaxKind::is_roxygen_prose_content`
         (`syntax.rs`) that the formatter's `is_blank`/`is_tag_prose_kind` share. The 8
         silent `matches!` lists are gone; `expr.rs`'s atom fallthrough was already an
         exhaustive anchor and was left as-is. Pure refactor, byte-identical (projector
         + format-stability gates unmoved).
      2. **~~Split `roxygen.rs` along its real phase boundaries.~~ DONE
         (2026-06-24).** Carved the 1686-line file into a thin parent
         (`roxygen.rs`: macro-classification tables + `scan_balanced`/`utf8_len`
         + re-exports) and three submodules over the phase boundaries:
         `roxygen/lex.rs` (sub-lexing, text → `Vec<Token>`, + the lexer tests),
         `roxygen/group.rs` (block grouping / section-paragraph skeleton,
         `Vec<Token>` → `Vec<Event>`), and `roxygen/build.rs` (the
         `emit_block_*`/`emit_md_list` Rd-macro + markdown structure builder).
         Pure refactor, byte-identical (projector 93/66 + format-stability gates
         unmoved, clippy + fmt clean). **Non-goal (decided, not "deferred"):**
         hoisting the builder onto the shared `core.rs`/`cursor.rs`/`recovery.rs`
         infra. On inspection that "infra" is *not* a richer abstraction to adopt:
         `cursor.rs` is the **same** `fn(tokens, i) -> usize` index-threading idiom
         the builder already uses (no `Cursor` type with `bump`/`peek`), and
         `recovery.rs` builds `ERROR` nodes for malformed **R expressions** — a
         model roxygen **deliberately rejects** (greedy + lossless, no close
         delimiter, no ERROR nodes; Tenets 3/4). The only honest reuse is 3–4
         *lookahead-only* whitespace skips → `cursor::skip_ws` (the builder's other
         skips emit the trivia as events, which `skip_ws` can't), and that's a
         **drive-by** for whenever we next edit `build.rs`, not a session. The
         file-size pain that motivated the cleanup is gone (4 modules,
         113/200/430/996), so the marginal structural win doesn't justify a
         behavior-touching rewrite.
      3. **Watch the block-opener Form A/Form B split.** Because the lexer greedily
         eats balanced `{…}` groups, `is_block_macro_line` has two structurally
         different entry forms and macro-arity logic is split lexer↔tree-builder
         (`scan_rd_macro` ↔ `build_rd_macro`). Correct but intricate; a *third* form
         appearing is the signal to reconsider the lex-time greediness.
      - *Lower-stakes, documented known-gaps (revisit only if forced).* Roxygen is
        non-incremental — edits fall back to block/full reparse (`reparse.rs`; Tenet 2
        gap, but doc comments are statement-level so a full reparse is cheap), and
        roxygen owns ~⅓ of all `SyntaxKind`s (69/213); the markdown roadmap will keep
        inflating both. The projector being `pub` only to reach a test crate is an
        accepted minor layering smell.

## Linter

Closest precedent: **jarl** (`etiennebacher/jarl`, Rust + rowan + air-parser,
55+ rules, suppression directives, LSP, autofix). The foundation pass borrows
shape (diagnostic + `Violation` trait, `PackageOrigin` enum, `# arity-ignore`
suppression directive style, annotate-snippets rendering) but stays its own
project --- arity is a unified formatter + linter + LSP binary on arity's own
in-tree parser, not a drop-in jarl replacement.

### Rule roadmap

Adapt jarl's catalog (https://jarl.etiennebacher.com/rules) to arity's
architecture, phased so high-value, low-effort rules land first and anything
blocked on missing infrastructure is sequenced right after that infra. Five
rules ship today: `undefined-symbol`, `unused-binding`, `duplicate-formal`,
`duplicated-arguments`, `equals-na` (`correctness/`), `assignment-in-condition`,
`shadowed-builtin`, `redundant-equals`, `redundant-ifelse` (`suspicious/`).

Cost model driving the order: a rule is **cheap** (`syn`) when it only needs the
CST + AST + literal inspection; **medium** (`ns`) when it must confirm a callee
resolves to base R (not a user redefinition) via `SymbolProvider`; **expensive**
(`sem`) when it needs the `SemanticModel` (scopes/flow). Anything needing R
evaluation or type inference is **out of scope** --- arity stays static. Pure
layout (quotes, leading zero, spacing) is the **formatter's** job (Tenet 1), not
the linter's.

**Category directories.** Keep `correctness/` and `suspicious/`; add
`readability/`, `performance/`, `meta/` (suppression-directive rules), and
`pkg/dplyr/` + `pkg/testthat/`. No `style/` dir --- pure layout is the
formatter's. Public rule IDs stay flat kebab-case (category is a directory
concern, as `all_rule_ids()` already is).

#### Phase 0 --- Infrastructure (unblocks everything)

- [x] **§I0 Single-walk dispatch** (landed). Rules declare interest via
      `Rule::interests() -> &[SyntaxKind]` and receive `Rule::check(element, ctx,
      sink)` once per matching element during *one* shared
      `descendants_with_tokens()` traversal (dispatch table is a flat
      `Vec<Vec<usize>>` indexed by `kind as usize`, sized by `SyntaxKind::COUNT`).
      Model-/comment-driven rules leave `interests` empty and override
      `Rule::check_file(ctx, sink)`, a once-per-file pass. New node-shape rules
      must subscribe via `interests`/`check` rather than walking the CST
      themselves.
- [x] **§I1 Matchers** (`src/linter/rules/matchers.rs`, landed): `call_named`,
      `callee_name`, `is_callee` (moved out of `shadowed_builtin`), `args`/
      `nth_arg`/`named_arg`, `binary_parts`, literal classifiers
      (`is_true`/`is_false`, `is_na`, `is_null`, `is_nan`, `is_bool_symbol` for
      T/F), plus `element_text` and an `is_atom` precedence guard for negating
      rewrites. Reduced each syntactic rule to ~30 lines.
- [x] **§I3 Namespace-confirmation helper** (landed): `RuleContext::resolves_to_base`
      confirms a bare call invokes base R --- simple-name callee that is a base
      export (`symbols.is_base`), not namespace-qualified (`pkg::f`), not shadowed
      by a local binding (`model.resolve_local` over the callee read), and not
      masked by an attached package (effective `symbols.origin(...)` is a default
      package). Unblocks confident Phase 2.
- [x] **§I7 CLI `--select`/`--ignore`** (landed). `arity lint` now accepts
      `--select`/`--ignore` (repeatable or comma-separated); CLI values replace
      the configured `select`/`ignore`, applied before fixes. Unknown IDs error
      via the existing `LintError::UnknownRule`. Covered by `tests/config.rs`.
- [x] **Registration single source of truth** (landed). `all_rules()` is now
      the sole list; `ALL_RULE_IDS` is replaced by `all_rule_ids()`, which
      derives the valid-ID set from `all_rules()` so the two can't drift.

#### Phase 1 --- High-signal, purely syntactic, safe fixes (`syn`)

Match a call/operator shape with deterministic fixes. Match bare names for now;
harden against shadowing in Phase 4.

- [ ] `browser` (suspicious, safe-delete) --- leftover debug call.
- [x] `equals-na` `x == NA` -> `is.na(x)` (correctness, safe; landed --- `==`
      form only). Still open: `equals-nan` -> `is.nan` (safe); `equals-null`
      (correctness, none/unsafe --- `== NULL` rewrite is less mechanical).
- [ ] `empty-assignment` (correctness, none).
- [x] `duplicated-arguments` `f(a = 1, a = 2)` (correctness, none; landed) ---
      mirrors `duplicate-formal`. Warning (not error): `c(a = 1, a = 2)` is legal.
- [x] `redundant-equals` `x == TRUE` -> `x`, `x == FALSE` -> `!x` (suspicious,
      safe; landed --- `!`-rewrite withheld for non-atom operands via `is_atom`).
- [x] `redundant-ifelse` `ifelse(c, TRUE, FALSE)` -> `c`,
      `ifelse(c, FALSE, TRUE)` -> `!c` (suspicious, safe; landed).
- [x] `true-false-symbol` `T`/`F` -> `TRUE`/`FALSE` (readability, safe; landed).
      Graduated early rather than deferred to the Phase 4 hardening sub-pass:
      `SemanticModel::idents()` already keeps `T`/`F` as reads (excluding
      name-positions and reserved literals) and `resolve_local()` is
      scope-accurate, so the rule reports/fixes only reads that resolve to base
      R and skips locally-rebound `T`/`F`. The same-span token swap never alters
      layout, so the fix is `Safe`.
- [x] `repeat` `while (TRUE)` -> `repeat` (suspicious, safe; landed). Matches
      only the reserved literal `TRUE` (rebindable `T` left to
      `true-false-symbol`); the fix replaces the `while (TRUE)` header with
      `repeat` and is withheld when the clause carries a comment.
- [x] `vector-logic` `&`/`|` -> `&&`/`||` in `if`/`while` condition
      (correctness, safe; landed). Flags only operators in conditional context
      --- the walk descends from the condition through parens, `!`, and
      `&&`/`||`/`&`/`|`, but stops at a function call (`if (any(a | b))` is left
      alone). The fix doubles the operator token, a tight format-clean edit.
- [x] `comparison-negation` `!(a == b)` -> `a != b` (readability, safe; landed).
      Matches both the parenthesized `!(a == b)` and the bare `!a == b` (the `!`
      precedence bug that previously blocked the bare form is now fixed; see the
      `## Parser` section). The replacement (a comparison) binds tighter than the
      `!` it replaces, so no parent guard is needed; fix withheld on a commented
      operand. The replacement (a comparison) binds tighter than the `!` it
      replaces, so no parent guard is needed; fix withheld on a commented clause.
- [x] `outer-negation` `any(!x)` -> `!all(x)`, `all(!x)` -> `!any(x)` De Morgan
      (readability, safe; landed). Direction matches lintr's `outer_negation`
      (pull negation out). Fires only when every positional arg is `!`-negated
      (`na.rm` preserved). The rewrite drops a primary to a `!`-expr, so the fix
      is withheld in parent contexts that bind tighter than `!` (`is_safe_context`).
- [ ] `implicit-assignment` (suspicious, none) --- scope to avoid overlap with
      existing `assignment-in-condition`.

#### Phase 2 --- Call-rewrite idioms, namespace-confirmed (`ns`)

- [ ] **§I2 regex/string-literal helper** first: read a `STRING` token's
      unquoted contents; classify regex metachars / single anchor (`^`/`$`).
      Blocks `string-boundary`, `fixed-regex`.
- [x] `any-is-na` `any(is.na(x))` -> `anyNA(x)` (performance, safe; landed) ---
      flagship. First rule in the new `performance/` category. Fires only on the
      clean shape (`any` with one positional arg that is `is.na` with one
      positional arg), namespace-confirmed via `resolves_to_base` on *both*
      callees (a local/qualified/masked redefinition of either is left alone),
      so the fix is `Safe`. The replacement `anyNA(...)` is a primary like the
      `any(...)` it replaces, so no precedence guard is needed; the fix is
      withheld when a comment outside the preserved inner argument would be
      dropped (a stray comment parses as a value-less `ARG`, so matching is on
      value-bearing args).
- [x] `any-duplicated` `any(duplicated(x))` -> `anyDuplicated(x) > 0`
      (performance, `ns`, safe). Fires only on the clean single-positional-arg
      shape and namespace-confirms both `any` and `duplicated` resolve to base R.
      Unlike `any-is-na`, the replacement is a *comparison* (binds looser than the
      `any(...)` call it replaces), so the fix is withheld in a context that binds
      tighter than a comparison (arithmetic, indexing, `$`/`@`, ...) where the bare
      rewrite would misparse, and also when a comment outside the preserved inner
      argument would be dropped. The finding is still reported in both cases.
- [ ] `lengths` `sapply(x, length)` -> `lengths(x)` (performance, safe).
- [ ] `nzchar` `nchar(x) > 0` -> `nzchar(x)` (performance, safe).
- [ ] `seq`/`seq2` `1:length(x)` -> `seq_along`, `1:n` -> `seq_len`
      (performance, safe) --- off-by-one safety, high value.
- [ ] `is-numeric` (correctness, safe); `class-equals` `class(x) == ...` ->
      `inherits` (performance, unsafe --- `class()` is a vector).
- [ ] `string-boundary` `grepl("^a", x)` -> `startsWith` (readability, safe when
      fixed literal + single anchor); `fixed-regex` add `fixed = TRUE`
      (performance, safe).
- [ ] `sort` `sort(x)[1]` -> `min`, etc. (performance, unsafe).
- [ ] `internal-function` `pkg:::fn` via
      `BinaryExpr::namespace_access().internal` (correctness, none) --- cheap.

#### Phase 3 --- SemanticModel rules + config plumbing

- [ ] **§I4 per-rule config**: add a `[lint.rules.<id>]` TOML table + typed
      per-rule struct in `src/config.rs`, threaded into rules via a
      `config`/`&RuleConfig` field on `RuleContext`. **Blocks**
      `undesirable-function`, `download-file`.
- [x] `unreachable-code` after `return()`/`stop()` (correctness, ns,
      unsafe-delete; landed) --- flags statements following an unconditional
      base-R `return()`/`stop()` that is a direct block statement (`return`
      gated on an enclosing function); namespace-confirmed, fix withheld when it
      would drop a comment. Both-branches-return (needs CFG) is out of scope.
- [ ] `if-always-true` literal `if (TRUE/FALSE)` only --- no const-folding
      (correctness, unsafe).
- [ ] `unused-function` (suspicious, sem, none) --- reuse
      `unused_local_bindings`; **default-off** (exported pkg funcs look unused).
- [ ] `duplicated-function-definition` (suspicious, sem, none).
- [ ] `for-loop-index`/`for-loop-dup-index` (suspicious, sem, none).
- [ ] `unnecessary-nesting` collapsible nested `if` / single-stmt block
      (readability, sem, unsafe).
- [ ] `coalesce` `if (is.null(x)) y else x` (performance, sem, unsafe) --- may
      want §I5 multi-edit fix.
- [ ] `undesirable-function` (suspicious, ns + config, none) --- needs §I4;
      **default-off**. `download-file` (correctness, ns, none) --- low priority.

#### Phase 4 --- Meta (suppression) rules + hardening

- [ ] **§I6 suppression refactor**: have `SuppressionMap` expose the parsed
      directive list (rule, range, has-reason, raw) and surface it on
      `RuleContext` (`suppressions`). `outdated-suppression` also needs the
      driver (`check.rs`/`run_rules`) to record which suppressions actually
      matched a diagnostic --- a post-pass, not a per-rule concern.
- [ ] `misnamed-suppression` (vs `ALL_RULE_IDS`, safe), `blanket-suppression`
      (none), `unexplained-suppression` (none, **default-off**),
      `outdated-suppression` (safe-delete). These subsume the reserved
      `arity-ignore-unused` follow-up below.
- [ ] **Hardening sub-pass**: upgrade Phase 1/2 fixes from bare-name to
      `resolves_to_base`-confirmed + shadow-checked, graduating the call-rewrite
      rules Unsafe -> Safe and suppressing FPs where `any`/`is.na` etc. are
      user-redefined. (`true-false-symbol` already shipped shadow-checked.)

#### Phase 5 --- Package-aware rules

Gated on the package being attached (`model.loaded_packages()`).

- [ ] `pkg/testthat/` as one cohesive PR (shared `expect_*` matcher):
      `expect-true-false`, `expect-length`, `expect-named`, `expect-null`,
      `expect-type`, `expect-s3-class`, `expect-match`/`expect-no-match` (all ns,
      safe). High value for test-heavy repos.
- [ ] `pkg/dplyr/`: `dplyr-filter-out` `filter(!(x %in% y))` (ns, safe). Defer
      `dplyr-group-by-ungroup` --- needs **§I8 pipe-chain abstraction**
      (`%>%`/`|>` stage walk) that doesn't exist yet.

#### Out of scope (recorded so they aren't silently dropped)

- **Formatter's domain (Tenet 1):** `quotes`, `numeric_leading_zero`, spacing,
  indentation, semicolons, trailing whitespace --- excluded from the linter.
- **Needs R evaluation / type inference (arity is static):** `all_equal`,
  `length_levels`, `length_test`, `matrix_apply`, `list2df`, `which_grepl`,
  `grepv`, `sample_int`, `system_file`, `sprintf` arg-checking, full `coalesce`,
  const-folded `if_always_true`. Implement only an exact-AST-shape subset where
  one exists, else defer.
- **Too noisy without opt-in:** `undesirable-function`, `unused-function`,
  `unexplained-suppression` --- all **default-off**.

#### `RuleContext`/`Rule` extensions implied above

0. `Rule::interests`/`check`/`check_file` single-walk dispatch (§I0, landed).
1. `RuleContext.config` (§I4). 2. `RuleContext.suppressions` (§I6).
3. Registration single source of truth. 4. Multi-edit `Fix` (§I5) only when a
non-contiguous fix is needed. 5. Optional `Rule::category()` for category-level
`--select` later.

## Language Server

Today the server advertises **formatting** (whole-document + range), **hover**
(index-backed), **quick-fix code actions**, **pushed diagnostics**, and
**intra-file rename** (`prepareRename` + `rename`, local bindings only)
(`src/lsp.rs` `server_capabilities`). Everything else below is unimplemented.
Much of
it is closer than it looks: the per-file `SemanticModel` (`src/semantic.rs`:
scope tree, bindings, identifier *read* sites, `loaded_packages`,
`referenced_packages`) plus the salsa queries (`semantic_model`, `file_exports`,
`file_free_reads`, `source_edges`) and the harvested package index
(`src/rindex/`) already supply most of the analysis these features need --- the
work is mostly wiring resolution results to LSP responses, not new analysis.
Roughly ordered by leverage-to-effort:

### Prerequisites & blockers

There are **no hard architectural blockers** --- the parser and salsa model are
already shaped for this. A grounded audit (2026-06-11) found the load-bearing
infrastructure present and reusable:

- **Binding def spans + read-site provenance are present.** `Binding` carries
  `def_range: TextRange` (`src/semantic/binding.rs`); read sites
  (`IdentRef { name, range, scope }`, `src/semantic.rs`) resolve to a
  `BindingId` via `resolve_local`. So go-to-def, document symbols, intra-file
  references/highlight, and intra-file rename are wiring over existing data.
- **Position mapping is present.** `LineIndex` (`src/text/line_index.rs`) does
  byte↔UTF-16-`Position` conversion, already used by hover and diagnostics.
- **The read-snapshot path is present.** The lint thread owns the persistent
  db; read requests get a cheap `Analysis` snapshot (`src/incremental.rs`
  `snapshot`, dispatched in `src/lsp.rs`). New read-only features drop into the
  same path --- no new threading. (Caveat: any feature that *caches* a location
  must validate against the current version, as `hover_via_db` already does
  with its `file_text != text` check, because def `TextRange`s shift on edit.)
- **Project-level aggregation already exists in principle:** an interned
  `Project` key + `project_graph`/`visible_symbols` tracked queries aggregate
  `file_exports`/`file_free_reads`/`source_edges` across members
  (`src/project/graph.rs`).

Two genuine gaps gated the **cross-file** half of the list (both soft --- new
infra that builds *with* the grain, not architectural fights). The first has
landed; the second is still open but only matters for cross-edit-stable handles:

- [x] **Reverse `source_edges` index + an explicit workspace file-set.** Done:
  `reverse_source_edges(db, project)` (`src/project/graph.rs`) is the
  who-sources-me map (`Eq`, backdates), and the file-set is now the explicit
  salsa `Workspace` input (`src/incremental.rs`) from which the interned
  `Project` is derived by `workspace_project` — no per-request disk walk. See
  *Cross-cutting prerequisite* below for the full landed shape.

- [x] **Stable cross-edit node references.** Done, landed with its first
  consumer (intra-file rename). Three pieces: (1) rowan's typed
  same-revision handles `AstPtr`/`SyntaxNodePtr` re-exported from
  `src/ast.rs`; (2) arity's canonical `NodePtr` (`src/syntax/ptr.rs`) — a
  `(kind, range)` handle that owns its construction (rowan's is closed) and
  derives `serde`, so it can be mapped onto a new revision *and* ride an LSP
  `data` field, with a hand-written `try_to_node` via `covering_element`;
  (3) the cross-edit layer `map_range_through_edit(s)`
  (`src/parser/reparse.rs`) that shifts a stored range through an `Edit`
  (or returns `None` when the edit overlaps the node), surfaced as
  `Analysis::resolve_ptr` (`src/incremental.rs`, the db-backed re-resolution)
  and exercised live by `compute_rename_with_anchor` (`src/lsp.rs`, which
  resolves against the authoritative buffer). The `prepareRename` anchor that
  "survives typing" is the worked example; a persistent call-hierarchy item
  reuses the same `NodePtr` + `serde` form for its `data` round-trip.

### Navigation

- [x] **Go-to-definition** (`textDocument/definition`). Intra-file resolves a
  read site (or the def itself) to its local binding via the shared
  `resolve_local_target` and reports `Binding::def_range`
  (`compute_definition` / `definition_via_db`). Cross-file resolves a bare
  top-level name against the workspace `project_defs` index — its first
  consumer — via the new `Analysis::workspace_def_sites`, recovering each
  span per file with `def_range_in`. Package-export / namespaced targets have
  no in-tree location, so they return nothing and lean on hover (as planned).

- [x] **Go-to-references / find-all-references** (`textDocument/references`). The
  inverse of go-to-definition, in the same two phases. Intra-file: the cursor
  resolves to a local binding (shared `resolve_local_target`) and every
  `idents()` read of it is reported via the shared `local_occurrences`
  (`compute_references` / `references_via_db`), honoring
  `context.includeDeclaration`. Cross-file: a *file-scope* (top-level) binding
  or a bare free read is matched against the new project-wide `project_reads`
  aggregate — the read-site mirror of `project_defs`, built over the range-free
  `file_free_reads` firewall — via `Analysis::workspace_read_sites`, recovering
  each read span per file with `read_ranges_in`. Nested locals stay intra-file;
  namespaced (`pkg::name`) names have no in-tree reads.

- [x] **Document highlight** (`textDocument/documentHighlight`). The degenerate
  same-file references query, sharing `local_occurrences`
  (`compute_document_highlights`): the definition as `WRITE`, each read as
  `READ`. Pure (no workspace snapshot), so it runs straight on the read pool.

- [ ] **Go-to-declaration / type-definition / implementation**. Low priority for
  R's dynamic semantics; likely alias to definition or omit.

### Symbols

- [x] **Document symbols** (`textDocument/documentSymbol`). A hierarchical
  `DocumentSymbol` outline of the file's function and variable bindings
  (`compute_document_symbols` / `on_document_symbol`). The name set is the
  `SemanticModel`'s `Local`/`Implicit` bindings at *every* scope (the
  `file_exports` predicate lifted past file scope; params and `for`-vars
  excluded); the CST then supplies the tree and each symbol's full/selection
  spans. A binding's children are the symbols nested in its value side, and
  non-binding nodes (`if`/`for`/`{}`, which introduce no symbol) are
  descended through so every binding surfaces at the right level. Pure and
  single-file (no workspace), so it runs straight on the read pool like
  document highlight. `R6`/`setClass` shapes deferred. Kind is `FUNCTION` vs
  `VARIABLE`; `detail` (signatures) is a follow-up.

- [x] **Workspace symbols** (`workspace/symbol`). Fuzzy name search across all
  project files (`src/lsp/workspace_symbols.rs` `workspace_symbols_via_db`).
  Reuses the cross-file index that references and rename already built:
  `Analysis::workspace_symbols` scans `project_defs` (the salsa-tracked,
  name-keyed `DefIndex` aggregated across workspace members), filters names with
  a dependency-free case-insensitive subsequence matcher, and recovers each
  span per site via `def_range_in` against the file's current text. A db-backed
  read job (like definition/references), so it runs on the read pool against a
  snapshot. Returns modern `WorkspaceSymbol`s with full `Location`s; kind is
  `FUNCTION` vs `VARIABLE`. Scope is file-scope top-level defs only (nested
  locals excluded). Empty in single-file mode (no workspace seeded).
  `container_name`/`detail` and a real fuzzy ranking are follow-ups.

### Rename

- [x] **Rename symbol** (`textDocument/rename` + `textDocument/prepareRename`).
  Intra-file rename of a *local* binding (`src/lsp.rs`
  `compute_prepare_rename`/`compute_rename`): resolve the cursor to a
  `BindingId` (read site via `resolve_local`, or the def site), collect the
  def + all in-scope reads into a `WorkspaceEdit`, validate the new name
  against R's syntactic identifier rules (`is_syntactic_r_name`), and anchor
  the prepare→rename handshake on a `NodePtr` so it survives an edit (see the
  cross-edit references prerequisite above). Cross-file rename (`src/lsp.rs`
  `rename_via_db`): a file-scope binding's reads (and a bare workspace free
  read's def + reads) are gathered off the same reverse index as cross-file
  references (`workspace_read_sites`/`workspace_def_sites`) and returned as
  one multi-URI `WorkspaceEdit`. Remaining work is tracked below: the
  cross-file path is name-keyed rather than scope-aware (see "scope-aware
  cross-file resolution"); still open beyond that are backtick-quoting of
  non-syntactic names and renaming package-qualified names.

- [ ] **Scope-aware cross-file resolution** (rename *and* references). Today
  `references_via_db`/`rename_via_db` resolve through
  `workspace_read_sites`/`workspace_def_sites` →
  `project_defs`/`project_reads`, which are range-free, name-only
  `BTreeMap<name, set<path>>` indices. The visibility model (`ProjectScope`:
  `sees`/`visible`/`used_by_others`) is *never consulted* on this path --- it
  only backs the undefined-symbol/unused lints. So the workspace is treated
  as one flat global namespace when R's top-level scope is really a set of
  disjoint visibility islands (package members; directional `source()`
  edges). Consequence: renaming a top-level `foo` rewrites *every* `foo` in
  the workspace, including an unrelated sibling's --- a false positive. The
  fix is one provenance-aware resolution primitive both handlers consume; in
  R there's no module system, so cross-file binding identity genuinely *is*
  "the name, within a visibility-connected component" --- the current code
  keys on name over the wrong (global) scope. Rename carries two soundness
  duties at once (never rewrite an unrelated binding; never miss a read of
  the renamed one), so when the static model is uncertain it must
  refuse-or-warn, not guess. Stage it:

  - [x] **Phase A --- component partitioning (no ordering).** Landed.
    `ProjectScope` now retains `sees` (the reachability relation) and a
    `package_siblings` map, exposed via `sees`/`seen_by`/`package_siblings`
    accessors (`src/project/scope.rs`), all span-free. `Analysis::cross_file_binding`
    (`src/incremental.rs`) resolves a `(def_file, name)` to its `cohort` (def_file
    + package siblings that also define it --- the flat-namespace aliases; a
    `source()`-connected redefinition is a *shadow*, not an alias, so it stays
    out), `readers` (files that can see def_file, free-read the name, and don't
    shadow it), a `conflict` flag (≥2 defs in the component), and a
    `project_has_dynamic_source` flag. `rename_via_db`/`references_via_db`
    (`src/lsp.rs`) consume it through `cross_file_rename_edits` /
    `cross_file_reference_locations`; a bare free read resolves via
    `Analysis::visible_def_files`. Rename **refuses** (returns `None`) on
    conflict, on any project dynamic source (chosen project-wide for soundness),
    or on a bare read that resolves to ≠1 visible def; references is
    non-destructive so it **over-reports** the cohort instead. Computed
    on-demand off the read snapshot --- no new tracked query, so backdating is
    untouched. This killed the cross-component false positive.

    - [x] *Follow-up: the dynamic-source refusal was project-wide and blunt.*
      Landed: narrowed from a name-blind project flag to a name-keyed,
      reachability-scoped check (`dynamic_source_risk` in `cross_file_binding`,
      `src/incremental.rs`). A dynamic `source()` in file `d` injects a hidden
      `d -> ?` edge; the files it could affect are `d`'s blast radius
      `{d} ∪ seen_by(d)`. The rename refuses only when a *free-reader of the
      renamed name* falls in that radius --- otherwise the dynamic source can
      neither hide a read nor divert one, so it is irrelevant and no longer
      blocks. Reuses Phase A's `seen_by` reachability and the `project_reads`
      reader index off the snapshot; no new infra. Reads-only is sufficient
      (a definer with no in-reach reader changes nothing observable).

  - [x] **Phase B --- load-order resolution.** Landed, both ordering axes.
    *Package collation order*: a workspace package is one flat namespace built
    before any function runs, so multiple sibling defs of a name are aliases of
    one slot --- a sound **rename-all**, not the blanket `conflict` refusal Phase
    A used. `CrossFileBinding` now splits that into `cohort_incomplete`: a
    multi-def cohort refuses only when the package's analyzed member set doesn't
    cover its `R/*.[RrSsQq]` sources (`expected = dir glob ∪ Collate:`, computed
    by `read_collations` and frozen into the interned `Project.collations`, so it
    stays pure and backdates; `parse_dcf` lifted from `rindex::harvest`). Only the
    *set* of collated files is needed --- order never changes which reads resolve
    where. *`source()` position*: a new range-free, order-bearing per-file
    firewall `top_level_events` (`Define`/`SourceEdge`/`Read`, order = `Vec`
    position, span-free so it backdates across body edits like `source_edges`;
    `collect_top_level_events` in `src/project/sequence.rs`) drives
    `ProjectScope::top_level_read_binding`, a cycle-guarded replay resolving what
    a file's *top-level* reads bind to under sequential execution.
    `cross_file_binding` consumes it as `order_ambiguous`: rename refuses when a
    reader's top-level read of the name doesn't bind to the cohort (sits before
    the `source()` that injects the def, binds elsewhere, or is poisoned).
    References over-reports as before. Give-ups: `local=TRUE`
    (`Dependency::Skip`), computed paths (`Unresolved`), non-top-level/
    conditional `source()` (only root children scanned), `sys.source()` (mapped
    to `Dynamic` --- a deliberate tightening from silently ignored), same-name
    across one sourced closure (`OrderUnknown`), and `Collate:`/unanalyzed package
    members (`cohort_incomplete`). Took the on-demand route like Phase A: no new
    *tracked* provenance query; `top_level_read_binding`/`package_complete` are
    `ProjectScope` accessors read off the snapshot, and `visible_def_files` stays
    position-blind (the readers refinement covers both the rename-from-def and
    bare-read paths, since the reading file is always in the readers set).

    - [x] *Follow-up: precise per-reader range filtering (B2.4).* Landed.
      Instead of refusing the whole rename when a reader has a top-level read
      that doesn't bind to the cohort, rename now rewrites the cohort-bound reads
      (post-`source()` and function-body reads) and **skips** the rest. Order-aware
      span recovery is done off the live tree+model at rename time, keeping the
      range-free firewall intact: `collect_top_level_events_spanned`
      (`src/project/sequence.rs`) produces the same sequence with read spans, and
      `collect_top_level_events` is now its span-stripping projection (byte-identical
      output by construction). `ProjectScope::top_level_read_provenance`
      (`src/project/scope.rs`) replays it per occurrence into a new `ReadSite`
      (`Bound(path)`/`Unbound`/`Unknown`), reusing the same `live`/`poisoned`/
      `name_ambiguous` tracking as `top_level_read_binding`.
      `Analysis::reader_rename_ranges` (`src/incremental.rs`) consumes it: a fast
      path (all top-level reads bind to the cohort, or none exist) renames every
      free read; otherwise it drops the `Unbound`/bound-elsewhere reads and
      **refuses** (`None`) only on an undecidable `Unknown` read (two static
      closure definers — the dynamic-source case is still refused project-wide by
      `dynamic_source_risk`). The old aggregate `order_ambiguous` flag is gone.
      `references` still over-reports. *Known limitation (pre-existing, unchanged):*
      reads inside a `source()` call's own arguments aren't in the sequence, so
      they aren't position-classified --- the same gap `top_level_read_binding`
      already had.

    - [x] *Follow-up: body reads bind to the final scope, which may be a shadow,
      not the cohort.* Landed. Function-body reads run at call time against the
      reader's final post-execution scope, so they all share one binding ---
      previously assumed to be the cohort and kept by construction. When a reader
      sources a cohort def **and then** a later same-name def outside the cohort
      (a `source()`-shadow, e.g. `source("a.R"); source("z.R")` both defining
      `foo`), the final scope binds `foo` to `z.R`, so co-renaming the body read
      was wrong. New `ProjectScope::final_scope_binding` (`src/project/scope.rs`)
      runs the same range-free load-order replay as `top_level_read_binding` but
      reports the end-of-file binding as a single `ReadSite`
      (`Bound`/`Unbound`/`Unknown`). `reader_rename_ranges` (`src/incremental.rs`)
      resolves it once per reader: `Bound` to a non-cohort file drops the body
      reads (every free read that isn't a classified top-level read), `Unknown`
      refuses the whole rename (like a top-level `Unknown`), and `Unbound` keeps
      them --- the package-sibling flat-namespace case, where the def *is* the
      cohort and carries no `source()` event. Span-free, so the salsa firewall is
      intact (backdates across body edits). `references` still over-reports
      harmlessly.

  - **Salsa / incrementality (Tenet 2).** Several constraints, all learnable
    from the existing graph layer:

    - *Don't break the firewall.* Phase B reintroduces position, which would
      break the range-free firewall that lets `project_defs`/`project_reads`
      backdate across body edits. Keep it by modeling a per-file *top-level
      sequence* --- an ordered list of `define name`/`source-edge` events that
      carries order but **not** spans --- so a body edit leaves it unchanged and
      it backdates like today's firewalls; collation order is path-derived and
      already stable.

    - *Never depend a tracked query on `project_graph`.* It's `no_eq` (holds
      non-`Eq` `HashMap`s) so it never backdates when it re-runs --- any export
      change anywhere re-runs the whole graph. Project what you need through a
      thin `Eq` firewall, the way `visible_symbols`/`Visibility` already does.
      The provenance map (name → defining file, order-resolved) is a *new* such
      projection, fed by the top-level sequence. (Phase A took the on-demand
      route: `sees`/`package_siblings` are exposed as `ProjectScope` accessors
      and the handlers read the `no_eq` graph off the read snapshot rather than
      memoizing --- fine because rename/references aren't tracked queries. If
      Phase B wants a *tracked* consumer of order-resolved provenance, it must go
      through a thin `Eq` projection instead, the way `visible_symbols` does.)

    - *Stays read-only.* Resolution consumes already-aggregated member firewalls
      + the graph, all readable on a snapshot, so rename/references stay on the
      read pool and need **no** writes --- no change to the single-writer lint
      thread. Precondition: discovery has driven members into the db (it has).

    - *Keep source() traversal in one pure query*, cycle-guarded with a
      `visited` set like `ProjectScope::build` --- not mutually-recursive tracked
      queries, which would pull in salsa's fixpoint machinery for no gain.

- [x] **File rename** (`workspace/willRenameFiles` / `workspace/didRenameFiles`,
  advertised via the `fileOperations` server capability for `**/*.{R,r}`). Done:
  `willRenameFiles` returns a `WorkspaceEdit` rewriting `source("old")` literals
  in dependents (`Analysis::source_rename_edits` → `will_rename_via_db`), found
  via `reverse_source_edges` (normalized on both sides, since its keys are
  un-normalized) and rewritten with a new range-bearing extractor
  (`collect_source_literal_edges`, `src/project/source.rs`) that preserves the
  quote and recomputes the relative spelling (`relative_path`). `didRenameFiles`
  refreshes db membership on the lint thread (`apply_file_renames`) and re-lints.
  Dynamic `source(var)` targets are left untouched. Folder renames are still a
  follow-up.

### Completion & signatures

- [x] **Completion** (`textDocument/completion` + `completionItem/resolve`).
  Scope-aware locals + library exports from the index; `pkg::`/`pkg:::` triggers
  member completion (with a bundled names-only fallback for unharvested
  packages), plus base-R names. `resolve` lazily attaches docs/signature.
  Name-only insertion (no snippet). See `src/lsp/completion.rs`. Follow-ups:
  snippet/paren insertion, `$`/`@` member completion, fuzzy/case-insensitive
  prefix matching, function-vs-variable kind for locals.

- [x] **Signature help** (`textDocument/signatureHelp`). Inside a call, show the
  callee's formals/usage. The index already carries `formals` and the
  `\usage` block (same data hover renders) --- the new work is detecting
  "inside call argument N" from the CST and tracking the active parameter.
  Done in `src/lsp/signature.rs`: resolves the enclosing call's callee via the
  shared hover index path, builds parameters from `formals` (with UTF-16 label
  offsets) or falls back to the `\usage` label, and tracks the active parameter
  by top-level commas with a `name = ` override. Follow-up: clamp the active
  parameter into a `...` formal under R's variadic semantics.

### Diagnostics & misc protocol surface

- [x] **Pull diagnostics** (`textDocument/diagnostic`). The server pushes
  diagnostics from the lint thread; the pull model (LSP 3.17) lets clients
  request on demand and is friendlier to the coalescing/versioning the lint
  thread already does. Document pull is implemented and auto-suppresses push for
  pull-capable clients; cross-file/index changes drive a re-pull via
  `workspace/diagnostic/refresh`. Full `workspace/diagnostic` (reports across
  closed files) is not implemented (`workspace_diagnostics: false`).
  
- [x] **Semantic tokens** (`textDocument/semanticTokens/full`). Scope-aware
  highlighting (distinguish function calls, locals, package-qualified names,
  arguments) from the same `SemanticModel`; degrades gracefully if omitted.
  v1 is *augment-only* (emits just the semantically-resolved identifiers:
  function/variable/parameter/property/namespace, with a `definition`
  modifier) and purely syntactic/scope (pure read-pool job, no salsa db). See
  `src/lsp/semantic_tokens.rs`. Follow-ups: base-R/loaded-package
  `defaultLibrary` modifier, `range`/delta variants, and `USER_OP` operators.

- [x] **Folding ranges** (`textDocument/foldingRange`). Pure CST walk ---
  brace blocks, function/parameter and argument lists, parenthesized and
  subscript expressions, comment runs. No semantic model needed.
- [ ] **Selection ranges** (`textDocument/selectionRange`). Pure CST walk:
  incremental scope expansion from the cursor outward through enclosing nodes.

- [x] **Call hierarchy** (`textDocument/prepareCallHierarchy` + incoming/
  outgoing). Caller/callee graph; rides the same cross-file reference index
  as workspace symbols and references. Done in `src/lsp/call_hierarchy.rs`:
  `prepare` parses the live buffer and resolves the cursor to the top-level
  function it names (intra-file binding else `workspace_def_sites`), filtered to
  function defs; `incoming`/`outgoing` work off the db snapshot, recovering the
  target from the round-tripped item's `uri` + `name` (no `data` payload).
  Incoming walks the visibility component (`cross_file_binding`) for
  callee-position reference sites and groups them by enclosing top-level
  function; outgoing walks the function body's `CALL_EXPR`s, resolving each
  callee intra-file then via `visible_def_files`.
  - **v1 scope:** items are **top-level (file-scope) functions only** — the
    names the cross-file index keys on; a call inside a nested function is
    attributed to its enclosing top-level function. Edges are strict
    *callee-position* uses `F(...)`, never value uses (`lapply(xs, F)`).
  - **Known limitations / follow-ups:** nested/local functions are not items
    (so calls *to* a nested function don't appear as outgoing edges, and a
    nested function never appears as a caller/callee item); call sites at script
    top-level (inside no function) are dropped from incoming; ambiguous
    cross-file callees (a name visibly defined in >1 sibling) resolve to the
    first sorted def; string/backtick callees (`` `+`(…) ``) are skipped.

- [ ] **Inlay hints** (`textDocument/inlayHint`). E.g. argument-name hints at
  call sites (matching positional args to index formals). Speculative. Not
  loved by all users, possibly opt-in or omit altogether.

### Cross-cutting prerequisite

- [x] **Workspace-wide symbol/reference index.** Done --- all three pieces
  landed, keyed on the interned `Project`: (1) an explicit, salsa-tracked
  workspace file-set, the singleton `Workspace` input at `Durability::MEDIUM`
  with a conditional setter, from which the interned `Project` is *derived* by
  the `workspace_project` query (`src/incremental.rs`, `src/project/graph.rs`)
  --- the CLI and LSP both go through it, and the LSP seeds it from
  `initialize` `workspaceFolders`/`rootUri` plus a lazy per-file backstop
  (`src/lsp.rs`, `seed_workspace_for`); (2) the reverse `source_edges` map
  `reverse_source_edges` (`Eq`, backdates; keeps `local=TRUE` and out-of-set
  targets, unlike the forward scope builder); (3) the name → def-site
  aggregate --- range-free `file_def_sites`/`DefKind` firewall +
  project-wide `project_defs`, with spans recovered per-request via
  `Analysis::def_range_in` from the fresh `semantic_model`. Backdating proofs
  in `tests/salsa_incremental.rs`. The cross-file *consumers* (workspace
  symbols, references, rename, file rename, call hierarchy) now have no index
  work left --- they sit on these queries.

  - [x] Follow-up (model (b)): `workspace_project` is now **pure** — the
    per-root NAMESPACE texts, expected-source sets, and package-root
    markers live in a new `PackageGraph` salsa input (`src/incremental.rs`),
    populated in the write-phase by `IncrementalDatabase::refresh_package_graph`
    (the sole disk reader, via `project::discover_packages`) and refreshed in
    lockstep with `set_workspace_members`. A keystroke re-run does only
    in-memory work, and the public `refresh_package_graph` gives a future
    `didChangeWatchedFiles` watcher a direct invalidation entry point. Purity
    proof in `tests/salsa_incremental.rs`
    (`workspace_project_is_pure_namespace_not_reread_on_keystroke`). Still
    pairs with the `vfs`/`SourceRoot` follow-up under *Thin `FileId`*.

- [x] Downloadable CRAN sidecar — names-only client (escalation of the bundled
      lists above). A dynamic, disk-cached, version-keyed `RemoteExports` tier
      (`src/rindex/remote.rs`) sits between the harvested index and the bundled
      lists in `resolve_origin`, carried in the salsa `LibraryIndex`'s `remote`
      field at HIGH durability (`src/incremental.rs`). The LSP lint thread fetches
      per-package export lists on demand over a CDN (`Sidecar` + `ureq`, gzip via
      `flate2`), opt-in via the `ARITY_REMOTE_URL` environment variable (a
      per-user/per-machine consent decision, deliberately *not* in the shared
      `arity.toml`; default off so arity stays offline). Lifts the whole-file
      `undefined-symbol` suppression for uninstalled, unbundled packages and
      feeds `pkg::`/bare completion.
      Remaining escalations:
  - [ ] Server pipeline + hosting (separate repo): install all of CRAN via PPM
        binaries, dump per-package names keyed by current version + a
        `pkg → version` manifest, publish gzipped to a CDN (Pages/Releases),
        refresh weekly and additively. arity ships only the client + default URL.
  - [ ] Full-metadata tier (formals + Rd docs) so hover/signature help work for
        uninstalled packages — a richer payload reusing the same fetch path.
  - [ ] Bulk/CI prefetch path (download-once snapshot, no per-file network).
  - [ ] Pin-aware versions: resolve the project's actual version from
        renv.lock/DESCRIPTION (needs CRAN Archive coverage); the URL/disk schema
        is already version-keyed for this.
  - [ ] Feed DESCRIPTION `Imports`/`Depends` and `import(pkg)` into the referenced
        and resolved sets so the `resolution_incomplete` poison
        (`src/project/scope.rs`) clears once the sidecar can enumerate exports.

- [x] Data-masking / tidy-eval suppression (landed). A bare name in a
  data-masking verb's arguments (`mutate(b = a + 1)`) resolves to a data-frame
  *column*, not a binding or export, so flagging it is a false positive. The
  builder (`src/semantic/builder.rs`) tracks a `mask_depth`: a call whose callee
  is in `is_data_masking_callee` (`src/semantic/symbols.rs` — base `with`/
  `within`/`subset`/`transform`, the dplyr verbs, tidyr/tidyselect, ggplot2
  `aes`) walks its callee unmasked (so a typo'd verb is still flagged) but its
  argument list with `mask_depth` bumped; reads recorded there carry
  `IdentRef::data_masked`, which both `undefined-symbol` paths skip. The read is
  still recorded so an enclosing binding used only inside a masked expression
  isn't mis-flagged unused. Match is name-only and over-masks conservatively
  (the whole arg subtree, nested calls included) — over-matching only ever
  suppresses, the safe direction for a false-positive-only rule.
  - [ ] Follow-ups: data.table's `dt[i, j, by]` masking is `[`-shaped, not a
    call, so it's unhandled. Masking is not package-gated (a user's own
    non-NSE `filter`/`transform` under-flags its args); gate on the verb's
    package actually being attached if that proves too coarse. Mask carries
    into inline `function(...)` bodies inside a masked arg (lexically those
    aren't masked) — deliberately conservative for now.

- [x] Meta-package attachment (Option A — static table, landed). A meta-package
  like `tidyverse` attaches a fixed set of core packages (dplyr, ggplot2,
  tibble, …) via its `.onAttach` hook; those names are *not* in the
  meta-package's own export list, so `library(tidyverse); tibble(...)` used to
  false-positive on `undefined-symbol`. `meta_package_members`
  (`src/semantic/symbols.rs`) maps a meta-package → its attached core set;
  `resolve_origin` (`src/rindex/provider.rs`) expands each loaded meta-package
  with its members before masking, and both conservative gates
  (`external_resolution` in `src/project/graph.rs`, `run_standalone` in
  `undefined_symbol.rs`) require every member be indexed too. Members resolve
  against the bundled/remote/harvested tiers as usual (all nine tidyverse core
  packages are already bundled). The set is `.onAttach`-driven, *not* `Depends`,
  so it genuinely needs the curated table.
  - [ ] Follow-up (Option B — harvest-time attach capture). The static table is
    correct but hand-maintained and offline-only. When a package is actually
    installed/harvested, detect what it attaches (diff `search()` across a clean
    `library()` call) and record `attaches: Vec<SmolStr>` in `PackageIndex`
    (`src/rindex/schema.rs`, bump `SCHEMA_VERSION`); `resolve_origin` would
    prefer the harvested attach set over the static table for installed
    meta-packages, leaving the table as the offline fallback. Generalizes beyond
    tidyverse (tidymodels, fastverse, …) without growing the curated list.

- [ ] Follow-up: prune packages that vanish from CRAN out of the bundled set.
  The refresh is now **additive** --- `scripts/rank_cran_downloads.sh` unions
  each run's top-N (30-day window) into `scripts/cran_top_packages.txt` and
  never drops by ranking, and `scripts/dump_cran_symbols.R` preserves a
  member's last-known exports when it can't be installed this run. So an
  archived/removed package lingers with stale exports forever: there is no
  "couldn't produce exports for N consecutive runs --> drop" counter yet. The
  preserve path is the hook to build it on. Benign (extra coverage, never a
  wrong answer, since bundled is the lowest-precision tier), so deferred until
  dead packages actually accumulate.

- [x] Thin `FileId` + file-source map (retire the `<mem>` hack). `SourceFile`
  now carries an opaque `FileId` and an *optional* path
  (`src/incremental.rs`): in-memory files have `None` (no more synthetic
  `<mem>/{uuid}.R`), and a small normalized-path index (`FileSourceMap`)
  dedups equivalent path spellings to one input, so cwd/path-form no longer
  leaks into salsa keys. `file_path` is now `Option<&Path>`; `source_edges`
  reads the optional path as before. The `uuid` dependency is gone. Scoping
  is unchanged --- multi-root layouts (package + scripts) are governed by
  `package_root`/`ProjectScope`, not the file key. See
  `ARCHITECTURE_AUDIT.md` §3.3.

  - [ ] Follow-up: full `vfs`/`SourceRoot` model ---
    opaque-`FileId`-at-the-URI boundary in `src/lsp.rs` and
    `SourceRoot`-scoped durability --- when multi-root workspaces
    actually need it. Lower leverage for a single-crate tool (the wart
    is already gone).

## Misc

- [ ] `arity-ignore-unused` meta-diagnostic: emit a finding for suppression
  comments that didn't actually suppress anything (rule ID is reserved but
  the rule is not yet wired in).

- [ ] **Harvest lazy-data symbols.** The index now covers R's default packages
  (so hover/signatures work for base-R functions), but `harvest_package`
  only reads `NAMESPACE`/object exports --- it skips a package's lazy-data
  (`.getNamespaceInfo(ns, "lazydata")`). So `datasets` harvests 0 symbols and
  hovering a dataset (e.g. `iris`) resolves the package but finds no entry.
  The static name lists already include lazydata; the harvest does not.
