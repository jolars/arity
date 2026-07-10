---
name: add-lint-rule
description: Add a new built-in lint rule to the arity linter—implement it
  against the single-walk dispatch, register it in the one source of truth, add
  TDD fixtures plus the autofix-correctness (parse-clean) case, and wire up the
  snapshot-pinned generated docs and the two hand-maintained book indexes.
---

Use this skill when asked to add a new built-in lint rule (correctness,
suspicious, readability, performance, ...), whether or not it ships an auto-fix.
The roadmap of planned rules lives in `TODO.md` under "Rule roadmap"—when
the request names a roadmap item, follow its category/cost/safety annotation and
check the item off when done.

## Tenets that constrain a rule (from `AGENTS.md`)

- **Tenet 1—formatting is the formatter's job.** Pure layout (quotes,
  leading zeros, spacing, indentation, semicolons, trailing whitespace) is **out
  of scope** for the linter. If the "rule" is really a formatting preference, it
  does not belong here.
- **Tenet 3—parsing is the parser's job.** Do not paper over parser mistakes
  in a rule. If the CST does not expose what you need, add a typed wrapper in
  `src/ast/nodes.rs` (re-exported from `src/ast.rs`) rather than re-lexing or
  re-parsing inside the rule.
- **Autofix correctness (Tenet 1 corollary)—a fix is a textual edit, never a
  formatter.** The bar is *correctness*, not formatting: applying the fix must
  leave code that still **parses** and stays **lossless** (no misbinding
  `!a + b`, no dropped comments). Make the edit correct *by construction* (tight
  span, atom-guarded, whitespace-preserving), or **withhold the fix** for that
  shape (still report the finding). A fix does **not** owe line-width—layout
  is the formatter's job and the pipeline is fix-then-format; never run the
  formatter inside `--fix`. Enforced by
  `tests/lint.rs::fixed_output_is_parseable_and_clean` (the curated cases are
  width-safe, so they also stay format-clean).

## Cost model (drives which infra you may touch)

A rule is **cheap (`syn`)** when it only needs CST + AST + literal inspection;
**medium (`ns`)** when it must confirm a callee resolves to base R (not a user
redefinition) via `RuleContext::resolves_to_base`; **expensive (`sem`)** when it
needs the `SemanticModel` (scopes/flow). Anything needing R evaluation or type
inference is **out of scope**—arity stays static. Prefer the cheapest tier
the rule's correctness actually requires.

## Key files

- `src/linter/rules.rs`—the `Rule` trait, `RuleContext`, and
  **`all_rules()`, the single source of truth**. `all_rule_ids()` derives from
  it, so there is no second list to sync. Every new rule is added here exactly
  once.
- `src/linter/rules/<category>.rs`—the category module
  (`correctness`/`suspicious`/`readability`; add `performance`/`meta`/`pkg/...`
  per the roadmap when first needed). Holds `mod <id>;` +
  `pub use <id>::<Name>;`.
- `src/linter/rules/<category>/<id>.rs`—one file per rule: a unit
  `pub struct` implementing `Rule`. (File name may differ from the rule id when
  the id is a Rust keyword, e.g. `repeat` lives in `suspicious/repeat_loop.rs`.)
- `src/linter/rules/matchers.rs`—reusable CST/AST shape matchers
  (`call_named`, `callee_name`, `args`/`nth_arg`/`named_arg`, `binary_parts`,
  literal classifiers `is_true`/`is_false`/`is_na`/`is_null`/`is_bool_symbol`,
  `element_text`, `is_atom`). **Prefer these over re-walking raw CST.** Add a
  matcher here when a shape recurs across rules.
- `src/linter/diagnostic.rs`—`Diagnostic`, `Fix`
  (`Fix::safe`/`Fix::unsafe_`), `Applicability`, `Severity`,
  `ViolationData::new(..).with_suggestion(..)`.
- `src/ast/nodes.rs` (`src/ast.rs`)—typed AST wrappers (`CallExpr`,
  `IfExpr`, `WhileExpr`, `BinaryExpr`, ...). `src/syntax.rs`—`SyntaxKind`/`SyntaxElement`.
