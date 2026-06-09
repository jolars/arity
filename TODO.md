# TODOs

## Parser

- [ ] Comment-aware suppression placement edge cases: directives inside
      `R_CALL_ARGUMENTS` between `( ... , <directive> , next_arg )` need special
      handling so they attach to `next_arg` instead of the argument list. (Jarl
      solved this by overriding biome's `place_comment`; ravel's
      next-non-trivia-sibling walk already handles most cases.)
- [ ] Incremental reparse (token/block) beneath `parsed_document`
      (`src/incremental.rs`): rowan-style `reparse_token` → `reparse_block` →
      full-reparse fallback (cf. rust-analyzer `reparsing.rs`), splicing reused
      green subtrees. Serves Tenet 2; **benchmark first** — folds into the open
      "incremental-reparse benchmarks" item under Language Server. Add
      `SyntaxNodePtr`/`AstPtr` only when a feature needs a stable cross-edit
      reference (none does today). See `ARCHITECTURE_AUDIT.md` §3.4.

## Formatter

- [x] Native-IR migration tail. The Wadler-IR migration is complete for
      `if`/`else`: comment relocation is built natively (no string bridge), the
      eligibility gate and all legacy if/else string renderers are deleted, and
      with them the entire legacy line-rendering subsystem (`format_line`,
      `format_expr_with_optional_comment`, `format_block_expr_with_prefixed_comments`,
      `FormatLineFn`, `indent_text`). A too-wide bare value-position branch now
      braces (air-aligned) instead of wrapping unbraced --- see
      `if_value_position_wide_bare_braces` / `if_else_wide_bare_branches` /
      `if_comment_wide_branch`.
  - [ ] **Function-body re-render hack still required** (`functions.rs`,
        bare-body branch). Comment-bearing if/else no longer bakes indent, but the
        `if`/`while` **condition** is still spliced as a baked-indent `Ir::verbatim`
        (`control_flow.rs` `ir_if_expr_impl`/`try_format_if_with_external_body`).
        A wide condition in a bare function body wraps at the build indent, so the
        body must be re-rendered at `indent + 1` when brace-wrapped --- guarded by
        `function_body_wide_if_condition`. Removing the hack is blocked on
        migrating the condition splice to native IR. Calls/functions with comment
        relocation may also still bake indent --- audit before removing.
  - Air divergence recorded: ravel hugs an over-width `if` condition to `if (` and
    leaves a bare consequence un-braced (Tenet 1); air breaks the condition onto
    its own line and braces it. See `tests/air_compat_allowlist.toml`.

## Linter

Closest precedent: **jarl** (`etiennebacher/jarl`, Rust + rowan + air-parser,
55+ rules, suppression directives, LSP, autofix). The foundation pass borrows
shape (diagnostic + `Violation` trait, `PackageOrigin` enum, `# ravel-ignore`
suppression directive style, annotate-snippets rendering) but stays its own
project --- ravel is a unified formatter + linter + LSP binary on ravel's own
in-tree parser, not a drop-in jarl replacement.

## Language Server

- [x] LSP refinements: `initializationOptions` /
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
- [x] DESCRIPTION / NAMESPACE parsing for R-package authoring contexts.
      NAMESPACE `export()`/`exportPattern()`, `importFrom()`, and `import()` are
      parsed (`rindex::harvest::parse_namespace`) and folded into cross-file
      resolution (`src/project/scope.rs`): exported bindings aren't flagged
      `unused-binding`, `importFrom` names resolve, and `import(pkg)` suppresses
      `undefined-symbol`.
- [x] Cross-file scope awareness: a binding defined in `a.R` resolves from `b.R`
      when both belong to the same package (shared `R/` namespace) or `source()`
      closure. Implemented in `src/project/` (`ProjectScope`) and wired into
      both the batch linter and the LSP (`check_document_in_project`).
- [x] Salsa-cached `semantic_model` query in `src/incremental.rs`. The CST is
      now cached as a `rowan::GreenNode` (via `no_eq, unsafe(non_update_types)`)
      and `semantic_model` is a tracked query; the linter and LSP reuse them
      instead of re-parsing from text.
