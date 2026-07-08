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
  non-salsa per-file previous-parse cache (a pure perf hint—a successful
  reparse is byte-identical to a full parse, so it never changes query
  output). Correctness is pinned by an oracle property test
  (`tests/incremental_reparse.rs`: `reparse == parse(new)` in tree *and*
  diagnostics across the corpus) plus a salsa-level test
  (`body_edit_uses_incremental_reparse_and_stays_correct`). On a \~100 KB
  file reparse is \~200× faster than a full parse (`benches/parse.rs`).
  Serves Tenet 2. No `SyntaxNodePtr`/`AstPtr` added (no feature needs a
  stable cross-edit reference yet). See `ARCHITECTURE_AUDIT.md` §3.4.

  - [x] Follow-up: top-level-statement reparse (non-braced). Added
    `reparse_toplevel` (`src/parser/reparse.rs`), tried after token/block:
    it reparses a single top-level statement (a direct child of `ROOT`) in
    isolation, pinning the boundary with a *consume-all* guard (rejects the
    statement shrinking) and a *forward-merge* guard against the next sibling
    (rejects it growing, e.g. a trailing operator). The backward direction is
    inherently safe (R continues only via a *trailing* operator). Covered by
    the oracle sweep (extended `SOURCES` with flat multi-statement fixtures)
    plus a strategy/boundary test and a salsa-level test
    (`toplevel_edit_uses_incremental_reparse_and_stays_correct`); benched as
    `reparse_toplevel`.
  - [ ] Follow-up: use the LSP's precise edit ranges instead of the
    prefix/suffix text diff. The LSP declares `TextDocumentSyncKind::FULL`, so
    `parsed_document` recovers the edit via a whole-text `diff_edit`; threading
    the client's exact change ranges (switching to INCREMENTAL sync) would keep
    disjoint edits from coalescing into one wide span.

## AST wrappers

The typed AST layer (`src/ast/`) is a read-only, rust-analyzer-style navigation
view over the CST (see the Architecture note in `AGENTS.md`). The foundation and
the linter-matcher fold have landed; the remaining consumer migrations are pure
cleanup and must keep behavior byte-identical (a passing suite is the proof).

- [x] **Foundation.** `AstToken` trait + token wrappers (`Ident`/`StringLit`/
  `IntLit`/…), `RConstant`, the missing node wrappers
  (`RepeatExpr`/`SubsetExpr`/`Subset2Expr`), and accessors on the previously thin
  wrappers (`BinaryExpr` lhs/op/rhs/parts, `UnaryExpr`, `ParenExpr`, `BlockExpr`,
  `Arg` name/value). Shared kind predicates in `ast::kinds`.
