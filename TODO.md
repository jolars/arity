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

- [ ] Tibbles

- [ ] Roxygen syntax formatting

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
- [ ] `true-false-symbol` `T`/`F` -> `TRUE`/`FALSE` (readability, **unsafe**
      until shadow-checked in Phase 4 --- T/F are rebindable). Token change, not
      layout, so linter-owned.
- [ ] `repeat` `while (TRUE)` -> `repeat` (suspicious, safe).
- [ ] `vector-logic` `&`/`|` -> `&&`/`||` in `if`/`while` condition
      (correctness, safe).
- [ ] `comparison-negation` `!(a == b)` -> `a != b` (readability, safe);
      `outer-negation` `!any(...)`/`!all(...)` De Morgan (readability, safe).
- [ ] `implicit-assignment` (suspicious, none) --- scope to avoid overlap with
      existing `assignment-in-condition`.

#### Phase 2 --- Call-rewrite idioms, namespace-confirmed (`ns`)

- [ ] **§I2 regex/string-literal helper** first: read a `STRING` token's
      unquoted contents; classify regex metachars / single anchor (`^`/`$`).
      Blocks `string-boundary`, `fixed-regex`.
- [ ] `any-is-na` `any(is.na(x))` -> `anyNA(x)` (performance, safe) --- flagship.
- [ ] `any-duplicated` `any(duplicated(x))` -> `anyDuplicated(x) > 0`
      (performance, safe).
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
- [ ] `unreachable-code` after `return()`/`stop()` (correctness, sem,
      unsafe-delete).
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
      `resolves_to_base`-confirmed + shadow-checked, graduating
      `true-false-symbol` and call-rewrite rules Unsafe -> Safe and suppressing
      FPs where `any`/`is.na` etc. are user-redefined.

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

    - [ ] *Follow-up: body reads bind to the final scope, which may be a shadow,
      not the cohort.* A reader's **function-body** reads are treated as binding
      to the renamed cohort (they run at call time against the final
      post-execution scope, and the reader is in `seen_by(def_file)` and doesn't
      shadow the name). But if the reader sources a cohort def **and then** a
      later same-name def that is *not* in the cohort (a `source()`-shadow, e.g.
      `source("a.R"); source("z.R")` where both define `foo`), its final scope
      binds `foo` to `z.R`, not `a.R` --- so co-renaming the body read is wrong
      (it isn't a reference to the cohort def). This predates B2.4 (the
      position-blind `seen_by` membership assumed final scope == cohort) and
      isn't narrowed by it: `reader_rename_ranges` keeps body reads by
      construction, and `top_level_read_provenance` only classifies *top-level*
      reads. A precise fix would resolve each body read against the reader's
      final-scope binding (last-writer-wins across its `source()` closure) and
      skip the ones that bind to a non-cohort shadow --- conceptually the same
      replay as `top_level_read_provenance` but evaluated at end-of-file rather
      than per-position. Rare (needs two same-name defs, one sourced-shadow, both
      reachable from one reader); deferred. `references` over-reports it
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

- [ ] **Pull diagnostics** (`textDocument/diagnostic` + `workspace/diagnostic`).
  The server currently *pushes* diagnostics from the lint thread; the pull
  model (LSP 3.17) lets clients request on demand and is friendlier to the
  coalescing/versioning the lint thread already does. Additive alongside
  push.
  
- [ ] **Semantic tokens** (`textDocument/semanticTokens`). Scope-aware
  highlighting (distinguish function calls, locals, package-qualified names,
  arguments) from the same `SemanticModel`; degrades gracefully if omitted.
  Maybe omit and rely on native editor syntax highlighting.

- [x] **Folding ranges** (`textDocument/foldingRange`). Pure CST walk ---
  brace blocks, function/parameter and argument lists, parenthesized and
  subscript expressions, comment runs. No semantic model needed.
- [ ] **Selection ranges** (`textDocument/selectionRange`). Pure CST walk:
  incremental scope expansion from the cursor outward through enclosing nodes.

- [ ] **Call hierarchy** (`textDocument/prepareCallHierarchy` + incoming/
  outgoing). Caller/callee graph; rides the same cross-file reference index
  as workspace symbols and references.

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
