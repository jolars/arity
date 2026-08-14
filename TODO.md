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

- [ ] Report an **outdated `# arity-format` directive** — one whose marked span
  the formatter would not have changed anyway. It is a `format --check` fact,
  not a semantic one, so it belongs to the formatter, not to
  `outdated-suppression`: computing it means formatting the span and comparing,
  which the linter must not do.

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

### `undefined-symbol` false positives (rlang sweep, 2026-08-13)

- [ ] **`useDynLib(pkg, .registration = TRUE)` binds native routines arity cannot
  enumerate.** They live in the C sources, so a reference outside a `.Call`
  head — passed as a value (`capture_arg = ffi_enquo`) or compared
  (`identical(capture_arg, ffi_enquo)`) — is a false positive. ~12 findings in
  rlang (`R/nse-defuse.R`, `R/dots.R`). The head-position case is already
  handled by the `.C`/`.Call`/`.Fortran`/`.External`/`.External2` arm in
  `semantic/builder.rs`. Either suppress unresolved bare names in a package
  declaring `.registration = TRUE`, or harvest the routine names from `src/`'s
  `R_CallMethodDef` table.

- [ ] **rlang's defusing operators are not data-masking-aware.** `quote()`,
  `bquote()`, `substitute()`, and `expression()` mask their bodies
  (`is_quoting_call`, `semantic/builder.rs`), but `quo()`, `quos()`, `expr()`,
  `exprs()`, `enquo()`, `enquos()` do not, so every captured symbol reads as
  undefined. Confirmed: `quo(fn(this, that))` yields three findings, the base
  `quote(fn(this, that))` none. A fix has to respect unquoting — `!!`, `!!!`,
  and `{{ }}` *do* evaluate — so it is not just a longer name list.

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
  `#[ignore]`d, `task description-oracle`. 53 cases (the rindex fixtures,
  the reference checkouts when present, and a planted-defect table), and a
  missing `Rscript` is a skip, as in the other two oracles.

  Four checkers, because one is not enough:
  `.check_package_description(strict = TRUE)`,
  `.check_package_description_authors_at_R_field(strict = 2L)` (the outer
  checker calls it at `strict = FALSE`, so the per-person name, role, ORCID,
  and ROR signals are unreachable from there), the `duplicates` half of
  `.check_package_description2`, and the two version components of
  `.check_package_CRAN_incoming(localOnly = TRUE)`. The last two are
  cherry-picked, not taken whole: their other components need installed
  packages, a `src/`, files, or the network, which a text-only oracle has no
  business simulating. Note the CRAN checker reaches the version only after a
  `Maintainer` and a `Title` it can inspect, and errors on the `NA`
  otherwise—a planted case that wants those signals has to carry both.

  **Two-sided by construction**, because arity implements a fraction of what
  R checks. `GATES` holds the rules arity ships and requires containment,
  not parity: every finding arity reports must be backed by an R signal on
  that case. The reverse is not gated—`description-version-constraint`
  deliberately says nothing about a malformed package *name*, and demanding
  parity would be demanding a rule that does not exist. `PLANNED` holds the
  signals no rule covers, each tagged with the rule above that will claim
  it; they are counted and ranked, never failed, and that ranking is the
  work-list. Today: text-format 10, authors-at-r 4, encoding 3, then one or
  two each for the rest.

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

- [ ] `description-encoding`. Non-ASCII bytes with no `Encoding` field (R's
  `missing_encoding`), and non-ASCII in the fields R requires be ASCII
  (`Package`, `Version`, `License`, `Encoding`). arity already reads the file
  as UTF-8, so "is this valid UTF-8" is decided, which makes
  `Encoding: UTF-8` a **safe fix**.

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
- [ ] `description-unknown-field`. **Not** a whitelist—`Config/*`, `Remotes`,
  and `RoxygenNote` are legal and everywhere—but a *near-miss* check: edit
  distance 1 from a standard field name. `Suggest:`, `Depend:`, and
  `Mantainer:` are silently ignored by R today. The trimmed-field-name
  divergence under stage 1 wants a home too, and this is it.

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
  `documentOnTypeFormattingProvider` (`src/lsp/server.rs`). Small wiring over the
  existing `format_range` path, but **gated on the CRLF bug already logged under
  Formatter** (line-ending config isn't threaded into `format_range`, so a range
  edit in a CRLF buffer splices LF); fix that first, then advertise. (2026-07-02
  languageserver survey.)

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

