# TODOs

## Parser

- [ ] Comment-aware suppression placement edge cases: directives inside
      `R_CALL_ARGUMENTS` between `( ... , <directive> , next_arg )` need special
      handling so they attach to `next_arg` instead of the argument list. (Jarl
      solved this by overriding biome's `place_comment`; ravel's
      next-non-trivia-sibling walk already handles most cases.)

## Formatter

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
