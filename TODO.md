# TODOs

## Parser

## Formatter

- [ ] Reconsider comment attachment when bracing a bare `if` consequence. For
  `if (condition) # rationale` followed by a bare consequence, the formatter
  currently follows air and moves the comment inside the synthetic block,
  immediately before the consequence. That preserves executable syntax plus
  comment text and order, but it can change the apparent referent from the
  condition to the consequence—`sf`'s `# +- 4%` is a concrete example.
  Decide whether to keep air's behavior, retain the comment on the `if`
  header, or hoist it above the `if`; then pin the intended attachment and
  consider whether verification needs a stronger comment invariant.
  Follow-up to [issue #125](https://github.com/jolars/arity/issues/125).

- [ ] Honor `# arity-format skip` and `off`/`on` in a `DESCRIPTION`. Only
  `skip-file` works there today; the other verbs need the field-class
  planner (`formatter/description/plan.rs`) to learn to hand a field's lines
  back verbatim. `misplaced-suppression` does not yet report the ones that
  are inert there.

## Linter

- [ ] Report calls to deprecated or defunct functions defined in project
  source. Start with unconditional, base-resolved `.Deprecated()` and
  `.Defunct()` calls at function entry, and extract literal replacement names
  when available. Carry the metadata through single-file semantics and
  range-free project projections so diagnostics follow resolved function
  identities, including across files. Respect shadowing of both the marker
  and the deprecated function; do not infer whole-function deprecation from
  conditional, quoted, or nested calls. Add positive and negative fixtures,
  including cross-file resolution and incremental updates. Report at the use
  site with a replacement suggestion when known; no general autofix.

- [ ] Extend deprecation detection to installed dependencies during package
  indexing. Inspect closure bodies statically for `.Deprecated()` and
  `.Defunct()`, preserving the expression structure associated with compiled
  bodies that `rindex::rds` currently discards. Store status and optional
  replacement metadata with the indexed package version, and feed it into the
  same diagnostics for resolved bare and namespace-qualified calls. Never
  execute R; missing or unreadable metadata stays unknown. Pin compiled and
  uncompiled package fixtures. Once the base R markers work, extend detection
  to explicit `lifecycle::deprecate_*()` calls, with documentation annotations
  as supplementary coverage. Keep argument-specific deprecations separate
  from whole-function status, and superseded APIs opt-in.

- [ ] Remove the `# arity-ignore`/`# arity-ignore-file` aliases once
  `deprecated-suppression` has been out for a release or two. Parser side is one
  branch each in `crates/arity-parser/src/directive.rs`, plus `Spelling` and the
  rule itself.

### Rule-candidate audit: RStudio (2026-10-11)

Audited [RStudio](https://github.com/rstudio/rstudio) at
`ff9f819a5459f399508a550619307503b8afc40c`, focusing on its diagnostics
implementation rather than linting its own R source. Sources: [call validation
and special
checks](https://github.com/rstudio/rstudio/blob/ff9f819a5459f399508a550619307503b8afc40c/src/cpp/session/modules/SessionRParser.cpp),
[diagnostic
tests](https://github.com/rstudio/rstudio/blob/ff9f819a5459f399508a550619307503b8afc40c/src/cpp/session/modules/SessionDiagnosticsTests.cpp),
[Unicode
checks](https://github.com/rstudio/rstudio/blob/ff9f819a5459f399508a550619307503b8afc40c/src/gwt/src/org/rstudio/studio/client/workbench/views/output/lint/ConfusableCharacterLinter.java),
and the [user
guide](https://github.com/rstudio/rstudio/blob/ff9f819a5459f399508a550619307503b8afc40c/docs/user/rstudio/ide/guide/code/diagnostics.qmd).
Reduced examples below were checked with R 4.6.1. Prioritize the first two;
these are candidates, not implemented rules. Follow `add-lint-rule` for new
rules, and keep all production analysis static.

- [ ] Validate calls against statically resolved closure signatures. Start with
  local and project functions, then indexed package formals when available.
  Report unmatched names, excess positional arguments, ambiguous partial
  matches, and multiple actual arguments bound to one formal. R rejects
  `f <- function(x) x; f(y = 1)` and `f <- function() 1; f(2)`;
  `f <- function(alpha, alpine) 1; f(al = 1)` is ambiguous, and
  `f <- function(alpha) alpha; f(a = 1, al = 2)` matches `alpha` twice.
  Model exact, partial, and positional matching in that order, respecting
  the `...` boundary and exact-only formals after it. Reuse signature
  resolution where practical, but extend `semantic::match_args_to_formals`
  with explicit mismatch outcomes before using it as a diagnostic authority:
  its current use for model-frame masking does not distinguish these errors.
  Keep project signature facts range-free. Unknown callees, dynamic
  rebinding, dispatch, forwarded `...`, and pipe insertion/placeholders
  require conservative handling. No general autofix.

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
- [x] `boolean-arithmetic`: recognize `length(which(p)) == 0` and
  `sum(logical) == 0`-style existence tests and prefer `!any(p)` (plus the
  positive variants). Start with shapes whose NA behavior is provably
  preserved; lintr's broad family needs an oracle matrix before porting.
  Implemented as a default-on `ns`-tier warning for exact base
  `length(which(p))` calls and base `sum()` calls over a syntactically logical
  argument with `na.rm = TRUE`. Empty/non-empty comparisons against zero or one
  and mirrored spellings are covered; fixes preserve `NA` behavior but are
  unsafe because `Summary` method dispatch can differ, and are withheld around
  comments, a shadowed introduced `any`, or a tight negation context.
- [x] `list-comparison`: flag direct comparisons of known list-producing calls
  such as `lapply(...) > 1`, which rely on awkward coercion; recommend a typed
  iterator rather than guessing a fix.
  Implemented as a default-on `ns`-tier suspicious warning for direct base
  `list`, `pairlist`, `lapply`, and `Map` comparisons, including parenthesized
  operands. No fix is offered because the intended element type is unknown.
- [x] `terminal-close`: flag a function whose final action is `close(conn)` and
  recommend registering cleanup with `on.exit()` near acquisition. Default-off
  until a corpus pass establishes that ownership-transfer patterns are not
  noisy.
  Implemented as an opt-in `ns`-tier warning for direct base-resolved function
  tails, including unbraced bodies and transparent blocks or parentheses. No fix
  is offered because ownership, cleanup timing, and return values can differ.
- [x] `routine-registration`: flag string-named `.Call`/`.C`/`.Fortran`/
  `.External` calls in packages and recommend registered native symbols. Reuse
  the existing NAMESPACE/native-registration project facts; do not invoke R or
  inspect the local installation.
  Implemented as a default-on packaging warning for base-resolved calls in
  package `R/` sources, including qualified calls and named `.NAME` arguments.
  Registered symbol arguments reuse project visibility; no fix is offered.
- [ ] `package-hooks`: validate `.onLoad`, `.onAttach`, `.Last.lib`,
  `.onDetach`, and `.onUnload` signatures and statically forbidden/noisy calls
  per Writing R Extensions. Package-only, cross-checked against `R CMD check`'s
  implementation rather than copied blindly from lintr.
- [x] `strings-as-factors`: when the R compatibility floor crosses 4.0, flag a
  `data.frame()` with known character columns and no explicit
  `stringsAsFactors`. No fix can choose the intended behavior.
  Implemented as a default-on `ns`-tier warning for declared floors below 4.0,
  with literal strings and base `c()` vectors of atomic literals. Unknown
  column types remain silent; explicit settings are preserved.
- [ ] Extend the existing Phase 5 package-aware plan with jarl/lintr's remaining
  testthat rewrites: `expect-not`, `expect-s4-class`, `expect-comparison`,
  `expect-identical`, `expect-shape`, and yoda argument order. Keep the family
  default-off initially: these improve test intent/messages rather than program
  correctness. The already-listed shared matcher should own all of them.

Tier 3—useful but policy-heavy; consider only default-off after corpus evidence:

- [x] `cyclomatic-complexity`, parameterized by a threshold. This is a
  maintainability metric, not layout; define the count over arity's CFG instead
  of depending on R's `cyclocomp` package.
  Implemented as an opt-in readability warning with a configurable maximum of
  20, native CFG decision counting, and no autofix.
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

### Rule-candidate audit: `DavisVaughan/extrachecks` (2026-09-01)

Audited [extrachecks](https://github.com/DavisVaughan/extrachecks) at
`a37e1ee87e47e99ba0ed88a4dfe417141ca5d89e`. The repository is a checklist,
not an R package, so there was no source corpus to lint. Retained only five
objective candidates from the audit. The three `DESCRIPTION` candidates refine the
existing `description-title-format` and `description-text-format` entries under
**More DESCRIPTION rules** rather than creating parallel TODOs.

- [ ] `roxygen-internal-example-call` (documentation; project; no fix): flag an
  unqualified call from `@examples` to a documented, unexported function in the
  same package. Confirm exports against the actual NAMESPACE, skip `@noRd`, and
  exclude bindings introduced inside the extracted example before reporting.
  A minimal package was verified against R 4.6.1: an unqualified `internal()`
  makes `R CMD check` fail with "could not find function", while
  `pkg:::internal()` passes. Keep this report-only initially because adding
  `pkg:::` and suppressing the topic with `@noRd` are both legitimate repairs.

- [ ] `installed-packages` (performance; ns; no fix): report
  `utils::installed.packages()` only in statically recognizable
  package-availability tests, where R's own help recommends `find.package()`,
  `system.file()`, or `requireNamespace()` instead. Namespace-confirm the call
  and scope the rule to package projects; a package manager that intentionally
  enumerates every installed package is not a violation. There is no general
  fix because the recommended functions answer different questions.

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
- [x] **Resolve manual roxygen aliases and usage across blocks.** A block with
  `@usage g(x)`, `@param x`, `@return`, and `@aliases g` can document a later
  export-only `g <- function(x) x`; roxygen2 generates the shared topic, while
  arity reports missing title, parameters, and return on `g`. Future's
  `%packages%`, `mandelbrot_tiles`, `as.raster.Mandelbrot`, `plot.Mandelbrot`,
  and `as.FutureGlobals` expose this. Extend the range-free topic projection;
  do not bolt aliases onto individual rules.
- [x] **Do not let one inherited topic member suppress owner coverage.** The
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

### Package-aware rules

The package-specific rules below are gated on the package being attached
(`model.loaded_packages()`).

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

## Object-oriented programming

Coverage audit at `12b8191` (2026-11-10), prompted by [issue
#183](https://github.com/jolars/arity/issues/183). Support S3, S4, Reference
Classes, R6, and S7 through shared semantic and project facts, with separate
rules for each system's environments, inheritance, and dispatch. Keep production
analysis static; unknown runtime behavior must remain unknown.

### Audited coverage

These notes describe the original audit. Completed slices are recorded in the
implementation checklist below.

Reduced examples for all five systems run in R 4.6.1, parse losslessly, and pass
formatter verification. R6 was checked at 2.6.1 and S7 at the released 0.2.2.
These examples establish a baseline, not comprehensive parser or formatter
coverage. The existing class projection, type hierarchy, field completion, and
class incrementality checks pass (33 focused tests).

  | System            | What works today                                                                                                                                                                                                                      | Main gaps                                                                                                                                                                                                                              |
  | ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
  | S3                | Ordinary function definitions; two- and three-argument NAMESPACE `S3method()` entries; conservative dotted-method exemptions in unused checks; implicit reads for `NextMethod()`; exemptions for `.Generic`, `.Method`, and `.Class`. | No structured generic-to-method registry, class evidence, ordered dispatch, or generic-to-method navigation. `.GenericCallEnv` and `.GenericDefEnv` produce false positives; the three existing exemptions apply even outside methods. |
  | S4                | Literal `setClass()` names and `contains` edges feed type hierarchy; NAMESPACE class/method directives contribute flattened names; package dependency checks recognize use of `methods`.                                              | No slots, generic/method signatures, generated generic bindings, multiple dispatch, or coercion model. Valid `.Methods`, `.defined`, `.target`, and calls to a generic created by `setGeneric()` produce false positives.              |
  | Reference Classes | Literal `setRefClass()` names and `contains` edges feed type hierarchy; normal R syntax and binding analysis work around the declaration.                                                                                             | No field/method environments, member schemas, generator APIs, or superclass calls. Valid bare field reads and `.self` produce false positives.                                                                                         |
  | R6                | Literal `R6Class()` names and simple `inherit = Parent` edges feed type hierarchy; normal syntax and ordinary bindings work.                                                                                                          | No public/private/active member model, implicit method environment, constructor signatures, or class mutation model. Valid `self`, `private`, and `super` reads produce false positives, including #183.                               |
  | S7                | Released `new_class()`/`new_generic()` assignments, `method(...) <- function(...)`, and `@` access parse and format as ordinary R. The reduced examples lint cleanly.                                                                 | No S7-specific class, property, generic, method, inheritance, or dispatch facts. Clean lint output does not establish that these relationships are understood.                                                                         |

The shared implementation has several limits that the work below must replace:

- [`project/classes.rs`](src/project/classes.rs) records only a system and
  parent names. Bare and namespace-qualified defining calls work, but the match
  uses the final callee spelling without checking its origin or shadowing. A
  user-defined `setClass()` creates a phantom class. Named
  `Class =`/`classname =` arguments are missed, as are unnamed R6 generators.
  `Generator <- R6Class("Display")` is indexed as `Display`, while a child's
  `inherit = Generator` records `Generator` as its parent. Duplicate names
  within a file keep the first declaration, and the project aggregate merges
  inheritance by unqualified name across packages and systems.
- [`semantic/builder.rs`](src/semantic/builder.rs) has no class-owned method
  environments. It globally drops the three recognized implicit method variables
  and treats every `:=` as a nonbinding update or injection operator. Neither
  approach is a general OOP model. Class declarations, generator bindings,
  string-based registrations, and ordinary lexical variables need distinct
  identities and occurrence roles.
- [`lsp/completion.rs`](src/lsp/completion.rs) harvests `$`/`@` usage by
  receiver text across the whole file, plus fields of a few record constructors.
  `self$` therefore mixes fields from unrelated R6 classes and misses members
  declared only in `public`, `private`, or `active`. No declared S4 slots or S7
  properties are inferred. Those field completion items have no signatures or
  documentation. Document/workspace symbols and call hierarchy primarily
  describe assigned functions and values; an R6 generator appears as a variable
  without its methods. Member definition and signature probes return no result.
- [`rindex/harvest.rs`](src/rindex/harvest.rs) flattens class/method namespace
  declarations into name sets. The index schema has no OOP relationships;
  environment contents are discarded by the RDS reader, and S4 objects are
  opaque apart from attributes. Export names, closure formals, and help are
  useful foundations, but cannot describe dependency classes and methods.
- Roxygen recognizes relevant tags, including slots, fields, and properties, but
  [`project/roxygen.rs`](src/project/roxygen.rs) and its usage projection
  recover callable definitions mainly from plain function assignments or
  explicit `@usage`. Class-owned methods and generated constructors do not yet
  supply a shared documentation surface.

### Implementation order

Follow the stages below. Stages 1–2 establish the model and address the false
positives behind #183 as part of that model. Build out R6 and Reference Classes
first, then S3, S4, and S7. Deliver editor capabilities for each supported
subset as its facts become available; dependency indexing and full dispatch
coverage need not block useful workspace support.

#### 0. Preserve the coverage baseline

- [x] Add a hand-registered OOP fixture matrix covering definitions, use sites,
  incomplete editor input, shadowing, and unsupported dynamic forms for all
  five systems. Turn the reduced audit examples into permanent regressions
  before changing behavior. Cover CLI lint and the corresponding LSP paths,
  not just declaration extraction.
  Added 30 fixtures in `tests/fixtures/oop/`, registered by
  `tests/lsp_protocol/oop.rs`. The shared CLI and LSP baselines cover live edits
  and label known gaps explicitly; the parser check also covers incomplete
  input losslessly.
- [x] Add a developer-only differential oracle with pinned R, R6, and S7
  versions. Record class and member declarations, method environments,
  constructor formals, and dispatch outcomes as small reviewed fixtures
  usable by pure Rust tests. Keep R execution out of production and ordinary CI.
  Assert parser losslessness, formatter idempotence, and full and
  incremental parse agreement for the same examples; add grammar work only
  for demonstrated syntax gaps.
  Added five runtime fixtures in `tests/fixtures/oop_oracle/`, pinned to R
  4.6.1, R6 2.6.1, and S7 0.2.2. `task oop-oracle` checks original and formatted
  runtime observations; ordinary Rust tests consume the pins and verify the
  supported class projection and parser, formatter, and reparse invariants.

#### 1. Shared declarations and identity

- [x] Establish shared class declaration identities for R6, Reference Classes,
  and S4. Keep declared names separate from generator bindings and direct
  aliases, confirm defining-call origins and R argument matching, preserve
  ambiguous and incomplete facts, and use those identities in type hierarchy.
  Inheritance respects package and source visibility and emits only proven
  edges. This first slice does not yet model members, generics, registrations,
  dispatch, cross-file aliases, or competing definitions selected by load order.

- [x] Extend the current class projection into a shared model of classes,
  generics, methods, members, and the sites that declare or use them.
  Distinguish a declared class name from its constructor or generator
  binding and aliases. Include package or lexical owner, OOP system, generic
  identity, and dispatch signature where relevant. Preserve ambiguous
  declarations instead of merging unrelated classes or choosing the first
  method.
  `FileClassFacts::oop` now carries generic, method, member, and signature
  facts with lexical identities; the existing project index supplies file and
  package ownership. Candidate queries retain duplicate declarations, and
  `oop_sites` recovers declaration, alias, and use spans from the current CST.
  Literal S7 classes also feed type hierarchy. Pinned runtime observations and
  incremental tests cover the shared projection; broader call recognition and
  project resolution remain the next steps below.
- [x] Recognize defining and registration calls through resolved callee identity
  and R argument matching. Cover bare imports, `pkg::` forms, positional and
  named arguments, aliases whose target is known, and backticked operator
  and replacement names. Reject shadowed lookalikes. Represent computed
  names, conditional registration, custom environments, and unresolved
  aliases explicitly as incomplete facts; do not guess from a name alone.
  Class, generic, and method extraction now share callee provenance and
  checked argument matching. The project index confirms bare origins using
  imports and the search path. S7 registrations resolve `method<-`, and
  `setReplaceMethod()` arguments match its wrapper before forwarding.
  Ambiguous aliases retain possible origins, and uncertain declarations do
  not establish definite generic aliases or members.
- [x] Keep per-file semantics independent of the project. Project queries
  combine range-free class, member, and signature facts using package
  visibility, NAMESPACE, source closures, and Collate; recover spans from
  the current CST. Reuse `file_class_defs`, `project_classes`, and existing
  resolution plumbing rather than creating separate registries for lint and
  LSP. Extend `tests/salsa_incremental.rs`: body/prose edits backdate, while
  changes to members, parents, registrations, or imports invalidate the
  appropriate consumers. Keep unrelated packages and source closures
  isolated.

#### 2. Method environments and generated bindings

- [x] Model R6 `self`, `private`, and `super` in their actual method
  environment, covering public and private methods, active bindings,
  defaults, and nested closures. Distinguish methods from ordinary
  function-valued fields and functions merely constructed inside a method.
  Respect local shadowing, known parent and member availability, and
  uncertain inheritance. Make #183 a regression of this semantic model; do
  not exempt these names globally or mask every read inside an R6 call.
  Lint and navigation now share explicit R6 method enclosures. Project queries
  cache binding availability without ranges, and the current CST supplies
  method implementation locations. Regressions cover #183, inherited copies,
  aliases, defaults, nested closures, shadowing, and rename safety across
  cached and live editor reads. Generator `$set()` and bare member lookup for
  `portable = FALSE` remain for later stages.
- [ ] Model Reference Class bare field and method lookup, `.self`,
  `.refClassDef`, and inherited helpers such as `callSuper()` and
  `initFields()`. Distinguish normal methods from external methods with an
  explicit `.self` parameter. Track field updates through `<<-` without
  manufacturing file-level lexical bindings or dead-store findings. Share
  appropriate environment machinery with R6 `portable = FALSE`, while
  preserving their behavioral differences.
- [ ] Replace the global S3 and S4 implicit-variable exemption with
  method-context facts. Cover `.Generic`, `.Method`, `.Class`, `.Group`,
  `.GenericCallEnv`/`.GenericDefEnv`, and S4's
  `.Methods`/`.defined`/`.target` where each dispatch mechanism actually
  supplies them. Preserve conservative behavior when method status is
  uncertain. Account for literal-name `setGeneric()`/`setReplaceMethod()`
  effects and registrations that create a generic without an assignment; do
  not treat every class-name string as a lexical binding. Ensure definition,
  references, rename, symbols, and unused checks handle implicit identities
  without fictitious editable definitions.

#### 3. R6 and Reference Class members

- [ ] Extract R6 public and private fields and methods, active bindings,
  initializer signatures, and generator versus instance APIs. Resolve
  `inherit` through generator identity, including aliases and
  namespace-qualified parents, instead of equating the binding and display
  names. Account for omitted `classname`, `class = FALSE`, `cloneable`,
  overrides, private inheritance, `super$method()`, and method functions
  supplied by a resolvable binding.

- [ ] Extract Reference Class `fields`, `methods`, `contains`, field types,
  accessor functions, initialization, and inherited members. Support
  generator calls, `$new()`, `getRefClass()`, built-in object methods, and
  `callSuper()` relative to the owning method. Keep Reference Class fields
  distinct from S4 slots even though the class participates in S4
  inheritance.

- [ ] Track statically known `Generator$set(...)` and `Generator$methods(...)`
  additions or replacements, and methods installed from another file.
  Respect load order and known class/object locking. Record dynamic
  mutation, unknown member lists, and custom parent environments as
  incomplete. Model active/accessor members as executable reads and writes
  rather than plain stored fields; never execute their functions to obtain
  completion or type information.

  Semantic references: the [Reference Class
  manual](https://stat.ethz.ch/R-manual/R-devel/library/methods/html/refClass.html)
  and [R6 reference](https://r6.r-lib.org/reference/R6Class.html).

#### 4. S3 classes, registration, and dispatch

- [ ] Recover conservative class evidence from literal
  `structure(..., class = ...)`, `class(x) <- ...`, `oldClass`, and
  recognizable constructor returns. Preserve class-vector order and
  distinguish explicit evidence from an unknown runtime class. S3 has no
  required class declaration: dotted method names alone must not manufacture
  a formal class hierarchy.

- [ ] Index explicit `UseMethod()` generics, known internal/group generics,
  `generic.class` methods, and NAMESPACE two-/three-argument and delayed
  `S3method()` registrations. Include statically recoverable `.S3method()`
  and `registerS3method()` calls. Keep the generic, class, implementation
  binding, registration owner, and ordinary export status separate. Retain
  conservative unused-method protection for possible methods whose generic
  cannot be resolved; absence from the registry is not proof of dead code.

- [ ] Resolve method candidates using class-vector order, `.default`, and
  registration visibility. Cover `NextMethod()`, internal generics, group
  generics, and operator/replacement methods as separate tested cases rather
  than one dotted-name split. Model `UseMethod()`'s nonreturning control
  flow and argument forwarding, including the R 4.4 change to generic-local
  variables. Reuse the existing `NextMethod()` formal-read handling and
  signature safety gates. Unknown classes or runtime registration yield
  candidates or unknown results, never a confidently selected
  implementation.

  Semantic references: [S3
  dispatch](https://stat.ethz.ch/R-manual/R-devel/library/base/html/UseMethod.html),
  [group
  generics](https://stat.ethz.ch/R-manual/R-devel/library/base/html/groupGeneric.html),
  and [script
  registration](https://stat.ethz.ch/R-manual/R-devel/library/base/html/S3method.html).

#### 5. S4 classes, slots, and multiple dispatch

- [ ] Extend `setClass()` extraction to slots and legacy `representation`,
  prototypes, virtual/sealed classes, and multiple inheritance. Add
  `setClassUnion()`, `setIs()` relationships and coercions, and
  `setOldClass()` bridges to S3. Distinguish constructor bindings returned
  by `setClass()` from class metadata installed by the call. Recover
  declared slot schemas and their inheritance for `new()`, class-generator
  calls, `@`, `slot()`, and replacement forms. Keep validity and coercion
  bodies unevaluated.

- [ ] Index `setGeneric()`, `setMethod()`, `setReplaceMethod()`, generic
  defaults, `signature(...)`, named and positional signatures, and the
  generic's selected dispatch arguments. Preserve multiple arguments, `ANY`,
  `missing`, class unions, group generics, and method ownership across
  packages. Model `standardGeneric()`, `callNextMethod()`, and
  `selectMethod()` with the same generic and method identities; do not
  borrow S3's single-class selection rule.

- [ ] Resolve inherited method candidates with explicit ambiguity and
  incompleteness. Preserve the separate meanings of `exportClasses`,
  `exportMethods`, `importClassesFrom`, and `importMethodsFrom`, rather than
  flattening their class and method relationships into lexical name sets.
  Cover methods for primitives and generics owned by another package.
  Integrate Reference Class inheritance and S3 bridges without claiming
  every S4 slot is a Reference Class field.

  Semantic references: [class
  definitions](https://stat.ethz.ch/R-manual/R-devel/library/methods/html/setClass.html),
  [generics](https://stat.ethz.ch/R-manual/R-devel/library/methods/html/setGeneric.html),
  and
  [methods](https://stat.ethz.ch/R-manual/R-devel/library/methods/html/setMethod.html).

#### 6. S7 classes, properties, and dispatch

- [ ] Extract `new_class()` parents, package identity, stored/dynamic
  properties, `new_property()` types, defaults, getters, and setters, custom
  constructors, validators, and `new_object()` context. Derive default
  constructor signatures from properties and inheritance. Support `@`,
  `prop()`, `props()`, and replacement forms without assuming that an S7
  property is an S4 slot or evaluating validation and accessor code.

- [ ] Index `new_generic()` dispatch arguments and
  `method(generic, signature) <- definition`, including definition bindings,
  multiple dispatch, unions, base classes, `class_any`, `class_missing`,
  `S7_dispatch()`, and `super()`. Treat replacement registration as an
  effect on the generic, not as an ordinary function definition called
  `method`. Account for external classes and generics and package
  registration through `methods_register()` and the package hooks supported
  by the targeted S7 version.

- [ ] Version the S7 oracle and metadata carefully. Released 0.2.2 and the
  current 0.2.2.9000 development documentation differ: the latter introduces
  binding `:=` and expanded S4 interoperability. Before supporting that
  operator, replace the builder's unconditional nonbinding interpretation
  with resolved S7 operator semantics, preserving data.table and rlang
  behavior and import and masking precedence. Cover both
  `Foo <- new_class("Foo", ...)` and the newer naming form without assuming
  that development APIs exist in every installation. Test supported S3/S4
  bridges, unions, and external registration against each pinned version
  instead of inferring compatibility from shared syntax.

  Semantic references: [released S7
  0.2.2](https://cran.r-project.org/web/packages/S7/index.html),
  [classes](https://rconsortium.github.io/S7/reference/new_class.html),
  [generics and
  methods](https://rconsortium.github.io/S7/articles/generics-methods.html),
  [development
  `:=`](https://rconsortium.github.io/S7/reference/named-bind.html),
  [compatibility](https://rconsortium.github.io/S7/articles/compatibility.html),
  and [package
  registration](https://rconsortium.github.io/S7/articles/packages.html).

#### 7. Receiver evidence and dependency metadata

- [ ] Add bounded receiver and class evidence for direct construction, aliases,
  declared slot, property, and field types, method dispatch parameters, and
  simple constructor returns. Start with `new("Class")`, S4/Reference Class
  generators, R6 `$new()`, and S7 constructors. Keep unions of plausible
  classes, invalidation after reassignment, and unknown results explicit. Do
  not require whole-program type inference before resolving `self`,
  `private`, `.self`, or a directly constructed local instance.
- [ ] Extend the versioned rindex schema and native harvesting to retain
  available class, member, generic, and method metadata, S3 registration
  tuples, origins, signatures, and help associations. Preserve the
  RDS/lazy-load structure needed for S4 metadata, R6 and Reference Class
  generators, and S7 class and generic objects. Use fixed installed-package
  fixtures, including malformed and unsupported metadata, rather than the
  current machine's libraries. Missing metadata remains incomplete; never
  load a namespace or invoke R to discover production behavior. Keep
  indexing on its existing isolated pool and preserve the precision of
  names-only and remote fallbacks.

#### 8. Editor and documentation features

- [ ] Replace textual receiver harvesting with class-owned member resolution
  when identity is known. Complete visible and inherited fields, slots,
  properties, and methods with correct kinds, signatures, and help. Scope
  `self` and `private` to their owning class, distinguish generator and
  instance APIs, and preserve a clearly limited usage-based fallback for
  unknown receivers. Add constructor and member signature help and local and
  project generic signatures; generic calls should show their generic
  surface and only claim a selected method when dispatch evidence permits
  it.
- [ ] Add class, generic, method, and member document and workspace symbols,
  semantic token roles, hover, definition, references, and implementation
  navigation. Connect string-based registrations and class references to
  semantic identities without treating arbitrary matching strings as uses.
  Extend type hierarchy to the supported S3 evidence and S7 declarations,
  qualified parents, and aliases. Give member and registered methods their
  own call-hierarchy items; keep ordinary generic calls, candidate dispatch
  edges, and superclass calls distinguishable. Keep incoming/outgoing edges
  consistent and do not invent source locations for installed metadata.
- [ ] Add conservative class, member, and generic rename only after definitions
  and references are reliable. Update linked declarations, proven string
  registrations, member accesses, and method signatures within the known
  ownership and visibility boundary. Refuse ambiguous or dynamically
  incomplete renames, preserve quoting, and never rename implicit receiver
  bindings. Test same-name members in unrelated classes/packages, stale
  snapshots, live-buffer fallback, and edits that shift every declaration
  span.
- [ ] Feed roxygen topic rules and help association from the same declarations:
  S3 and S4 methods, class slots, Reference Class and R6 fields and methods,
  and S7 properties and constructors. Recover generated callable surfaces
  and inherited documentation conservatively rather than duplicating OOP
  recognition in lint rules. Preserve parser ownership of tag structure and
  the test-only projector's faithful role; use `roxygen-parity` for actual
  CST parity gaps.

#### 9. Diagnostics and validation on real packages

- [ ] Make existing undefined, unused, signature, and dependency checks consume
  the shared facts, including dispatch reachability, active and accessor
  reads, registration side effects, and generated bindings. Retain
  conservative exemptions where the new model is incomplete. Do not equate
  an absent direct call with an unused registered method, or a list-shaped
  value with an object whose member set is closed.
- [ ] After the model is established, evaluate precise diagnostics for proven
  nonexistent or inaccessible members, invalid literal constructor
  arguments, incompatible generic and method formals, duplicate
  registrations, and impossible inheritance or signatures. Report only when
  the required class and registration facts are complete; dynamic objects
  and open class extensions stay quiet. Use `add-lint-rule` for new rules
  and avoid autofixes that depend on runtime dispatch, validation, or
  active-binding behavior.
- [ ] Run the linter-investigation workflow on pinned packages exercising each
  system and mixed systems. Verify suspected bugs with R and retain reduced
  regressions. At every implementation stage, test single-file and package
  resolution, cross-file edits, NAMESPACE and Collate changes, CLI and LSP
  agreement, losslessness, and idempotence. Check incremental invalidation
  and request behavior as the new facts are wired in. Publish the supported
  static subset and remaining dynamic limitations in the user docs.

## Language Server

### Navigation

- [ ] **Go-to-declaration/type-definition/implementation**. Low priority for
  general R bindings; likely alias to definition or omit. Class/member and
  generic-method implementation navigation belongs to the
  [OOP roadmap](#object-oriented-programming), where those relations can be
  established statically.

### Symbols

- [x] Function `detail` (signatures with defaults) in hierarchical document
  symbols, and `container_name` (enclosing binding) in flat document
  symbols.

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

- [x] Follow-up: prune confirmed archived CRAN packages from the bundled set.
  `scripts/dump_cran_symbols.R` checks CRAN's unfiltered source index, then
  requires an explicit archive notice on a missing package's CRAN page
  before removing its membership and exports. Failed status checks and
  failed installs preserve prior coverage. Successful dumps replace each
  package's entire section, removing symbols and lazy datasets it no longer
  exports, including when the new section is empty. Local fixture tests run
  via `task test-cran-symbols` and in CI.

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

- [x] **Reuse document-symbol outlines for unchanged buffers.** The LSP retains
  a typed outline per open buffer and serializes it on the read pool without
  another database snapshot. Edits replace the cache cell, so an old in-flight
  read cannot populate the new buffer's cache. Cold outlines prune CST branches
  without semantic bindings, and assignment operand accessors no longer collect
  child vectors.

  On the three largest tidyr benchmark files, 4,500 pinned release-LSP requests
  per implementation showed a 1.23x median speedup and a 1.28x minimum speedup.
  First requests after edits showed no material regression. JSON construction
  now accounts for about 39% of sampled CPU time on repeated requests.

  **Do not repeat the JSON-value cache experiment without new evidence.**
  Caching `serde_json::Value` instead of the typed outline improved the median
  only 1.8%, worsened the minimum 0.8%, and made edit-to-symbol median latency
  0.9% worse while retaining a larger object graph. Keep the typed cache; the
  JSON construction and transport costs need a different approach.

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

- [x] **The four sequential disk passes were not a useful hotspot.** A coherent
  discovery snapshot was built and fed to both `excluded_package_sources` and
  salsa workspace setup, eliminating their duplicate package-metadata reads.
  `strace` confirmed that the candidate removed two `getdents64` calls and three
  `openat` calls on `tidyr/R`, but 30 quiet runs on one pinned core were flat:
  41.089 -> 41.090 ms median (+0.003%), and 40.099 -> 40.150 ms minimum
  (+0.13%). The experiment was removed. The old 3.15 ms attribution conflated
  removable metadata reads with the source-buffer opens and ignore-aware walk
  that a lint run still has to perform; merging their control flow does not
  remove that I/O. Do not repeat this without new cold-filesystem evidence.

- [ ] **Consider a filesystem snapshot only as a broader ownership cleanup.**
  The rejected batch-only shortcut was not easier to maintain: it added a
  transient metadata model, a second salsa workspace-installation path, and
  special error plumbing while leaving the other discovery consumers alone. A
  future refactor is worthwhile only if one snapshot becomes the shared
  authority for file discovery, excluded package scope, package metadata, and
  source buffers. It must preserve their deliberately different inclusion
  policies, deterministic read errors and UTF-8 skips, parallel source reads,
  and the LSP's watched-file refresh lifecycle. Until that design removes
  ownership boundaries rather than duplicating them, the current passes are
  clearer despite their redundant syscalls.

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

## Misc

## Documentation site

- [ ] Split `docs/src/guide/editors.md` into focused editor recipes and a short
  overview. Preserve the published URLs and anchors when moving content.
- [ ] Consolidate configuration setup between `docs/src/guide/configuration.md`
  and `docs/src/reference/configuration.md`. Keep practical setup in the guide
  and exhaustive keys, defaults, and resolution rules in the reference.
- [ ] Reduce duplicated commands and integration examples across `README.md`,
  Getting Started, and the usage guides. Keep one detailed recipe per task and
  link to it from introductory pages.
