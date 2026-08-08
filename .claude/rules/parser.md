---
paths:
  - "crates/arity-parser/**/*.rs"
  - "src/incremental.rs"
---

# Parser rules

Crate: `crates/arity-parser` (`syntax`, `ast`, `parser`). Re-exported by the
root crate as `arity::{syntax, ast, parser}`, so intra-repo consumers keep
writing `crate::parser::…`.

## Hard invariants

- **Losslessness.** `reconstruct(text) == text`, byte for byte — whitespace,
  comments, `%...%`, `[[`/`]]`, line endings, all of it. Every new feature needs
  a losslessness assertion.
- **Semantics stay static.** No R evaluation, ever. The parser recognizes
  lexical shape; meaning is the semantic layer's job.
- **Errors never abort the parse.** Diagnostics ride a side channel; the tree is
  always produced. Prefer a stable, recoverable CST shape over early semantic
  precision.
- **Parsing is the parser's job** (Tenet 3). If the formatter or linter trips
  over a mis-parse, fix it here — never paper over it downstream, and never let
  parsing logic creep into the formatter.
- **The crate stays dependency-thin and salsa-free**: `rowan`, `serde`,
  `smol_str`. Salsa sits *above* it, in the root crate's `src/incremental.rs`.

## Pipeline

```
lex (lexer.rs) → Vec<Token>
parse_expr (expr.rs, Pratt) + structural.rs (recursive descent) → Vec<Event>
build_tree (tree_builder.rs) → rowan SyntaxNode (CST)
```

`core::parse` drives the loop; `events.rs` defines `Event` (start node, token,
finish node); `cursor.rs`, `context.rs`, `recovery.rs`, and `diagnostics.rs`
support it. `SyntaxKind` is rowan-style `SCREAMING_SNAKE_CASE`.

`parser::expr` and `parser::roxygen` are `pub` for low-level cross-crate use but
documented as **semver-loose** — don't grow the surface casually.

## Roxygen

Roxygen is parsed, not treated as opaque comments: `parser/roxygen/`
sub-tokenizes any `^#+'` line (`lex.rs` sub-lexing, `group.rs` block grouping
plus the section/paragraph skeleton, `build.rs` block-level Rd and markdown
constructs, `inline.rs`). **The sub-tokens' texts must tile the line's bytes
exactly** — that is what keeps losslessness true. Parity work against roxygen2
has its own rules file (`roxygen.md`) and skill (`roxygen-parity`).

## Typed AST wrappers

`ast/` is a zero-cost typed **navigation** view over the CST (rust-analyzer's
mould), not a re-model.

- It is **read-only**: adding a wrapper changes no parser or formatter output,
  so losslessness and idempotence are unaffected. Keep it that way.
- R's atomic operands are **bare tokens**, not `LITERAL` nodes (`1 + 2` is
  `BINARY_EXPR { INT, PLUS, INT }`, and `TRUE`/`NA`/`NULL` are `IDENT`s
  classified by text). So operand accessors return `SyntaxElement`, and `Expr`
  (`expr.rs`) casts from a `SyntaxElement` with both node variants and
  token-atom variants — one `match Expr::cast(el)` covers any expression.
- `HasArgList` is the shared trait for `CallExpr`/`SubsetExpr`/`Subset2Expr`;
  `kinds.rs` holds shared `SyntaxKind` predicates. Reach for these before adding
  another bespoke predicate.
- Consumers (linter `matchers.rs`, semantic builder, LSP) navigate **through the
  wrappers**, not by re-walking raw CST. The **formatter deliberately stays on
  raw CST** for byte-level layout precision (Tenet 1) and is not migrated.

## Incrementality (Tenet 2)

- Keep the reparse ladder viable: token → block → top-level statement → full
  reparse (`parser/reparse.rs`). Any grammar change has to survive it.
- `syntax/ptr.rs` node pointers are **position-independent** — don't bake
  offsets into them.
- Store **green** nodes in salsa, never red: `SyntaxNode` is not `Send`/`Eq`.

## Testing

- Fixtures live in `crates/arity-parser/tests/fixtures/parser/<case>/input.R`;
  the suite snapshots the CST plus diagnostics and asserts losslessness.
- **Fixtures are hand-registered**: a case only runs once its name is in
  `fixture_names()` in `tests/parser_snapshots.rs`.
- `insta` snapshots: `cargo insta review`. Never accept a snapshot you have not
  read.
- `tests/air_parser_harness.rs` compares against the `air_r_parser` crate (git
  dev-dependency from posit-dev/air). It is a **hardening oracle**: port its
  cases in as fixtures; divergence is a question, not a build failure.
- Parser-level performance work goes through `task bench-parse` (criterion,
  `benches/parse.rs`), not the shell benchmark.
