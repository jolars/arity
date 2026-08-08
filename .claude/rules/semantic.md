---
paths:
  - "src/semantic.rs"
  - "src/semantic/**/*.rs"
  - "src/project.rs"
  - "src/project/**/*.rs"
  - "src/incremental.rs"
---

# Semantic and project rules

Two layers, deliberately separate: `src/semantic/` is **strictly single-file**,
`src/project/` is its **cross-file** counterpart. Keep cross-file logic out of
`semantic/`.

## Semantic model (single file)

- Scope tree, bindings, identifier resolution, and in-file `library()` tracking
  (`builder.rs`, `scope.rs`, `binding.rs`), plus a per-region control-flow graph
  (`cfg.rs`).
- **Semantics stay static** — no R evaluation, ever.
- `symbols.rs` resolves against package namespaces: `StaticBaseR` (R's seven
  default packages, `base_r/*.txt`) and `BundledPackages` (top-N CRAN packages
  by download count, `cran/exports.txt`).
- **Those symbol lists are generated — never hand-edit them.**
  `scripts/dump_base_symbols.R`, `scripts/dump_cran_symbols.R`, ranked by
  `scripts/rank_cran_downloads.sh`, refreshed by
  `.github/workflows/cran-symbols.yml`.

## Project layer (cross file)

- The `source()` dependency graph (`source.rs`, `sequence.rs`), an R package's
  implicit shared namespace, per-file export projection (`exports.rs`), the
  S4/R6/reference-class inheritance index (`classes.rs`), and `scope.rs`'s pure
  `ProjectScope::build`. `graph.rs` wires it into salsa.
- **Per-file projections are deliberately range-free.** Keeping spans out of
  them is what lets a projection backdate across a body edit so the project
  graph's memo survives. `tests/salsa_incremental.rs` guards this — adding a
  range to a projection will look harmless and will silently cost every
  keystroke a graph rebuild.
- `FileScope` keeps the three reasons a top-level binding is "not unused"
  **apart**, and they must stay apart: `read_elsewhere` (a sibling reads it),
  `exported_by_namespace` (public API), `is_s3_method` (reached by dispatch).
  `used_elsewhere` is the union of the first two, which is what
  `unused-binding` asks; `unused-function` asks for each separately.
- `description.rs` derives the `[compat]` floors per file from the enclosing
  `DESCRIPTION` (`Depends: R (>= …)`; `Config/roxygen2/version`, then
  `RoxygenNote`) when config sets none.

## Incrementality

- `src/incremental.rs` models file text → CST → semantic model as salsa queries.
  The parser crate itself stays salsa-free; salsa lives only above it.
- **Store green nodes in salsa, never red** (`SyntaxNode` is not `Send`/`Eq`).
- Salsa is **strictly single-writer**. Anything that writes must respect the
  LSP's lint-thread ownership (`.claude/rules/lsp.md`).
