# TODOs

## Parser

- [ ] Comment-aware suppression placement edge cases: directives inside
      `R_CALL_ARGUMENTS` between `( ... , <directive> , next_arg )` need special
      handling so they attach to `next_arg` instead of the argument list. (Jarl
      solved this by overriding biome's `place_comment`; arity's
      next-non-trivia-sibling walk already handles most cases.)
- [x] Incremental reparse (token/block) beneath `parsed_document`
      (`src/incremental.rs`): rowan-style `reparse_token` → `reparse_block` →
      full-reparse fallback (cf. rust-analyzer `reparsing.rs`), splicing reused
      green subtrees (`src/parser/reparse.rs`). `parsed_document` recovers the
      edit from the old/new text via a prefix/suffix diff and splices off a
      non-salsa per-file previous-parse cache (a pure perf hint --- a successful
      reparse is byte-identical to a full parse, so it never changes query
      output). Correctness is pinned by an oracle property test
      (`tests/incremental_reparse.rs`: `reparse == parse(new)` in tree *and*
      diagnostics across the corpus) plus a salsa-level test
      (`body_edit_uses_incremental_reparse_and_stays_correct`). On a \~100 KB
      file reparse is \~200× faster than a full parse (`benches/parse.rs`).
      Serves Tenet 2. No `SyntaxNodePtr`/`AstPtr` added (no feature needs a
      stable cross-edit reference yet). See `ARCHITECTURE_AUDIT.md` §3.4.
      - [ ] Follow-up: top-level-statement reparse (non-braced). v1 reparses
            only brace blocks + single tokens; edits elsewhere fall back to a
            full parse (correct, just not incremental). Could also use the LSP's
            precise edit ranges instead of the prefix/suffix text diff.

## Formatter

- [ ] Tibbles

- [ ] Roxygen syntax formatting

## Linter

Closest precedent: **jarl** (`etiennebacher/jarl`, Rust + rowan + air-parser,
55+ rules, suppression directives, LSP, autofix). The foundation pass borrows
shape (diagnostic + `Violation` trait, `PackageOrigin` enum, `# arity-ignore`
suppression directive style, annotate-snippets rendering) but stays its own
project --- arity is a unified formatter + linter + LSP binary on arity's own
in-tree parser, not a drop-in jarl replacement.

## Language Server

Today the server advertises **formatting** (whole-document + range), **hover**
(index-backed), **quick-fix code actions**, **pushed diagnostics**, and
**intra-file rename** (`prepareRename` + `rename`, local bindings only)
(`src/lsp.rs` `server_capabilities`). Everything else below is unimplemented.
Much of
it is closer than it looks: the per-file `SemanticModel` (`src/semantic.rs`:
scope tree, bindings, identifier *read* sites, `loaded_packages`,
`referenced_packages`) plus the salsa queries (`semantic_model`, `file_exports`,
`file_free_reads`, `source_edges`) and the harvested package index
(`src/rindex/`) already supply most of the analysis these features need --- the
work is mostly wiring resolution results to LSP responses, not new analysis.
Roughly ordered by leverage-to-effort:

### Prerequisites & blockers

There are **no hard architectural blockers** --- the parser and salsa model are
already shaped for this. A grounded audit (2026-06-11) found the load-bearing
infrastructure present and reusable:

- **Binding def spans + read-site provenance are present.** `Binding` carries
  `def_range: TextRange` (`src/semantic/binding.rs`); read sites
  (`IdentRef { name, range, scope }`, `src/semantic.rs`) resolve to a
  `BindingId` via `resolve_local`. So go-to-def, document symbols, intra-file
  references/highlight, and intra-file rename are wiring over existing data.
- **Position mapping is present.** `LineIndex` (`src/text/line_index.rs`) does
  byte↔UTF-16-`Position` conversion, already used by hover and diagnostics.
- **The read-snapshot path is present.** The lint thread owns the persistent
  db; read requests get a cheap `Analysis` snapshot (`src/incremental.rs`
  `snapshot`, dispatched in `src/lsp.rs`). New read-only features drop into the
  same path --- no new threading. (Caveat: any feature that *caches* a location
  must validate against the current version, as `hover_via_db` already does
  with its `file_text != text` check, because def `TextRange`s shift on edit.)