- [x] **3. DESCRIPTION lint rules.** Done. Five rules in a new `Packaging`
  category, the one category spanning both grammars:
  `undeclared-dependency` (R-side, default on), `unused-dependency`
  (DESCRIPTION-side, **default off**), `description-missing-field`,
  `description-duplicate-field`, `description-version-constraint`. None
  ships an autofix; each repair needs a value or a decision only the author
  has, and for `unused-dependency` a fix would also mean editing a
  comma-separated list, which is stage 5's job.

  A `DcfRule` runs over a parsed `DESCRIPTION` the way `Rule` runs over R,
  and both register in the **same** `rules_by_category` via `AnyRule`, so
  rule IDs stay one namespace and `all_rule_ids`, `select`/`ignore`, and the
  reference page are still derived from one list. A second registry was
  rejected: merging two per-category lists back together to render one
  section *is* a second source of truth for catalogue order. `ResolvedRules`
  splits the two dispatch tables once, in `with_config`, so a run over R
  files never pays for the DCF table. `linter::render` and
  `to_lsp_diagnostic` needed no change, as predicted.

  `# arity-ignore` works in `DESCRIPTION`. A comment line is a child of the
  field it follows—a `FIELD` stays open across its continuation lines—so a
  directive attaches to its enclosing field only when a value line still
  follows it, and otherwise points at the next field, which is what its
  author meant.

  `collect_lint_files` splits discovery by grammar. A walk takes a
  `DESCRIPTION` only at a package root that is not itself inside another
  package: the first half skips `inst/extdata` fixtures, the second skips
  the complete fake packages roxygen2 and devtools keep under `tests/`,
  which a corpus sweep found immediately. An explicitly named one is always
  linted. Reading is *not* gated on the rule set: `syntax-error` is not a
  rule, so a `DESCRIPTION` `read.dcf` would reject surfaces under `--select`
  exactly as a broken `.R` file does.

  The exempt set for `undeclared-dependency` is R's own, read off
  `tools:::.check_packages_used` and pinned against R by
  `tests/deps_oracle.rs` (`task deps-oracle`): the base-priority packages
  minus `methods` and `stats4`. That is deliberately **not**
  `default_packages()`, which answers what a session *attaches* and differs
  in both directions—`parallel` and `tools` ship unattached, `methods` is
  attached and still has to be declared.

  `unused-dependency` reports on *absence*, so it is the one rule here that
  could talk a maintainer into deleting a dependency their package needs.
  Hence default-off, `PackageUsage::complete` (the whole `R/` set analyzed,
  a NAMESPACE read, at least one source), and exemptions for a `LinkingTo`
  co-declaration, `methods` under S4, and any package named as a plain
  string. `PackageReferences` records load calls at **any depth**—
  `requireNamespace()` in a function body is the conditional-dependency
  idiom, and `SemanticModel::loaded_packages` is top-level-only because it
  models attachment, a narrower fact. Both it and the `package_usage` fold
  are range-free `Eq` firewalls, guarded with negative controls in
  `tests/salsa_incremental.rs`.

  A sweep over 15 real packages (tidyverse, r-lib, data.table, Rcpp) found
  zero findings; injecting an unused `Imports` entry and deleting a used one
  proved both dependency rules fire. That sweep is the gate for ever
  flipping `unused-dependency` on, and it is not enough on its own—the
  `linter-investigation` skill is.

  Two rules deliberately **not** written here. `library-in-package`:
  `library(dplyr)` in `R/` is wrong even when `dplyr` *is* in `Imports`, and
  reporting that under an ID meaning "not declared" would read as a false
  positive. `unconditional-suggest`: flagging unguarded use of a `Suggests`
  package is a control-flow question over `ctx.cfg`, not a name-set one, and
  R exempts `Suggests` for exactly that reason.