- [x] **`Expr` union + `HasArgList`.** `Expr` casts from a `SyntaxElement` and
  carries token-atom variants (R's leaves are tokens); `HasArgList` unifies
  `CallExpr`/`SubsetExpr`/`Subset2Expr` argument access.
- [x] **Fold `matchers.rs`** onto the layer (public free-fn surface preserved).
- [ ] **Migrate linter rules** to call `Arg`/`BinaryExpr::parts`/`Ident`/
  `StringLit`/`Expr` directly, then drop the element-level matcher shims once no
  rule imports them. `tests/lint.rs` snapshots must stay byte-identical.
- [ ] **Migrate the semantic builder** (`src/semantic/builder.rs`): replace
  `walk_node`'s hand-rolled dispatch and the duplicated `Node/Token(IDENT)` arms
  with `match Expr::cast(el)` + `Ident`; `handle_for`/param scans -> `ForExpr`/
  `FunctionExpr` accessors; quote-stripping -> `StringLit::unquote`.
- [ ] **Migrate LSP** (`hover`/`signature`/`semantic_tokens`/`folding`): drop the
  redundant `kind()==X` pre-checks before `X::cast()`; use `HasArgList`/`Arg`/
  `Ident`.

## Formatter

- [ ] Tribbles

- [ ] `line-ending` config (landed for whole-document `format_node`) is **not**
  applied by `format_range` (`src/formatter/core.rs`), which always emits `\n`.
  Range/on-type formatting in a CRLF document via the LSP can therefore splice LF
  into a CRLF buffer (mixed endings). Thread the source/ending into `format_range`
  like `format_node` does. Low urgency: the CLI `format` path (whole-document) is
  correct; this only affects LSP range edits in CRLF files.

- [x] `exclude`/`extend-exclude` (landed for the CLI `format`/`lint` walk via
  `ExcludeFilter` in `src/file_discovery.rs`) is now consulted by LSP workspace
  seeding (`src/lsp/lint_thread.rs` `seed_workspace`), salsa sibling discovery
  (`src/linter/check.rs` `seed_workspace_for`/`check_document_in_project`), and
  `arity index` package discovery (`src/rindex/discover.rs`). All resolve config
  through the shared `Config::exclude_filter`; the single-document seed paths use
  `check::resolve_exclude_at`. Excluded files no longer get indexed/linted
  in-editor, while `check::scope_members` re-adds generated package sources
  (`cpp11.R`, `RcppExports.R`, …) so cross-file resolution stays complete.

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
    a block is no longer line-flat—the parser builds **logical content**
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
      kinds—`ROXYGEN_CODE` (`` `…` ``), `ROXYGEN_RD_MACRO`
      (`\code{…}`/`\link[pkg]{…}`), `ROXYGEN_MD_LINK` (`[t](u)`/`[func()]`)—via
      conservative, line-scoped, byte-exact recognizers in
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
      header (`@tag [arg]`, single-spaced—the "normalize tag-internal spacing"
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
    - [x] (3b) hang *list* block Rd macros under a prose-bearing tag. A
      `\describe`/`\itemize`/`\enumerate` block that directly continues an open
      `TagUnit` (e.g. `\describe{}` after `@format <prose>`) inherits the tag's
      two-space hanging indent, so the block no longer sits flush while the tag's
      prose hangs (the inconsistency behind the `@format` describe-block report).
      Idempotent by construction: the shift is *anchored to the opener* (opener at
      `marker + 1 + hang`, inner lines keep their offset relative to it), so a
      reparse re-derives the same result. Gated to **list** macros only
      (`is_list_block_macro`), whose inter-item/leading whitespace is insignificant
      in the rendered Rd; verbatim-content macros (`\preformatted`/`\verb`) stay
      flush because re-indenting them would inject *literal* spaces and change the
      output. Fixtures `roxygen_rd_macro_hangs_under_tag` (list hangs) and
      `roxygen_rd_verbatim_not_hung` (verbatim untouched); baseline re-blessed.
    - [x] **Systematically bucket roxygen tags (and block Rd macros) for layout.**
      Layout is now chosen by a single classification (`enum TagClass` +
      `classify(name)` in `src/formatter/roxygen.rs`), **never** by the input's
      written form: `@details x` and `@details`⏎`x` render identically in roxygen2,
      so they must format identically (Tenet 1). The formatter canonicalizes,
      gathering a section's body from *both* places the parser can put it (inline in
      the `ROXYGEN_TAG` node for form-1, a sibling `ROXYGEN_PARAGRAPH` for form-2)
      and re-emitting per class. The seven classes (from roxygen2's own tag-parser
      model): **NameBearingProse** (`@param`/`@slot`/`@field`, `tag_name_description`)
      hang under the inline `@tag name` header; **SectioningProse** (`tag_markdown`:
      `@description`/`@details`/`@return`/`@value`/`@format`/`@note`/`@references`/
      `@source`/`@seealso`/`@author`/`@title`) go inline when the single-paragraph
      body fits, else form-2 (bare `#' @tag`, body flush); **Code**
      (`@examples`/`@usage`/`@eval*`) verbatim; **AtomicValue** (`tag_value`, e.g.
      `@family single table verbs`) one line, interior spaces preserved, overflow
      tolerated; **TokenList** (`tag_words`/namespace) joined onto one line;
      **Toggle** (`@export`/`@noRd`/`@md`) bare; **Section** title inline, body
      form-2. Reclassifying a tag is a one-line `match` edit; unknown tags default to
      SectioningProse (the sole guessed assignment). This resolves the open question:
      **section-body tags do not hang** — a wrapped body drops to form-2 (matching
      the corpus and `style/documentation.qmd:40-70`); only the name-bearing label
      tags hang. Consequently the 3b list-macro `+2` hang was a divergence and is
      **removed** (`is_list_block_macro` deleted): a `\describe`/`\itemize` under a
      section sits flush beneath the bare `#' @tag` (the block content-forces form-2),
      matching what people write. Fixtures: convergence proofs
      `roxygen_section_return_form{1,2}` (both → `#' @return A value.`),
      `roxygen_name_bearing_pulled_up`, `roxygen_section_{inline_if_fits,
      multiparagraph,null,block_flush}`, `roxygen_token_list_join`,
      `roxygen_atomic_value_overflow`; the `@format`/`@details` + list corpus cases
      now flush (baseline re-blessed, 9 cases). R oracle stays green (cross-boundary
      prose movement is Rd-meaning-preserving).
    - [x] (4) run arity's own formatter on embedded R in
      `@examples`/`@examplesIf` (`ExampleBody` in `src/formatter/roxygen.rs`).
      The body lines are collected, stripped of their markers (reusing
      `content_text`), formatted as one R source unit via `format_with_style`,
      and re-prefixed. The body line-width budget is reduced by the marker prefix
      and indentation so the `#'`-prefixed output respects the line width
      (Tenet 1). **Conservative fallback**: a body that does not parse cleanly as
      R falls back to the current marker-normalized passthrough, byte-for-byte—this
      covers Rd-macro wrappers (`\dontrun{}`/`\donttest{}`/`\dontshow{}`,
      not valid R: `\` lexes as the lambda token, so a following identifier is a
      parse error) and any other unparseable example. Idempotent by construction
      (extraction inverts prefixing; the embedded formatter is itself idempotent
      at a fixed width). Fixtures `roxygen_examples_{format,multiline,idempotent,
      dontrun_passthrough}` and `roxygen_examplesif_format`. No air-compat
      allowlist entry: air leaves roxygen untouched, so it preserves arity's
      formatted output and the fixed-point gauge sees no divergence.
    - [x] **Pipe-bearing prose reflows; only table *delimiter rows* are
      structured** (fixes #49, the tidyverse/tidyr idempotence regression).
      `is_structured`'s `contains('|')` table heuristic predated the parser's
      GFM table model and hit any prose with an R pipe (`|>`), `||`, or
      `x | y` — but only on the physical-line path: the parser folds
      pipe-bearing continuations into a same-line-value `ROXYGEN_TAG`, where
      `TagUnit` reflowed them freely, so the two written forms of one `@param`
      disagreed and pass 1's output reflowed differently on pass 2. The
      heuristic is now the parser's own `is_table_delim_row` (Tenet 3): a
      matched table arrives as a `ROXYGEN_MD_TABLE` block macro (`@md` mode)
      and stays verbatim, a lone unmatched delimiter row stays a structured
      boundary, and everything else containing a pipe is prose and reflows.
      Fixture `roxygen_pipe_prose_reflow`.
      - *Known edge (backlog):* in `@md` mode, re-wrapping a paragraph that
        sits directly above an *unmatched* delimiter row can land the
        paragraph's last line on a matching cell count, so a table forms on
        reparse (a render change; the output is still idempotent since the
        new table is already marker-normalized). Contrived — needs a stray
        delimiter row plus an exact wrap landing — and the folded-tag path had
        the same exposure before this change.

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

## Linter

Closest precedent: **jarl** (`etiennebacher/jarl`, Rust + rowan + air-parser,
55+ rules, suppression directives, LSP, autofix). The foundation pass borrows
shape (diagnostic + `Violation` trait, `PackageOrigin` enum, `# arity-ignore`
suppression directive style, annotate-snippets rendering) but stays its own
project—arity is a unified formatter + linter + LSP binary on arity's own
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
evaluation or type inference is **out of scope**—arity stays static. Pure
layout (quotes, leading zero, spacing) is the **formatter's** job (Tenet 1), not
the linter's.

**Category directories.** Keep `correctness/` and `suspicious/`; add
`readability/`, `performance/`, `meta/` (suppression-directive rules), and
`pkg/dplyr/` + `pkg/testthat/`. No `style/` dir—pure layout is the
formatter's. Public rule IDs stay flat kebab-case (category is a directory
concern, as `all_rule_ids()` already is).