- **Project-level aggregation already exists in principle:** an interned
  `Project` key + `project_graph`/`visible_symbols` tracked queries aggregate
  `file_exports`/`file_free_reads`/`source_edges` across members
  (`src/project/graph.rs`).

Two genuine gaps gated the **cross-file** half of the list (both soft --- new
infra that builds *with* the grain, not architectural fights). The first has
landed; the second is still open but only matters for cross-edit-stable handles:

- [x] **Reverse `source_edges` index + an explicit workspace file-set.** Done:
      `reverse_source_edges(db, project)` (`src/project/graph.rs`) is the
      who-sources-me map (`Eq`, backdates), and the file-set is now the explicit
      salsa `Workspace` input (`src/incremental.rs`) from which the interned
      `Project` is derived by `workspace_project` — no per-request disk walk. See
      *Cross-cutting prerequisite* below for the full landed shape.
- [x] **Stable cross-edit node references.** Done, landed with its first
      consumer (intra-file rename). Three pieces: (1) rowan's typed
      same-revision handles `AstPtr`/`SyntaxNodePtr` re-exported from
      `src/ast.rs`; (2) arity's canonical `NodePtr` (`src/syntax/ptr.rs`) — a
      `(kind, range)` handle that owns its construction (rowan's is closed) and
      derives `serde`, so it can be mapped onto a new revision *and* ride an LSP
      `data` field, with a hand-written `try_to_node` via `covering_element`;
      (3) the cross-edit layer `map_range_through_edit(s)`
      (`src/parser/reparse.rs`) that shifts a stored range through an `Edit`
      (or returns `None` when the edit overlaps the node), surfaced as
      `Analysis::resolve_ptr` (`src/incremental.rs`, the db-backed re-resolution)
      and exercised live by `compute_rename_with_anchor` (`src/lsp.rs`, which
      resolves against the authoritative buffer). The `prepareRename` anchor that
      "survives typing" is the worked example; a persistent call-hierarchy item
      reuses the same `NodePtr` + `serde` form for its `data` round-trip.

### Navigation

- [x] **Go-to-definition** (`textDocument/definition`). Intra-file resolves a
      read site (or the def itself) to its local binding via the shared
      `resolve_local_target` and reports `Binding::def_range`
      (`compute_definition` / `definition_via_db`). Cross-file resolves a bare
      top-level name against the workspace `project_defs` index — its first
      consumer — via the new `Analysis::workspace_def_sites`, recovering each
      span per file with `def_range_in`. Package-export / namespaced targets have
      no in-tree location, so they return nothing and lean on hover (as planned).
- [x] **Go-to-references / find-all-references** (`textDocument/references`). The
      inverse of go-to-definition, in the same two phases. Intra-file: the cursor
      resolves to a local binding (shared `resolve_local_target`) and every
      `idents()` read of it is reported via the shared `local_occurrences`
      (`compute_references` / `references_via_db`), honoring
      `context.includeDeclaration`. Cross-file: a *file-scope* (top-level) binding
      or a bare free read is matched against the new project-wide `project_reads`
      aggregate — the read-site mirror of `project_defs`, built over the range-free
      `file_free_reads` firewall — via `Analysis::workspace_read_sites`, recovering
      each read span per file with `read_ranges_in`. Nested locals stay intra-file;
      namespaced (`pkg::name`) names have no in-tree reads.
- [x] **Document highlight** (`textDocument/documentHighlight`). The degenerate
      same-file references query, sharing `local_occurrences`
      (`compute_document_highlights`): the definition as `WRITE`, each read as
      `READ`. Pure (no workspace snapshot), so it runs straight on the read pool.
- [ ] **Go-to-declaration / type-definition / implementation**. Low priority for
      R's dynamic semantics; likely alias to definition or omit.

### Symbols

- [x] **Document symbols** (`textDocument/documentSymbol`). A hierarchical
      `DocumentSymbol` outline of the file's function and variable bindings
      (`compute_document_symbols` / `on_document_symbol`). The name set is the
      `SemanticModel`'s `Local`/`Implicit` bindings at *every* scope (the
      `file_exports` predicate lifted past file scope; params and `for`-vars
      excluded); the CST then supplies the tree and each symbol's full/selection
      spans. A binding's children are the symbols nested in its value side, and
      non-binding nodes (`if`/`for`/`{}`, which introduce no symbol) are
      descended through so every binding surfaces at the right level. Pure and
      single-file (no workspace), so it runs straight on the read pool like
      document highlight. `R6`/`setClass` shapes deferred. Kind is `FUNCTION` vs
      `VARIABLE`; `detail` (signatures) is a follow-up.
