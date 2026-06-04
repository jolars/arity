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
- [x] **Control-flow bracing is left flat.** Ravel kept
      `if (a) 1 else if (b) 2` and bare control-flow function bodies
      (`function(p) if (cond) {...}`) flat; air force-braces consequences /
      bodies onto their own lines. Adopted air's always-brace (a faithful,
      position-aware port: statement-position `if` always braces; a simple
      value-position one-liner stays flat unless it is a nested-if / `else if`
      chain or overflows the line width). Air's leading-newline forcing is
      *not* ported (Tenet 1: input line breaks never influence output). Bare
      control-flow function bodies now wrap in their own braces, matching air.
      Fixtures: `if_else_if_bare_flat`, `if_nested_consequence`,
      `function_bare_control_flow_body`, plus `if_statement_position_simple`,
      `if_value_position_stays_flat`, `if_value_position_nested_braces`,
      `if_block_position_boundary`.
- [x] **`fn(NULL = )` spacing.** Named arg with a missing value: ravel emitted
      `fn(NULL =)`, air keeps the trailing space `fn(NULL = )`. Matched air via an
      `ArgSlot::ends_with_eq` flag that keeps a space before a same-line comma or
      closing bracket. Fixture: part of `air_call`.
- [x] **Pipe / nested-call indent depth.** In a pipeline, a broken RHS call's
      args sat one level too shallow and the closing paren dangled at the base
      indent --- not a flatter style but a genuine bug: the pipe builder
      (`ir_binary_expr`) wrapped only the continuation `hard_line` in
      `Ir::indent`, leaving the RHS operand itself at the base indent, so the
      RHS call's own arg breaks used the wrong indent context. Fixed by moving
      the RHS inside the indent (`Ir::indent([hard_line, rhs])`); args now nest
      relative to the pipe stage and the close paren aligns with the call head,
      matching air. Fixtures: `air_pipelines`, plus the `mutate()` case in
      `air_call`.
- [x] **Hug vs explode when the call head exceeds the line width.** Resolved in
      favor of a uniform rule: a trailing-element hug applies only when the whole
      prefix up to the hugged block's opening `{` fits flat on the line
      (`callee(leading, function(params) {` or `callee(leading, {`); the
      function's own params never break to "rescue" the hug). When it does not
      fit, the call explodes one argument per line --- line width wins over the
      hug. The same principle now governs function *parameter* lists: a
      brace-block default (`function(a = { ... }, b)`) forces the list to expand
      rather than hugging the brace mid-list. Trailing functions now route
      through the flat-only `group_hug` (the break-aware `build_arg_hug_conditional`
      was removed), and brace defaults render as native `Ir::indent` (no baked
      `Verbatim`), so they indent correctly even when the function is itself a
      nested, exploded call argument. This made `air_function_definition`,
      `function_definition_misc`, and `call_trailing_inline_function` exact air
      fixed points. The one remaining divergence in this family --- air keeping an
      over-width `test_that("...", {` prefix on one line --- is air's callee
      special case; ravel explodes deterministically (Tenet 1) and this is now the
      recorded deviation `air_test_that` in `tests/air_compat_allowlist.toml`.
- [ ] **"Breaking must reduce overflow" rule (the `test_that` explosion,
      reconsidered).** For `test_that("<very long desc>", { ... })`, ravel
      explodes the call one-arg-per-line, but the description string *still*
      overflows on its own line afterward --- so the explosion costs four lines
      and a deeper indent and buys no width reduction. Air keeps it compact via a
      callee/shape special case (string first arg + trailing block), which we
      correctly reject under Tenet 1 (it guesses author intent from syntax;
      `extended_test_that` and any look-alike call would trigger it). The open
      question is whether a *general, deterministic* rule captures the better
      outcome without any callee-awareness: **do not break an argument list to
      make room for an element that would overflow its own line anyway** --- only
      break when breaking actually reduces overflow. This would keep the
      `test_that` block hugged as pure layout logic. Risks to evaluate before
      adopting: it must suppress only the break *caused by* the unbreakable atom
      (a string/long symbol), not all breaking; and it softens the line-width
      contract, so measure the corpus fallout. If it doesn't survive, keep the
      explosion as the honest cost of determinism and leave `air_test_that`
      recorded. Fixture: `air_test_that`.

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