#### Phase 0—Infrastructure (unblocks everything)

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
      confirms a bare call invokes base R—simple-name callee that is a base
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
- [x] **Severity is engine-stamped, not per-finding** (landed). Rules build
      findings with a placeholder severity (`Default::default()`, like `path`);
      `run_rules` stamps each with its rule's `default_severity()`. This makes
      the override live (it was previously dead—rules hardcoded the literal, so
      overriding `default_severity()` did nothing) and is the seam for a future
      per-rule severity config override (`config.unwrap_or(default_severity())`).
- [x] **Rule-set-derived dispatch state precomputed once** (landed).
      `ResolvedRules` now carries the node-dispatch table (`by_kind`), the
      `any_node_rules` flag, and the severity map, all built in `resolve` instead
      of rebuilt per file in `run_rules`. The LSP lint worker caches the
      `Arc<ResolvedRules>` per config, so resolution (registry instantiation +
      table build) leaves the per-keystroke path; `resolve` also instantiates the
      registry once rather than twice.
- [ ] *Speculative micro-opt (deferred):* `resolves_to_base` does a linear
      `model.idents().iter().any(...)` scan for the callee's shadow check. It runs
      only after a rule fully shape-matches (`any(is.na(x))`, unreachable
      `return`/`stop`), so the call count is tiny and it is not currently hot—not
      worth an offset->ident index yet. If it ever becomes hot, resolve via the
      covering element at the callee offset instead of scanning.

#### Phase 1—High-signal, purely syntactic, safe fixes (`syn`)

Match a call/operator shape with deterministic fixes. Match bare names for now;
harden against shadowing in Phase 4.

- [ ] `browser` (suspicious, safe-delete)—leftover debug call.
- [x] `equals-na` `x == NA` -> `is.na(x)` (correctness, safe; landed—`==`
      form only). Still open: `equals-nan` -> `is.nan` (safe); `equals-null`
      (correctness, none/unsafe—`== NULL` rewrite is less mechanical).
- [ ] `empty-assignment` (correctness, none).
- [x] `duplicated-arguments` `f(a = 1, a = 2)` (correctness, none; landed)—mirrors
      `duplicate-formal`. Warning (not error): `c(a = 1, a = 2)` is legal.
- [x] `redundant-equals` `x == TRUE` -> `x`, `x == FALSE` -> `!x` (suspicious,
      safe; landed—`!`-rewrite withheld for non-atom operands via `is_atom`).
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
      (correctness, safe; landed). Flags only operators in conditional context—the
      walk descends from the condition through parens, `!`, and
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
- [ ] `implicit-assignment` (suspicious, none)—scope to avoid overlap with
      existing `assignment-in-condition`.

#### Phase 2—Call-rewrite idioms, namespace-confirmed (`ns`)

- [ ] **§I2 regex/string-literal helper** first: read a `STRING` token's
      unquoted contents; classify regex metachars/single anchor (`^`/`$`).
      Blocks `string-boundary`, `fixed-regex`.
- [x] `any-is-na` `any(is.na(x))` -> `anyNA(x)` (performance, safe; landed)—flagship.
      First rule in the new `performance/` category. Fires only on the
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
- [x] `crossprod` `t(x) %*% y` -> `crossprod(x, y)`, `x %*% t(y)` ->
      `tcrossprod(x, y)` (performance, `ns`, safe; landed). Fires on a `%*%`
      `BINARY_EXPR` where one operand is a single-positional-arg `t()` call
      (left preferred; both-`t()` yields `crossprod(x, t(y))`), namespace-confirms
      `t` resolves to base R. Collapses to the single-arg form when both operands
      are the same bare symbol. The replacement is a call (a primary), so no
      precedence guard is needed and the fix is `Safe`; withheld when a comment
      outside the preserved operands would be dropped.
- [ ] `lengths` `sapply(x, length)` -> `lengths(x)` (performance, safe).
- [ ] `nzchar` `nchar(x) > 0` -> `nzchar(x)` (performance, safe).
- [ ] `seq`/`seq2` `1:length(x)` -> `seq_along`, `1:n` -> `seq_len`
      (performance, safe)—off-by-one safety, high value.
- [ ] `is-numeric` (correctness, safe); `class-equals` `class(x) == ...` ->
      `inherits` (performance, unsafe—`class()` is a vector).
- [x] `string-boundary` `grepl("^a", x)` -> `startsWith`, `grepl("a$", x)` ->
      `endsWith` (readability, `ns`); `fixed-regex` add `fixed = TRUE`
      (performance, `ns`, safe). Both landed. `string-boundary` fires only on the
      clean two-positional-arg shape with a one-end-anchored plain-literal pattern
      and namespace-confirms `grepl`; its fix is **unsafe** (not safe as first
      sketched)—`startsWith`/`endsWith` diverge from `grepl` on `NA` (`NA` vs
      `FALSE`) and non-character input (error vs coercion). `fixed-regex` fires on
      the base regex functions (`grepl`/`grep`/`sub`/`gsub`/`regexpr`/`gregexpr`/
      `regexec`) when the first positional arg is a metacharacter-free string
      literal and no `fixed`/`ignore.case`/`perl` flag is set; the fix is a pure
      insertion of `, fixed = TRUE` (lossless, safe). Both withhold/skip per the
      autofix-correctness discipline (`string-boundary` withholds on a dropped
      comment; `fixed-regex` needs none—it drops nothing).