- [ ] **Workspace symbols** (`workspace/symbol`). Fuzzy name search across all
      project files. Needs a persistent, queryable symbol index keyed by name
      (aggregate `file_exports` across the workspace) plus project-wide file
      discovery driven into salsa. This is the foundational cross-file index
      that references, rename, and call hierarchy all reuse --- build it once.

### Rename

- [ ] **Rename symbol** (`textDocument/rename` + `textDocument/prepareRename`).
      Intra-file rename of a *local* binding has landed (`src/lsp.rs`
      `compute_prepare_rename`/`compute_rename`): resolve the cursor to a
      `BindingId` (read site via `resolve_local`, or the def site), collect the
      def + all in-scope reads into a `WorkspaceEdit`, validate the new name
      against R's syntactic identifier rules (`is_syntactic_r_name`), and anchor
      the prepare→rename handshake on a `NodePtr` so it survives an edit (see the
      cross-edit references prerequisite above). Cross-file rename has landed
      (`src/lsp.rs` `rename_via_db`): a file-scope binding's reads (and a bare
      workspace free read's def + reads) are gathered off the same reverse index
      as cross-file references (`workspace_read_sites`/`workspace_def_sites`) and
      returned as one multi-URI `WorkspaceEdit`. Like cross-file references, the
      index keys on *name*, so a sibling file that redefines the same top-level
      name is rewritten too. Still open: backtick-quoting of non-syntactic names,
      and renaming package-qualified names.
- [ ] **Revisit: cross-file rename is name-keyed, not binding-aware.**
      `rename_via_db` rewrites every workspace site of the *name* (mirroring
      cross-file references), so a sibling file that independently redefines the
      same top-level name is renamed along with the intended target --- a false
      positive when the two are unrelated bindings. This is inherited from the
      `project_defs`/`project_reads` reverse index, which is range-free and
      name-only. Decide whether rename should be *stricter* than references here:
      respect package/`source()` scope visibility (`ProjectScope` already models
      it) so only sites that can actually see the renamed definition are touched,
      and/or surface a conflict when a name is defined in more than one place.
      Tracked separately because it also affects cross-file references.
- [ ] **File rename** (`workspace/willRenameFiles` / `workspace/didRenameFiles`,
      advertised via `fileOperations` server capability). On an `.R` file move,
      rewrite `source("old/path.R")` string literals in dependents to the new
      path. Depends on `source_edges` already resolving those literals; needs
      the reverse edge map (who sources me) and a string-literal edit that
      preserves quoting.

### Completion & signatures

- [ ] **Completion** (`textDocument/completion` + `completionItem/resolve`).
      Scope-aware locals + library exports from the index; `pkg::` triggers
      member completion (the index already has per-package export lists with
      formals + help). `resolve` lazily attaches docs/signature. Large surface;
      probably the biggest single feature.
- [ ] **Signature help** (`textDocument/signatureHelp`). Inside a call, show the
      callee's formals/usage. The index already carries `formals` and the
      `\usage` block (same data hover renders) --- the new work is detecting
      "inside call argument N" from the CST and tracking the active parameter.

### Diagnostics & misc protocol surface

- [ ] **Pull diagnostics** (`textDocument/diagnostic` + `workspace/diagnostic`).
      The server currently *pushes* diagnostics from the lint thread; the pull
      model (LSP 3.17) lets clients request on demand and is friendlier to the
      coalescing/versioning the lint thread already does. Additive alongside
      push.
- [ ] **Semantic tokens** (`textDocument/semanticTokens`). Scope-aware
      highlighting (distinguish function calls, locals, package-qualified names,
      arguments) from the same `SemanticModel`; degrades gracefully if omitted.
      Maybe omit and rely on native editor syntax highlighting.
- [ ] **Folding ranges** (`textDocument/foldingRange`) and **selection ranges**
      (`textDocument/selectionRange`). Pure CST walks --- brace blocks, function
      bodies, call argument lists, comment runs. Cheap, no semantic model
      needed.