- [x] Cross-file follow-ups: the project scope is now tracked salsa queries.
      Per-file firewalls (`file_exports`, `file_free_reads`, `source_edges` in
      `src/incremental.rs`, returning `Eq` values so a body edit backdates) feed
      `project_graph` + `visible_symbols` (`src/project/graph.rs`), keyed on an
      interned `Project` membership snapshot. A function-body edit no longer
      rebuilds the project graph (`SourceFile` gained a `path` field; the range
      is dropped via `SourceEdgeKey` so the graph input stays `salsa::Update`).
      Guarded by `body_edit_does_not_rebuild_project_scope` and friends.
- [x] LSP read-path: hover/formatting reuse the salsa db. The lint thread (db
      owner) mints a short-lived clone per read job and runs it on rayon
      (`run_read`), formatting/hovering off the cached parse tree when the
      tracked buffer matches the live text; a cache miss or a `salsa::Cancelled`
      from a racing write falls back to a fresh parse. Code actions are served
      from the last lint's findings (cached per URI by version) with no re-lint
      on a version match. `IncrementalDatabase` is now `Clone` (shared storage
      handle).
- [x] LSP read-path follow-up: preemptive lint cancellation. The lint is split
      into a write-phase (`prepare_document_in_project`, `&mut db`, on the lint
      thread) and a read-phase (`analyze_prepared`, `&db` only) that now runs on
      a rayon worker holding a db clone, wrapped in `salsa::Cancelled::catch`.
      The lint thread returns to its `select!` right after the cheap
      write-phase, so reads are no longer delayed behind a long lint. A
      strictly-newer edit of the *same* URI calls `db.trigger_cancellation()` to
      unwind the in-flight analyze (a `decide` scheduler keeps at most one in
      flight and never cross-cancels a different URI, so multi-URI `RelintAll`
      still publishes every file).
- [x] Honor editor-supplied `initializationOptions` /
      `workspace/didChangeConfiguration` for `line-width` / `indent-width`.
      Editor settings are the *fallback*: a discovered `ravel.toml` is
      authoritative and ignores them. Parsed in `src/lsp.rs` (`EditorSettings`),
      applied via `resolve_format_style`; a config change clears the resolution
      cache so the next pull picks up the new fallback.
- [x] Range formatting (`textDocument/rangeFormatting`) once the formatter gains
      a range API.
- [ ] Add parse performance and incremental-reparse benchmarks. (Prereq for the
      token/block incremental-reparse work under Parser.)
- [ ] Type-level read/write split (rust-analyzer `Analysis`/`AnalysisHost`):
      wrap `IncrementalDatabase` in an `Analysis` newtype exposing only `&self`
      read queries, handed to read jobs (`run_read`, the analyze worker), keeping
      the `&mut` handle private to the lint worker. Makes "lint thread is the
      sole writer" a compile-time guarantee instead of a convention. Files:
      `src/incremental.rs`, `src/lsp.rs`, `src/linter/check.rs`. See
      `ARCHITECTURE_AUDIT.md` §3.1.
- [ ] Salsa durability for rarely-changing inputs: set `Durability::HIGH` on
      installed-package exports / NAMESPACE / DESCRIPTION inputs so a keystroke
      (LOW write) skips revalidating the library subgraph. Longer-term, model
      library symbols as HIGH-durability salsa queries instead of the external
      `Arc<CompositeProvider>`. Dovetails with the CRAN-manifest item above.
      Files: `src/incremental.rs`, `src/rindex/provider.rs`,
      `src/project/graph.rs`. See `ARCHITECTURE_AUDIT.md` §3.2.
- [ ] `FileId` / VFS abstraction: replace direct `PathBuf` keys + the synthetic
      `<mem>/{uuid}.R` hack with an opaque `FileId` + a small file-source map
      (rust-analyzer `vfs`/`SourceRoot` model), so paths/cwd don't leak into the
      analysis and `SourceRoot`-scoped durability becomes possible. Files:
      `src/incremental.rs`, `src/project/graph.rs`, `src/lsp.rs` (URI↔FileId
      boundary). See `ARCHITECTURE_AUDIT.md` §3.3.

## Misc

- [ ] `ravel-ignore-unused` meta-diagnostic: emit a finding for suppression
      comments that didn't actually suppress anything (rule ID is reserved but
      the rule is not yet wired in).