- [ ] `sort` `sort(x)[1]` -> `min`, etc. (performance, unsafe).
- [ ] `internal-function` `pkg:::fn` via
      `BinaryExpr::namespace_access().internal` (correctness, none)—cheap.

#### Phase 3—SemanticModel rules + config plumbing

- [ ] **§I4 per-rule config**: add a `[lint.rules.<id>]` TOML table + typed
      per-rule struct in `src/config.rs`, threaded into rules via a
      `config`/`&RuleConfig` field on `RuleContext`. **Blocks**
      `undesirable-function`, `download-file`.
- [x] `unreachable-code` after `return()`/`stop()` (correctness, ns,
      unsafe-delete; landed)—flags statements following an unconditional
      base-R `return()`/`stop()` that is a direct block statement (`return`
      gated on an enclosing function); namespace-confirmed, fix withheld when it
      would drop a comment. Both-branches-return (needs CFG) is out of scope.
- [ ] `if-always-true` literal `if (TRUE/FALSE)` only—no const-folding
      (correctness, unsafe).
- [ ] `unused-function` (suspicious, sem, none)—reuse
      `unused_local_bindings`; **default-off** (exported pkg funcs look unused).
- [ ] `duplicated-function-definition` (suspicious, sem, none).
- [ ] `for-loop-index`/`for-loop-dup-index` (suspicious, sem, none).
- [ ] `unnecessary-nesting` collapsible nested `if`/single-stmt block
      (readability, sem, unsafe).
- [ ] `coalesce` `if (is.null(x)) y else x` (performance, sem, unsafe)—may
      want §I5 multi-edit fix.
- [ ] `undesirable-function` (suspicious, ns + config, none)—needs §I4;
      **default-off**. `download-file` (correctness, ns, none)—low priority.

#### Phase 4—Meta (suppression) rules + hardening

- [ ] **§I6 suppression refactor**: have `SuppressionMap` expose the parsed
      directive list (rule, range, has-reason, raw) and surface it on
      `RuleContext` (`suppressions`). `outdated-suppression` also needs the
      driver (`check.rs`/`run_rules`) to record which suppressions actually
      matched a diagnostic—a post-pass, not a per-rule concern.
- [ ] `misnamed-suppression` (vs `ALL_RULE_IDS`, safe), `blanket-suppression`
      (none), `unexplained-suppression` (none, **default-off**),
      `outdated-suppression` (safe-delete). These subsume the reserved
      `arity-ignore-unused` follow-up below.
- [ ] **Hardening sub-pass**: upgrade Phase 1/2 fixes from bare-name to
      `resolves_to_base`-confirmed + shadow-checked, graduating the call-rewrite
      rules Unsafe -> Safe and suppressing FPs where `any`/`is.na` etc. are
      user-redefined. (`true-false-symbol` already shipped shadow-checked.)

#### Phase 5—Package-aware rules

Gated on the package being attached (`model.loaded_packages()`).

- [ ] `pkg/testthat/` as one cohesive PR (shared `expect_*` matcher):
      `expect-true-false`, `expect-length`, `expect-named`, `expect-null`,
      `expect-type`, `expect-s3-class`, `expect-match`/`expect-no-match` (all ns,
      safe). High value for test-heavy repos.
- [ ] `pkg/dplyr/`: `dplyr-filter-out` `filter(!(x %in% y))` (ns, safe). Defer
      `dplyr-group-by-ungroup`—needs **§I8 pipe-chain abstraction**
      (`%>%`/`|>` stage walk) that doesn't exist yet.

#### Documentation rules (roxygen2), `documentation/`

Lint the roxygen2 blocks the parser now models. All `syn`, no fixes (adding a
tag/title means inventing prose; deleting one drops prose the author wrote).
Shared helpers live in `src/linter/rules/roxygen.rs`: `documented_function`
(strictly conservative next-sibling association—`setMethod`/R6/`"_PACKAGE"`
yield `None` and the function-shape checks skip), the `KNOWN_TAGS` registry,
`inherits_docs`/`wants_rd_topic` gates, `param_doc`, and the token-concat
`extract_examples` + offset map (robust to `@md` fragmentation). Kept honest by
the **lint differential oracle** `task roxygen-lint-oracle`
(`tests/roxygen_lint_oracle.rs` + driver op `lint-warnings`): compares against
roxygen2's own signals per comparable event class, allowlist-ratcheted
(`tests/oracle/roxygen-lint-allowlist.txt`); arity-stricter findings are
excluded from the diff by construction. `KNOWN_TAGS` validated against
roxygen2 7.3.3.

- [x] `roxygen-unknown-tag`—tag roxygen2 doesn't understand (mirrors "is not
      a known tag").
- [x] `roxygen-title`—documented function with no title/intro (mirrors
      "Skipping; no name and/or title"); also fires on `@export` with no docs
      at all (roxygen2 silent; `R CMD check` flags the undocumented export).
- [x] `roxygen-return`—`@export` without `@return`/`@returns` (arity-extra:
      roxygen2 never warns; CRAN requires `\value`). Skips `@noRd` and
      inherited/merged topics.
- [x] `roxygen-param`—missing/nonexistent/duplicate `@param` (arity-extra)
      plus name-and-description two-part check (mirrors "requires two parts").
      Coverage skipped under `@inheritParams`/`@rdname`/…; duplicates always
      checked.
