# TODOs

## Parser

- [ ] Comment-aware suppression placement edge cases: directives inside
  `R_CALL_ARGUMENTS` between `( ... , <directive> , next_arg )` need special
  handling so they attach to `next_arg` instead of the argument list. (Jarl
  solved this by overriding biome's `place_comment`; arity's
  next-non-trivia-sibling walk already handles most cases.)
- [ ] Give the test-only Rd projector (`src/roxygen/project_rd/section.rs`
  `block_md`) a package-wide markdown default, so oracle cases from
  markdown-first packages become representable without a per-block `@md`. The
  rest of that wiring landed: `ParseOptions.roxygen_markdown_default`
  (issue #94), static discovery from `DESCRIPTION`'s `Roxygen` field and
  `man/roxygen/meta.R` (`src/project/description.rs`; note roxygen2 7.3.3 has
  no `Config/roxygen2/markdown` field, contrary to the issue), and the format
  CLI and cache, linter, salsa layer (`SourceFile.roxygen_markdown` input), and
  LSP all resolve it. Known static limit: a `meta.R` that *computes* its list
  is unresolvable and defers to the `DESCRIPTION` field.

## AST wrappers

- [ ] *Optional polish:* migrate the remaining individual lint rules to call the
  wrappers directly where it reads better than the `matchers` free-fns
  (`comparison-negation` already uses `UnaryExpr`). Low priority — the fold
  already put the rules on the typed layer; this is cosmetic and per-rule.

## Formatter

- [ ] Tribbles

## Linter

- [ ] *Speculative micro-opt (deferred):* `resolves_to_base` does a linear
      `model.idents().iter().any(...)` scan for the callee's shadow check. It runs
      only after a rule fully shape-matches (`any(is.na(x))`, unreachable
      `return`/`stop`), so the call count is tiny and it is not currently hot—not
      worth an offset->ident index yet. If it ever becomes hot, resolve via the
      covering element at the callee offset instead of scanning.
- [ ] **Hardening sub-pass**: upgrade Phase 1/2 fixes from bare-name to
      `resolves_to_base`-confirmed + shadow-checked, graduating the call-rewrite
      rules Unsafe -> Safe and suppressing FPs where `any`/`is.na` etc. are
      user-redefined. (`true-false-symbol` already shipped shadow-checked.)

### Phase 5—Package-aware rules

Gated on the package being attached (`model.loaded_packages()`).

- [ ] `pkg/testthat/` as one cohesive PR (shared `expect_*` matcher):
      `expect-true-false`, `expect-length`, `expect-named`, `expect-null`,
      `expect-type`, `expect-s3-class`, `expect-match`/`expect-no-match` (all ns,
      safe). High value for test-heavy repos.
- [ ] `pkg/dplyr/`: `dplyr-filter-out` `filter(!(x %in% y))` (ns, safe). Defer
      `dplyr-group-by-ungroup`—needs **§I8 pipe-chain abstraction**
      (`%>%`/`|>` stage walk) that doesn't exist yet.

### Documentation rules (roxygen2), `documentation/`

- [x] `roxygen2-compat` (documentation; syn; no fix) and `r-compat`
      (correctness; syn; safe lambda fix only)—version-aware rules keyed on
      the `[compat]` floors (explicit `arity.toml` keys, else derived from
      `DESCRIPTION`; silent with neither). `roxygen2-compat` flags roxygen2
      8.0.0-only syntax under an older target (`@prop`/`@R6method`,
      `` `Rd expr` `` spans, `@inheritParams` filters, backtick-quoted spaced
      names) and multiline single-line tags at an 8.0.0 target; `r-compat`
      flags raw strings (4.0), `|>`/`\(x)` (4.1), and the `_` placeholder
      (4.2) below their floors.
- [ ] Follow-ups (deferred): run the full rule set over extracted example code
      (needs package-context symbol handling to avoid FPs); unsafe-delete fixes
      for duplicate/nonexistent `@param`; a missing-description variant of
      `roxygen-title` (roxygen2 auto-copies the title into `\description`, so
      it never warns—decide against CRAN's stance first); mine the oracle's
      "uncovered signals" table (mismatched braces/quotes, markdown-link
      plain-text restriction) for new rules.

## Static analysis

- [ ] Only if a concrete rule (dead-store, redundant reassignment) justifies it:
      a flow-sensitive fixpoint over the Phase B CFG, lattice over bindings. This
      is the first analysis that is real work to keep incremental—revisit after
      B ships and a rule demands it.

## Language Server

### Navigation

- [ ] **Go-to-declaration/type-definition/implementation**. Low priority for
  R's dynamic semantics; likely alias to definition or omit.

### Symbols

- [ ] `detail` (signatures) and `container_name` (enclosing binding) for each
  symbol.

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

### Completion & signatures

- Completion (`textDocument/completion` + `completionItem/resolve`).
  - [ ] Snippet/paren insertion
  - [ ] Fuzzy/case-insensitive prefix matching
  - [ ] Function-vs-variable kind for locals

### Diagnostics & misc protocol surface

- [ ] Workspace diagnostics (`workspace/diagnostic`)
  
- Semantic tokens (`textDocument/semanticTokens/full`)
  - [ ] base-R/loaded-package `defaultLibrary` modifier
  - [ ] `range`/delta variants
  - [ ] `USER_OP` operators

- [x] **Call hierarchy** (`textDocument/prepareCallHierarchy` + incoming/
  outgoing). Caller/callee graph; rides the same cross-file reference index
  as workspace symbols and references. Done in `src/lsp/call_hierarchy.rs`:
  `prepare` parses the live buffer and resolves the cursor to the function it
  names (intra-file binding else `workspace_def_sites`), filtered to function
  defs; `incoming`/`outgoing` work off the db snapshot, recovering the target
  from the round-tripped item's `uri` + `data` name chain. Incoming walks the
  visibility component (`cross_file_binding`) for callee-position reference sites
  and groups them by enclosing function; outgoing walks the `FUNCTION_EXPR`'s
  `CALL_EXPR`s, resolving each callee through the scope tree then via
  `visible_def_files`.
  - **Scope:** items are **named function definitions at any scope**—file-scope
    functions (the names the cross-file index keys on) and nested/local ones—plus
    the synthetic per-file script scope that owns top-level calls. An
    item's identity is its enclosing-function name chain, round-tripped in
    `CallHierarchyItem::data`; a range would go stale, since `prepare` reads the
    live buffer while incoming/outgoing read the db snapshot the lint thread only
    catches up to asynchronously. Edges are strict *callee-position* uses
    `F(...)`, never value uses (`lapply(xs, F)`).
  - [x] **Nested/local functions are items.** A call is attributed to the
    innermost enclosing *named* function (anonymous bodies fall through to their
    nearest named ancestor); outgoing reports only an item's own calls, so a
    nested function's calls are its own edges; and callees resolve through the
    scope tree, so a nested `helper` no longer misresolves to a sibling file's
    top-level `helper`. Nested names are file-private, so their incoming edges
    are intra-file by construction.
  - [x] **Script top-level call sites are items.** A call inside no function is
    attributed to the file's synthetic **script-scope** item (`script_item`):
    `SymbolKind::FILE`, named after the file, identified by `ItemData::script`
    rather than a name chain (nothing names a top level). It is never a callee, so
    `incoming` on it is empty, while `outgoing` lists the file's top-level calls.
    Attribution stays the one `enclosing_function` predicate—`None` now means the
    script scope instead of "drop"—so the two directions still cannot disagree.
  - [x] **Ambiguous cross-file callees report every candidate.** A free read that
    more than one visible sibling defines yields one outgoing edge per definition,
    not the first sorted one. Which one R reaches is a runtime fact
    (`visible_def_files` treats >1 as unresolved for the same reason), and
    `prepare` already returns one item per candidate, so this makes the two ends
    agree. A locally bound callee still resolves to exactly one target.
  - [ ] String/backtick callees (`` `+`(…) ``, `"foo"()`) are skipped. **Not a
    call-hierarchy fix**: the semantic model records a backticked read's name
    *with* its backticks (so `` `foo`() `` never resolves to binding `foo`), and
    records no ident at all for a `STRING` callee. Both ends of call hierarchy read
    the model's binding and read sets, so normalizing in this layer alone would put
    `incoming` and `outgoing` out of step. The fix belongs in `semantic/builder.rs`
    (unquote `IDENT` names, treat a `STRING` callee as a read), and its blast
    radius is the hazard: `binding.name` is what rename writes back, so unquoting
    without re-quoting on the write side would emit invalid R. Do it there, with
    rename and references covered first.

- [ ] **On-type formatting** (`textDocument/onTypeFormatting`). The R
  languageserver advertises it with first-trigger `\n` and more-triggers `)`,
  `]`, `}`—reformat the current statement as the user closes a bracket or presses
  enter. arity advertises full + range formatting but **not**
  `documentOnTypeFormattingProvider` (`src/lsp/server.rs`). Small wiring over the
  existing `format_range` path, but **gated on the CRLF bug already logged under
  Formatter** (line-ending config isn't threaded into `format_range`, so a range
  edit in a CRLF buffer splices LF); fix that first, then advertise. (2026-07-02
  languageserver survey.)

- [ ] **Inlay hints** (`textDocument/inlayHint`). E.g. argument-name hints at
  call sites (matching positional args to index formals). Speculative. Not
  loved by all users, possibly opt-in or omit altogether.

### Audit vs Ark (2026-07-30)

Gaps surfaced by auditing Posit's **Ark** (`posit-dev/ark`)—the R kernel that
also embeds the language server behind Positron—against arity. **Net finding:
arity is broadly ahead of Ark on standard editor LSP surface.** Ark advertises
*no* semantic tokens, call hierarchy, type hierarchy, document color, document
links, document highlight, or even document/range formatting (it delegates
formatting to `air`). Ark's edge is that it is *kernel-embedded*: its extra
surface is console-integration custom requests tied to a live R session. So the
genuine deltas are narrow and **mostly reinforce items already logged above**;
only the first item below is new actionable content. (Non-gap: Ark advertises
`implementationProvider` but its handler is a `// TODO` stub returning `Ok(None)`
in `main_loop.rs`, so go-to-implementation is *not* something Ark actually
ships—the existing low-priority note under "Navigation" stands, unelevated.)

- [ ] **Positron console-integration custom requests** (new, speculative).
  Ark serves `positron/textDocument/statementRange` (given a cursor, return the
  range of the complete top-level statement to send to the REPL;
  `statement_range.rs`) and `inputBoundaries` (split pasted console input into
  executable units; `input_boundaries.rs`). Both are **CST-only** computations
  arity could produce from its existing statement/selection-range machinery
  (`src/lsp/selection_range.rs` + the CST), and are useful to any editor with a
  "send code to console/terminal" runner—not just Positron. But they are a
  Positron-**proprietary** protocol extension and arity isn't console-embedded,
  so gate on a client that would actually consume them. Ark's sibling custom
  requests (`helpTopic`, virtual documents) are genuinely out of scope—they
  need a live R session.

### Cross-cutting prerequisite

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
  - [x] Feed DESCRIPTION `Imports`/`Depends` and `import(pkg)` into the referenced
        and resolved sets so the `resolution_incomplete` poison
        (`src/project/scope.rs`) clears once the sidecar can enumerate exports.
        Landed with DESCRIPTION stage 2 below, as one change.

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

  - [x] data.table's `[`-shaped masking (landed). `handle_subset` masks a
    `SUBSET_EXPR`'s argument list on either of two prongs: a marker unique to
    `[.data.table` (a `by`/`keyby`/`.SDcols`/… named argument, a `:=` anywhere
    inside, or a pronoun like `.N`/`.SD`—`is_data_table_arg_name` and
    `is_data_table_pronoun` in `src/semantic/symbols.rs`), or a base known to
    hold a table. The latter is what catches the marker-free filter idiom
    `dt[x > 3]`, which is shaped exactly like vector indexing: `ctx.data_tables`
    records names assigned from `is_data_table_constructor` calls, from
    `setDT(df)`, and from any data.table-shaped subscript, so identity
    propagates through `en <- data.table(...)[, x := y][]`. `[[` is excluded,
    and a `:=` inside the mask now records a *column* read instead of a
    binding—`dt[, newcol := 1]` binds nothing in the frame. Direct calls to
    `` `[.data.table` `` join the masking-callee table for the same reason.
    Over-matching only ever suppresses, the safe direction here.

  - [x] Gate the name-only masking match (landed, but on *shadowing*, not on
    package attachment). `apply_shadow_gate` (`src/semantic/builder.rs`) runs
    after `resolve_reads` and clears `data_masked` on reads whose masking verb
    resolves to a local binding: a file defining its own non-NSE `filter` is
    calling *that* function, which evaluates its arguments. Gating on
    `library(<pkg>)` instead was rejected—package code using `@importFrom`
    never calls `library()`, so that gate would stop suppressing exactly where
    suppression is needed. Only bare data-masking verbs are gateable; quoting
    callees, formulas, opaque `%op%` operands, model-frame arguments, and
    data.table subscripts are pinned, as is a read nested in a second verb.
    Reusing `resolve_reads`'s frame ordering means a top-level call *above* the
    definition stays masked, matching what R does at runtime.

  - [x] Mask carrying into inline `function(...)` bodies is *correct*, not
    conservative—the follow-up's premise was wrong. A closure written inside a
    masked argument is created in the mask environment, so the mask is its
    lexical parent and a bare column name in its body resolves. Verified
    against R: `with(d, sapply(col, function(v) v + other[1]))` finds `other`
    in `d`, and the same holds for rlang's data mask. Locked by
    `mask_carries_into_inline_function_body`.

  - [ ] Remaining: masking a subscript still needs a marker or a known base, so
    `dt[x > 3]` on a table arriving as a function parameter is unmasked. A
    lightweight per-binding data.table type would close that.

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

  - [x] Follow-up (Option B—harvest-time attach capture, landed). Harvest
    records `attaches: Vec<SmolStr>` in `PackageIndex` (schema v2), captured
    two ways: a default pure-Rust heuristic (`detect_attaches` in
    `src/rindex/harvest.rs` fetches well-known attach-set variables—the
    tidyverse/tidymodels `core` convention—from the namespace lazy-load DB,
    gated on `.onAttach` existing and validated all-or-nothing against
    installed packages), and an opt-in `search()`-diff probe
    (`src/rindex/attach_probe.rs`, `arity index --attach-probe` or
    `ARITY_ATTACH_PROBE`—it executes attach hooks, so consent is per-user/per
    -run like `ARITY_REMOTE_URL`, and it runs as a sequential post-harvest
    phase so the parallel harvest stays subprocess-free). `attach_members`
    (`src/rindex/provider.rs`) prefers a non-empty harvested set; the static
    table remains the fallback (uninstalled metas, names-only remote/bundled
    tiers, failed capture). Both undefined-symbol gates and the LSP's
    `packages_to_build` expand through the shared lookup.

    Remaining follow-ups:

    - [ ] Transitive attaches: a meta-package attaching another meta-package
      expands one level only (matches the old static behavior; no known case).
    - [ ] Attach sets do not flow through the remote sidecar or bundled tiers
      (names-only formats); a sidecar v2 could carry them.
    - [ ] Grow `ATTACH_SET_VARS` beyond `core` as evidence of other
      conventions appears; `Depends`-driven attachment could also be captured
      statically from `DESCRIPTION` without any probe.

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
  `package_root`/`ProjectScope`, not the file key.

  - [ ] Follow-up: full `vfs`/`SourceRoot` model—opaque-`FileId`-at-the-URI
    boundary in `src/lsp.rs` and
    `SourceRoot`-scoped durability—when multi-root workspaces
    actually need it. Lower leverage for a single-crate tool (the wart
    is already gone).

### Performance

- [x] **Maintain the line index across edits instead of rebuilding it.** Done.
  An open document is now an `Arc<TextBuffer>` (`src/text/buffer.rs`) holding
  the text next to a `LineIndex` that `apply_edit` *splices* per edit, shared
  with the lint thread and every read job. `LineIndex` keys wide chars by
  absolute offset in a flat `Vec` (the old per-line `HashMap` renumbered on
  every line insert, so it could not splice) and scans line starts with
  `memchr`.

  Measured at 1 MB, criterion (`task bench-line-index`): building the index
  went 1.36 ms -> 356 us; a keystroke's index cost 354 us -> 13 us, and a
  10-change `didChange` batch 3.34 ms -> 54 us. A second run on a loaded
  machine gave 524 us -> 25 us and 5.99 ms -> 61 us; the ratios (>20x and
  >60x) are what hold, not the absolute numbers.

  Against the ~160-180 us incremental reparse it precedes, the index was
  **68%** of a keystroke and is now well under a fifth. Do not quote a tighter
  figure than that from `pipeline/`: `patch_then_reparse` minus `reparse_only`
  is a ~15 us difference between two ~170 us measurements, which is noise. The
  `keystroke/patch` number is the one to trust, since it times the patch
  directly.

  The representation switch alone also made conversions 35-46% faster and the
  CJK-heavy build 3.4x faster.

  The remaining `LineIndex::new` calls are on re-parse fallbacks
  (`compute_hover`, `compute_rename`, `compute_format_range_edits`,
  `roxygen_code_action`), where a parse dwarfs the index. Follow-ups worth
  doing separately:

  - `src/linter/rules/suspicious/duplicated_function_definition.rs` builds an
    index over `ctx.root.text().to_string()` — a full text materialization per
    rule run, unrelated to the LSP path.
  - Salsa's `SourceFile.text` is still a `String`, so the lint thread's write
    phase makes one owned copy per keystroke. Making it an `Arc<str>` would
    remove the last copy.

## DESCRIPTION and package metadata

`DESCRIPTION` used to be scraped for four facts and was otherwise invisible to
arity. The end state is the `Cargo.toml`/rust-analyzer analogue: declared
dependencies drive name resolution, the file itself carries diagnostics,
completion and hover, and it formats. Staged so each step is useful alone.

- [x] **1. A principled DCF parser** (`crates/arity-parser/src/dcf/`). A second
      `rowan::Language` alongside the R grammar: lossless
      (`reconstruct(text) == text`), spanned, record-aware, with diagnostics on
      the usual side channel (malformed line, orphan continuation, empty field
      name). Typed wrappers (`dcf::ast`) are the only surface consumers touch,
      so nothing outside the module names the second `SyntaxKind`. Replaced
      `parse_dcf` and all five of its consumers with no behavior change. Lives
      in the published parser crate so a dprint plugin can reach it at stage 5.

- [x] **2. DESCRIPTION as an analysis input.** Done. `dcf::deps` parses
      dependency entries (name plus version constraints, spanned), and
      `DescriptionFacts` (`src/project/description.rs`) derives every fact in
      one parse: `package_name`, `description_compat`, the `Roxygen` field, and
      `expected_r_sources`'s `Collate` half are all projections of it, and
      `r_depends_floor` became a lookup over the entries rather than a bespoke
      string splitter. `harvest` is deliberately untouched—it reads *installed*
      packages in a library directory, a different problem with no database and
      no watcher.

      DESCRIPTION is a salsa input holding **text** (`DescriptionFile`), with
      `description_facts` the `Eq` projection over it. That split is the whole
      point: a `Description:` prose edit re-derives the facts, they compare
      equal, and salsa backdates—so `workspace_project` is never re-executed.
      `discover_packages` no longer reads DESCRIPTION at all
      (`PackageInfo.expected_sources` became `dir_sources`).

      Declared packages feed both sets. `Depends` joins the resolved set via
      `attached_names`; `Imports` deliberately does **not** (R does not attach
      it). All five fields join the referenced set, so `arity index` and the
      sidecar fetch cover a dependency no `.R` file mentions. `import(pkg)` no
      longer poisons: `ProjectScope::build` stays pure and records the packages,
      and `external_resolution`—which holds the library index—runs them through
      the existing enumerability gate, so the suppression lifts by itself once
      the package can be enumerated. `resolution_incomplete` now means only "a
      dynamic or unanalyzed `source()`".

      Invalidation is no longer blunt: `WatchedFilesBatch` carries each changed
      path with its kind, the refreshers report whether they actually wrote, and
      a save that changed nothing no longer relints.

      Two consequences worth knowing, both the conservative-correct direction
      and both pinned by tests: a `Depends` we cannot enumerate now suppresses
      the whole file (exactly as an unindexed `library()` already did), and
      lifting the `import(pkg)` poison exposes findings in every package using
      it—which is how the item below was found.

- [x] **A backticked name never resolves against a package export list.** Fixed.
      `` e$a <- `:` `` and ``map_lgl(imp, `%in%`, x = topic)`` were flagged
      `undefined-symbol`. The backticks are part of the `IDENT` token, which is
      *correct* and load-bearing for user operators (`src/semantic/builder.rs`
      records a `` `%+%` `` binding backtick-quoted so references match), but
      the base and CRAN export lists store `:` and `%in%` unquoted, so the
      lookup missed.

      `semantic::symbols::unbacktick` strips a *matched* backtick pair, and
      every leaf provider lookup now applies it: `StaticBaseR`'s
      `origin`/`is_base`/`package_of`, `BundledPackages::exports`,
      `RemoteExports::exports`, and `IndexedProvider`'s `exports`/`lookup`. Put
      at the leaves rather than in `resolve_origin` so all four resolution
      tiers, `StaticBaseR` used as a bare provider, and hover's rich `lookup`
      are covered by one rule. Nothing changed in the builder, which must keep
      quoting bindings.

      Predated stage 2 (reproduced on `2c5168c`); it was invisible in packages
      using `import(pkg)` only because the wholesale-import poison suppressed
      the whole file.

- [ ] **3. DESCRIPTION lint rules.** `undeclared-dependency` (`pkg::x` or
      `library(pkg)` in `R/` with `pkg` absent from `Depends`/`Imports`) and
      `unused-dependency` (an `Imports` entry no `::` or `importFrom()`
      reaches—cross-file, so it needs the project layer and NAMESPACE, both of
      which exist). Plus file-local rules: `duplicate-field` (also the place to
      fix the first-vs-last divergence below, visibly), missing required fields,
      unparseable version constraints. Needs a non-`.R` path through
      `file_discovery.rs` (the extension filter is hard-coded `"r"`) and the
      lint driver; `linter::render` and `to_lsp_diagnostic` are already
      file-type-agnostic and need no change.

- [ ] **4. DESCRIPTION in the LSP.** `didOpen` for DESCRIPTION (today it is only
      a watched file, never a document), a `documentSelector` entry in
      `editors/code`, then: diagnostics from stages 1+3; **completion of package
      names** in dependency fields off the rindex plus the bundled CRAN
      lists—the flashiest item here and nearly free; hover showing a
      dependency's installed version and `Title`.

- [ ] **5. DESCRIPTION formatting.** Canonical style is what `desc`/`usethis`
      write: field order, dependency lists one per line with `,\n    `, wrapped
      `Description`. arity's differentiator is that `Authors@R` and `Roxygen`
      are *R code*, which we can format with our own formatter—`desc` cannot.
      Must be opt-in: `usethis`, `devtools` and `R CMD build` rewrite this file
      on their own schedule, and a field-order fight is a fast way to lose
      trust. Ships through the dprint plugin too, which is why stage 1 lives in
      the parser crate.

- [x] **A `read.dcf` differential oracle** (`tests/oracle/dcf_oracle.R` +
      `tests/dcf_oracle.rs`, `#[ignore]`d, `task dcf-oracle`). R's `read.dcf`
      *is* the definition of what a DESCRIPTION means, so the parser is checked
      against it rather than against comments claiming what R does. 71 cases:
      the committed DCF fixtures, the rindex DESCRIPTIONs, the untracked
      `roxygen2-ref` checkout when present, and an adversarial table mirroring
      the parser's losslessness cases. The three divergences below are
      normalized; **anything else fails**, so closing one is a matter of
      deleting its normalization and watching the oracle prove the fix. It
      earned its keep immediately by finding divergence 3, which had been
      assumed away.

- [ ] **Known divergences from R's `read.dcf`**, deliberate, normalized in the
      oracle and pinned by tests in `dcf/parser.rs`; each is its own future
      commit, never a drive-by:
  - A field whose own line is empty folds with a leading `\n`
    (`Collate:\n a.R\n b.R` -> `"\na.R\nb.R"`); R drops the empty segment.
  - A duplicate field resolves to the **first** occurrence; R takes the last.
  - A field name is trimmed. R does *not*: `Package : p` declares a field
    literally named `"Package "`, so R sees no `Package` at all. arity is
    deliberately lenient here (it reads the obvious intent of a typo'd header),
    and the CST keeps the whitespace as its own token, so a DESCRIPTION lint
    can flag it precisely instead of the parser guessing.

- [ ] **`desc` is a style reference for stage 5, not an oracle.** Tested against
      desc 1.4.3: `desc::desc_normalize()` reorders fields, splits dependency
      lists one per line, and quotes `Collate` entries—all of which stage 5
      wants—but it **drops comments even on a plain parse->write with no
      normalization**, and emits a trailing space after `Depends:`. Matching it
      byte for byte would mean deleting user content, which contradicts the
      invariant the DCF parser exists to uphold. So measure against it the way
      `air` is measured for R: soft, one-directional, never a gate, with comment
      preservation and the trailing space as known divergences.

## Misc

