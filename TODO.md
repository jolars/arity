# TODOs

## Parser

- [ ] Comment-aware suppression placement edge cases: directives inside
      `R_CALL_ARGUMENTS` between `( ... , <directive> , next_arg )` need special
      handling so they attach to `next_arg` instead of the argument list. (Jarl
      solved this by overriding biome's `place_comment`; ravel's
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
shape (diagnostic + `Violation` trait, `PackageOrigin` enum, `# ravel-ignore`
suppression directive style, annotate-snippets rendering) but stays its own
project --- ravel is a unified formatter + linter + LSP binary on ravel's own
in-tree parser, not a drop-in jarl replacement.

## Language Server

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

- [ ] `ravel-ignore-unused` meta-diagnostic: emit a finding for suppression
      comments that didn't actually suppress anything (rule ID is reserved but
      the rule is not yet wired in).