- [x] `roxygen-examples`—`@examples` body or `@examplesIf` condition that
      does not reparse as R (condition mirrors roxygen2's "condition failed to
      parse"; body is arity-extra). Rd wrappers (`\dontrun{}` etc.) neutralized
      name-only so offsets survive.
- [ ] Follow-ups (deferred): run the full rule set over extracted example code
      (needs package-context symbol handling to avoid FPs); unsafe-delete fixes
      for duplicate/nonexistent `@param`; a missing-description variant of
      `roxygen-title` (roxygen2 auto-copies the title into `\description`, so
      it never warns—decide against CRAN's stance first); mine the oracle's
      "uncovered signals" table (mismatched braces/quotes, markdown-link
      plain-text restriction) for new rules.
- [ ] Parser notes surfaced by this work: (a) roxygen2 never
      markdown-processes `tag_code` bodies, but arity tokenizes markdown inside
      `@examples` under `@md` (harmless for the lint—extraction is
      token-concat—but a CST-fidelity gap); (b) a stray closing delimiter at
      top level (`f(1))`) is recovered losslessly *without* a parse
      diagnostic, though R itself errors—`roxygen-examples` and plain-file
      linting both inherit the leniency.

#### Out of scope (recorded so they aren't silently dropped)

- **Formatter's domain (Tenet 1):** `quotes`, `numeric_leading_zero`, spacing,
  indentation, semicolons, trailing whitespace—excluded from the linter.
- **Needs R evaluation/type inference (arity is static):** `all_equal`,
  `length_levels`, `length_test`, `matrix_apply`, `list2df`, `which_grepl`,
  `grepv`, `sample_int`, `system_file`, `sprintf` arg-checking, full `coalesce`,
  const-folded `if_always_true`. Implement only an exact-AST-shape subset where
  one exists, else defer.
- **Too noisy without opt-in:** `undesirable-function`, `unused-function`,
  `unexplained-suppression`—all **default-off**.

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
(`src/rindex/`) already supply most of the analysis these features need—the
work is mostly wiring resolution results to LSP responses, not new analysis.
Roughly ordered by leverage-to-effort:

### Prerequisites & blockers

There are **no hard architectural blockers**—the parser and salsa model are
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
  same path—no new threading. (Caveat: any feature that *caches* a location
  must validate against the current version, as `hover_via_db` already does
  with its `file_text != text` check, because def `TextRange`s shift on edit.)
- **Project-level aggregation already exists in principle:** an interned
  `Project` key + `project_graph`/`visible_symbols` tracked queries aggregate
  `file_exports`/`file_free_reads`/`source_edges` across members
  (`src/project/graph.rs`).

Two genuine gaps gated the **cross-file** half of the list (both soft—new
infra that builds *with* the grain, not architectural fights). The first has
landed; the second is still open but only matters for cross-edit-stable handles:

- [x] **Reverse `source_edges` index + an explicit workspace file-set.** Done:
  `reverse_source_edges(db, project)` (`src/project/graph.rs`) is the
  who-sources-me map (`Eq`, backdates), and the file-set is now the explicit
  salsa `Workspace` input (`src/incremental.rs`) from which the interned
  `Project` is derived by `workspace_project`—no per-request disk walk. See
  *Cross-cutting prerequisite* below for the full landed shape.

- [x] **Stable cross-edit node references.** Done, landed with its first
  consumer (intra-file rename). Three pieces: (1) rowan's typed
  same-revision handles `AstPtr`/`SyntaxNodePtr` re-exported from
  `src/ast.rs`; (2) arity's canonical `NodePtr` (`src/syntax/ptr.rs`)—a
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
  (`compute_definition`/`definition_via_db`). Cross-file resolves a bare
  top-level name against the workspace `project_defs` index—its first
  consumer—via the new `Analysis::workspace_def_sites`, recovering each
  span per file with `def_range_in`. Package-export/namespaced targets have
  no in-tree location, so they return nothing and lean on hover (as planned).

- [x] **Go-to-references/find-all-references** (`textDocument/references`). The
  inverse of go-to-definition, in the same two phases. Intra-file: the cursor
  resolves to a local binding (shared `resolve_local_target`) and every
  `idents()` read of it is reported via the shared `local_occurrences`
  (`compute_references`/`references_via_db`), honoring
  `context.includeDeclaration`. Cross-file: a *file-scope* (top-level) binding
  or a bare free read is matched against the new project-wide `project_reads`
  aggregate—the read-site mirror of `project_defs`, built over the range-free
  `file_free_reads` firewall—via `Analysis::workspace_read_sites`, recovering
  each read span per file with `read_ranges_in`. Nested locals stay intra-file;
  namespaced (`pkg::name`) names have no in-tree reads.

- [x] **Document highlight** (`textDocument/documentHighlight`). The degenerate
  same-file references query, sharing `local_occurrences`
  (`compute_document_highlights`): the definition as `WRITE`, each read as
  `READ`. Pure (no workspace snapshot), so it runs straight on the read pool.

- [ ] **Go-to-declaration/type-definition/implementation**. Low priority for
  R's dynamic semantics; likely alias to definition or omit.

### Symbols

- [x] **Document symbols** (`textDocument/documentSymbol`). A hierarchical
  `DocumentSymbol` outline of the file's function and variable bindings
  (`compute_document_symbols`/`on_document_symbol`). The name set is the
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

- [ ] **RStudio-style code sections** (outline + folding). R tooling (RStudio,
  and the R languageserver's `section.R`) treats a trailing run of 4+
  `-`/`#`/`=`/`+`/`*` markers on a comment line (`# Foo ----`, `#### Bar ####`)
  as a named section header, with the leading `#`s giving nesting depth, and
  surfaces the resulting tree in **both** `documentSymbol` (a file outline) and
  `foldingRange` (fold a section down to its next same-or-higher-level sibling).
  arity surfaces neither: document symbols are binding-only
  (`compute_document_symbols`) and folding is CST-structural (brace blocks,
  comment runs). Both would consume one section scanner over comment trivia—
  purely lexical (no semantic model), so it drops onto the read pool like the
  existing symbol/folding walks. Convention, not language; gate behind a setting
  if it proves noisy. (Gap surfaced by the 2026-07-02 languageserver survey.)

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
  `sees`/`visible`/`used_by_others`) is *never consulted* on this path—it
  only backs the undefined-symbol/unused lints. So the workspace is treated
  as one flat global namespace when R's top-level scope is really a set of
  disjoint visibility islands (package members; directional `source()`
  edges). Consequence: renaming a top-level `foo` rewrites *every* `foo` in
  the workspace, including an unrelated sibling's—a false positive. The
  fix is one provenance-aware resolution primitive both handlers consume; in
  R there's no module system, so cross-file binding identity genuinely *is*
  "the name, within a visibility-connected component"—the current code
  keys on name over the wrong (global) scope. Rename carries two soundness
  duties at once (never rewrite an unrelated binding; never miss a read of
  the renamed one), so when the static model is uncertain it must
  refuse-or-warn, not guess. Stage it:

  - [x] **Phase A—component partitioning (no ordering).** Landed.
    `ProjectScope` now retains `sees` (the reachability relation) and a
    `package_siblings` map, exposed via `sees`/`seen_by`/`package_siblings`
    accessors (`src/project/scope.rs`), all span-free. `Analysis::cross_file_binding`
    (`src/incremental.rs`) resolves a `(def_file, name)` to its `cohort` (def_file
    + package siblings that also define it—the flat-namespace aliases; a
    `source()`-connected redefinition is a *shadow*, not an alias, so it stays
    out), `readers` (files that can see def_file, free-read the name, and don't
    shadow it), a `conflict` flag (≥2 defs in the component), and a
    `project_has_dynamic_source` flag. `rename_via_db`/`references_via_db`
    (`src/lsp.rs`) consume it through `cross_file_rename_edits` and
    `cross_file_reference_locations`; a bare free read resolves via
    `Analysis::visible_def_files`. Rename **refuses** (returns `None`) on
    conflict, on any project dynamic source (chosen project-wide for soundness),
    or on a bare read that resolves to ≠1 visible def; references is
    non-destructive so it **over-reports** the cohort instead. Computed
    on-demand off the read snapshot—no new tracked query, so backdating is
    untouched. This killed the cross-component false positive.

    - [x] *Follow-up: the dynamic-source refusal was project-wide and blunt.*
      Landed: narrowed from a name-blind project flag to a name-keyed,
      reachability-scoped check (`dynamic_source_risk` in `cross_file_binding`,
      `src/incremental.rs`). A dynamic `source()` in file `d` injects a hidden
      `d -> ?` edge; the files it could affect are `d`'s blast radius
      `{d} ∪ seen_by(d)`. The rename refuses only when a *free-reader of the
      renamed name* falls in that radius—otherwise the dynamic source can
      neither hide a read nor divert one, so it is irrelevant and no longer
      blocks. Reuses Phase A's `seen_by` reachability and the `project_reads`
      reader index off the snapshot; no new infra. Reads-only is sufficient
      (a definer with no in-reach reader changes nothing observable).

  - [x] **Phase B—load-order resolution.** Landed, both ordering axes.
    *Package collation order*: a workspace package is one flat namespace built
    before any function runs, so multiple sibling defs of a name are aliases of
    one slot—a sound **rename-all**, not the blanket `conflict` refusal Phase
    A used. `CrossFileBinding` now splits that into `cohort_incomplete`: a
    multi-def cohort refuses only when the package's analyzed member set doesn't
    cover its `R/*.[RrSsQq]` sources (`expected = dir glob ∪ Collate:`, computed
    by `read_collations` and frozen into the interned `Project.collations`, so it
    stays pure and backdates; `parse_dcf` lifted from `rindex::harvest`). Only the
    *set* of collated files is needed—order never changes which reads resolve
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
    to `Dynamic`—a deliberate tightening from silently ignored), same-name
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
      closure definers—the dynamic-source case is still refused project-wide by
      `dynamic_source_risk`). The old aggregate `order_ambiguous` flag is gone.
      `references` still over-reports. *Known limitation (pre-existing, unchanged):*
      reads inside a `source()` call's own arguments aren't in the sequence, so
      they aren't position-classified—the same gap `top_level_read_binding`
      already had.

    - [x] *Follow-up: body reads bind to the final scope, which may be a shadow,
      not the cohort.* Landed. Function-body reads run at call time against the
      reader's final post-execution scope, so they all share one binding—previously
      assumed to be the cohort and kept by construction. When a reader
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
      them—the package-sibling flat-namespace case, where the def *is* the
      cohort and carries no `source()` event. Span-free, so the salsa firewall is
      intact (backdates across body edits). `references` still over-reports
      harmlessly.

  - **Salsa/incrementality (Tenet 2).** Several constraints, all learnable
    from the existing graph layer:

    - *Don't break the firewall.* Phase B reintroduces position, which would
      break the range-free firewall that lets `project_defs`/`project_reads`
      backdate across body edits. Keep it by modeling a per-file *top-level
      sequence*—an ordered list of `define name`/`source-edge` events that
      carries order but **not** spans—so a body edit leaves it unchanged and
      it backdates like today's firewalls; collation order is path-derived and
      already stable.

    - *Never depend a tracked query on `project_graph`.* It's `no_eq` (holds
      non-`Eq` `HashMap`s) so it never backdates when it re-runs—any export
      change anywhere re-runs the whole graph. Project what you need through a
      thin `Eq` firewall, the way `visible_symbols`/`Visibility` already does.
      The provenance map (name → defining file, order-resolved) is a *new* such
      projection, fed by the top-level sequence. (Phase A took the on-demand
      route: `sees`/`package_siblings` are exposed as `ProjectScope` accessors
      and the handlers read the `no_eq` graph off the read snapshot rather than
      memoizing—fine because rename/references aren't tracked queries. If
      Phase B wants a *tracked* consumer of order-resolved provenance, it must go
      through a thin `Eq` projection instead, the way `visible_symbols` does.)

    - *Stays read-only.* Resolution consumes already-aggregated member firewalls
      + the graph, all readable on a snapshot, so rename/references stay on the
      read pool and need **no** writes—no change to the single-writer lint
      thread. Precondition: discovery has driven members into the db (it has).

    - *Keep source() traversal in one pure query*, cycle-guarded with a
      `visited` set like `ProjectScope::build`—not mutually-recursive tracked
      queries, which would pull in salsa's fixpoint machinery for no gain.

