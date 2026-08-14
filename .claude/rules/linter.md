---
paths:
  - "src/linter.rs"
  - "src/linter/**/*.rs"
---

# Linter rules

`src/linter/`. Recipe for adding a rule: the `add-lint-rule` skill. Triaging the
linter against a real R codebase: the `linter-investigation` skill.

## Scope

- **The linter is purely semantic.** Anything the formatter's `--check` mode can
  catch belongs to the formatter, not here.
- Parse diagnostics **block** linting a file: `check_paths` reports
  `LintStatus::{Clean, Findings, ParseDiagnostics}`. That holds for
  `DESCRIPTION` too; the whole policy is `lint_description_source`.
- Lint inputs are `.R` files **and** a package's `DESCRIPTION`
  (`collect_lint_files`). A walked `DESCRIPTION` must sit at a package root that
  is not itself inside another package — the fake packages under a project's
  `tests/` are fixture data, not its metadata. An explicitly named one is always
  linted, and reading is never gated on the rule set (`syntax-error` is not a
  rule).

## Dispatch

- **No rule walks the tree on its own.** Join the driver's single shared walk:
  declare the `SyntaxKind`s you care about via `Rule::interests` and implement
  `Rule::check`. A whole-file rule leaves `interests` empty and overrides
  `Rule::check_file`.
- `src/linter/rules.rs` is the **single source of truth**: `rules_by_category`
  is what `all_rules`, `all_rule_ids`, and the reference page's category
  sections are all derived from. Register there, once.
- **Two grammars, one catalogue.** A `DESCRIPTION` rule implements `DcfRule`
  and is registered as `AnyRule::Dcf` in that same list, so rule IDs are one
  namespace and everything derived from the registry sees both. `run_dcf_rules`
  mirrors `run_rules`; `ResolvedRules` splits the two dispatch tables exactly
  once, in `with_config`, leaving the R hot path untouched. Never add a second
  registry — merging two per-category lists back together to render one section
  *is* a second source of truth for catalogue order.
- `run_rules` owns **suppression filtering** — it is the only place holding both
  the directive map and the findings — plus the post-suppression pass.
- `Rule::check_suppressions` is the post-pass for facts that only exist after
  every rule has emitted (`outdated-suppression` asks "did this directive match
  anything", which is a fact about the driver's filtering step).
- The `meta` rules lint arity's own `# arity` directives rather than R code,
  reading the parsed list off `RuleContext::suppressions`. `misplaced-suppression`
  asks the **formatter** where a format directive takes effect
  (`arity_formatter::formatter::directive::is_honored_position`) instead of
  re-deriving it — a report about behavior must not drift from the behavior.

## Rule identity

- A rule `id` is stable kebab-case and **user-visible**: it is the
  `# arity-lint skip` target, the reported rule, and the `select`/`ignore` and
  `[lint.rules.<id>]` key. Renaming one is a breaking change. It must never be
  one of the directive verbs (`skip`/`skip-file`/`off`/`on`), which sit where a
  rule ID would go; a test in `tests/lint.rs` guards that.
- Every rule needs a description and `examples()`. The examples are run through
  the **real linter** to render the docs page, so they are behavior, not prose.
  A rule whose subject is a *package-level* fact is silent on the single-file
  path, so it declares the synthetic package its examples live in via
  `doc_package` — otherwise the example renders with no finding.

## Autofix correctness

A fix is a **textual edit**, so the bar is **correctness, not formatting**.

- Applying a fix must leave code that still parses and is still lossless — never
  broken syntax (a negating rewrite that misbinds, `!a + b`) and never dropped
  trivia (a relocation that loses a comment).
- **A fix does not owe line-width.** It may leave a line the formatter would
  re-break: layout is the formatter's job (Tenet 1) and the pipeline is
  fix-then-format, not fix-alone. **Never invoke the formatter from a fix.**
- When an edit cannot meet the bar for some shape, make it correct by
  construction (tight span, atom-guarded) or **withhold the fix for that
  shape** — and still report the finding. That withhold/atom-guard discipline is
  what keeps the current fixes safe.
- `Safe` fixes apply under `lint --fix`; the rest need `--unsafe-fixes`.

## Directives (`src/linter/suppression.rs`)

`# arity-lint skip|off|on|skip-file [<rule>]`, plus the `# arity` column that
addresses the formatter too. The grammar is `arity_parser::directive` — shared,
never re-parsed here; this module only decides what it *means*: where a
directive attaches and which findings it removes.

- `# arity-ignore`/`# arity-ignore-file` are **deprecated aliases** of
  `skip`/`skip-file`, tagged `Spelling::Deprecated`. They behave identically;
  only a rule that rewrites them should look at the tag. Keep them out of docs
  and examples.
- **Three scopes, one predicate.** `Coverage::{File, Range, Nothing}` is what
  `is_suppressed` tests. A lint region is a plain byte range, so an unclosed
  `off` runs past a closing brace to end of file — unlike the formatter's, which
  is list-local.
- **A blanket directive never silences a finding spanned on a directive
  comment.** Otherwise `blanket-suppression` and `misplaced-suppression` would
  be unreportable in the very cases they exist for. A directive that *names* a
  meta rule still silences it — that is how an author says "I know".

## Config

- Per-rule options live in `[lint.rules.<id>]`, typed one struct per rule on
  `RulesConfig`, so a mistyped rule ID there is a **parse** error (unlike an
  unknown ID in `select`/`ignore`, which is reported at lint time). Only
  `undesirable-function` takes options today; per-rule severity is reserved
  (`TODO.md` §I4).
- Options reach rules as `RuleContext::config`, carried on `ResolvedRules` —
  **not** through `run_rules`' parameter list. Keep it that way.
- Version-aware rules read the resolved floors via
  `RuleContext::r_compat_floor`/`roxygen2_compat_floor`. With no floor at all
  (no `[compat]`, no `DESCRIPTION`), they stay **silent**.

## Testing and docs

- Lint has no fixture directory: add a `#[test]` in `tests/lint.rs` — or in
  `tests/lint_description.rs` for anything about `DESCRIPTION` — plus the rule's
  own `examples()`. Write the failing test first.
- A package fixture's `DESCRIPTION` should be *complete* (`TEST_DESCRIPTION` in
  `tests/lint.rs`), so a test only ever reports what it is about.
- `tests/lint.rs` checks that fixed output parses, and stays format-clean on the
  curated width-safe cases.
- The rule reference (`docs/src/reference/rules.md`) is **generated** by
  `cargo run --example docgen` and pinned by `tests/rule_docs.rs`. Never
  hand-edit it; regenerate with `task docs-gen`.