- [x] **4. DESCRIPTION in the LSP.** Done. `didOpen` routes a DESCRIPTION to the
  DCF pipeline; diagnostics, package-name completion in dependency fields, and
  hover with a dependency's installed version and `Title`.

  The premise turned out to be understated. The server had **no** file-type
  routing at all—`didOpen` accepted any document and linted it as R—so the
  gap was not a missing feature but a live hazard: a DESCRIPTION parsed as R
  published eight bogus syntax errors, and `textDocument/formatting` answered
  with the file reflowed as R, rewriting `Package: testpkg` to
  `Package:testpkg`. Only `editors/code` not claiming the file kept it
  theoretical. `DocumentKind` is therefore step one, and the guard is
  structural rather than a checklist: `doc_snapshot` is gone, replaced by
  `r_doc_snapshot` (18 R-only handlers) and `doc_snapshot_any` (diagnostics,
  hover, completion), so there is no un-annotated way to get a buffer and a
  handler added later cannot inherit the wrong grammar by accident.

  **The file name beats the client's `languageId`.** `editors/code` already
  registers `NAMESPACE` under language `r`, so a client reporting `r` for a
  DESCRIPTION is entirely plausible—and trusting it would format DCF as R.
  Read off the URI's last segment, not `uri::to_path`, which gives up on
  `git:`/`untitled:` schemes.

  Diagnostics are gated on `is_own_package_root`, mirroring the CLI *walk*
  rather than its explicit-path policy: an editor sends every file a user
  glances at. The accepted cost is silence on a hand-edited
  `tests/testthat/testpkg/DESCRIPTION`, and on a skeleton with no `R/` yet.

  An open buffer is authoritative in salsa, so an unsaved `Imports: dplyr`
  clears `undeclared-dependency` in open R files. The fan-out is gated on
  `DescriptionFacts` actually moving, so prose keystrokes cost one DCF parse
  instead of re-linting the package; it terminates after exactly one extra
  generation only because `upsert_description` short-circuits on equal text,
  which `a_facts_change_relints_once_and_settles` pins. Two ways the buffer
  could be silently reverted, both closed: seeding is ordered **before** the
  upsert (`refresh_package_graph` re-reads every DESCRIPTION from disk), and a
  watched `CHANGED` event under an open buffer is now dropped, as it already
  was for `.R`. `didClose` restores the on-disk facts by reusing the
  watched-file path rather than adding a second message.

  `Title` is **harvested** (`SCHEMA_VERSION` 2 → 3), not read on demand. The
  deciding argument is completion, not hover: labeling every candidate at once
  rules out a file read per candidate, and on-demand reading is not even
  independent of the index—it needs `PackageIndex::lib_path` to find the
  file, so it answers a strict subset. The bump costs one background
  re-harvest, since `packages_to_build` queues everything the now-empty index
  no longer covers; an additive field without a bump would instead have left
  the `Title` missing forever for already-indexed packages.

  Completion items carry an **explicit** `textEdit`, unlike the R path, which
  leans on the client's word pattern—that pattern belongs to a language id
  we just invented, and the wrapped one-per-line `Imports` that `usethis`
  writes is exactly what it would get wrong. Every range comes from the CST via
  `dependency_entries`; a range off `folded_value()` does not index the buffer.
  No new `ReadJob` variants: `hover_via_db`/`completion_via_db` branch on the
  path, because a new variant means a new arm in `run_read` *and* in the drain
  match, where a miss leaks a request forever.

  Deliberately **not** here. Field-name completion (`Package`, `Version`, …):
  a different context and a different candidate list, whose natural home is
  stage 5, which must own the canonical field order anyway—building that list
  twice is the failure mode. And no `configurationDefaults` formatter entry for
  the new `r-description` language: declaring ourselves the formatter for a
  file we answer `null` for makes format-on-save silently do nothing, which
  reads as a broken extension. (Stage 5 landed both halves of that: the entry
  is there now, and it answers edits.)

  Neovim ships **no** filetype for `DESCRIPTION` (verified, not assumed), so
  `docs/src/guide/editors.md` hands users a `vim.filetype.add` line.

