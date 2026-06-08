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

- [ ] LSP refinements: `initializationOptions` /
      `workspace/didChangeConfiguration` are now honored for `line-width` /
      `indent-width` (a discovered `ravel.toml` wins; editor settings are the
      fallback). Still pending: `textDocument/rangeFormatting`, once the
      formatter gains a range API. (`textDocument/codeAction` QuickFix hooks
      shipped alongside autofix --- see Phase 6.x autofix above.)
- [ ] CRAN-wide symbol manifest as a downloadable sidecar. Shape: per-package
      export lists keyed by package version. With a manifest in place, enable
      `undefined-symbol` by default and stop returning `Unknown` for names from
      `library()`-attached packages. Would also let DESCRIPTION `Imports`/
      `Depends` feed name resolution (the `import(pkg)` case currently only
      marks resolution incomplete, in `src/project/scope.rs`).
- [x] DESCRIPTION / NAMESPACE parsing for R-package authoring contexts. NAMESPACE
      `export()`/`exportPattern()`, `importFrom()`, and `import()` are parsed
      (`rindex::harvest::parse_namespace`) and folded into cross-file resolution
      (`src/project/scope.rs`): exported bindings aren't flagged `unused-binding`,
      `importFrom` names resolve, and `import(pkg)` suppresses `undefined-symbol`.
- [x] Cross-file scope awareness: a binding defined in `a.R` resolves from `b.R`
      when both belong to the same package (shared `R/` namespace) or `source()`
      closure. Implemented in `src/project/` (`ProjectScope`) and wired into both
      the batch linter and the LSP (`check_document_in_project`).
- [x] Salsa-cached `semantic_model` query in `src/incremental.rs`. The CST is now
      cached as a `rowan::GreenNode` (via `no_eq, unsafe(non_update_types)`) and
      `semantic_model` is a tracked query; the linter and LSP reuse them instead
      of re-parsing from text.
- [ ] Cross-file follow-ups: wrap the project scope as tracked salsa queries
      (`file_exports` firewall, `source_edges`, `project_graph`, `visible_symbols`)
      so a body edit doesn't rebuild the whole project scope; today
      `ProjectScope` is recomputed per lint over cached per-file parses.
- [x] LSP read-path: hover/formatting reuse the salsa db. The lint thread (db
      owner) mints a short-lived clone per read job and runs it on rayon
      (`run_read`), formatting/hovering off the cached parse tree when the tracked
      buffer matches the live text; a cache miss or a `salsa::Cancelled` from a
      racing write falls back to a fresh parse. Code actions are served from the
      last lint's findings (cached per URI by version) with no re-lint on a
      version match. `IncrementalDatabase` is now `Clone` (shared storage handle).
- [x] LSP read-path follow-up: preemptive lint cancellation. The lint is split
      into a write-phase (`prepare_document_in_project`, `&mut db`, on the lint
      thread) and a read-phase (`analyze_prepared`, `&db` only) that now runs on a
      rayon worker holding a db clone, wrapped in `salsa::Cancelled::catch`. The
      lint thread returns to its `select!` right after the cheap write-phase, so
      reads are no longer delayed behind a long lint. A strictly-newer edit of the
      *same* URI calls `db.trigger_cancellation()` to unwind the in-flight analyze
      (a `decide` scheduler keeps at most one in flight and never cross-cancels a
      different URI, so multi-URI `RelintAll` still publishes every file).
- [x] Honor editor-supplied `initializationOptions` /
      `workspace/didChangeConfiguration` for `line-width` / `indent-width`.
      Editor settings are the *fallback*: a discovered `ravel.toml` is
      authoritative and ignores them. Parsed in `src/lsp.rs` (`EditorSettings`),
      applied via `resolve_format_style`; a config change clears the resolution
      cache so the next pull picks up the new fallback.
- [ ] Range formatting (`textDocument/rangeFormatting`) once the formatter gains
      a range API.
- [ ] Add parse performance and incremental-reparse benchmarks.

## Misc

- [ ] `ravel-ignore-unused` meta-diagnostic: emit a finding for suppression
      comments that didn't actually suppress anything (rule ID is reserved but
      the rule is not yet wired in).
