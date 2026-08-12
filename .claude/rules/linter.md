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
  `LintStatus::{Clean, Findings, ParseDiagnostics}`.

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
- The `meta` rules lint arity's own `# arity-ignore` directives rather than R
  code, reading the parsed list off `RuleContext::suppressions`.

## Rule identity

- A rule `id` is stable kebab-case and **user-visible**: it is the
  `# arity-ignore` target, the reported rule, and the `select`/`ignore` and
  `[lint.rules.<id>]` key. Renaming one is a breaking change.
- Every rule needs a description and `examples()`. The examples are run through
  the **real linter** to render the docs page, so they are behavior, not prose.

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

- Lint has no fixture directory: add a `#[test]` in `tests/lint.rs`, plus the
  rule's own `examples()`. Write the failing test first.
- `tests/lint.rs` checks that fixed output parses, and stays format-clean on the
  curated width-safe cases.
- The rule reference (`docs/src/reference/rules.md`) is **generated** by
  `cargo run --example docgen` and pinned by `tests/rule_docs.rs`. Never
  hand-edit it; regenerate with `task docs-gen`.