- [ ] **Call hierarchy** (`textDocument/prepareCallHierarchy` + incoming/
      outgoing). Caller/callee graph; rides the same cross-file reference index
      as workspace symbols and references.
- [ ] **Inlay hints** (`textDocument/inlayHint`). E.g. argument-name hints at
      call sites (matching positional args to index formals). Speculative. Not
      loved by all users, possibly opt-in or omit altogether.

### Cross-cutting prerequisite

- [x] **Workspace-wide symbol/reference index.** Done --- all three pieces
      landed, keyed on the interned `Project`: (1) an explicit, salsa-tracked
      workspace file-set, the singleton `Workspace` input at `Durability::MEDIUM`
      with a conditional setter, from which the interned `Project` is *derived* by
      the `workspace_project` query (`src/incremental.rs`, `src/project/graph.rs`)
      --- the CLI and LSP both go through it, and the LSP seeds it from
      `initialize` `workspaceFolders`/`rootUri` plus a lazy per-file backstop
      (`src/lsp.rs`, `seed_workspace_for`); (2) the reverse `source_edges` map
      `reverse_source_edges` (`Eq`, backdates; keeps `local=TRUE` and out-of-set
      targets, unlike the forward scope builder); (3) the name → def-site
      aggregate --- range-free `file_def_sites`/`DefKind` firewall +
      project-wide `project_defs`, with spans recovered per-request via
      `Analysis::def_range_in` from the fresh `semantic_model`. Backdating proofs
      in `tests/salsa_incremental.rs`. The cross-file *consumers* (workspace
      symbols, references, rename, file rename, call hierarchy) now have no index
      work left --- they sit on these queries.
      - [ ] Follow-up (model (b)): `workspace_project` still reads
            `package_root`/`NAMESPACE` from disk (model (a)), so a keystroke
            re-runs it (it backdates to the same `Project`, so the graph is
            spared). Carry per-root NAMESPACE text + package-root markers as
            salsa inputs so the query is fully pure and a future
            `didChangeWatchedFiles` watcher invalidates it correctly. Pairs with
            the `vfs`/`SourceRoot` follow-up under *Thin `FileId`*.

- [ ] Full downloadable CRAN sidecar (escalation of the bundled lists above).
      Shape: per-package export lists keyed by package version, covering the
      long tail the bundled set omits. Carries an out-of-band cost (a
      CRAN-processing pipeline + hosting + refresh cadence) the bundled lists
      avoid; add it as an additive `SymbolProvider` layer when long-tail/CI
      completeness is worth that. Would also let DESCRIPTION `Imports`/`Depends`
      feed name resolution (the `import(pkg)` case currently only marks
      resolution incomplete, in `src/project/scope.rs`). Names-only `pkg::name`
      resolution for bundled-but-not-installed packages is a smaller related
      follow-up.

- [ ] Follow-up: prune packages that vanish from CRAN out of the bundled set.
      The refresh is now **additive** --- `scripts/rank_cran_downloads.sh` unions
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
      is unchanged --- multi-root layouts (package + scripts) are governed by
      `package_root`/`ProjectScope`, not the file key. See
      `ARCHITECTURE_AUDIT.md` §3.3.
      - [ ] Follow-up: full `vfs`/`SourceRoot` model ---
            opaque-`FileId`-at-the-URI boundary in `src/lsp.rs` and
            `SourceRoot`-scoped durability --- when multi-root workspaces
            actually need it. Lower leverage for a single-crate tool (the wart
            is already gone).

## Misc

- [ ] `arity-ignore-unused` meta-diagnostic: emit a finding for suppression
      comments that didn't actually suppress anything (rule ID is reserved but
      the rule is not yet wired in).
- [ ] **Harvest lazy-data symbols.** The index now covers R's default packages
      (so hover/signatures work for base-R functions), but `harvest_package`
      only reads `NAMESPACE`/object exports --- it skips a package's lazy-data
      (`.getNamespaceInfo(ns, "lazydata")`). So `datasets` harvests 0 symbols and
      hovering a dataset (e.g. `iris`) resolves the package but finds no entry.
      The static name lists already include lazydata; the harvest does not.
