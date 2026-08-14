# Agent Instructions

This file provides guidance to coding agents when working with code in this
repository. It carries the things that are true everywhere: what arity is, the
tenets, the commands, and the cross-cutting invariants.

**Per-subsystem directives live in `.claude/rules/*.md`**, path-scoped in
frontmatter so each loads only when you read that subsystem's files: `parser`,
`formatter`, `linter`, `lsp`, `semantic` (semantic + project layers), `rindex`,
`roxygen`, `config`, `docs` (docs site + benchmarks), and `release` (packaging,
workflows, versioning).

Keep each rule file terse and under 200 lines: a rule, the one clause that keeps
it from looking arbitrary, and a pointer. A rule that must hold *before* any
file is read belongs here instead — path-scoped rules only load once Claude
reads a matching file, and they are not re-injected after a compaction.

Worked examples and issue archaeology belong in neither file: they live in the
issue tracker, in `git log`, and above all in named tests and fixtures, which
are what fails when a rule is violated. `TODO.md` is the live roadmap and
records known issues and follow-ups; when in doubt about scope or priority,
read it.

## What this project is

Arity is a Rust CLI providing a language server, formatter, and linter for the R
language. It is a Cargo workspace (edition 2024) whose root package, `arity`
(binary *and* library, published to crates.io), hosts the CLI, LSP, linter,
semantic model, project graph, and introspection index, and builds on two
independently published member crates:

- **`crates/arity-parser`** — `syntax` (SyntaxKind, node pointers), `ast` (typed
  wrappers), `parser` (lossless CST parser + incremental reparse), plus `dcf`, a
  **second, independent grammar** for the format of `DESCRIPTION` (its own
  `Language`, its own `syntax`/`parser`/`ast`). Depends only on `rowan`,
  `serde`, `smol_str`.
- **`crates/arity-formatter`** — the formatting engine, for embedders such as a
  dprint plugin. Depends on `arity-parser`; optional `serde`/`schema` features
  derive serde and schemars on `FormatStyle`.

The root crate re-exports the parser crate's modules
(`pub use arity_parser::{ast, parser, syntax}`), and `src/formatter.rs` bridges
the engine while hosting the CLI-side batch `check` API and the persistent
format `cache` — so `arity::parser`, `arity::formatter`, etc. stay the paths
everything uses. The member crates' low-level cross-crate helpers
(`parser::expr`, `parser::roxygen`) are `pub` but semver-loose.

Beyond the crate the repo ships the distribution surfaces: a VS Code extension
(`editors/code`), npm packages (`npm/`), a PyPI package (via maturin), the docs
site (`docs/`), and benchmarks (`benches/`, `scripts/bench.sh`).

The dev environment is `devenv`/Nix: R (with `roxygen2`, `commonmark`, `styler`,
`languageserver`) plus `go-task`, `mdbook`, `cargo-insta`, `air-formatter`,
`jarl`, `hyperfine`, and friends. `devenv.nix` also declares the git hooks that
run on commit: `clippy`, `rustfmt`, `biome`.

## Tenets

1. **Deterministic, rule-based formatting.** Output is decided solely by the
   formatter's rules and the layout engine. Push back against attempts to
   hard-code special cases for specific constructs. Unlike air (arity's closest
   relative), arity does **not** honor "persistent line breaks" — the input's
   existing line breaks never influence the result. Because the formatter is the
   **sole authority on layout**, autofixes are textual edits that never invoke
   it: a fix decides *what* to rewrite, never *how to lay it out*. The pipeline
   is fix-then-format.
2. **Incremental parsing is first-class**, not an afterthought. Parser/CST work
   must keep the salsa-based reparse path (`src/incremental.rs`) viable.
3. **Parsing is the parser's job.** Never paper over parser mistakes in the
   formatter, and never let parsing logic creep into the formatter. If the
   formatter hits something the parser handled wrong, fix it in the parser.
4. **Losslessness is the parser's job.** The parser preserves all text so that
   `reconstruct(text)` is always `text`. The formatter may assume a lossless CST.

Air compatibility is a **soft, one-directional gauge**, strictly subordinate to
Tenet 1 and **never a quality gate**: we measure how often air would leave
arity's output unchanged and treat air's maturity as a free differential oracle.
Full policy in `.claude/rules/formatter.md`.

## Commands