- `src/semantic.rs`—`SemanticModel` (`idents()`, `resolve_local`,
  `loaded_packages`) for `sem`-tier rules.
- `tests/lint.rs`—integration tests + helpers: `diagnostics(src)`,
  `fixed_output(src, rule)` (single safe fix applied),
  `fixed_output_all(src,   rule)` (all of a rule's fixes), and the
  autofix-correctness harness `assert_fixed_output_is_clean` and
  `fixed_output_is_parseable_and_clean`.
- `examples/docgen.rs`—generates `docs/src/reference/rules/<id>.md` from
  `render_rule_doc` (and stamps `version.md`). Run with
  `cargo run --example   docgen`. It only writes per-rule pages; it does **not**
  touch the indexes.
- `tests/rule_docs.rs`—pins each rule's rendered page via an **`insta`
  snapshot** (`rule_docs_render`), and asserts every rule has a non-empty
  `description()` + at least one `examples()` entry, **and that each example
  actually triggers its own rule**. Accept new snapshots with
  `cargo insta   accept`.
- `docs/src/reference/rules.md` and `docs/src/SUMMARY.md`—**hand-maintained**
  rule indexes. `docgen` does NOT regenerate these; add the
  new rule's line to both, in the right category, matching `all_rules()` order.
- `TODO.md`—the live roadmap. Check off the rule's item (with a one-line
  note on scope/safety, mirroring the other landed entries).

## Workflow

1. **Pick the rule id (kebab-case).** This is the public `id()`, the `--select`/
   `--ignore` key, and the docs slug—unique and stable; renaming is a
   breaking change. Match existing tone (`redundant-ifelse`,
   `true-false-symbol`). Public ids stay flat kebab-case even though the file
   lives under a category dir.

2. **Decide gating and safety before writing code:**
   - **Category** = directory (`correctness`/`suspicious`/`readability`/...); it
     does not appear in the id.
   - **Severity**—set per-`Diagnostic` (rules push `severity: Severity::...`
     directly; `Warning` is the norm, `Error` only for genuine bugs).
   - **`default_enabled()`**—default `true`; override to `false` for noisy
     opt-in rules (`undesirable-function`, `unused-function`, ...).
   - **Dispatch shape**—node-shape rules declare `interests()` (a slice of
     `SyntaxKind`) and implement `check(el, ctx, sink)`, called once per
     matching element in the *one* shared `descendants_with_tokens()` walk.
     Model-/ comment-driven rules leave `interests()` empty and override
     `check_file(ctx, sink)` (a once-per-file pass). **Never walk the CST
     yourself from a node-shape rule.**
   - **Fix safety**—ship `Fix::safe` only when the edit is unambiguous and
     correct (parse-clean, lossless) by construction; otherwise `Fix::unsafe_`
     (applied only under `--unsafe-fixes`), or withhold the fix entirely and
     still report (autofix correctness). A negating/dropping rewrite needs an
     atom operand—guard with `matchers::is_atom` (see `redundant-ifelse`,
     `redundant-equals`).

3. **Write the failing test first** (TDD per `AGENTS.md`) in `tests/lint.rs`:
   - Positive case via `fixed_output`/`fixed_output_all` (asserts the fix
     output) or `diagnostics(src)` filtered by `d.rule == "<id>"`.
   - Negative ("should not flag") case, and any edge the rule explicitly handles
     (e.g. fix withheld on a complex operand/commented clause—assert
     `d.fix.is_none()`).
   - Add the canonical shape(s) to the `cases` array in
     `fixed_output_is_parseable_and_clean` so autofix correctness is checked
     (keep the shapes width-safe). Run it and watch it fail before implementing.

