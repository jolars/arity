# Agent Instructions

This file carries global, cross-cutting rules for agents in this repository.
Subsystem specifics live in `.claude/rules/*.md` and should not be duplicated
here.

## Where instructions live

- Global rules that must hold before reading any file belong here.
- Path-scoped subsystem rules live in `.claude/rules/*.md`:
  `parser`, `formatter`, `linter`, `lsp`, `semantic`, `rindex`, `roxygen`,
  `config`, `docs`, and `release`.
- Keep each rule file terse (target under 200 lines): rule, brief rationale,
  pointer to code/tests.
- Do not turn rules into issue archaeology or tutorials. Put those in tests,
  issues, or `git log`.

## Project summary

Arity is a Rust CLI for R with formatter, linter, parser, and language-server
capabilities.

- Root crate (`arity`) hosts CLI, LSP, lint, semantic/project layers, and
  rindex.
- Member crates:
  - `crates/arity-parser` (R CST parser + `DESCRIPTION` DCF grammar + AST
    wrappers)
  - `crates/arity-formatter` (format engine)
- Distributed surfaces include `editors/code`, `npm/`, `pyproject.toml`, and
  `docs/`.

## Tenets

1. **Deterministic formatting.** The formatter is the sole layout authority.
   Autofixes are text rewrites only; pipeline is fix-then-format.
2. **Incremental parsing is first-class.** Parser/CST changes must preserve the
   reparse path in `src/incremental.rs`.
3. **Parsing belongs in the parser.** Do not patch parser mistakes downstream in
   formatter or linter.
4. **Parser owns losslessness.** `reconstruct(text) == text` byte-for-byte.
5. **Formatting is idempotent.** `format(format(x)) == format(x)`.

Air compatibility is a soft, one-directional gauge and never a quality gate;
policy lives in `.claude/rules/formatter.md`.

## Commands

```sh
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Useful checks:

```sh
cat file.R | cargo run -- parse --verify --quiet
cargo run -- format --check <path>
cargo run -- format --verify <file.R>
cargo run -- lint --fix <path>
```

`task <name>` (`Taskfile.yml`) wraps common workflows (`lint`, `format`, `test`,
`docs-gen`, `air-compat`, `corpus`, `bench`, `profile`, and roxygen oracles).

## Architecture map (short)

- Parser: `crates/arity-parser/src/` (`.claude/rules/parser.md`)
- Formatter: `crates/arity-formatter/src/` + bridge `src/formatter.rs`
  (`.claude/rules/formatter.md`)
- Linter: `src/linter/` (`.claude/rules/linter.md`)
- LSP: `src/lsp.rs`, `src/lsp/` (`.claude/rules/lsp.md`)
- Semantic/project layers: `src/semantic/`, `src/project/`
  (`.claude/rules/semantic.md`)
- R index: `src/rindex/` (`.claude/rules/rindex.md`)
- Roxygen projector: `src/roxygen/` (`.claude/rules/roxygen.md`)
- Config/discovery: `src/config.rs`, `src/file_discovery.rs`
  (`.claude/rules/config.md`)
- Docs/bench/release: `.claude/rules/docs.md`, `.claude/rules/release.md`

## Invariants and conventions

- CI workflows in `.github/workflows/` are the quality-gate source of truth.
- Preserve losslessness and idempotence; do not introduce R evaluation.
- Dependency changes must remain clean under `cargo-audit` and `cargo-deny`.
- Benchmarks/profiles are opt-in local measurements, not release gates.

## Commits and versioning

- Use Conventional Commits (`type(scope): subject`).
- Never hand-edit `CHANGELOG.md` or version fields; `versionary` owns them.
- Keep commits atomic per area (root crate vs member crate vs `editors/`).

## Testing

- Use TDD: reproduce with a failing test first, then fix.
- Run `cargo test --workspace` (bare `cargo test` only runs the root crate).
- Fixture suites are hand-registered (`fixture_names()`); new fixtures must be
  registered to run.
- Snapshot workflow uses `cargo insta review`; do not accept unread snapshots.
- Use suite-local tests:
  - parser: `crates/arity-parser/tests/`
  - formatter: `crates/arity-formatter/tests/`
  - root integrations (lint/LSP/salsa/roxygen/rindex/config): `tests/*.rs`

Recurring workflows use skills: `add-lint-rule`, `linter-investigation`,
`roxygen-parity`, `smoke-test-triage`, `perf-investigation`.

## Optional reference checkouts

Some local oracle/reference directories are untracked and optional (`air/`,
`style/`, `roxygen2-ref/`). Their absence in a fresh clone is expected.
