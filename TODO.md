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

### Rename

- [x] **Folder renaming.** `r_file_rename_registration` now carries a second
  filter (glob `**`, `matches: Folder`) alongside the `.R` one (pinned to `File`),
  so a directory move reaches `willRenameFiles`/`didRenameFiles` at all.
  `expand_dir_renames` (`src/incremental.rs`) fans a directory pair out into one
  `old -> new` entry per known path beneath it—workspace members *plus* the
  resolved targets in `reverse_source_edges`, since a sourced file need not be a
  member—with the deepest matching prefix winning for nested pairs, and drops a
  pair it expanded. It is deliberately disk-free: `willRenameFiles` fires before
  the move and `didRenameFiles` after, so a stat would answer differently
  depending on which side asked, while membership holds the *old* paths in both.
  Fixing folder renames also fixed two latent bugs in `source_rename_edits`:
  spellings are now recomputed against the sourcer's **new** parent (literals are
  still *resolved* against the old one), and a renamed file is itself a candidate
  sourcer, so a moved file rebases its own literals even when its targets stayed
  put (this was wrong for single-file cross-directory moves too). An edit is
  emitted only when the literal as written would no longer resolve from the new
  location, so a folder move that carries sourcer and target together produces
  nothing rather than flooding the client. Also switched off `parse(text).cst`
  onto the memoized `parsed_tree`, since a folder rename reparses every candidate
  on the read pool while the editor blocks on the rename dialog.
  - [x] **Follow-up: `apply_file_renames` now checks workspace scope.** A
    destination the seed would not have found is dropped rather than tracked. The
    original note's example was wrong: nothing excludes `inst/`, and
    `collect_r_files` walks the whole root, so `R/a.R` -> `inst/extdata/a.R` stays
    a member (correctly—a fresh seed finds it too). The real out-of-scope
    destinations are outside every root, an excluded or ignored path, and a
    non-`.R` name (`R/a.R` -> `R/a.txt`, which used to stay a member and get
    parsed as R). The amortization landed first as `WorkspaceScope`
    (`src/lsp/workspace_scope.rs`): one walk per *touched* root, built per
    notification and never cached across them, over a new `scope_members_at`
    kernel that `seed_workspace` shares so the seed and the incremental checks
    can't drift. `in_workspace_scope` is gone. The scope check in turn forced
    `rebase_roots`—a root is often just a package directory or a file's parent,
    so renaming one would otherwise leave `roots` pointing at a dead path and
    every file it carried judged out of scope.

### Completion & signatures

- Completion (`textDocument/completion` + `completionItem/resolve`).
  - [ ] Snippet/paren insertion
  - [x] **`$`/`@` member completion** (static, no eval). New `Field` context in
    `src/lsp/completion.rs` mirrors the `pkg::` path: it harvests field names used
    with the same operator on the same receiver anywhere in the file, and—for `$`
    only—infers the named fields of a local
    `list()`/`data.frame()`/`tibble()`/`data.table()` construction bound to the
    receiver. Also stops the prior bare-name leak after `$`/`@`. Triggers on `$`
    and `@` are advertised (`src/lsp/server.rs`).
    - **v1 limits:** the receiver is keyed by whitespace-normalized source text,
      so a chained `a$b$` recover-path key is the immediate token (`b`), not
      `a$b`; `@` harvests only (S4 `new()`/`setClass` slot inference deferred);
      fields carry no docs or signature.
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
    functions (the names the cross-file index keys on) and nested/local ones. An
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
  - [ ] Call sites at script top-level (inside no function) are dropped from
    incoming.
  - [ ] Ambiguous cross-file callees (a name visibly defined in >1 sibling)
    resolve to the first sorted def.
  - [ ] String/backtick callees (`` `+`(…) ``) are skipped.

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

- [ ] **Maintain the line index across edits instead of rebuilding it.**
  `LineIndex::new` is linear in the *document*, not in the edit, and the live
  buffer pays it repeatedly: once per ranged change inside the `didChange` loop
  on the main loop (`src/lsp/state.rs`, the `Some(range)` arm), then again in
  every handler that answers against the buffer (`definition_via_db` and the
  rest of `src/lsp/navigation.rs`, `semantic_tokens.rs`, `document_color.rs`,
  `document_links.rs`, `type_hierarchy.rs`). `src/lsp/state.rs` also does
  `d.text.clone()` per read request, copying the whole buffer.

  Measured on a repeated `tests/oracle/roxygen_oracle.R` (machine under load, so
  upper bounds): 22 us at 16 KB, 162 us at 130 KB, 1259 us at 1 MB. Our builder
  is heavier than it needs to be—`char_indices()` plus a `HashMap`, where a
  `memchr` scan over line starts would do most of the work.

  fatou fixed the same thing (its issue #76, commits `6f949d5` + `a054ecf`): a
  `TextBuffer` holds the text next to its line-start table and *patches* the
  table per edit—starts at or before the edit are untouched, those inside the
  replaced span splice out, the tail shifts by the byte delta—and an open
  document is an `Arc<TextBuffer>` shared with the analysis thread and every
  read job, so nobody rescans and nobody copies the text. That took a keystroke
  on a 1 MB buffer from ~690 us to ~4 us. See `benches/line_index.rs` there for
  the harness, and the rule note in fatou's `.claude/rules/lsp.md`.

  Two things make it more than a copy-paste here:

  - We are already ahead on the *other* half: `line_index` is a salsa tracked
    query returning an owned, `Eq` index (`src/incremental.rs`), so db-routed
    consumers do not rescan and a line-structure-preserving edit backdates the
    query. fatou has no equivalent. So what is actually missing is the
    patch-on-edit half, plus routing the live-buffer handlers through the
    buffer.
  - Our wide-char table is a `HashMap<usize, Vec<WideChar>>` keyed by line
    number, so inserting a line renumbers every later key—not the flat-`Vec`
    splice fatou patches. Either re-key the tail, or move wide chars into a
    `Vec` parallel to `line_starts` so the two splice together. R is nearly all
    ASCII (56 wide chars in that 130 KB file), so the table is nearly empty
    either way.

  Worth sizing before doing: the number that made fatou's case was the rescan
  measured against the *incremental reparse it precedes*. That ratio is unknown
  here, and R files are typically small—the largest in this tree is 16 KB, where
  22 us is below anything a user perceives. The win only shows up on 100 KB+
  files (generated code, large Shiny apps).

## Misc

