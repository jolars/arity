# TODOs

## Parser

- [ ] Comment-aware suppression placement edge cases: directives inside
  `R_CALL_ARGUMENTS` between `( ... , <directive> , next_arg )` need special
  handling so they attach to `next_arg` instead of the argument list. (Jarl
  solved this by overriding biome's `place_comment`; arity's
  next-non-trivia-sibling walk already handles most cases.)

- [x] Incremental reparse (token/block) beneath `parsed_document`
  (`src/incremental.rs`)

  - [ ] Follow-up: use the LSP's precise edit ranges instead of the
    prefix/suffix text diff. The LSP declares `TextDocumentSyncKind::FULL`, so
    `parsed_document` recovers the edit via a whole-text `diff_edit`; threading
    the client's exact change ranges (switching to INCREMENTAL sync) would keep
    disjoint edits from coalescing into one wide span.

## AST wrappers

- [ ] *Optional polish:* migrate the remaining individual lint rules to call the
  wrappers directly where it reads better than the `matchers` free-fns
  (`comparison-negation` already uses `UnaryExpr`). Low priority — the fold
  already put the rules on the typed layer; this is cosmetic and per-rule.

## Formatter

- [ ] Tribbles

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

## Linter

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

### Phase 0—Infrastructure (unblocks everything)

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

### Phase 1—High-signal, purely syntactic, safe fixes (`syn`)

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

### Phase 2—Call-rewrite idioms, namespace-confirmed (`ns`)

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

### Phase 3—SemanticModel rules + config plumbing

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
- [x] Parser note surfaced by this work: roxygen2 never markdown-processes
      `tag_code` bodies, but arity tokenized markdown inside `@examples` under
      `@md` (harmless for the lint—extraction is token-concat—but a CST-fidelity
      gap). The lexer and inline builder now suppress markdown for the code tags
      (`is_code_tag`/`tag_body_skips_markdown`), mirroring `@rawRd`; fixture
      `roxygen_md_examples_code_body`. (The Rd inline spans these bodies still
      tokenize—`ROXYGEN_CODE` for a backtick span—remain a separate,
      pre-existing gap.)
- [x] Parser leniency surfaced by this work: a stray closing delimiter at top
      level (`f(1))`) was recovered losslessly *without* a parse diagnostic,
      though R itself errors, so `roxygen-examples` and plain-file linting both
      inherited the leniency. The top-level loop now emits an `unexpected '<tok>'`
      diagnostic (still lossless); fixture `stray_close_paren_toplevel`.

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
  - [ ] `$`/`@` member completion
  - [ ] Fuzzy/case-insensitive prefix matching
  - [ ] Function-vs-variable kind for locals

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