4. **Implement the rule** in `src/linter/rules/<category>/<id>.rs`:
   - Module doc comment explaining what it flags, why, and any safety reasoning.

   - Cast the dispatched element with the typed wrapper (`el.as_node()` →
     `CallExpr::cast`/`WhileExpr::cast`, or `el.as_token()`), then use
     `matchers::*` for the shape checks.

   - Keep the diagnostic **span tight**—point at the offending construct,
     not the whole statement (it drives the CLI caret and LSP underline).

   - For `ns`-tier rules, gate the bare-name rewrite behind
     `ctx.resolves_to_base(&call)` so a user redefinition, namespaced, or
     shadowed callee is not rewritten.

   - **Required for docs to pass:** implement `description()` (non-empty) and
     `examples()` (≥1 `Example` whose `source` actually triggers the rule).

   - Trait skeleton:
     ```rust
     impl Rule for MyRule {
         fn id(&self) -> &'static str { "<id>" }
         fn description(&self) -> &'static str { "…" }
         fn examples(&self) -> &'static [Example] { EXAMPLES }
         fn interests(&self) -> &'static [SyntaxKind] { &[SyntaxKind::CALL_EXPR] }
         fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
             // … push Diagnostic { rule, severity, path: Default::default(), range, message, fix } …
         }
     }
     ```

5. **Register it** in the single source of truth:
   - `mod <id>;` + `pub use <id>::<Name>;` in `src/linter/rules/<category>.rs`.
   - One `Box::new(<category>::<Name>)` line in `all_rules()`
     (`src/linter/rules.rs`), in roadmap/category order. Nothing else—selection
     and `--select`/`--ignore` validation derive from this list.

6. **Generate and pin the docs:**
   - `cargo run --example docgen` → writes `docs/src/reference/rules/<id>.md`.
   - `cargo test --test rule_docs` will fail on the new snapshot; eyeball the
     `.snap.new` (the example must show the finding and, if fixable, the
     after-fix block), then `cargo insta accept`.
   - **Manually** add the rule to `docs/src/reference/rules.md` (under its
     category heading) **and** `docs/src/SUMMARY.md`—`docgen` does not.

7. **Update `TODO.md`**—check off the item and add a one-line scope/safety
   note matching the other landed entries.

8. **Validate** in order:
   - Targeted: `cargo test --test lint <name>`, `cargo test --test rule_docs`.
   - Full gates (CI parity): `cargo test`,
     `cargo clippy --all-targets --all-features -- -D warnings`,
     `cargo fmt -- --check`. (`task lint`/`task test`/`task docs-gen` wrap
     these.)

## Dos and don'ts

- **Do** reach for `matchers::*` and typed AST wrappers before raw CST walking;
  add a matcher when a shape recurs.
- **Do** keep spans tight and fixes parse-clean/lossless by construction, or
  withhold.
- **Do** make `examples()` snippets that genuinely trigger the rule—the docs
  tests reject a plausible-but-inert example.
- **Don't** add a second registration list or an `if`-guard; `all_rules()` is
  the only list.
- **Don't** implement a formatting/layout preference as a lint rule (Tenet 1).
- **Don't** work around a parser/CST gap inside the rule (Tenet 3)—fix or
  extend the parser/AST instead.
- **Don't** run the formatter inside a fix (Tenet 1: the formatter is the sole
  layout authority), or ship a fix that produces broken or lossy code (autofix
  correctness). A fix needn't satisfy line-width—that's the formatter's job.
- **Don't** hand-edit `docs/src/reference/rules/<id>.md`—regenerate via
  `docgen`. (But the two indexes *are* hand-edited.)

## Report-back format

When done, report:

1. Rule id, category, severity, and whether it ships a safe/unsafe fix (or
   none).
2. Cost tier (`syn`/`ns`/`sem`) and `default_enabled`.
3. New files (rule module) and updated files (`<category>.rs`, `rules.rs`
   `all_rules()`, `tests/lint.rs`, the generated `rules/<id>.md` + accepted
   snapshot, `rules.md`, `SUMMARY.md`, `TODO.md`).
4. Targeted test names plus the autofix-correctness case added.
5. Full-gate results: `cargo test`, clippy `-D warnings`, `cargo fmt --check`.