- [x] **File rename** (`workspace/willRenameFiles`/`workspace/didRenameFiles`,
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
  `\usage` block (same data hover renders)—the new work is detecting
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

- [x] **Folding ranges** (`textDocument/foldingRange`). Pure CST walk—brace
  blocks, function/parameter and argument lists, parenthesized and
  subscript expressions, comment runs. No semantic model needed.
- [x] **Selection ranges** (`textDocument/selectionRange`). Pure CST walk:
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
  - **v1 scope:** items are **top-level (file-scope) functions only**—the
    names the cross-file index keys on; a call inside a nested function is
    attributed to its enclosing top-level function. Edges are strict
    *callee-position* uses `F(...)`, never value uses (`lapply(xs, F)`).
  - **Known limitations/follow-ups:** nested/local functions are not items
    (so calls *to* a nested function don't appear as outgoing edges, and a
    nested function never appears as a caller/callee item); call sites at script
    top-level (inside no function) are dropped from incoming; ambiguous
    cross-file callees (a name visibly defined in >1 sibling) resolve to the
    first sorted def; string/backtick callees (`` `+`(…) ``) are skipped.

- [ ] **On-type formatting** (`textDocument/onTypeFormatting`). The R
  languageserver advertises it with first-trigger `\n` and more-triggers `)`,
  `]`, `}`—reformat the current statement as the user closes a bracket or presses
  enter. arity advertises full + range formatting but **not**
  `documentOnTypeFormattingProvider` (`src/lsp/server.rs`). Small wiring over the
  existing `format_range` path, but **gated on the CRLF bug already logged under
  Formatter** (line-ending config isn't threaded into `format_range`, so a range
  edit in a CRLF buffer splices LF); fix that first, then advertise. (2026-07-02
  languageserver survey.)

- [x] **Document links** (`textDocument/documentLink`). Emits a `DocumentLink`
  for every string literal that resolves to an existing regular file: a new pure
  extractor `collect_link_literals` (`src/project/source.rs`, raw-string aware)
  walks *all* string tokens, and `compute_document_links`
  (`src/lsp/document_links.rs`) resolves + `stat`s each on the read pool. Gated by
  a `link_file_size_limit` editor setting. Deviations from the languageserver
  survey: resolution is relative to the file's own directory (arity's `source()`
  convention), not the workspace root, and targets are resolved eagerly (no
  `documentLink/resolve` round-trip). (2026-07-02 languageserver survey; done
  2026-07-08.)

