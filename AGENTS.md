# Agent Instructions

This file provides guidance to coding agents when working with code in this
repository.

## Project

Arity is a Rust CLI providing a language server, formatter, and linter for the R
language. Single-crate Cargo package (published to crates.io as `arity`, edition
2024; the binary and library crate are both named `arity`), not a workspace.

**Strategy (see `TODO.md`):** the parser + formatter foundation was brought to
near-completion *first*; the linter and LSP were then built out on top of it and
are now substantially complete. When in doubt about scope/priority, `TODO.md` is
the live roadmap and records known issues and follow-ups.

The dev environment is provided via `devenv`/Nix (`devenv.nix`, `.envrc`) and
includes `R`.

## Tenets

1. **Deterministic, rule-based formatting.** Output is decided solely by the
   formatter's rules and the layout engine. Push back against attempts to
   hard-code special cases or exceptions for specific constructs. Unlike air
   (arity's closest relative), arity does **not** honor "persistent line
   breaks"—the input's existing line breaks never influence the result. Because the
   formatter is the **sole authority on layout**, autofixes are textual edits
   that never invoke it: a fix decides *what* to rewrite, never *how to lay it
   out*. Producing well-formatted output after a fix is a separate format pass's
   job, not the fixer's (see the autofix-correctness note under the linter).
2. **Incremental parsing is first-class**, not an afterthought. Parser/CST work
   must keep the `salsa`-based incremental reparse path (`src/incremental.rs`)
   viable.
3. **Parsing is the parser's job.** Never paper over parser mistakes in the
   formatter, and never let parsing logic creep into the formatter. If the
   formatter hits something the parser handled wrong, fix it in the parser.
4. **Losslessness is the parser's job.** The parser must preserve all text
   (whitespace, comments, etc.) so that `reconstruct(text)` is always `text`.
   The formatter can assume the CST is lossless and focus on formatting logic.

## Air compatibility (soft target)

Arity tracks a **soft, one-directional compatibility target** with the `air`
formatter (a la ruff's "% Black-compatible" number)—but this is **strictly
subordinate to Tenet 1** and is **never a quality gate**. We do not match air;
we measure how often air would leave arity's output unchanged, and treat air's
maturity as a free differential oracle for finding our own inconsistencies.

- The gauge lives in `tests/air_compat.rs` (`#[ignore]`d, so it never runs in
  `cargo test` and cannot fail CI). Run it with `task air-compat`; it
  regenerates `AIR_COMPAT.md`.
- It measures the *fixed point* `air(arity(x)) == arity(x)`, not a head-to-head
  diff—this cancels out the persistent-line-break difference (which is the
  whole point of arity) by construction, leaving only genuine rule divergences.
- Divergences are triaged into two buckets. **Adopt** when air's output is
  simply more idiomatic and arity is being inconsistent (fix the rule).
  **Record** when the divergence is a deliberate arity choice—add it to
  `tests/air_compat_allowlist.toml` with a rationale.
- Diverging from air is allowed, but should **raise tension**: it is a
  conscious, documented decision (an allowlist entry), not a silent one. An
  unexplained divergence in `AIR_COMPAT.md` is an open question, never an excuse
  to fail a build.

## Commands

```sh
cargo build                       # dev build
cargo build --release
cargo test                        # all tests (CI: cargo test --verbose)
cargo test <substring>            # run tests matching a name
cargo test --test parser_snapshots   # one integration test file (also: formatter, lint, ast_wrappers, salsa_incremental, line_endings, air_parser_harness, lsp_protocol)
cargo clippy --all-targets --all-features -- -D warnings   # lint; warnings are errors
cargo fmt -- --check              # rustfmt check (keep changes rustfmt-clean)
```

CLI usage:

```sh
cargo run -- parse <file.R>                  # print CST; stdin if no file
cat file.R | cargo run -- parse --verify --quiet   # losslessness round-trip check
cargo run -- format <file.R>                 # format to stdout (stdin if omitted)
cargo run -- format --check <path>           # check without writing (multi-path requires --check)
cargo run -- format --verify <file.R>        # check idempotence; does not write
cargo run -- lint <path>                     # lint (stdin if no path); exits 1 on findings
```

The documentation site (`docs/`) is an mdBook. Its reference pages are
generated: `build.rs` writes `docs/src/reference/cli.md` from the clap CLI, and
`cargo run --example docgen` renders the per-rule pages (and `version.md`) by
running the real linter on each rule's examples. `mdbook build docs` then builds
the site; `.github/workflows/docs.yml` deploys it to GitHub Pages. The rendered
rule docs are pinned by `tests/rule_docs.rs` so they can't drift from behavior.

Snapshot tests use `insta`: review/accept with `cargo insta review` or
`cargo insta accept`. Logging honors `RUST_LOG` (e.g.
`RUST_LOG=debug cargo test`) via `env_logger`. `task <name>` (Taskfile.yml)
wraps the above: `lint`, `format`, `test`, `test-debug`, `audit`, `deny`,
`docs-gen`, `docs-build`, `docs-preview`.

## Architecture

**Parse pipeline** (`src/parser/`, public API `parse`/`reconstruct` re-exported
from `src/parser.rs`): lossless `rowan` CST built via an event-based pipeline.

```
lex (lexer.rs) → Vec<Token>
parse_expr (expr.rs, Pratt) + structural.rs (recursive descent) → Vec<Event>
build_tree (tree_builder.rs) → rowan SyntaxNode (CST)
```

- `core::parse` drives the loop; `events.rs` defines `Event` (start node, token,
  and finish node); `cursor.rs`, `context.rs`, `recovery.rs`, `diagnostics.rs`
  support the parser. `src/syntax.rs` defines `SyntaxKind` (rowan-style
  `SCREAMING_SNAKE_CASE`).
- **Losslessness is the core invariant:** all whitespace, newlines, comments,
  and `%...%`/`[[`/`]]` tokens are preserved; `reconstruct(text)` must equal
  `text`. Parser work prioritizes stable, recoverable CST shape over early
  semantic precision. Semantics stay **static**—no R evaluation.
- The **AST-wrapper layer** (`src/ast/`) is a zero-cost typed *navigation* view
  over the CST, in rust-analyzer's mould: `AstNode` wrappers (`nodes.rs`) type
  each node kind (`AssignmentExpr`, `IfExpr`, `FunctionExpr`, …) and `AstToken`
  wrappers (`tokens.rs`, arity's own trait—rowan ships no `AstToken`) type each
  leaf kind (`Ident`, `StringLit`, `IntLit`, …). Because R's atomic operands are
  **bare tokens**, not `LITERAL` nodes—`1 + 2` is `BINARY_EXPR { INT, PLUS,
  INT }`, and every special constant (`TRUE`, `NA`, `NULL`, …) is an `IDENT`
  classified by text—accessors over operands return `SyntaxElement`
  (node-or-token), and the `Expr` union (`expr.rs`) casts from a `SyntaxElement`
  with both node variants and token-atom variants (`Name`, `IntLiteral`, …) so a
  single `match Expr::cast(el)` covers any expression. `HasArgList` is the shared
  trait for the argument-bearing nodes (`CallExpr`/`SubsetExpr`/`Subset2Expr`);
  `kinds.rs` holds shared `SyntaxKind` predicates. The linter's `matchers.rs` and
  the other consumers (semantic model builder, LSP) navigate through these
  wrappers rather than re-walking raw CST. The **formatter deliberately stays on
  raw CST** (byte-level layout precision, Tenet 1) and is not migrated. This is a
  read-only layer—it changes no parser or formatter output, so losslessness and
  idempotence are unaffected; `tests/ast_wrappers.rs` is its integration-test
  home.
- `src/incremental.rs` models file text → tokens → events → CST as `salsa`
  queries for incremental reparse.

**Formatter** (`src/formatter/`, public API in `src/formatter.rs`): consumes the
CST and uses a Wadler/Prettier-style document IR (`ir.rs`) printed by a single
best-fit layout engine (`printer.rs`) that makes all line-break decisions.
`rules/` builds the IR per construct; `core.rs` exposes `format` and
`format_with_style`; `check.rs` exposes `check_paths`; `style.rs` is
`FormatStyle`; `trivia.rs`/`context.rs`/`render.rs` are support. Target style is
the tidyverse R style guide. The native-IR migration is complete
(subset/call/function arg-lists, curly-curly, parens, if/else including
comment-bearing chains, and external-body control flow all build native IR, with
comment relocation handled structurally). `Ir::verbatim`/`verbatim_forced` no
longer bridges a whole construct; it now only carries relocated comment text and
rendered-flat `conditional_group` candidates.

**Linter** (`src/linter/`): `check_paths` walks files, parses, and reports
`LintStatus` (`Clean`/`Findings`/`ParseDiagnostics`); parse diagnostics
block linting a file. Ships 26 rules across five categories (correctness,
suspicious, readability, performance, documentation) with autofixes, suppression
handling, and generated per-rule docs; `src/linter/rules.rs` is the registry.

*Autofix correctness.* A fix is a textual edit, so the bar it must clear is
**correctness, not formatting**: applying it must leave code that still parses
and is still lossless—never broken syntax (e.g. a negating rewrite that
misbinds, `!a + b`) or dropped trivia (e.g. a relocation that loses a comment).
When an edit can't meet that bar for some shape, make it correct by construction
(tight span, atom-guarded) or **withhold the fix for that shape**—the finding
is still reported. A fix does **not** owe line-width: it may leave a line the
formatter would re-break, because layout is the formatter's job (Tenet 1), and
the intended pipeline is fix-then-format, not fix-alone. The withhold/atom-guard
discipline is what keeps the current rules' fixes safe; `tests/lint.rs` checks
that fixed output parses (and stays format-clean on the curated width-safe
cases).

**Language server** (`src/lsp.rs`, a facade over the `src/lsp/*` submodules; CLI
`arity lsp`): a stdio JSON-RPC server on the `lsp-server` crate (rust-analyzer's
transport)—offers formatting (document and range), pushed and pull diagnostics,
quick-fix code actions, hover, completion, signature help, go-to-definition and
references, rename, document and workspace symbols, semantic tokens, folding and
selection ranges, document links, and call and type hierarchy, plus on-disk
change detection via dynamically-registered `workspace/didChangeWatchedFiles`
watchers (`arity.toml`, `DESCRIPTION`, `NAMESPACE`, `.R`; see
`src/lsp/watched_files.rs`) and `workspace/didChangeWorkspaceFolders`, backed by
the introspection index and a per-file semantic model. The main loop owns
no salsa database: read-only requests run on a purpose-built read `TaskPool` (not
rayon's global pool), and linting is serialized
on a **dedicated thread** that owns the persistent `IncrementalDatabase`. This is
forced by salsa being strictly single-writer (a `set_*` setter blocks until all
other db handles drop) combined with cross-file lint *writing* sibling files into
the db—so lint can't run on a shared read snapshot. The lint thread
*coalesces* requests (latest version per URI wins) in lieu of a debounce. See the
module doc for the full rationale.

**File discovery** (`src/file_discovery.rs`): `collect_r_files` walks paths for
`.R` files (via `ignore`); rejects non-`.R` explicit file paths.

## Invariants & conventions

- Treat CI as the source of truth for quality gates (`.github/workflows/`):
  cross-platform build/test, `cargo-audit` + `cargo-deny`, clippy `-D warnings`,
  and the rustfmt check.
- Formatter output must be **idempotent** (`format(format(x)) == format(x)`);
  the formatter and parser test suites guard losslessness +
  idempotence—byte-identical output is the bar for "behavior-preserving"
  refactors.
- Dependency changes must stay compatible with `cargo-audit` and `cargo-deny`
  (`deny.toml`).

## Commits & versioning

- **Conventional Commits** (`type(scope): subject`) and **semantic versioning**.
- Subject line: aim for ≤ 60 chars, ≤ 72 is fine, longer only if truly needed.
- Bodies are short and to the point.
- **Never edit the changelog by hand**—`versionary` generates it.

## Testing layout

**Use test-driven development.** Write the test first, watch it fail, then make
it pass. For a bug, always start by adding a failing test that reproduces it
(typically a new fixture case or snapshot) before touching the fix.

- Integration tests in `tests/*.rs`; fixtures in
  `tests/fixtures/{parser,formatter}/<case>/`. Parser fixtures hold `input.R`
  (snapshot the CST + diagnostics, assert losslessness); formatter fixtures hold
  `input.R` + `expected.R`.
- `insta` snapshots live in `tests/snapshots/`.
- Both fixture suites are **hand-registered**: a new case only runs once its
  name is added to `fixture_names()` in `tests/parser_snapshots.rs` /
  `tests/formatter.rs`.
- `tests/air_parser_harness.rs` compares against the `air_r_parser` crate (a git
  dev-dependency from posit-dev/air)—AIR snapshot cases are ported into the
  parser fixtures as hardening input.
- `tests/corpus.rs` is the Tier 0 corpus smoke test (`#[ignore]`d; run with
  `ARITY_CORPUS=<dir> task corpus`): losslessness + idempotence over a large
  body of real R sources, with unparseable files skipped rather than failed.
  `.github/workflows/smoke-test.yml` runs it weekly over cloned R package repos
  and files one deduped issue per (repo, failure category); triage those with
  the `smoke-test-triage` skill.

## Reference-only directories (not part of the build, untracked)

- `air/`—a local checkout of posit-dev/air (tree-sitter-based R tooling)
  kept for reference/comparison. **Not** in the Cargo build and not exposed via
  this CLI. It has its own `air/CLAUDE.md` describing *that* project's
  conventions (e.g. `just test`, `air.toml`)—do not apply those to arity.
- `style/`—vendored copy of the tidyverse R style guide (the formatter's
  target style).
