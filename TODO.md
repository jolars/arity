# TODOs

### Parser

- [ ] Comment-aware suppression placement edge cases: directives inside
      `R_CALL_ARGUMENTS` between `( ... , <directive> , next_arg )` need special
      handling so they attach to `next_arg` instead of the argument list. (Jarl
      solved this by overriding biome's `place_comment`; ravel's
      next-non-trivia-sibling walk already handles most cases.)

### Formatter

#### Air-compat divergences (from the soft gauge)

Surfaced by `task air-compat` / `AIR_COMPAT.md`. These are cases where `air`'s
output is the more idiomatic one and ravel is being inconsistent --- "adopt"
work, not a quality gate (Tenet 1 still rules). Fixing the holes item alone
clears \~6 fixtures and is the biggest compat jump. Each fix lands its own
failing fixture first (TDD), and must hold idempotence + losslessness.

- [x] **`{{ }}` embracing is expanded.** Ravel expanded the rlang embracing
      operator in a function body (`function(x) {{ x }}`) into nested multi-line
      braces; air keeps `{{ x }}` inline when the function is a call argument.
      Now kept inline in call-argument position (matching ravel's existing direct
      `{{ x }}` arg rule and air); standalone / assignment-RHS bodies still
      expand, as air does. Fixtures: `call_trailing_inline_function`, `air_call`,
      `function_body_curly_curly`.
- [ ] **Control-flow bracing is left flat.** Ravel keeps
      `if (a) 1 else if (b) 2` and bare control-flow function bodies
      (`function(p) if (cond) {...}`) flat; air force-braces consequences /
      bodies onto their own lines. **Decision: adopt air's always-brace.**
      Biggest single divergence. Fixtures: `if_else_if_bare_flat`,
      `if_nested_consequence`, `function_bare_control_flow_body`.
- [ ] **`fn(NULL = )` spacing.** Named arg with a missing value: ravel emits
      `fn(NULL =)`, air keeps the trailing space `fn(NULL = )`. Trivial; match
      air. Fixture: part of `air_call`.
- [ ] **Pipe / nested-call indent depth.** In a pipeline, ravel indents a broken
      RHS call's args one level; air uses an extra level. Investigate whether
      this is a bug in ravel's indent model or a deliberate flatter style before
      deciding adopt vs record. Fixture: `air_pipelines`.
- [ ] **Hug vs explode when the call head exceeds the line width (design
      question, not a clear bug).** Ravel breaks an over-width call head onto
      multiple lines; air keeps the head over-width to preserve the trailing hug
      (e.g. `test_that("very long desc", {`). The hugging itself is a deliberate
      ravel choice (the one recorded deviation, `air_function_definition` in
      `tests/air_compat_allowlist.toml`); what needs a principled rule is
      whether hugging should win over line width. Resolve, then either fix or
      record. Fixtures: `air_test_that`, `function_definition_misc`, part of
      `call_trailing_inline_function`.

## Linter

Closest precedent: **jarl** (`etiennebacher/jarl`, Rust + rowan + air-parser,
55+ rules, suppression directives, LSP, autofix). The foundation pass borrows
shape (diagnostic + `Violation` trait, `PackageOrigin` enum, `# ravel-ignore`
suppression directive style, annotate-snippets rendering) but stays its own
project --- ravel is a unified formatter + linter + LSP binary on ravel's own
in-tree parser, not a drop-in jarl replacement.

## Language Server

- [ ] LSP refinements: honor `initializationOptions` /
      `workspace/didChangeConfiguration`; add `textDocument/rangeFormatting`
      once the formatter gains a range API. (`textDocument/codeAction` QuickFix
      hooks now shipped alongside autofix --- see Phase 6.x autofix above.)
- [ ] CRAN-wide symbol manifest as a downloadable sidecar. Shape: per-package
      export lists keyed by package version. With a manifest in place, enable
      `undefined-symbol` by default and stop returning `Unknown` for names from
      `library()`-attached packages.
- [ ] DESCRIPTION / NAMESPACE parsing for R-package authoring contexts. Match
      jarl's behavior: track `importFrom()` direct mappings and `export()`
      declarations so `unused-binding` doesn't flag exported package symbols.
- [ ] Cross-file scope awareness: a binding defined in `a.R` should resolve from
      `b.R` when both belong to the same package or project.
- [ ] Salsa-cached `semantic_model` query in `src/incremental.rs`. The current
      `parse_file` query stores only a debug-formatted CST string; both the
      linter and LSP rebuild the semantic model from text. Adding a tracked
      query requires a `salsa::Update`-friendly snapshot type (the rowan
      `SyntaxNode` itself isn't easy to wire in).
- [ ] Honor editor-supplied `initializationOptions` /
      `workspace/didChangeConfiguration` for `line-width` / `indent-width`.
- [ ] Range formatting (`textDocument/rangeFormatting`) once the formatter gains
      a range API.
- [ ] Add parse performance and incremental-reparse benchmarks.

## Misc

- [ ] Rmd / Qmd chunk extraction; chunk-level suppression directives via
      Quarto-style `#| ravel-ignore-chunk` comments.
- [ ] `ravel-ignore-unused` meta-diagnostic: emit a finding for suppression
      comments that didn't actually suppress anything (rule ID is reserved but
      the rule is not yet wired in).
