# TODOs

## Parser

- [ ] Comment-aware suppression placement edge cases: a **lint** directive
  inside `R_CALL_ARGUMENTS` between `( ... , <directive> , next_arg )` needs
  special handling so it attaches to `next_arg` instead of the whole argument
  list — over-broad rather than inert, so no rule reports it. (Jarl solved this
  by overriding biome's `place_comment`; arity's next-non-trivia-sibling walk
  already handles most cases.) The **format** half of this is closed: the
  formatter acts on statement lists only, and `misplaced-suppression` reports a
  format directive that lands anywhere else.
## AST wrappers

- [ ] *Optional polish:* migrate the remaining individual lint rules to call the
  wrappers directly where it reads better than the `matchers` free-fns
  (`comparison-negation` already uses `UnaryExpr`). Low priority — the fold
  already put the rules on the typed layer; this is cosmetic and per-rule.

## Formatter

- [ ] Measure a **line-spanning reflow chunk** by its widest segment, not its
  flattened length. Only a soft-wrapped `\verb{…}` still reaches this — every
  other inline Rd macro joins its wrap (`join_soft_breaks`) — but there
  `wrap_chunks_hanging` counts the whole multi-line text as one word, so the
  chunk is over-wide wherever it lands and the ones after it are measured from
  the wrong column. Output is stable and render-preserving either way; only the
  wrap points are off. `SectionUnit::flush` special-cases the inline/form-2
  decision for such a chunk, which this would subsume.

- [ ] Honor `# arity-format skip` and `off`/`on` in a `DESCRIPTION`. Only
  `skip-file` works there today; the other verbs need the field-class planner
  (`formatter/description/plan.rs`) to learn to hand a field's lines back
  verbatim. `misplaced-suppression` does not yet report the ones that are inert
  there.

## Linter

- [x] `deprecated-suppression` flags the **deprecated `# arity-ignore`
  spellings** and rewrites them to `# arity-lint skip` / `# arity-lint
  skip-file`. Keyed off `Spelling::Deprecated`; `Safe` fix over the parsed
  directive's `prefix` range alone, so the rule ID, the reason, and the author's
  spacing are untouched. A migration aid, not a correctness fix — enabled by
  default, since a deprecation nobody sees drives no migration.

- [ ] Remove the `# arity-ignore`/`# arity-ignore-file` aliases once
  `deprecated-suppression` has been out for a release or two. Parser side is one
  branch each in `crates/arity-parser/src/directive.rs`, plus `Spelling` and the
  rule itself.

### Rule-candidate audit: lintr and jarl (2026-08-18)

