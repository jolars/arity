# Agent Instructions

This file provides guidance to coding agents when working with code in this
repository.

## Project

Ravel is a Rust CLI providing a language server (planned), formatter, and linter
for the R language. Single-crate Cargo package (`ravel`, edition 2024), not a
workspace.

**Strategy (see `TODO.md`):** bring the parser + formatter foundation to
near-completion *first*; the linter and LSP are deferred to later phases. When
in doubt about scope/priority, `TODO.md` is the live roadmap and records known
issues and follow-ups.

The dev environment is provided via `devenv`/Nix (`devenv.nix`, `.envrc`) and
includes `R`.

## Tenets

1. **Deterministic, rule-based formatting.** Output is decided solely by the
   formatter's rules and the layout engine. Push back against attempts to
   hard-code special cases or exceptions for specific constructs. Unlike air
   (ravel's closest relative), ravel does **not** honor "persistent line breaks"
   --- the input's existing line breaks never influence the result.
2. **Incremental parsing is first-class**, not an afterthought. Parser/CST work
   must keep the `salsa`-based incremental reparse path (`src/incremental.rs`)
   viable.
3. **Parsing is the parser's job.** Never paper over parser mistakes in the
   formatter, and never let parsing logic creep into the formatter. If the
   formatter hits something the parser handled wrong, fix it in the parser.
4. **Losslessness is the parser's job.** The parser must preserve all text
   (whitespace, comments, etc.) so that `reconstruct(text)` is always `text`.
   The formatter can assume the CST is lossless and focus on formatting logic.
5. **Autofixes never introduce formatting errors.** A lint fix is not a
   formatter and need not fully format unformatted input, but it must never
   make formatted code unformatted: `format` → `lint --fix` → `format --check`
   must pass. This is a correctness property of each fix, enforced by an
   automated test (`tests/lint.rs`), not by formatting at fix time. When a fix
   would introduce a formatting error, fix it in the rule --- make the edit
   format-clean by construction, or withhold the fix for that shape (the
   finding is still reported). Don't run the formatter inside `--fix`.

## Air compatibility (soft target)

Ravel tracks a **soft, one-directional compatibility target** with the `air`
formatter (a la ruff's "% Black-compatible" number) --- but this is **strictly
subordinate to Tenet 1** and is **never a quality gate**. We do not match air;
we measure how often air would leave ravel's output unchanged, and treat air's
maturity as a free differential oracle for finding our own inconsistencies.

- The gauge lives in `tests/air_compat.rs` (`#[ignore]`d, so it never runs in
  `cargo test` and cannot fail CI). Run it with `task air-compat`; it regenerates
  `AIR_COMPAT.md`.
- It measures the *fixed point* `air(ravel(x)) == ravel(x)`, not a head-to-head
  diff --- this cancels out the persistent-line-break difference (which is the
  whole point of ravel) by construction, leaving only genuine rule divergences.
- Divergences are triaged into two buckets. **Adopt** when air's output is simply
  more idiomatic and ravel is being inconsistent (fix the rule). **Record** when
  the divergence is a deliberate ravel choice --- add it to
  `tests/air_compat_allowlist.toml` with a rationale.
- Diverging from air is allowed, but should **raise tension**: it is a conscious,
  documented decision (an allowlist entry), not a silent one. An unexplained
  divergence in `AIR_COMPAT.md` is an open question, never an excuse to fail a
  build.

## Commands

```sh
cargo build                       # dev build
cargo build --release
cargo test                        # all tests (CI: cargo test --verbose)
cargo test <substring>            # run tests matching a name
cargo test --test parser_snapshots   # one integration test file (also: formatter, lint, ast_wrappers, salsa_incremental, line_endings, air_parser_harness)
cargo clippy --all-targets --all-features -- -D warnings   # lint; warnings are errors
cargo fmt -- --check              # rustfmt check (keep changes rustfmt-clean)
```

CLI usage (also how CI checks `docs/`):

```sh
cargo run -- parse <file.R>                  # print CST; stdin if no file
cat file.R | cargo run -- parse --verify --quiet   # losslessness round-trip check
cargo run -- format <file.R>                 # format to stdout (stdin if omitted)
cargo run -- format --check docs/            # check without writing (multi-path requires --check)
cargo run -- format --verify <file.R>        # check idempotence; does not write
cargo run -- lint --check docs/              # lint currently REQUIRES --check
```

Snapshot tests use `insta`: review/accept with `cargo insta review` /
`cargo insta accept`. Logging honors `RUST_LOG` (e.g.
`RUST_LOG=debug cargo test`) via `env_logger`. `task <name>` (Taskfile.yml)
wraps the above: `lint`, `format`, `test`, `test-debug`, `audit`, `deny`,
`docs-preview`.

## Architecture

**Parse pipeline** (`src/parser/`, public API `parse`/`reconstruct` re-exported
from `src/parser.rs`): lossless `rowan` CST built via an event-based pipeline.

```
lex (lexer.rs) → Vec<Token>
parse_expr (expr.rs, Pratt) + structural.rs (recursive descent) → Vec<Event>
build_tree (tree_builder.rs) → rowan SyntaxNode (CST)
```

- `core::parse` drives the loop; `events.rs` defines `Event` (start node / token
  / finish node); `cursor.rs`, `context.rs`, `recovery.rs`, `diagnostics.rs`
  support the parser. `src/syntax.rs` defines `SyntaxKind` (rowan-style
  `SCREAMING_SNAKE_CASE`).
- **Losslessness is the core invariant:** all whitespace, newlines, comments,
  and `%...%`/`[[`/`]]` tokens are preserved; `reconstruct(text)` must equal
  `text`. Parser work prioritizes stable, recoverable CST shape over early
  semantic precision. Semantics stay **static** --- no R evaluation.
- `src/ast/nodes.rs` (`src/ast.rs`) provides zero-cost typed AST wrappers over
  the CST using rowan's `AstNode` support (e.g. `AssignmentExpr`, `IfExpr`,
  `FunctionExpr`).
- `src/incremental.rs` models file text → tokens → events → CST as `salsa`
  queries for incremental reparse.

**Formatter** (`src/formatter/`, public API in `src/formatter.rs`): consumes the
CST and uses a Wadler/Prettier-style document IR (`ir.rs`) printed by a single
best-fit layout engine (`printer.rs`) that makes all line-break decisions.
`rules/` builds the IR per construct; `core.rs` exposes `format` /
`format_with_style`; `check.rs` exposes `check_paths`; `style.rs` is
`FormatStyle`; `trivia.rs`/`context.rs`/`render.rs` are support. Target style is
the tidyverse R style guide. Note (per `TODO.md` follow-ups):
subset/call/function arg-lists are still bridged into the IR via `Verbatim`
using legacy string-based wrapping, pending native IR re-implementation.

**Linter** (`src/linter/`): `check_paths` walks files, parses, and reports
`LintStatus` (`Clean` / `Findings` / `ParseDiagnostics`); parse diagnostics
block linting a file. Largely a placeholder ahead of Phase 6.

**File discovery** (`src/file_discovery.rs`): `collect_r_files` walks paths for
`.R` files (via `ignore`); rejects non-`.R` explicit file paths.

## Invariants & conventions

- Treat CI as the source of truth for quality gates (`.github/workflows/`):
  cross-platform build/test, `cargo-audit` + `cargo-deny`, clippy `-D warnings`,
  rustfmt check, and `format --check docs/` + `lint --check docs/`.
- Formatter output must be **idempotent** (`format(format(x)) == format(x)`);
  the formatter and parser test suites guard losslessness + idempotence ---
  byte-identical output is the bar for "behavior-preserving" refactors.
- Dependency changes must stay compatible with `cargo-audit` and `cargo-deny`
  (`deny.toml`).

## Commits & versioning

- **Conventional Commits** (`type(scope): subject`) and **semantic versioning**.
- Subject line: aim for ≤ 60 chars, ≤ 72 is fine, longer only if truly needed.
- Bodies are short and to the point.
- **Never edit the changelog by hand** --- `versionary` generates it.

## Testing layout

**Use test-driven development.** Write the test first, watch it fail, then make
it pass. For a bug, always start by adding a failing test that reproduces it
(typically a new fixture case or snapshot) before touching the fix.

- Integration tests in `tests/*.rs`; fixtures in
  `tests/fixtures/{parser,formatter}/<case>/`. Parser fixtures hold `input.R`
  (snapshot the CST + diagnostics, assert losslessness); formatter fixtures hold
  `input.R` + `expected.R`.
- `insta` snapshots live in `tests/snapshots/`.
- `tests/air_parser_harness.rs` compares against the `air_r_parser` crate (a git
  dev-dependency from posit-dev/air) --- AIR snapshot cases are ported into the
  parser fixtures as hardening input.

## Reference-only directories (not part of the build, untracked)

- `air/` --- a local checkout of posit-dev/air (tree-sitter-based R tooling)
  kept for reference/comparison. **Not** in the Cargo build and not exposed via
  this
  CLI. It has its own `air/CLAUDE.md` describing *that* project's conventions
       (e.g. `just test`, `air.toml`) --- do not apply those to ravel.
- `style/` --- vendored copy of the tidyverse R style guide (the formatter's
  target style).
- `docs/` --- Quarto site (`task docs-preview`); CI formats/lints it as a
  corpus.