- [x] **5. DESCRIPTION formatting.** Done, and **on by default**. Canonical
  style is what `desc::desc_normalize()` writes: `desc:::field_order`,
  dependency lists one per line with `,\n    `, four-space continuations,
  quoted `Collate`. arity's differentiator is that `Authors@R` and `Roxygen`
  are *R code*, which we format with our own formatter—`desc` round-trips
  them through `deparse()`.

  This entry used to say "must be opt-in", on the grounds that `usethis`,
  `devtools` and `R CMD build` rewrite the file on their own schedule. Checked
  against R 4.6.1 with `desc` 1.4.3, that is mostly wrong. `desc$write()`
  rewrites **only the field it was asked to change**: `desc_set_dep()` (what
  `use_package()` calls) left field order, an unwrapped `Description`, and a
  hand-laid-out `Authors@R` untouched. `roxygen2::roxygenise()` is purely
  additive—it appends `Config/roxygen2/version` and nothing else. `R CMD build`
  does not touch the source file at all; `Packaged:`/`Built:` go into the
  tarball copy. The one real collision is `usethis::use_author()`, which
  deparses `Authors@R` through `desc`: one field, on an explicit and
  infrequent call. `desc_normalize()` does reorder, but nobody wires it into a
  pipeline. So the field-order fight does not exist in a normal workflow, and
  the mitigation is the off switch (`[format] description`) rather than an
  opt-in default.

  What makes default-on defensible is not that argument, though—it is the
  **closed class table whose default is `Opaque`**: a field arity does not
  recognize keeps its line structure byte for byte, so `read.dcf` sees an
  identical value. And restyling is **refused outright** wherever it could
  change what R reads: duplicate fields (R takes the last, our reader the
  first), multiple records, whitespace before a colon (`Package : p` declares a
  field named `"Package "`), a non-UTF-8 `Encoding`, a BOM.

  Comments are never dropped, unlike `desc`. They attach **forward**, to the
  next field, because that is what `next_meaningful_dcf_sibling` already
  implements for `# arity-ignore`—moving one relative to its anchor would
  silently retarget a suppression. A comment *interior* to a field freezes that
  field's value verbatim, since it has no position once the value is reflowed.

  No document IR: every break is decided by the field's class, and prose wants
  first-fit rather than the layout engine's all-or-nothing group. Lives in the
  published formatter crate so the dprint plugin can reach it, which is why
  stage 1 went in the parser crate.

  Verified in three layers: the fixture suite's meaning relation (pure Rust,
  every commit), `formatted_dcf_matches_read_dcf` and
  `formatted_authors_at_r_reads_identically` in the oracle (the latter through
  `utils:::.read_authors_at_R_field`, so the comparison is against the bytes
  `R CMD build` writes), and `dcf-*` categories in the corpus sweep.
  `task desc-compat` is the soft gauge; never a gate.

- [ ] **Follow-ups to stage 5.** Field-name completion in the LSP
  (`Package`, `Version`, …) now has a canonical order to draw from —
  `description/order.rs` owns it, so the candidate list is a projection of that
  rather than a second copy. Range formatting for `DESCRIPTION` stays
  deliberately unimplemented (field order is a whole-document property), so
  `editor.formatOnSaveMode: "modifications"` does nothing there. And
  `jolars/arity-pre-commit`'s `arity-format` hook still filters to `.R`: until
  its `files`/`types` widens, no pre-commit user sees any of this.

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
    `description-duplicate-field` now makes this visible at the duplicate
    instead of leaving it silent, which is the prerequisite for the flip.

  - A field name is trimmed. R does *not*: `Package : p` declares a field
    literally named `"Package "`, so R sees no `Package` at all. arity is
    deliberately lenient here (it reads the obvious intent of a typo'd header),
    and the CST keeps the whitespace as its own token, so a DESCRIPTION lint
    can flag it precisely instead of the parser guessing.

- [x] **`desc` is a style reference for stage 5, not an oracle.** Settled that
  way. Tested against desc 1.4.3: `desc::desc_normalize()` reorders fields,
  splits dependency lists one per line, and quotes `Collate` entries—all of
  which stage 5 took—but it **drops comments even on a plain parse->write with
  no normalization**, and emits a trailing space after `Depends:`. Matching it
  byte for byte would mean deleting user content, which contradicts the
  invariant the DCF parser exists to uphold.

  So it is measured the way `air` is for R: `task desc-compat`, soft,
  one-directional, never a gate, with comment preservation and the trailing
  space normalized away as recorded deviations. One departure from
  `air_compat.rs`: the primary number is taken at `line-width = 75`, desc's own
  hard-coded `strwrap` width, so it tracks rule divergence rather than the
  width we happen to default to. `DESC_COMPAT.md` is the artifact.

  Worth knowing: `desc` does not merely drop comments, it can **corrupt** a
  field it cannot read. An `Authors@R` of `person("Jo",` comes back from
  `desc_normalize()` as `c(\n .\n)`. arity leaves it byte-identical.

- [x] Inlay hints for dependency fields: show the installed version for Imports,
  Depends, Suggests. The label is anchored at the end of the whole entry, so a
  declared floor is not split from its name, and only locally harvested packages
  get one (`R` never does). `workspace/inlayHint/refresh` carries a late harvest
  to an already-open buffer.

## Misc