- [x] **Document color** (`textDocument/documentColor` + `colorPresentation`). The
  languageserver turns any single-line string literal matching `#RRGGBB[AA]` or a
  name in `grDevices::colors()` into a color swatch, and offers hex presentations
  for the editor's picker (`color.R`). A pure read-pool CST walk: a new generalized
  `collect_string_literals` (`src/project/source.rs`) feeds `compute_document_colors`
  (`src/lsp/document_color.rs`), which recognizes anchored `#RRGGBB`/`#RRGGBBAA` hex
  (6/8 digits only, rejecting base-R-invalid 3/4-digit short forms) and named colors
  via a generated static table (`src/lsp/color_names.rs`, regenerated by
  `scripts/gen_colors.R`). `colorPresentation` returns a single hex presentation
  whose edit rewrites the literal in place, preserving its quote. Deviation from the
  survey: named-color lookup is **case-insensitive** (mirroring base R's `col2rgb`,
  which resolves `"Red"`), not a strict lowercase `colors()`-membership test.
  (2026-07-02 languageserver survey; done 2026-07-08.)

- [x] **Type hierarchy** (`textDocument/prepareTypeHierarchy` + supertypes/
  subtypes). Shipped for **S4/R6/RefClass**: a static OOP class model
  (`src/project/classes.rs`) projects each file's class definitions and their
  inheritance edges (`setClass(contains=)`, `R6Class(inherit=)`,
  `setRefClass(contains=)`), aggregated into a range-free, backdating salsa index
  (`project_classes`/`ClassIndex` in `src/project/graph.rs`), consumed by
  `src/lsp/type_hierarchy.rs` (mirrors call hierarchy). The capability is injected
  into the initialize JSON—`lsp-types` 0.97 has no typed field. Follow-ups:
  - **S3** is out of scope (no formal class definition; inheritance is implicit
    via `class(x) <- c(...)` / `structure(class=)`, not statically reliable).
  - R6 `inherit = Symbol` is resolved by the identifier text, assuming the
    generator binding name equals the class string (the standard convention).
  - The `ClassIndex` now unblocks the deferred `R6`/`setClass` **document-symbol**
    shapes—wire `file_class_defs` into `src/lsp/symbols.rs`.

- [ ] **Minor capability-conformance gaps vs. the R languageserver** (2026-07-02
  survey). (a) arity's completion trigger set is `:` only; the languageserver
  also triggers on `.` (ubiquitous in R names)—fold into the existing completion
  trigger follow-ups. (b) arity advertises `workspace_folders: None` and seeds the
  workspace once from `initialize`; the languageserver advertises
  `workspaceFolders.changeNotifications`, so arity does not react to
  `workspace/didChangeWorkspaceFolders` (folders added or removed mid-session).
  (c) `textDocumentSync` is FULL-only with no `willSave`/`save` registration
  (benign). Note: the languageserver's `codeLens`, `executeCommand`,
  `linkedEditingRange`, `moniker`, and type/implementation-definition providers are
  **commented out in its own `capabilities.R`**, so they are *not* arity gaps.

