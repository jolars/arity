# TODOs

## Parser

- [ ] Comment-aware suppression placement edge cases: directives inside
  `R_CALL_ARGUMENTS` between `( ... , <directive> , next_arg )` need special
  handling so they attach to `next_arg` instead of the argument list. (Jarl
  solved this by overriding biome's `place_comment`; arity's
  next-non-trivia-sibling walk already handles most cases.)

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
- [ ] `internal-function` `pkg:::fn` via
      `BinaryExpr::namespace_access().internal` (correctness, none)—cheap.
- [ ] **§I4 per-rule config**: add a `[lint.rules.<id>]` TOML table + typed
      per-rule struct in `src/config.rs`, threaded into rules via a
      `config`/`&RuleConfig` field on `RuleContext`. **Blocks**
      `undesirable-function`, `download-file`.
- [ ] `unused-function` (suspicious, sem, none)—reuse
      `unused_local_bindings`; **default-off** (exported pkg funcs look unused).
- [ ] `duplicated-function-definition` (suspicious, sem, none).
- [ ] `for-loop-index`/`for-loop-dup-index` (suspicious, sem, none).
- [x] `unnecessary-nesting` collapsible nested `if` (readability, `syn`,
      unsafe). Landed as the purely-syntactic collapsible-`if` variant: an `if`
      with no `else` whose sole body is another `if` with no `else` collapses to
      `if (a && b) body`. Fix joins the conditions with `&&` (each non-primary
      condition parenthesized so grouping survives), unsafe (dedents the body →
      fix-then-format) and withheld on a dropped comment. The guard-clause /
      early-return (`sem`/CFG) variant was scoped out of v1 — see the deferred
      follow-up under Phase B.
- [ ] `undesirable-function` (suspicious, ns + config, none)—needs §I4;
      **default-off**. `download-file` (correctness, ns, none)—low priority.

### Phase 4—Meta (suppression) rules + hardening

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