```sh
cargo build                       # dev build; --release for release
cargo test --workspace            # all tests (bare `cargo test` runs only the root crate!)
cargo test --workspace <substring>                   # tests matching a name
cargo test -p arity-parser --test parser_snapshots   # one member-crate test file
cargo test --test lint                               # one root-crate test file (`ls tests/*.rs`)
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check        # keep changes rustfmt-clean
```

Subcommands are `parse`, `format`, `lint`, `index`, `lsp`, `init`, and
`completions` (`docs/src/reference/cli.md` is the generated reference). The
non-obvious flags:

```sh
cat file.R | cargo run -- parse --verify --quiet   # losslessness round-trip check
cargo run -- format --check <path>    # check without writing (multi-path requires --check)
cargo run -- format --verify <file.R> # check idempotence; does not write
cargo run -- lint --fix <path>        # safe autofixes only (--unsafe-fixes for the rest)
```

All commands honor an `arity.toml` found by an ancestor walk (`--config` forces
one, `--no-config` ignores); the repo's own `arity.toml` dogfoods that path and
documents the defaults. `task <name>` (`Taskfile.yml`) wraps the workflows:
`lint`, `format`, `test`, `audit`, `deny`, `docs-gen`, `docs-build`,
`docs-preview`, `air-compat`, `corpus`, `bench`, `bench-parse`, `profile`, and
the `roxygen-*` oracle and projector tasks. `task --list` shows them all.

**Logging is currently inert**: `env_logger` is never initialized and the
workspace has three log sites in total, so `RUST_LOG` has no effect and
`task test-debug` emits nothing. Wiring up a logger is an open task.

## Architecture map

Paths are relative to the owning crate (`syntax`, `ast`, `parser` in
`crates/arity-parser/src/`, the formatter in `crates/arity-formatter/src/`,
everything else in the root crate's `src/`); each entry's directives are in the
matching `.claude/rules/` file.

- **Parse pipeline** (`parser/`) — lossless `rowan` CST via an event-based
  pipeline: `lex` → `parse_expr` (Pratt) + `structural` (recursive descent) →
  `build_tree`. Roxygen is *parsed*, not treated as opaque comments
  (`parser/roxygen/`); `parser/reparse.rs` holds the incremental strategies.
  `ast/` is a zero-cost typed, read-only *navigation* view over the CST that
  consumers go through, while the formatter deliberately stays on raw CST.
- **Formatter** (`crates/arity-formatter`) — a Wadler/Prettier-style document IR
  printed by a single best-fit layout engine that makes every line-break
  decision. Target style is the tidyverse R style guide. Also formats a
  package's `DESCRIPTION` (`formatter/description/`), which needs no layout
  engine at all: every break there is decided by the field's class.
- **Semantic model** (`src/semantic/`) — strictly *single-file*: scopes,
  bindings, resolution, in-file `library()` tracking, per-region CFG, plus
  namespace resolution against base R and bundled CRAN symbol lists.
- **Project layer** (`src/project/`) — the *cross-file* counterpart: the
  `source()` graph, a package's implicit shared namespace, per-file export
  projection, the S4/R6/reference-class index, and the native routines
  `useDynLib()` binds, wired into salsa by `graph.rs`.
- **Linter** (`src/linter/`) — **purely semantic**; anything `format --check`
  catches belongs to the formatter. 61 rules across seven categories, with
  autofixes, `# arity-lint` suppression, and a generated rule reference. Runs
  over **two grammars**: `Rule` for `.R`, `DcfRule` for `DESCRIPTION`, one
  registry and one namespace of rule IDs.
- **Language server** (`src/lsp.rs` + `src/lsp/`) — stdio JSON-RPC on
  `lsp-server`, with a dedicated lint thread owning the salsa database and
  purpose-built read and index task pools.
- **R introspection index** (`src/rindex/`) — harvests installed packages
  **without an R runtime**, reading R's on-disk formats natively.
- **Roxygen analysis** (`src/roxygen/`) — the test-only CST → Rd-tree projector
  behind the parity gate.
- **File discovery** (`src/file_discovery.rs`) — walks for `.R` files honoring
  the config's `ExcludeFilter`; an unsupported explicit path is rejected unless
  force-excluded, so a runner like pre-commit staging one is skipped, not an
  error. `collect_source_files` adds the second grammar, a package's
  `DESCRIPTION`, and is what both `lint` and `format` walk; `collect_r_files`
  stays the R-only view its other callers need. `is_description_file` is the one
  path-to-grammar classifier, shared with the language server's `DocumentKind`.

## Invariants and conventions

- **Treat CI as the source of truth for quality gates** (`.github/workflows/`):
  cross-platform build/test, `cargo-audit` + `cargo-deny`, and `lint.yml`'s
  clippy `-D warnings`, rustfmt check, **panache** prose check (Markdown;
  `panache.toml` lists exclusions), and **biome** check (JS/TS; `biome.jsonc`
  scopes it to `editors/code/src` and `npm`).
- **Losslessness:** `reconstruct(text) == text`, byte for byte.
- **Idempotence:** `format(format(x)) == format(x)`. Byte-identical output is
  the bar for a "behavior-preserving" refactor.
- **Semantics stay static** — no R evaluation anywhere in the pipeline.
- Dependency changes must stay clean under `cargo-audit` and `cargo-deny`
  (`deny.toml`).
- Speed is **measured, not asserted**: benchmarks and profiles are opt-in,
  local, and never a gate (`.claude/rules/docs.md`). `task profile` samples one
  phase with perf; `task bench` is the published comparison.

## Commits and versioning

- **Conventional Commits** (`type(scope): subject`) and semantic versioning.
  Subject ≤ 60 chars where possible, ≤ 72 is fine. Bodies short and to the point.
- **Never hand-edit `CHANGELOG.md` or any version field** — `versionary`
  generates them and will overwrite you.
- Keep commits **atomic per area** (root crate, member crate, `editors/`): the
  release tooling routes versions by path. Details in `.claude/rules/release.md`.

## Testing

**Use test-driven development.** Write the test first, watch it fail, then make
it pass. For a bug, always start with a failing test that reproduces it (a new
fixture case or snapshot) before touching the fix. **Run
`cargo test --workspace`** — a bare `cargo test` covers only the root crate.

- Integration tests live with their crate — parser suites in
  `crates/arity-parser/tests/`, formatter suites in
  `crates/arity-formatter/tests/`, everything else (linter, LSP, salsa, roxygen,
  rindex, config, CLI-level format, corpus) in the root `tests/*.rs`. **Both
  fixture suites are hand-registered**: a case only runs once its name is in
  `fixture_names()`.
- `insta` snapshots live in each crate's `tests/snapshots/`; review with
  `cargo insta review`. **Never accept a snapshot you have not read.**
- Which suite to reach for: formatter bug → a formatter fixture; parser bug → a
  parser fixture + `cargo insta review`; lint rule → a `#[test]` in
  `tests/lint.rs` (no fixture dir), or `tests/lint_description.rs` for a
  `DESCRIPTION` rule, plus the rule's own `examples()`;
  cross-file/LSP work → `tests/lsp.rs`, `tests/lsp_protocol.rs`, and
  `tests/salsa_incremental.rs`, which guards that a body edit does *not*
  invalidate the project graph.
- `tests/corpus.rs` is the Tier 0 smoke test (`#[ignore]`d; run with
  `ARITY_CORPUS=<dir> task corpus`): losslessness + idempotence over real R
  sources, unparseable files skipped rather than failed.
  `.github/workflows/smoke-test.yml` runs it weekly and files one deduped issue
  per (repo, failure category); triage with the `smoke-test-triage` skill.

Skills for the recurring workflows: `add-lint-rule` (the full sequence for a new
rule), `linter-investigation` (triage the linter against a real R codebase),
`roxygen-parity` (close a roxygen2 gap), `smoke-test-triage`,
`perf-investigation` (profile a phase and recover wall time without changing
behavior).

## Reference-only directories (not part of the build, untracked)

**A fresh clone does not have these** — nothing in `cargo build`/`cargo test`
needs them, so their absence is normal, not a broken checkout. Clone one into
the repo root when you need that oracle or reference:

```sh
git clone https://github.com/posit-dev/air     # reference checkout; NOT in the Cargo build
git clone https://github.com/tidyverse/style   # the formatter's target style guide
git clone --branch "v$(cat tests/oracle/.roxygen2-source)" \
  https://github.com/r-lib/roxygen2 roxygen2-ref   # the roxygen oracles' reference
```

`air/` has its own `air/CLAUDE.md` describing *that* project's conventions
(`just test`, `air.toml`) — do not apply those to arity. Note `task air-compat`
needs the air **binary** on PATH (devenv provides it), not this checkout.