Audited [lintr](https://github.com/r-lib/lintr) at
`fbe442428833d4897d7e95c4d389f631b8a9057c` and
[jarl](https://github.com/etiennebacher/jarl) at
`0e74cdb9840c7adca36d7bbf1b10c6d87da8acd6`. This is a candidate list, not a
promise to reproduce either catalogue. Before implementing one, confirm its
semantics against base R/the relevant package, check callee resolution rather
than matching a bare spelling, and use the normal `add-lint-rule` TDD workflow.

Tier 1—clear correctness bugs, good default-on candidates:

- [x] `equals-nan` and `equals-null`: flag `==`, `!=`, and `%in%` comparisons
  against `NaN`/`NULL`; recommend `is.nan()`/`is.null()`. Safe fixes require a
  confirmed base helper and semantic equivalence: equality/inequality for both,
  plus only right-hand `NaN` membership; other membership shapes are report-only.
- [x] `missing-argument`: report interior empty call arguments such as
  `paste("a", , "b")`. A trailing comma is valid and common, and missing
  formals such as `function(x, y = )` are intentional R, so scope this to
  call arguments and do not offer a deletion fix. Implemented syntactically
  over empty call `ARG` nodes; reports the following comma with no autofix.
- [x] `rep-times-ignored`: flag base `rep(x, times = ..., length.out = ...)`,
  where `length.out` normally wins. Implemented as an `ns`-tier, report-only
  warning: invalid/`NA` `length.out` can make `times` matter, so deletion is not
  safe for arbitrary static expressions.
- [x] `sprintf`: statically validate literal formats—invalid conversions and
  definitely missing/excess arguments—and report the pointless
  `sprintf("literal")` case separately within the same rule. Format parsing
  must handle `%%`, positional fields, `*` width/precision, and recycling
  before this can claim correctness. Implemented as an `ns`-tier warning over
  scalar and `c()` literal formats; only the literal-only collapse has a safe
  fix.
- [ ] `glue`: for a statically known `glue()` template and delimiters, report
  unmatched/incomplete interpolation delimiters. Gate on the package/callee and
  parse the template rather than treating braces with a regexp. Reuse the
  semantic interpolation analyzer that supplies use-only reads for
  `unused-binding`. A template with no interpolation is at most an opt-in
  readability rule: `glue()` also provides trimming, concatenation, and a
  distinct result class, so a default warning would be noisy.
- [ ] `length-test`: flag the likely-parenthesization error
  `length(x == n)`/`length(x != n)` and suggest `length(x) == n`. Keep the
  diagnostic conservative around overloaded calls and non-atomic operands.
- [x] `all-equal`: flag truth-testing `all.equal()` directly (conditions,
  negation, `isFALSE`) because disagreement returns a character vector, not
  `FALSE`; recommend `isTRUE(all.equal(...))`. Fixes are unsafe because they
  can deliberately change existing behavior. Implemented as an `ns`-tier
  warning with namespace-confirmed callees and unsafe fixes.
- [x] `pipe-return`: report `return`/`return()` on the RHS of `%>%`; it does not
  return from the surrounding function. The native-pipe spelling is already a
  parse error. Implemented as a default-on `ns`-tier warning for direct,
  base-resolved RHS stages, with a shared `%>%`/`|>` chain matcher and no fix
  because the intended control flow cannot be inferred.
- [x] `function-return-assignment`: report assignments inside `return(...)`.
  Implemented as an `ns`-tier, report-only warning for direct assignment
  arguments to namespace-confirmed base `return()`; no automatic rewrite can
  infer whether the binding or the returned value was intended.

Tier 2—performance/readability transformations that the formatter cannot do:

- [x] Add a shared call-rewrite batch for `matrix-apply`
  (`apply(x, 1/2, sum/mean)` to row/column helpers), `which-grepl`, `rep-len`,
  `system-file`, `list2df` (R >= 4.0), and `length-levels`. Each requires
  base-resolution checks, exact named/positional argument matching, trivia-safe
  fixes, and version gating where applicable.
  Implemented as six `ns`-tier warnings over exact argument shapes; four fixes
  are safe, while `matrix-apply` and `list2df` are unsafe because array shape,
  method dispatch, or recycling can change behavior. All fixes preserve retained
  trivia, and `list2df` honors the R 4.0 floor.
- [ ] `boolean-arithmetic`: recognize `length(which(p)) == 0` and
  `sum(logical) == 0`-style existence tests and prefer `!any(p)` (plus the
  positive variants). Start with shapes whose NA behavior is provably
  preserved; lintr's broad family needs an oracle matrix before porting.
- [ ] `list-comparison`: flag direct comparisons of known list-producing calls
  such as `lapply(...) > 1`, which rely on awkward coercion; recommend a typed
  iterator rather than guessing a fix.
- [ ] `terminal-close`: flag a function whose final action is `close(conn)` and
  recommend registering cleanup with `on.exit()` near acquisition. Default-off
  until a corpus pass establishes that ownership-transfer patterns are not
  noisy.
- [ ] `routine-registration`: flag string-named `.Call`/`.C`/`.Fortran`/
  `.External` calls in packages and recommend registered native symbols. Reuse
  the existing NAMESPACE/native-registration project facts; do not invoke R or
  inspect the local installation.
- [ ] `package-hooks`: validate `.onLoad`, `.onAttach`, `.Last.lib`,
  `.onDetach`, and `.onUnload` signatures and statically forbidden/noisy calls
  per Writing R Extensions. Package-only, cross-checked against `R CMD check`'s
  implementation rather than copied blindly from lintr.
- [ ] `strings-as-factors`: when the R compatibility floor crosses 4.0, flag a
  `data.frame()` with known character columns and no explicit
  `stringsAsFactors`. No fix can choose the intended behavior.
- [ ] Extend the existing Phase 5 package-aware plan with jarl/lintr's remaining
  testthat rewrites: `expect-not`, `expect-s4-class`, `expect-comparison`,
  `expect-identical`, `expect-shape`, and yoda argument order. Keep the family
  default-off initially: these improve test intent/messages rather than program
  correctness. The already-listed shared matcher should own all of them.

Tier 3—useful but policy-heavy; consider only default-off after corpus evidence:

- [ ] `cyclomatic-complexity`, parameterized by a threshold. This is a
  maintainability metric, not layout; define the count over arity's CFG instead
  of depending on R's `cyclocomp` package.
- [ ] `unused-import`: report an attached package whose symbols are never used.
  This requires the project resolver to distinguish attachment-provided bare
  names from `pkg::name`, plus explicit exemptions for packages attached for
  side effects; do not infer availability from the user's installed library.
- [ ] `commented-code` and `todo-comment`: potentially useful repository-policy
  checks, but inherently noisy. If added, keep default-off, ignore roxygen, and
  require configurable markers/exceptions. Commented-code detection must use
  the parser and must not evaluate R.
- [ ] `nonportable-path`: only pursue a narrow, evidence-backed version (for
  example hard-coded Windows drive paths/backslashes). A blanket warning on
  string literals containing `/` is wrong for URLs, regexes, archive members,
  and deliberately POSIX-only code.

Already represented elsewhere in this file: the cohesive `pkg/testthat/`
matcher and `pkg/dplyr/` rules under **Phase 5—Package-aware rules**. Other jarl
ideas worth revisiting only after a shared pipe-chain abstraction exists are
`dplyr-group-by-ungroup`, `nested-pipe`, and `unnecessary-placeholder`.

Deliberately excluded as formatter-owned style: braces, commas, indentation,
infix spacing, line length, quote choice, semicolons, spaces around/inside
parentheses, trailing whitespace/blank lines, pipe continuation/layout, object
name/length conventions, assignment-token preference, and numeric leading-zero
spelling. The formatter is the sole layout authority; arity must not add lint
rules for these. Also rejected for now: `missing-package` (depends on the
current machine rather than project facts), broad `library()` placement
policy, and bare-name `object-overwrite` (substantially overlaps the more
semantic `shadowed-builtin` and is prone to noise).

### `undefined-symbol` false positives (rlang sweep, 2026-08-13)

- [x] **`useDynLib()` binds native routines arity could not enumerate.** They
  live in the C sources, so a reference outside a `.Call` head — passed as a
  value (`capture_arg = ffi_enquo`) or compared
  (`identical(capture_arg, ffi_enquo)`) — was a false positive. Closed by
  *harvesting*, not by suppressing: `src/project/native.rs` reads the
  `R_CallMethodDef`/`R_CMethodDef`/`R_FortranMethodDef`/`R_ExternalMethodDef`
  tables out of `src/` (recursively — rlang's is in `src/internal/internal.c`),
  handling both the string-literal shape and the stringifying `CALLDEF(fn, n)`
  macro, and `parse_namespace` learned `useDynLib`, so explicitly named routines,
  `alias = routine`, and `.fixes` all resolve too. Disk-derived in
  `discover_packages`, frozen into the interned `Project`, folded into each
  package member's `visible` set by `ProjectScope::build` (which stays pure).
  Blanket suppression under `.registration = TRUE` was the rejected alternative:
  it would silence `undefined-symbol` across the whole package. 9 findings
  dropped in rlang (`R/nse-defuse.R`, `R/hash.R`), none added; harvest verified
  exact against rlang, checkmate, bit, Matrix, and data.table. Known limits: a
  registration shape the scanner does not recognize leaves its false positives in
  place (the reporting direction), and an entry behind `#ifdef` is harvested
  whether or not this build compiles it (suppress-only). Costs ~7 ms of `src/`
  I/O per package discovery, and nothing at all for a package with no
  `useDynLib`.

- [x] **rlang's defusing operators are unquote-aware.** `quo`/`quos`/`expr`/`exprs`
  joined the base four in `quoting_callee_kind` (`semantic/builder.rs`), and the
  mask now has holes in it: `unquote_operand` matches `!!`/`!!!`/`{{ }}`
  structurally (no dedicated `SyntaxKind` — a doubled unary `!`, a doubly nested
  `BLOCK_EXPR` around a lone symbol) and `walk_evaluated` lifts every mask over
  the operand, so an unresolved name there is still reported. `bquote`'s
  `.()`/`..()` got the same treatment. `enquo`/`enexpr`/`ensym` are deliberately
  *not* masked — their argument must name a formal, so it already resolves, and
  masking would only discard the true positive `enquo(typo)`. Two deliberate
  limits: a base `quote()` clears the escape, so `expr(quote(!!x))` masks where
  rlang would unquote; and `!!` under a data-masking verb (`mutate(df, !!x)`)
  still masks. Both suppress only. `enter_quote_mask` also fixed the qualified
  path, which masked without raising `quote_depth` — `base::quote({n <- 1})` used
  to record `n` as a binding while the bare spelling did not.

### False positives (eulerr sweep, 2026-08-14)

- [x] **`unused-binding` missed a closure's read of a reassigned name.** Past the
  frame boundary `reads_reached` (`semantic/builder.rs`) returned only the *first*
  same-name binding, so in `fit <- 1; print(fit); fit <- 2; h <- function()
  print(fit)` the second `fit` looked unread — and its unsafe fix would have
  deleted the assignment `h()` actually reads. A closure body carries no textual
  ordering relative to the enclosing frame, so every candidate there is now
  marked, matching the conservatism the in-frame branch already applied to a
  reassignment. One finding dropped in eulerr, none added.

- [x] **`coalesce` fired on the definition of `%||%` itself**, advising that the
  operator be defined as a call to itself. A local polyfill is routine below the
  R 4.4 floor the rule's own fix warns about. Exempt via `defines_coalesce_operator`.

- [x] **NSE argument to a package-local function.** eulerr's `euler()`/`venn()` do
  `by <- substitute(by)`, so `euler(dat, by = list(sex, age))` never evaluates
  `sex`/`age` in the caller — but all 21 remaining `undefined-symbol` findings in
  eulerr were exactly that. Package-local calls now follow a conservative
  contract: `file_promise_seeds` projects range-free formal-use evidence,
  `project_promises` propagates eager behavior through package wrappers, and the
  rule checks an actual argument only when its matched formal is proven eager.
  Capture, opaque forwarding, unused promises, duplicate definitions, and
  ambiguous argument matching suppress instead. This avoids package-specific
  function lists and deliberately spends false negatives to keep false positives
  rare. `tests/salsa_incremental.rs` pins both backdating and invalidation when
  the promise contract changes. Reproducer: `R/euler.R:272` against
  `tests/testthat/test-plotting.R:672`.

- Deliberately **not** changed: `duplicated-arguments` on `list(b = 1, b = 2)`.
  Legal R (two elements, both named `b`), so it is the same shape as the `c()`
  exemption in 6f0db6d — but unlike `c()` it is usually a typo, and 6f0db6d chose
  to keep flagging it. Revisit only with a survey showing the noise is real.

### Roxygen topic rules

The rlang `roxygen-param` false positive is fixed, and topic resolution now
spans the package: `file_roxygen_topics` (`src/project/roxygen.rs`) is the
range-free per-file projection, `project_roxygen_topics` (`src/project/graph.rs`)
folds it into a `RoxygenTopicIndex` keyed by package root, and
`roxygen-param`/`roxygen-return`/`roxygen-title` judge an owner against its whole
topic wherever the joiners live (`RuleContext::topics`, with the file-local
`RoxygenTopics` as the single-document fallback). 28 findings dropped in rlang,
none added; rlang, ggplot2, and dplyr are byte-identical across the cross-file
step, which only ever removes findings. The Rd projector's `topic_name`
(`src/roxygen/project_rd.rs`) now reads tag values through
`RoxygenTag::value_text` too, so an `@md` topic name is no longer truncated at
the first `_`/`*`; the parity run showed an unchanged pass set (1030 cases), so
no pin or allowlist entry moved. What remains:

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

- [x] **Give the driver's per-file context a struct.** Done: `FileContext`
  (`src/linter/rules.rs`) carries `project`/`resolution`/`package`/`topics`, and
  both `#[allow(clippy::too_many_arguments)]`s are gone. `run_dcf_rules` is
  untouched — six parameters, a different set of inputs.

### `futureverse/future` linter sweep (2026-08-19)

Audited `futureverse/future` at
`f4847e564a983af2e75ded57b924e5cb4e6ace52` against R 4.6.1 and roxygen2 8.0.0.
The isolated call, namespace, replacement-function, default-dataset, loop-index,
and `@param` defects found by the sweep are covered in `tests/lint.rs`. What
remains needs broader modeling rather than a name-shaped exemption:

- [x] **Keep one roxygen block across ordinary comments.** roxygen2 skips an
  intervening `#`, `##`, or `#"` line and continues collecting the surrounding
  `#'` lines, while arity ends the first `ROXYGEN_BLOCK`. That produced 21 false
  `roxygen-param`/`roxygen-title`/`roxygen-return` findings. Minimal case:

  ```r
  #' Title
  #' @param x Value.
  #' @return Value.
  # comment
  #' @export
  f <- function(x) x
  ```

  `roxygen2:::parse_text()` returns one block with
  `title,param,return,export`; the parser fix must preserve the ordinary comment
  byte-for-byte and keep full and incremental parses equivalent.
- [ ] **Resolve manual roxygen aliases and usage across blocks.** A block with
  `@usage g(x)`, `@param x`, `@return`, and `@aliases g` can document a later
  export-only `g <- function(x) x`; roxygen2 generates the shared topic, while
  arity reports missing title, parameters, and return on `g`. Future's
  `%packages%`, `mandelbrot_tiles`, `as.raster.Mandelbrot`, `plot.Mandelbrot`,
  and `as.FutureGlobals` expose this. Extend the range-free topic projection;
  do not bolt aliases onto individual rules.
- [ ] **Do not let one inherited topic member suppress owner coverage.** The
  current topic-wide "any member inherits parameters" gate misses the genuinely
  undocumented `hooks` formal on `FutureBackend()`, which
  `tools::checkDocFiles()` reports. Preserve the unknown surface only for the
  member whose formals inheritance actually obscures.
- [x] **Model statically recoverable dynamic local reads without blanket-marking
  every binding used.** Literal `get("name")` and statically parsed glue/cli
  interpolation now emit scoped use-only reads. They share ordinary binding
  ordering and closure resolution, but stay out of `undefined-symbol`, rename,
  and reference spans; a separate range-free projection marks package siblings
  used without feeding name resolution.
- [ ] **Propagate captured-expression provenance through `eval()`.** Future's
  `data` in `R/backend_api-Future-class.R` and `d` in
  `inst/testme/test-globals,toolarge.R` are reached through expressions whose
  names cannot be recovered at the `eval()` call alone. Carry a conservative,
  range-free expression fact rather than gating the whole file.
- [ ] **Model dynamic assignment/injection escape hatches.** Future's `%<-%`
  assignment creates `opt1`/`opt2`, and `attachLocally()` plus
  `future(..., globals = ...)` supply `sumtwo`; R resolves all 14 reads, while
  `undefined-symbol` cannot see the runtime-created bindings. Any suppression
  must be tied to the affected expression/environment rather than disabling the
  whole rule for a file.
- [ ] **Recognize `detach(package:name)` as NSE package syntax.** The `package`
  token is not a scope read: R accepts
  `detach(package:datasets)`, but arity reports `package` undefined. Mask only
  the first argument's exact `package:<name>` shape; ordinary `:` expressions
  still contain real reads.
- [ ] **Track `.Random.seed` after a known RNG initializer.** `set.seed(1); x <-
  .Random.seed` is valid and produces an integer vector, accounting for two
  false `undefined-symbol` findings. A blanket exemption is wrong because a
  fresh-session read of `.Random.seed` errors before any RNG call.

Deliberately unchanged: 225 `roxygen-unknown-tag` findings on `inst/testme/`
all name the custom test-runner marker `@tags`. roxygen2 also reports it as
unknown, the spans are exact, and the rule already documents suppression for
custom tag systems.

### More DESCRIPTION rules (follow-ups to stage 3)

Stage 3 shipped three `description-*` rules; R's own checks define a much larger
checkable surface. The oracle is three functions, not the prose in R-exts:
`tools:::.check_package_description` (what `R CMD check` enforces, plus its
`strict = TRUE` Title/Description clauses), `.check_package_description2`
(cross-field dependency checks), and `.check_package_CRAN_incoming` (the CRAN
pretest NOTEs). Read those before writing any rule here; the manual paraphrases
them and is looser than the code.

Prerequisite worth doing first, and the thing that makes the rest defensible:

- [x] **A `.check_package_description` differential oracle.** Done:
  `tests/oracle/description_oracle.R` + `tests/description_oracle.rs`,
  `#[ignore]`d, `task description-oracle`. The corpus combines rindex fixtures,
  optional reference checkouts, and a planted-defect table; a missing `Rscript`
  is a skip, as in the other two oracles.

  Four checkers, because one is not enough:
  `.check_package_description(strict = TRUE)`,
  `.check_package_description_authors_at_R_field(strict = 3L)` (the outer
  checker calls it at `strict = FALSE`, so the per-person name, role, ORCID,
  and ROR signals are unreachable from there), the `duplicates` half of
  `.check_package_description2`, and the version, Maintainer, and Author
  components of `.check_package_CRAN_incoming(localOnly = TRUE)`. The last two are
  cherry-picked, not taken whole: their other components need installed
  packages, a `src/`, files, or the network, which a text-only oracle has no
  business simulating. Note the CRAN checker reaches the version only after a
  `Maintainer` and a `Title` it can inspect, and errors on the `NA`
  otherwise—a planted case that wants those signals has to carry both.

  **Two-sided by construction**, because arity implements a fraction of what
  R checks. `GATES` holds shipped rules with a direct R counterpart and requires
  containment, not parity: every finding from a gated rule must be backed by an
  R signal on that case. The reverse is not gated—`description-version-constraint`
  deliberately says nothing about a malformed package *name*, and demanding
  parity would be demanding a rule that does not exist. `PLANNED` holds signals
  no rule covers, tagged with the rule that may claim them; they are counted and
  ranked, never failed, and the current ranking is printed by
  `task description-oracle`.

  Two structural failures keep it honest, and both were verified by
  mutation rather than assumed: an **unknown signal fails** (R's checkers
  are data that changes with R, so a new check must be classified, not
  absorbed), and a **gated signal no case exercises fails** (an oracle that
  tests nothing passes quietly forever). Moving a signal from `PLANNED` to
  `GATES` is what "the rule landed" means.

  Facts it established, each of which contradicts a plausible reading of
  R-exts:

  - `person("A", "B", role = c("aut", "cre"))` with **no email** is rejected
    by R (`bad_authors_at_R_field_has_no_valid_maintainer`), because
    `Maintainer` is *derived* and the derivation needs one. See the
    `description-missing-field` gap above; the fixtures in `tests/lint.rs` and
    `tests/lint_description.rs` were themselves incomplete this way and now
    carry an `email`.
  - `missing_encoding` is **not** "contains non-ASCII": R's condition is
    `!all(.is_ISO_8859(db))`, so a Latin-1-representable `Café` does not
    trigger it and CJK does. A rule written from the manual would be wrong.
  - `valid_package_name` requires **at least two characters**, so `Package: p`
    is itself malformed—which quietly contaminates any hand-written fixture.
  - R buckets a bad dependency entry three ways and they are not the obvious
    ones: `dplyr (1.0.0)` is a `bad_dep_entry` (the parens do not hold
    `op version` at all), `dplyr (=> 1.0)` a `bad_dep_op`, `dplyr (>= foo)` a
    `bad_dep_version`. `description-version-constraint` cuts across all three,
    which is why it is gated against their union.
  - `.check_package_description` does **not** check for missing mandatory
    fields at all—that lives in `R CMD build`—so `description-missing-field`
    and `description-duplicate-field` have no counterpart signal and stay
    ungated. Worth knowing before hunting for one.

Tier 1—pure grammar, R is the oracle, no new machinery. None takes a fix:

- [x] `description-package-in-multiple-fields` (packaging; syn; no fix, default
  on). Done. Spans the *later* listing's bare package name and names the field
  holding the earlier one; source order, so the finding always points backwards
  up the file. Mirrors R's three exclusions, each of which a rule written from
  the manual would get wrong: `LinkingTo` is not compared at all (`LinkingTo` +
  `Imports` is the Rcpp idiom), `R` is never a dependency, and each field is
  uniqued first, so `Imports: dplyr, dplyr` is not this rule. No fix—which
  field to keep decides whether the code may rely on the package. `duplicates`
  moved from `PLANNED` to `GATES` in `tests/description_oracle.rs`.

- [x] `description-malformed-name` (packaging; syn; no fix, default on). Done.
  `Package` against R's `valid_package_name`
  (`[[:alpha:]][[:alnum:].]*[[:alnum:]]`), plus "this is the name of a base
  package" off `base_priority_packages()`. Three exclusions a rule written from
  the regexp alone would get wrong: R checks `^(R|<regexp>)$`, so the language's
  own name survives the two-character floor; `[[:alpha:]]` is *locale*-dependent
  and matches Unicode letters under UTF-8, so `café` is a name R accepts;
  and `Priority: base` exempts a base package from naming itself. An absent or
  empty `Package` stays `description-missing-field`'s. No fix—the name is also
  in the NAMESPACE, the file names, and every `pkg::`. `bad_package` moved from
  `PLANNED` to `GATES` in `tests/description_oracle.rs`, backed by the signal's
  presence rather than its detail (R's detail is its own message, not the name).

- [x] `description-malformed-version` (packaging; syn; no fix, default on).
  Done. `Version` against `valid_package_version`
  (`([[:digit:]]+[.-]){1,}[[:digit:]]+`, so **at least two components**—a bare
  `Version: 1` is rejected), plus CRAN's leading-zeroes (`1.01`, carving out
  `^[0-9]{4}[.-][0-9]{2}` calendar versioning) and absurd-component
  (threshold 1234) NOTEs. First clause wins; the repair is the same either
  way. Four things a rule written from the regexps alone would get wrong:
  `[[:digit:]]` is **ASCII** here, the opposite of `[[:alpha:]]` in
  `description-malformed-name`—confirmed against `grepl`, which rejects an
  Arabic-Indic digit; `Priority: base` exempts the field, since R guards
  `bad_version` with `!is_base_package` and a base package's `@VERSION@` is
  R's own to spell; CRAN's absurd-component carve-out is "equal to the
  *submission year*", which a linter cannot know without reading the clock, so
  arity exempts the whole 1900–2999 band instead (strictly more permissive, so
  containment holds through 2999); and CRAN NOTEs the trailing `.9000` of a
  development version, which is right for a pretest nobody submits one to and
  wrong for a linter reading packages in exactly that state, so a *trailing*
  component of 9000 or more is read as the marker `usethis` writes. No
  fix—which number a release carries is a decision about the release, and it
  is also in the tags, the `NEWS.md`, and every dependent's constraint.
  `bad_version` moved from `PLANNED` to `GATES` in
  `tests/description_oracle.rs`, joined there by the two CRAN signals the
  fourth checker now exposes; four planted cases exercise them, two of them
  pinning the places arity deliberately withholds.

- [x] `description-malformed-maintainer` (packaging; syn; no fix, default on).
  Done. `Maintainer` against `.valid_maintainer_field_regexp` (exactly one
  `Name <email>`, or the literal `ORPHANED`), plus the three CRAN-incoming
  checks that cover the *name* half: `empty_Maintainer_name`,
  `Maintainer_needs_quotes`, and `Maintainer_invalid_or_multi_person`. First
  clause wins; the missing-address case gets its own message, since it is the
  common one. R's regexp is ported as written and **not** tightened to RFC
  5322 — a quoted local part, a domain with no TLD, and a domain label
  starting with `-` are all addresses R accepts. Two things a rule written
  from the regexp alone would get wrong: **two maintainers pass it**, because
  the `.*` before the `<` swallows the first person, so the multi-person
  shape is caught only by the CRAN NOTE (the planted
  `maintainer-multi-person` case fires no `bad_maintainer` at all); and a
  `Maintainer` **wrapped across continuation lines** is one R accepts, since
  it matches the folded value with a `.` that takes a newline. No fix — an
  address cannot be invented, a name cannot be invented, and whether a comma
  separates a surname from a given name or separates two people is the
  author's call. `bad_maintainer` moved from `PLANNED` to `GATES` in
  `tests/description_oracle.rs`, joined by the three CRAN signals the driver
  now exposes; backed by presence, since three of the four are logical flags.
  Six planted cases exercise them, three of them pinning the places arity
  deliberately withholds.

- [ ] **Gap in the shipped `description-missing-field`.** It treats the mere
  presence of `Authors@R` as satisfying `Author` and `Maintainer`, but R only
  *derives* them if some person has role `"cre"`, a **valid email**, and a
  non-empty name; otherwise `R CMD check` errors with "Authors@R field gives
  no person with maintainer role, valid email address and non-empty name."
  So `person("Jane", "Doe", role = c("aut", "cre"))`—no `email`—is rejected
  by R and passes arity today. Confirmed against R. Fixing it means the rule
  consults the parsed `Authors@R` rather than testing the field for
  non-emptiness; `description-authors-at-r` has landed, so the resolver it
  needs (`src/linter/rules/packaging/authors.rs`) is there and the file is no
  longer silent—only this rule's own reading of it is. Write the failing test
  before the fix.

- [ ] `description-title-format`. The parts R and CRAN actually enforce: no
  continuation lines (R-exts says Title *cannot* have any, and
  `Field::value_lines` makes that a one-liner), no trailing period with R's
  own `et al.`/`...` carve-out, Title equal to or redundantly containing the
  package name, and the `usethis` placeholder `What the package does...`.

- [ ] `description-text-format`, on the `Description` field: must end in
  `[.!?]`, must start with a capital, must not start with the package name or
  `The`/`This`/`A`/`In this`/`In the` `package`, and bare `https?://` and
  `doi:` must be angle-bracketed. The angle brackets are the one **safe fix**
  in the tier.

- [ ] `description-date-format`. `Date`, if present, must be ISO 8601
  `yyyy-mm-dd`. Deliberately **not** porting CRAN's "over a month old" and
  "in the future" clauses: a lint whose result changes overnight with no edit
  is a bad lint.

- [ ] Extend `description-version-constraint` rather than adding a rule: it
  catches a missing or non-comparison operator but not an invalid version
  *string* (`dplyr (>= latest)`). Note R's special case allowing `r12345` svn
  revisions, but only for `Depends: R`.

Tier 2—needs a small bundled table or the project layer:

- [ ] `description-license`. Validate the spec grammar (`|` alternatives,
  `+ file LICENSE`, version restrictions, `file LICENCE`, `Unlimited`)
  against R's license db, which is ~50 stable entries and bundles like the
  CRAN symbol lists already do. The payoff:
  `tools:::.standardizable_license_specs_db` is an exact
  `"GPL 2.0" -> "GPL-2"` mapping table, making this the only rule in the set
  with a genuinely **safe autofix**.

- [x] `description-encoding` (packaging; syn; safe fix, default on). Text outside
  R's byte-level ISO-8859 set with no `Encoding` field (`missing_encoding`), and
  non-ASCII in the fields R requires be ASCII
  (`Package`, `Version`, `License`, `Encoding`). arity already reads the file
  as UTF-8, so "is this valid UTF-8" is decided, which makes
  `Encoding: UTF-8` a **safe fix**. Done; ASCII-only field findings are
  report-only because their replacement text requires the author.

- [x] `description-authors-at-r` (packaging; syn; no fix, default on). Done.
  The value is parsed with arity's own R parser and resolved statically, the
  trick `src/project/description.rs` plays on `Roxygen`: no `cre` with a name
  and an `email` (what R's derivation needs, and what makes this the
  prerequisite for the `description-missing-field` gap above), more than one
  `cre`, a person with no name or no role, a role outside the relator table,
  ORCID and ROR validation, a field that is not R, and a call
  `.read_authors_at_R_field(strict = TRUE)` refuses. The two CRAN-incoming
  `Author` clauses fold in as promised. Anything computed resolves to
  "unknown" and every finding depending on it is withheld, so the rule reports
  strictly less than R does. Three things a rule written from the checker
  alone would get wrong, each pinned by a planted oracle case: **`person()`
  with no arguments is a zero-length person vector**, not a nameless
  somebody — it really does end `xfun`'s `Authors@R`, and arity reported it
  until the corpus sweep caught it; **the per-person clauses are unreachable
  when the field yields no author at all**, so a lone nameless person is
  `has_no_author` and nothing else; and **no check component ever mentions an
  unknown role**, because `person()` drops it first, so the oracle driver had
  to grow a signal off the warning (and to call the reader itself, since the
  checker wraps it in `suppressWarnings`). `has_no_author_roles` is
  deliberately left to R: it fires on any package whose sole author writes
  `cre` without `aut`.
- [x] `description-empty-person` (packaging; syn; no fix, default on). Done,
  and it came out of the corpus sweep for the rule above: `person()` with no
  arguments is a **zero-length** person vector, so R concatenates it away
  without a word and `R CMD check` never mentions it. A leftover contributor
  someone opened and never filled in—style rather than correctness, which is
  the whole reason it is a rule of its own: `description-authors-at-r` is gated
  against R in `tests/description_oracle.rs`, and folding an opinion R does not
  share into it would make that gate's claim conditional. Separate ids keep the
  claim exact and let this one be suppressed on its own.

- [ ] Emails appear in exactly three places in a `DESCRIPTION`—`Maintainer`,
  `Authors@R`'s `email =`, and the rare `Contact` field—so there is **no**
  general "lint emails" rule to write. Validate them where they occur, in
  the two rules above, against R's regexp. A shared helper is the whole
  abstraction that is warranted.
- [ ] `description-collate-mismatch`. `Collate` must name every `.R` file and
  only files that exist. `DescriptionFacts::collate` is already computed and
  the file set comes from the project layer. A file missing from `Collate` is
  real breakage, not style. A fix means editing a list, so it belongs to
  stage 5.
- [x] `description-unknown-field` (packaging; syn; no fix, default on). Done as
  a *near-miss* check, not a whitelist: it reports a unique edit-distance-1
  match against the formatter-owned standard field list, plus whitespace
  between a standard name and its colon. `Config/*` and unrelated custom fields
  remain legal; renaming metadata is left to the author.

Tier 3—argued for default-off, or against:

- [ ] `description-authors-at-r-required`, **default off**. R-exts: "For CRAN,
  providing 'Authors@R' is required", and CRAN incoming emits
  `authors_at_R_missing` whenever the field is absent—*even with* a valid
  `Author` plus `Maintainer`, which is precisely the case
  `description-missing-field` treats as complete. Default off because it is
  CRAN policy rather than an R requirement, so it would fire on every
  non-CRAN package; a package targeting CRAN opts in. Keep it a separate ID
  from `description-missing-field` for that reason—one is "R rejects this",
  the other is "CRAN rejects this", and they want different defaults and
  different suppressions.

- [ ] `description-title-case`, **default off**. Porting `tools::toTitleCase`
  faithfully means porting its stopword list and the perl-regex carve-out
  exempting quoted software names. CRAN's own version is the noisiest NOTE in
  the pretest and maintainers routinely override it, so ship it only behind
  the oracle above—pinned against R's `toTitleCase` over a corpus of real
  titles—and default off, like `unused-dependency`.

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
  `documentOnTypeFormattingProvider` (`src/lsp/server.rs`). Wire it over the
  existing `format_range` path and pin the trigger characters plus CRLF edits in
  protocol tests. (2026-07-02 languageserver survey.)

- [ ] **Inlay hints for R** (`textDocument/inlayHint`). E.g. argument-name hints
  at call sites (matching positional args to index formals). Speculative. Not
  loved by all users, possibly opt-in or omit altogether. The provider itself now
  exists, serving a `DESCRIPTION`'s dependency versions; the R arm attaches in
  `inlay_hints_via_db`, and `on_inlay_hint`'s early decline for non-DESCRIPTION
  documents is what goes away. If this lands, the knob it wants is a granularity
  one (off / description-versions / all), not a bare boolean.

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
    remove the last copy. (Done — see the entry below.)

- [x] **`Arc<str>` document text end to end.** `TextBuffer`, `SourceFile`,
  `DescriptionFile`, and `PrevParse` share one allocation, so the write phase,
  the reparse base, and the staleness gates move a handle rather than a
  document, and the guards get an `Arc::ptr_eq` fast path in front of the
  content compare (`incremental::text_is`). Deref coercion absorbed nearly all
  of it: the only breaks were one `as_str()` and the two `upsert_*` guards.

  Measured with the new `benches/salsa_keystroke.rs` (130 KB / 1 MB):

  | row | before | after |
  | --- | --- | --- |
  | no-op upsert | 4.1 us / 32 us | **0.74 us / 0.75 us** |
  | write phase | 3.0 us / 20 us | 4.5 us / 35 us |
  | whole keystroke | 24 us / 182 us | 24 us / 181 us |

  The no-op upsert is now **flat in document size** — that row is a pointer
  test, not a `memcmp`, and it is what every re-lint of an unedited buffer
  pays (a `RelintAll` fan-out, a `didSave`, each sibling file). The write
  phase pays for the rebuild an `Arc<str>` forces in place of an in-place
  splice, but the whole keystroke did not move: the freed `PrevParse` clone
  covers it almost exactly. `line_index`'s rows are unaffected — that bench
  times index work only, never the text splice.

  Deferred halving option, should the write-phase row ever stop being dwarfed
  by the reparse: `Arc<String>` + `Arc::make_mut` costs one copy per splice
  rather than two (`Arc<str>` cannot adopt a `String`'s allocation).

- [x] **Verify a staged chain instead of rebuilding it.** This entry began as
  "bypass `diff_edit` when the staged chain is one verified edit," on fatou's
  premise that the reparse path declines a chain below two edits. **That was
  never true here.** `reparse_edits_with_options` split the chain with
  `split_first`, which accepts a one-element slice, and `parsed_document`
  reaches `diff_edit` through a lazy `or_else` — so a single keystroke already
  took the precise path and already skipped the diff. There was no `diff_edit`
  to bypass.

  The cost was one layer down and is now gone. The chain applied every edit to
  get a whole new `String`, then compared that whole `String` against the
  buffer, to answer a question that is three `memcmp`s wide:

  ```rust
  let mut text = first.apply(old_text);   // allocate + copy the document
  ...
  if text != target { return None; }      // compare the document
  ```

  Splitting the *last* edit off instead (`split_last`) makes the final text the
  one thing never built: `Edit::produces` decides `apply(old) == target` by
  comparing the three spans in place. A one-edit chain now allocates nothing.
  The guard also moved *ahead* of the reparse it gates, so a stale sequence no
  longer pays a splice before being thrown away.

  Measured with `benches/salsa_keystroke.rs` (130 KB / 1 MB, `--baseline`):

  | row | before | after |
  | --- | --- | --- |
  | whole keystroke | 23.7 us / 182 us | **21.3 us / 165 us** |
  | write phase (control) | 4.63 us / 163 us | 4.47 us / 162 us |
  | no-op upsert (control) | 0.74 us / 0.74 us | 0.72 us / 0.73 us |

  **-8.7% and -8.6%.** The two control rows never reach the chain and moved
  under 3%, which is this machine's run-to-run drift. What came off is 2.3 us
  at 130 KB against 17 us at 1 MB — linear in the document, which is what a
  whole-document allocate-copy-compare looks like and what nothing else in the
  row does.

  Verifying rather than applying also made the reparse surface **total**. An
  `Edit` is caller data: a range the text cannot take used to panic in rowan's
  `covering_element` assert or in a `replace_range`, and the lint thread owns
  the only database, so that took the server's analysis down rather than
  falling back. `produces` answers `false` for an out-of-bounds, inverted, or
  mid-character range, and `reparse_with_options` now declines one up front.
  Three tests in `incremental_reparse.rs` and one in `salsa_incremental.rs`
  pin it; all four panicked before.

  `edits_produce` is the same thing for a slice, and replaced the two
  `apply_edits(old, edits) == current` span-mapping guards (`resolve_ptr`,
  `rename_cursor_offset`), which had the same rebuild and the same panic.

  Not done, deliberately: skipping the `diff_edit` fallback when a verified
  single staged edit is declined by the ladder. `diff_edit` can return a
  *tighter* edit than the staged one — typing one character over a selection —
  so it sometimes reaches a cheaper tier. Dropping it would trade a correct
  optimization for a coarser tree.

- [x] **A didChange -> upsert -> parse pipeline bench.** `benches/salsa_keystroke.rs`
  replays alternating insert/delete keystrokes through the real path
  (`apply_edit` on the live buffer, `upsert_file`, stage, demand the tree),
  with three rows per size — no-op upsert (the staleness guard alone), write
  phase without a parse, and end-to-end. The existing `pipeline/` group times
  patch-plus-reparse but not the salsa write phase, and fatou's version of
  exactly this blind spot is what hid a 6x end-to-end regression in an
  otherwise well-benchmarked PR. Run it with
  `cargo bench --bench salsa_keystroke`; there is no `task` target.

- [ ] **Mechanize the keystroke bench's gate.** Both entries above are read off
  an eyeballed table, which is how a control row drifting 3% and a real row
  moving 9% end up needing a paragraph to tell apart. Consider panache's
  approach: declared per-case expectations plus a corpus-presence check.

#### The jarl gap (measured 2026-08-17)

**The gap is now scaling, not work.** On tidyr arity is **1.56x faster than
jarl single-threaded** (83.8 ms against 130.6) and **1.11x slower on 24
threads** (28.3 against 25.4); on MASS the 24-thread ratio is 1.01x. arity is
not doing more work — it converts less of it into wall time, getting 3.0x out
of 24 threads against jarl's 5.1x.

Five rounds of fixes got here; the archaeology is in `git log` (search
`perf(linter)` and `perf(project)`) and the traps that outlived them are in
doc comments next to the code they guard — `LayeredSet`'s asymmetric `removed`,
`PathSetView`'s disjointness invariant, and `ProjectScope::build`'s
`Some(r) == Some(r)` root test, each naming the test that fails when it is
violated. What remains open is below, largest first.

Re-measure with `hyperfine -i` (lint exits non-zero on findings) and `taskset`
for the single-threaded row; `--no-cache` is a `format` flag and does nothing
here. Interleave old and new round by round — the wall-time noise floor on a
28 ms run is 1-2 ms, wide enough to swallow a real 0.7 ms win, so a change that
small needs a phase timer rather than a stopwatch.

**Measured and rejected — do not redo.** `SmolStr` through the project layer
(`file_exports`/`file_free_reads`/`file_qualified_reads`, `FileFacts`,
`LayeredSet`, `build`'s internals; `FileScope`'s accessors need no change
because `SmolStr: Borrow<str>`): built in full, moved `project_graph`
1.335 -> 1.290 ms and wall time not at all, because sharing the per-file facts
as `Arc` handles had already removed the copying it targeted. Likewise `reads`
as `Vec<BTreeSet<&str>>` — deltas of -25, -9, -47 us over three interleaved
rounds, distributions overlapping.

- [ ] **The parallel passes get ~7x out of 24 threads, and that is now the whole
  gap.** Unattributed. Measure before touching anything: the last two rounds
  both found the serial region somewhere other than where the standing entry
  said it was.

  Both blocks scale about the same, and both are far off linear (in-process
  harness, tidyr, median of 60 runs; note this path passes an **empty** package
  index, so pass 2 is ~5 ms lighter here than in the real CLI):

  | phase | 1 thread | 24 threads | speedup |
  | ------------------ | -------- | -------- | ------- |
  | warm-up | 27.09 ms | 3.97 ms | 6.8x |
  | pass 2 | 31.28 ms | 4.30 ms | 7.3x |
  | whole run | 64.27 ms | 14.31 ms | 4.5x |

  In the real CLI those two are 5.94 ms and 9.61 ms of a 22.73 ms run, so **15.5
  of 22.7 ms sits in blocks running at ~29% efficiency**. Lifting them to 12x is
  worth more than the entire 6.49 ms serial region — of which the two entries
  after this one are 4.4 ms, and nothing else exceeds 1.35 ms.

  **The 1t column above is from the in-process harness, not the real CLI** — no
  one has run a real-CLI phase split at one thread, so the per-phase 1t figures
  with a live index are unknown. Get those first; the ranking below could change.

  Three candidates, none measured:

  - **Tail latency in pass 2.** The earlier dismissal ("rayon's indexed splitter
    already reaches single-item leaves at 86 items") answered the wrong
    question: fine splitting says nothing about *which* item runs last. tidyr has
    one 23 KB file among 86, and if it starts late it is the critical path
    however finely the range was split. Time each file in pass 2 and look at the
    distribution before assuming this is balanced.
  - **Salsa memo contention.** Every worker holds its own db clone over shared
    memo storage. `record_query`'s mutex was ruled out before, but only as
    O(files) rather than O(nodes) — that argument is about *volume*, not about
    24 threads contending on the same lock.
  - **Allocator or memory bandwidth.** mimalloc already won here once (~38% of a
    format under glibc against ~10%), but that was a single-file profile. Two
    dozen threads building green trees concurrently is a different regime.

  Whichever it is, `--mode lint-dir` with per-file timings decides between the
  first and the other two in one measurement.

  **Two ways to misread the profile, both already fallen for once.**
  `wait_until_cold` at 92.3% inclusive is *not* idling — rayon executes stolen
  jobs inside it, so every worker's real work hangs under that frame. `salsa::`
  at 54.8% inclusive is not overhead — it contains every tracked function body.
  User time growing while wall time does not is what rayon's spin-before-sleep
  looks like when workers park on a serial region, so it points at the serial
  region rather than at contention.

- [ ] **Four sequential disk passes, 3.15 ms, one walk's worth of information.**
  `collect_source_files` walks the tree; then `excluded_package_sources` and
  `discover_packages` each re-list every `R/` and re-read every `DESCRIPTION`;
  then pass 1 reads every file. The last round removed the *duplicated work
  inside* those passes (one root walk per directory instead of per file, one
  `DESCRIPTION` read instead of two) but left the pass structure alone. Feeding
  all four from a single walk is the structural fix, and the largest serial item
  left.

- [x] **Render re-read from disk what pass 1 already had.** Each report with
  diagnostics now retains the analyzed `Arc<str>` (clean reports retain no
  text), and the CLI renders through the shared buffer after the salsa database
  teardown is scheduled. On tidyr this removed one source-file open for each of
  the 18 files with findings (87 -> 69 opens under `R/`). Pretty lint output on
  one pinned core moved from 42.51 to 42.28 ms median (-0.55%, 20 interleaved
  runs); minima moved from 41.82 to 41.57 ms (-0.58%).

- [x] **`arity lint --fix` cost 61 ms on a four-line file.** Attributed, and the
  waste in it removed: the per-document project seed (`seed_workspace_for`) ran
  once per *file* and once per *fixpoint iteration*, so `--fix` over a directory
  re-walked the tree and re-read every sibling for every file it touched —
  quadratic in the package's file count. `ensure_workspace_for` skips the seed
  when the active file is already a workspace member, which is the guard the LSP
  already applied at its own call site and now shares.

  Median `arity lint --no-config --fix` on eulerr's `R/` (23 files, 292 KB, 12
  interleaved rounds, pinned to one core): 581.3 -> 100.3 ms (-82.7%, min
  -82.7%). Fixed trees and stdout byte-identical against the old binary under
  `--fix --unsafe-fixes` on eulerr, SLOPE, caugi, tactile, and qualpalr.

  **The headline framing was wrong: it was never a fixed cost.** On a lone
  four-line file with no siblings `--fix` adds 0.3 ms (3.1 -> 3.4 ms). The same
  file inside eulerr's `R/` costs 25.4 ms without `--fix` and 71.3 ms with it,
  and the 46 ms difference is the enclosing package's scope — reading, parsing,
  and modeling 23 siblings, serially. That is the price of the project scope
  `--fix` is required to have (`85d6d1e`), not waste, and it does not move with
  this change. `BUNDLED_EXPORTS` and the page-fault frame were both red
  herrings.

  Left open, and the reason the single-file figure is what it is: the reporting
  pass reads its files with `par_iter` while the seed reads its siblings in a
  serial loop, and everything the seed pulls in is then modeled on one thread.
  Worth attacking only together with the "four sequential disk passes" entry
  above — they are the same walk.

- [ ] **The synthetic tiers are still mildly superlinear in the tail.** Per-65 KB
  unit cost dips to ~16 ms mid-range, then climbs to ~23 ms at 1.6 MB. Confirm
  it on a non-degenerate input before chasing it: `scripts/bench.sh` builds the
  tiers as 24 identical copies of one block, which is its own plausible trigger
  (`duplicated-function-definition` and friends see 24 of every name). Left over
  from the `misplaced-suppression` fix, which removed the quadratic that
  dominated these tiers.

- [ ] **A names-only sidecar for the rindex membership view.** Making the lint
  CLI's index load lazy cut the *number* of `{pkg}@{ver}.json` files parsed, not
  the cost of parsing one, so a file that does `library(ggplot2)` still walks
  2 MB of JSON for ~30 KB of names — the rest is help bodies and formal defaults
  that `skip_to_escape`/`ignore_str` scan character by character only to discard.
  A names-only sidecar, or a non-JSON format, answers the membership questions
  (`resolve_origin`, `package_indexed`, `attach_members`) without them. Worth
  doing only once the attaching case measures as a real cost.

## DESCRIPTION and package metadata

- [ ] **Complete DESCRIPTION field names in the LSP.** Offer canonical field
  names (`Package`, `Version`, and so on) while editing a field header.
  Project the candidates and their order from
  `crates/arity-formatter/src/formatter/description/order.rs`; do not maintain
  a second ordered list in the LSP.

- [x] **Diagnose whitespace before a DESCRIPTION field colon.** Done through
  `description-unknown-field`: the CST remains lossless, the diagnostic spans
  `Package `, and arity's semantic lookup remains deliberately lenient even
  though R treats that field name as `"Package "`.

## Misc