- [ ] Follow-ups (deferred): run the full rule set over extracted example code
      (needs package-context symbol handling to avoid FPs); unsafe-delete fixes
      for duplicate/nonexistent `@param`; a missing-description variant of
      `roxygen-title` (roxygen2 auto-copies the title into `\description`, so
      it never warns—decide against CRAN's stance first); mine the oracle's
      "uncovered signals" table (mismatched braces/quotes, markdown-link
      plain-text restriction) for new rules.

## Static analysis (dataflow foundation)

Cross-cutting: a def-use index and a control-flow graph feeding **both** the
linter and the LSP. Motivated by an audit of flowR
(https://github.com/flowr-analysis/flowr), a mature static dataflow analyzer for
R (normalized AST -> hierarchical dataflow graph with typed multi-edges -> CFG ->
reaching-definitions fixpoint -> composable query API -> dataflow linter +
program slicing). arity's gap vs flowR is exactly here: today the semantic model
has a scope tree + bindings + name resolution but only a boolean `read` flag—no
def-use reverse index, no CFG, no reaching definitions, no DFG.

Everything below stays within arity's tenets: **static** (no R evaluation, no
type inference), **incremental-first** (each analysis is a `salsa` query in
`src/incremental.rs`, memoized like `semantic_model()`), consumed by both the
linter (`RuleContext`) and the LSP. TDD (fixtures first). Recommended ceiling is
**Phase B (CFG)**; Phase C is an optional later stretch.

### Phase A—Def-use reverse index (cheapest; do first)

- [x] Extend `SemanticModel` so a `Binding` exposes its read sites and each
      `IdentRef` resolves to its `BindingId`. Build it **during the existing
      single walk** in `src/semantic/builder.rs` (`reads_reached`)—no extra
      traversal; it's the reverse of the map the walk already computes. Types in
      `src/semantic/binding.rs`/`src/semantic.rs`. Still flow-insensitive.
- [x] Consume it in the linter: strengthen `unused-binding`
      (`src/linter/rules/correctness/unused_binding.rs`) to reason over the
      concrete read set rather than the boolean flag.
- [x] Consume it in the LSP: sharpen intra-file `references`/`rename`
      (`src/lsp/navigation.rs`) off the def-use edges.

### Phase B—CFG per function body (recommended ceiling)

- [x] New `src/semantic/cfg.rs`: per-function basic blocks + edges for
      `if`/`else`, `for`/`while`/`repeat`, `break`/`next`,
      `return()`/`stop()`, and sequential statements, built from the CST/AST
      wrappers and exposed as a salsa query (`control_flow`). Deterministic and
      local, so it stays keystroke-fast and incremental. Reachability falls out
      of the construction (`FileControlFlow::is_unreachable`); `always_diverges`
      is the shared divergence predicate.
- [ ] Unblock the Phase 3 lint rules that need reachability:
  - [x] `unreachable-code` both-branches-return case (was the documented CFG gap
        in `src/linter/rules/correctness/unreachable_code.rs`); now driven by the
        CFG's `is_unreachable` verdict, namespace-gated on the responsible
        `return`/`stop` leaves.
  - [x] `if-always-true` (literal `if (TRUE/FALSE)` reachability). Flags only
        the bare literals `TRUE`/`FALSE` (never folded constants or the
        rebindable `T`/`F`); purely syntactic (`syn`), no CFG needed. Unsafe fix
        splices in the statically-taken branch (`NULL` for a bare `if (FALSE)`),
        correct by construction and withheld when it would drop a comment.
  - [x] `unnecessary-nesting` collapsible nested `if` shipped as a purely
        syntactic rule (no CFG needed for this variant); see the Linter section
        entry. Deferred follow-up: the guard-clause / early-return de-nesting
        variant (`if (c) { body } else stop()` → early-exit guard) is the piece
        that actually needs CFG reachability (`always_diverges`).

### Phase C—Reaching definitions (optional stretch, not committed)

- [ ] Only if a concrete rule (dead-store, redundant reassignment) justifies it:
      a flow-sensitive fixpoint over the Phase B CFG, lattice over bindings. This
      is the first analysis that is real work to keep incremental—revisit after
      B ships and a rule demands it.

#### Out of scope (recorded so they aren't silently dropped)

Borrow/reject verdicts from the flowR audit—rejected because even flowR only
does these partially/conservatively, and they collide with arity's static tenet:

- **Full hierarchical DFG** (flowR's 5-vertex/9-edge graph). rowan CST +
  `SyntaxNodePtr` + the Phase A def-use index already give the AST<->flow
  linkage; add only the edges a rule needs, not a whole graph.
- **Program slicing** as a core feature—a possible future LSP command ("what
  affects this variable"), not lint/LSP-quality-critical now.
- **NSE / environment / `assign`/`get` simulation** and **lazy-eval/promise
  modeling**—keep the existing conservative data-masking + `resolution_incomplete`
  gates instead.
- **Type / signature-based inference**—an explicit arity non-goal; the
  introspection index stays names+formals+help only.

## Language Server

### Navigation

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

  Follow-ups:

  - [ ] `detail` (signatures) and `container_name` (enclosing binding) for each
    symbol.

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

  Follow-ups:

  - [ ] `container_name` (enclosing binding) and `detail` (signatures) for each
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

- [ ] Folder renaming

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
  - [x] **Label details** (`completionItem.labelDetailsSupport`). Advertised in
    `server_capabilities`; items carry a dimmed origin description (`dplyr`,
    `base`, `local`) plus a parenthesized signature `detail` for indexed
    functions, computed after prefix-filtering (over the survivors only, never the
    full base-R universe). Local-function signatures (from the in-file
    `FUNCTION_EXPR`) are a follow-up.

- Signature help (`textDocument/signatureHelp`). 
  - [ ] Clamp the active parameter into a `...` formal under R's variadic semantics.

### Diagnostics & misc protocol surface

- [ ] Workspace diagnostics (`workspace/diagnostic`)
  
- Semantic tokens (`textDocument/semanticTokens/full`)
  - [ ] base-R/loaded-package `defaultLibrary` modifier
  - [ ] `range`/delta variants
  - [ ] `USER_OP` operators

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

- [ ] **Minor capability-conformance gaps vs. the R languageserver** (2026-07-02
  survey). (a) *(resolved)* arity's completion trigger set now includes `.`, `$`,
  and `@` alongside `:` (`src/lsp/server.rs`; see the `$`/`@` member-completion
  and label-details items under "Completion & signatures"). (b) *(resolved)*
  arity now advertises
  `workspaceFolders.changeNotifications` and reacts to
  `workspace/didChangeWorkspaceFolders` by seeding added folders (removal-drop is
  a tracked follow-up); see the `didChangeWatchedFiles` entry above.
  (c) `textDocumentSync` is FULL-only with no `willSave`/`save` registration
  (benign). Note: the languageserver's `codeLens`, `executeCommand`,
  `linkedEditingRange`, `moniker`, and type/implementation-definition providers are
  **commented out in its own `capabilities.R`**, so they are *not* arity gaps.

- [ ] **Inlay hints** (`textDocument/inlayHint`). E.g. argument-name hints at
  call sites (matching positional args to index formals). Speculative. Not
  loved by all users, possibly opt-in or omit altogether.

### Audit vs rust-analyzer (2026-07-28)

Curated gaps vs rust-analyzer surfaced by the 2026-07-28 architecture audit.
The load-bearing pieces already match ra and are intentionally omitted here
(3-tier incremental reparse in `src/parser/reparse.rs`; the salsa single-writer
+ firewall/durability model in `src/incremental.rs`; the GlobalState/snapshot
split and `TaskPool` in `src/lsp/`). Priorities: **P1** correctness/robustness,
**P2** conformance/UX, **P3** polish.

- [x] **P1 — Request cancellation + stale-read protocol** (landed). Read
  handlers now register the in-flight request in `GlobalState::live_reads` before
  dispatch and route their replies back through the main loop as
  `Outbound::ReadReply` (`src/lsp/state.rs`, `src/lsp/read_jobs.rs`), so the
  single-threaded loop is the sole choke point that decides the final response.
  `$/cancelRequest` removes the entry and answers `RequestCancelled` (-32800); a
  reply whose document version advanced since dispatch is replaced with
  `ContentModified` (-32801) so the client re-requests. The registry is removed
  exactly once (cancel vs. reply race resolved by the loop's serialization), so a
  request is answered exactly once. Covered by the `cancellation_gate` unit tests
  (`src/lsp/state.rs`, deterministic) and `cancel_request_returns_request_cancelled`
  / `stale_read_returns_content_modified` (`tests/lsp_protocol.rs`, wire path).
  Follow-ups: pull-diagnostic (`pending_pull`) requests are not yet cancelable;
  cancellation is protocol-level only (a canceled read still runs to completion
  and its result is discarded) rather than interrupting the CPU work—salsa's
  edit-scoped cancellation (`src/lsp/lint_thread.rs:305`) is unrelated and
  unchanged.

- [x] **P1 — Lint-thread/main-loop panic resilience** (landed). Previously
  `catch_unwind` guarded only read-pool jobs (`src/lsp/task_pool.rs:52`), so a
  panic in `handle_lint_msg` / the analyze write-phase on the sole db-writer
  thread, or in the main `select!` loop, took down the whole server. Now a shared
  `guard()` (`src/lsp/lint_thread.rs`) wraps every lint-thread `select!` arm and
  every main-loop dispatch (`on_request`/`on_notification`/`on_outbound` in
  `src/lsp/server.rs`): a panic is logged and the offending request dropped, the
  thread and db stay alive. The db's internal mutexes now recover from poisoning
  (`.lock()` -> `unwrap_or_else(PoisonError::into_inner)` in `src/incremental.rs`)
  so a panic mid-write leaves the db usable rather than bricked. Covered by
  `guard_contains_a_panic_and_reports_completion` (lint_thread) and
  `db_recovers_from_a_poisoned_mutex` (incremental).

- [x] **P2 — `workspace/didChangeWatchedFiles` + dynamic file-watch
  registration** (landed). On-disk changes made outside the editor now reach
  cross-file analysis. When the client supports dynamic registration, the server
  registers watchers via `client/registerCapability` for `**/*.{R,r}`,
  `arity.toml`, `DESCRIPTION`, `NAMESPACE` (`GlobalState::register_file_watchers`).
  `didChangeWatchedFiles` is classified in `src/lsp/watched_files.rs`
  (`classify_watched_files`): an `arity.toml` edit drops the config cache and
  re-lints (main loop); `.R` create/delete reseeds the member set
  (`apply_r_membership`, excludes honored) and `DESCRIPTION`/`NAMESPACE` edits
  refresh the package graph (both on the lint thread, the sole db writer);
  content changes to tracked-but-unopened `.R` files re-`upsert` from disk while
  open buffers stay authoritative. Also advertises `workspaceFolders` and seeds
  **added** folders on `didChangeWorkspaceFolders`
  (`on_workspace_folders_changed`). **Follow-up:** dropping members under a
  *removed* workspace folder (left in place for now).

- [x] **P2 — Work-done progress for background jobs** (landed). The index pool's
  two background jobs now emit `$/progress` begin/report/end via a
  [`ProgressReporter`](src/lsp/progress.rs): the harvest (`build_index`,
  `maybe_build`) reports indeterminately (it fans across rayon underneath, so
  it's opaque to per-package percentages), and the sidecar fetch
  (`maybe_fetch_remote`) reports a determinate percentage per package. Routed
  through `Outbound::Progress` → the main loop's `on_progress`, which fires
  `window/workDoneProgress/create` then `$/progress`, gated on the client's
  `window.workDoneProgress` capability (there is no *server* capability to
  advertise for server-initiated progress). **Follow-up:** a per-package progress
  callback on `build_index` would let the harvest report a real percentage too.

- [x] **P2 — LSP integration/protocol test harness** (landed). The handler-level
  tests (`tests/lsp.rs`) are now backed by an in-memory harness
  (`tests/lsp_protocol.rs`) that drives the real server loop over
  `lsp_server::Connection::memory()`. `run` was split so the initialize handshake
  + `main_loop` live in a reusable `pub fn serve(connection)`
  (`src/lsp/server.rs`); the harness spawns `serve` on a thread and drives the
  client end. Coverage: initialize/capabilities, didOpen -> formatting response,
  didOpen -> published diagnostics, rapid didChange (coalesce/supersede, asserting
  only the final version's publish survives the `on_outbound` version gate), and
  shutdown/exit clean join. The cancellation case is present as a baseline
  (`cancel_request_is_currently_a_noop`): `$/cancelRequest` is unhandled today, so
  it documents the pre-P1 behavior and flips to expect `RequestCancelled` when the
  P1 work lands.

- [x] **P3 — `positionEncoding` negotiation (UTF-8).** The server now negotiates
  `general.positionEncodings` (preferring UTF-8, arity's native encoding, else the
  UTF-16 default) and advertises the choice via `ServerCapabilities.position_encoding`.
  `LineIndex` was rebuilt as an owned, self-contained structure (a newline table
  plus a per-line wide-char table, à la ra's `line_index`) whose conversions take a
  `PositionEncoding`; it is cached as a `salsa` query (`incremental::line_index`)
  and used for the cross-file/target-file conversions, so those no longer rescan a
  sibling's whole text per request. The session encoding is threaded from
  `initialize` through `GlobalState`/the lint worker to every byte-offset ↔
  LSP-position conversion. Live-buffer conversions still build a fresh `LineIndex`
  (the db copy can lag mid-edit).

- [x] **P3 — Per-URI content-derived pull `resultId`.** The global `result_seq`
  counter (and `bump_result_id`) is gone; the `resultId` is now a content hash of
  the file's findings (`content_result_id` in `src/lsp/state.rs`), so an unrelated
  file re-linted by a cross-file `RelintAll` keeps its id. The cold path was the
  real blocker—a parked pull was always answered `Full`—so `PendingPull` now
  carries the client's `previous_result_id` and `on_outbound` decides `Full` vs
  `Unchanged` per parked pull via the existing `report_kind`. `Unchanged` now
  actually fires. Minor bandwidth win, correctness unaffected.

- Cross-ref (Parser section, incremental reparse follow-up): the sync is now
  `INCREMENTAL` (Stage A). Feeding the precise per-change edit ranges through to
  the token/block/toplevel reparse to shrink the `diff_edit` coalescing is the
  remaining Stage B there.

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

- [x] **Completion trigger characters + label details** (done). arity now
  triggers completion on `$`, `@`, and `.` alongside `:`, and advertises
  `completionItem.labelDetailsSupport` with origin + signature label details.
  Unlike Ark (which resolves `$`/`@` members from a live R session), arity's
  member completion is static—harvested from usage and local construction. See
  the `$`/`@` member-completion and label-details items under "Completion &
  signatures".

- [ ] **Signature-help retrigger on `=`** (reinforce). Ark's signature-help
  trigger set is `(`, `,`, `=`; arity triggers on `(`, `,` only. Typing `=` for
  a named argument should re-trigger to refresh the active parameter. Small
  addition to the signature-help follow-ups above.

- **Package/CRAN index backend—still no *symbol* DB, but a static *source*
  tier now exists.** Ark has *no* CRAN symbol database in arity's sense. Being a
  kernel with a **live R session**, it resolves library *symbols* by calling
  into R via FFI (`harp::exec::RFunction`): `base::.packages()` for the search
  path (`completions/sources/composite/search_path.rs`), `getNamespace(pkg)` +
  `R_lsInternal(exports)` for `pkg::` (`.../unique/namespace.rs`), R's help DB
  for hover. It ships **no bundled/static export lists**, and its only CRAN-repo
  code (`repos.rs`) is just `options(repos=)` config for `install.packages`
  (P3M/PPM default)—not a symbol source. Its workspace `.R` indexer
  (`indexer.rs`, salsa) is the one piece analogous to arity's `DefIndex`. **But**
  for go-to-definition/source display it now fetches R *source text* from a
  static, curated tier (`oak_source`, feeding `lsp/sources.rs`'s
  `SourceHandler`): base-package sources for R 4.2.0 up to a pinned latest are
  packed into a compressed `r-source.tar.zst`, hosted as a GitHub release at
  `posit-dev/oak-r-sources`, downloaded once and cached (posit-dev/ark#1328; the
  PR body's `include_bytes!`-into-the-binary boast is aspirational—the merged
  code downloads+caches, not embeds), and CRAN package sources come from
  downloaded package tarballs. This decouples *source navigation* from the
  installed R (base versions are clamped to the latest present, not required to
  be installed). So the models stay opposite for *symbols*—Ark = live-session
  (version/install-exact, free, but needs a running R and only sees installed
  packages); arity = static/offline by tenet (the whole `src/rindex/`
  bundled+sidecar+harvest tier exists to avoid that dependency)—but the
  "installed packages only" framing no longer holds for *source*, where ark's
  curated `oak-r-sources` archive is a closer analogue to arity's bundled
  `src/rindex/` (different payload: source text vs. symbol/export index).
  Nothing to adopt wholesale for the symbol index. Two reinforcements, both
  already logged under "Cross-cutting prerequisite" above: Ark's P3M/PPM default
  backs the sidecar-hosting plan, and the version-exactness Ark gets for free is
  what the **pin-aware versions** follow-up chases statically.

- Cross-ref (already logged, reinforced by this audit):
  - **On-type formatting.** Ark advertises it with first-trigger `\n` and a
    tree-sitter reindent (`indent.rs`)—the reference model for the on-type
    formatting item above (still gated on the CRLF `format_range` bug).
  - **`INCREMENTAL` text sync.** Both Ark and arity now advertise INCREMENTAL
    (Stage A done); threading the precise ranges into the reparse is the Parser
    incremental-reparse Stage B.
  - **`positionEncoding` UTF-8.** Ark hardcodes UTF-16 today but threads a
    `PositionEncoding` type throughout (ready to negotiate UTF-8)—the same shape
    as the P3 `positionEncoding` item above.

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
  `package_root`/`ProjectScope`, not the file key.

  - [ ] Follow-up: full `vfs`/`SourceRoot` model—opaque-`FileId`-at-the-URI
    boundary in `src/lsp.rs` and
    `SourceRoot`-scoped durability—when multi-root workspaces
    actually need it. Lower leverage for a single-crate tool (the wart
    is already gone).

- `undefined-symbol` FP frontier from the cran/MASS investigation
  (2026-08-01). Four unmodeled binding mechanisms drove ~91% of MASS's
  `undefined-symbol` findings (75/82), each a distinct suppress-only fix. Three
  are now handled in the semantic-model builder (`src/semantic/builder.rs`
  `handle_call`), with the model-frame tail deferred:
  - [x] **`useDynLib(..., .registration = TRUE)` native routines** (14 findings,
    the only category hitting real `R/` source). NAMESPACE registers each
    C/Fortran entry point (`VR_sammon`, `mve_fitlots`, …) as a namespace object
    usable bare in `.C`/`.Call`/`.Fortran`/`.External`. A bare `IDENT` in the
    *head* (first-argument) position of those calls now has its read suppressed
    (reusing the `library()` `suppress_read` slot). Repro:
    `f <- function(x) .C(VR_sammon, as.double(x))`.
  - [x] **`attach(df)` scope-introducer** (49 findings, the biggest bucket) and
    **`load("*.rda")` binding-introducer**. Both introduce statically-unknowable
    bindings, so a file calling either sets `SemanticModel::attaches_opaque_env`
    and `undefined-symbol` gates the whole file (mirrors the
    `resolution_incomplete` gate). Repro: `attach(painters); table(School)`.
  - [x] **`data(name)` NSE loader** (12). `data(sole)` now introduces an
    `Implicit` binding `sole` in the calling frame, so the loader argument and
    every later `sole$…` read resolve. Repro:
    `data(sole); sole$off <- log(sole$a.1)`.
  - [ ] **Model-frame columns** (~1-2 findings, deferred). Non-`data`
    model-fitting args evaluated in the data frame (`polr(size ~ carrier, data =
    tonsils, weights = count)`—`count` is a `tonsils` column) extend the known
    `with`/`subset` data-variable frontier to `weights`/`subset`/`offset` on
    `lm`/`glm`/`polr`. Needs a new per-named-argument masking table (the existing
    masking masks a call's *whole* arg-list), so it is the remaining frontier
    item. (The 7 other residual MASS findings—`A5`/`pr3`/`labs`/`module`—are
    genuine dangling refs in incomplete book-excerpt scripts, correctly flagged.)

- [x] `unused-binding` FP frontier from the cran/MASS investigation
  (2026-08-01). The `$`/`@`-subscript index-drop FP was fixed earlier (see the
  Parser extract-precedence entry); the four remaining scope-asymmetry cases
  (all confirmed against `Rscript`, ~85% of the residual `R/`-source findings)
  are now fixed in the semantic-model builder via a **"deferred read"** primitive
  (`IdentRef::deferred`): a promise-evaluated read carries no intra-frame textual
  ordering, so `reads_reached` lets it reach a same-frame binding assigned
  *after* it (analogous to the existing `loop_range` relaxation). The fourth
  bucket is a separate quoted-binding suppression (`BuildCtx::quote_depth`).
  - **Default-argument expressions** (root cause, ~9 findings). A default is a
    promise in the function's own frame, so its reads are walked `deferred`.
    Repro: `f <- function(x, upper = hmax) { hmax <- sqrt(x); upper }` (also
    `panel = panel.lda` where `panel.lda` is a body-local closure).
  - **`on.exit` read-before-assign** (1). `on.exit(...)` runs at exit; its
    arguments are walked `deferred`. Repro:
    `f <- function() { on.exit(par(oldpar)); oldpar <- par(pty = "s"); plot(1) }`.
  - **`NextMethod()` reads the reassigned formal from the frame** (2). A
    `NextMethod()` call synthesizes a deferred read of each enclosing formal, so
    a reassigned formal (`x <- M`) is used. Repro: `print.foo <- function(x, ...)
    { M <- cbind(x); x <- M; NextMethod("print") }`.
  - **Bindings inside `expression({ ... })`** (4). A `<-` inside a quoting callee
    (`quote`/`bquote`/`substitute`/`expression`) is captured unevaluated and
    records no binding. Repro: `f <- function() { e <- expression({ n <- rep(1,
    nobs) }); e }`.
  - [x] **Unsafe autofix on a chained assignment** (found re-verifying the above
    against cran/MASS, `corresp.R:141`). The one genuine residual finding,
    `vlab.real <- vlab <- paste("Var", 1L:p)`, is a true positive (`vlab.real` is
    dead), but the deletion fix removed the *whole* statement, dropping the live
    inner `vlab <- ...` (read on the next line) and leaving `vlab` undefined
    (confirmed against `Rscript`). `deletion_fix` now withholds when the
    statement's value side is itself an `ASSIGNMENT_EXPR` (a chained assignment);
    the finding is still reported.

## Misc

- [x] Non-UTF-8 file aborts the whole lint run. `arity lint <dir>` bails on the
  first file that isn't valid UTF-8 (`error: failed to read ...: stream did not
  contain valid UTF-8`) instead of skipping it and continuing — one ISO-8859
  file (`r-source/tests/utf8-regex.R`) killed the entire run. Skip-and-warn like
  the corpus harness does for unparseable files. Fixed: `check_paths` now
  collects undecodable files into `LintResult::skipped` and continues; the CLI
  warns per skipped file (both `lint` and `--fix`). Other IO errors still abort.

- [ ] `arity-ignore-unused` meta-diagnostic: emit a finding for suppression
  comments that didn't actually suppress anything (rule ID is reserved but
  the rule is not yet wired in).

- [x] **Harvest lazy-data symbols.** The index now covers R's default packages
  (so hover/signatures work for base-R functions), but `harvest_package`
  only reads `NAMESPACE`/object exports—it skips a package's lazy-data
  (`.getNamespaceInfo(ns, "lazydata")`). So `datasets` harvests 0 symbols and
  hovering a dataset (e.g. `iris`) resolves the package but finds no entry.
  The static name lists already include lazydata; the harvest does not.
  Done: `harvest_package` now reads `data/Rdata.rdx` (the on-disk lazydata
  index) and folds those objects in as `Data` symbols, reusing the existing
  `Meta/Rd.rds`/help path for titles. `datasets` harvests all 108 symbols
  (`iris` included).