- [ ] **Inlay hints** (`textDocument/inlayHint`). E.g. argument-name hints at
  call sites (matching positional args to index formals). Speculative. Not
  loved by all users, possibly opt-in or omit altogether.

### Cross-cutting prerequisite

- [x] **Workspace-wide symbol/reference index.** Done—all three pieces
  landed, keyed on the interned `Project`: (1) an explicit, salsa-tracked
  workspace file-set, the singleton `Workspace` input at `Durability::MEDIUM`
  with a conditional setter, from which the interned `Project` is *derived* by
  the `workspace_project` query (`src/incremental.rs`, `src/project/graph.rs`)—the
  CLI and LSP both go through it, and the LSP seeds it from
  `initialize` `workspaceFolders`/`rootUri` plus a lazy per-file backstop
  (`src/lsp.rs`, `seed_workspace_for`); (2) the reverse `source_edges` map
  `reverse_source_edges` (`Eq`, backdates; keeps `local=TRUE` and out-of-set
  targets, unlike the forward scope builder); (3) the name → def-site
  aggregate—range-free `file_def_sites`/`DefKind` firewall +
  project-wide `project_defs`, with spans recovered per-request via
  `Analysis::def_range_in` from the fresh `semantic_model`. Backdating proofs
  in `tests/salsa_incremental.rs`. The cross-file *consumers* (workspace
  symbols, references, rename, file rename, call hierarchy) now have no index
  work left—they sit on these queries.

  - [x] Follow-up (model (b)): `workspace_project` is now **pure**—the
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

- [x] Downloadable CRAN sidecar—names-only client (escalation of the bundled
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
        uninstalled packages—a richer payload reusing the same fetch path.
  - [ ] Bulk/CI prefetch path (download-once snapshot, no per-file network).
  - [ ] Pin-aware versions: resolve the project's actual version from
        renv.lock/DESCRIPTION (needs CRAN Archive coverage); the URL/disk schema
        is already version-keyed for this.
  - [ ] Feed DESCRIPTION `Imports`/`Depends` and `import(pkg)` into the referenced
        and resolved sets so the `resolution_incomplete` poison
        (`src/project/scope.rs`) clears once the sidecar can enumerate exports.

- [x] Data-masking/tidy-eval suppression (landed). A bare name in a
  data-masking verb's arguments (`mutate(b = a + 1)`) resolves to a data-frame
  *column*, not a binding or export, so flagging it is a false positive. The
  builder (`src/semantic/builder.rs`) tracks a `mask_depth`: a call whose callee
  is in `is_data_masking_callee` (`src/semantic/symbols.rs`—base `with`/
  `within`/`subset`/`transform`, the dplyr verbs, tidyr/tidyselect, ggplot2
  `aes`) walks its callee unmasked (so a typo'd verb is still flagged) but its
  argument list with `mask_depth` bumped; reads recorded there carry
  `IdentRef::data_masked`, which both `undefined-symbol` paths skip. The read is
  still recorded so an enclosing binding used only inside a masked expression
  isn't mis-flagged unused. Match is name-only and over-masks conservatively
  (the whole arg subtree, nested calls included)—over-matching only ever
  suppresses, the safe direction for a false-positive-only rule.
  - [ ] Follow-ups: data.table's `dt[i, j, by]` masking is `[`-shaped, not a
    call, so it's unhandled. Masking is not package-gated (a user's own
    non-NSE `filter`/`transform` under-flags its args); gate on the verb's
    package actually being attached if that proves too coarse. Mask carries
    into inline `function(...)` bodies inside a masked arg (lexically those
    aren't masked)—deliberately conservative for now.

- [x] Meta-package attachment (Option A—static table, landed). A meta-package
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
  - [ ] Follow-up (Option B—harvest-time attach capture). The static table is
    correct but hand-maintained and offline-only. When a package is actually
    installed/harvested, detect what it attaches (diff `search()` across a clean
    `library()` call) and record `attaches: Vec<SmolStr>` in `PackageIndex`
    (`src/rindex/schema.rs`, bump `SCHEMA_VERSION`); `resolve_origin` would
    prefer the harvested attach set over the static table for installed
    meta-packages, leaving the table as the offline fallback. Generalizes beyond
    tidyverse (tidymodels, fastverse, …) without growing the curated list.

- [ ] Follow-up: prune packages that vanish from CRAN out of the bundled set.
  The refresh is now **additive**—`scripts/rank_cran_downloads.sh` unions
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
  is unchanged—multi-root layouts (package + scripts) are governed by
  `package_root`/`ProjectScope`, not the file key. See
  `ARCHITECTURE_AUDIT.md` §3.3.

  - [ ] Follow-up: full `vfs`/`SourceRoot` model—opaque-`FileId`-at-the-URI
    boundary in `src/lsp.rs` and
    `SourceRoot`-scoped durability—when multi-root workspaces
    actually need it. Lower leverage for a single-crate tool (the wart
    is already gone).

## Misc

- [ ] `arity-ignore-unused` meta-diagnostic: emit a finding for suppression
  comments that didn't actually suppress anything (rule ID is reserved but
  the rule is not yet wired in).

- [ ] **Harvest lazy-data symbols.** The index now covers R's default packages
  (so hover/signatures work for base-R functions), but `harvest_package`
  only reads `NAMESPACE`/object exports—it skips a package's lazy-data
  (`.getNamespaceInfo(ns, "lazydata")`). So `datasets` harvests 0 symbols and
  hovering a dataset (e.g. `iris`) resolves the package but finds no entry.
  The static name lists already include lazydata; the harvest does not.
