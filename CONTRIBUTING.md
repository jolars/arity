# Contributing to Arity

Thanks for your interest in improving Arity, a language server, formatter, and
linter for R. This guide covers how to set up a development environment, the
quality bar for changes, and the conventions we follow.

For a deeper tour of the architecture and the design tenets, read
[`AGENTS.md`](AGENTS.md); [`TODO.md`](TODO.md) is the live roadmap and records
known issues and follow-ups.

## Getting started

Arity is a single-crate Cargo package (edition 2024, `rust-version` pinned in
[`Cargo.toml`](Cargo.toml)). You need a matching Rust toolchain; the exact
version is pinned in [`rust-toolchain.toml`](rust-toolchain.toml), so `rustup`
will pick it up automatically.

The recommended setup uses [`devenv`](https://devenv.sh)/Nix, which provisions
the toolchain, `R`, and every auxiliary tool (`go-task`, `mdbook`,
`cargo-insta`, `cargo-audit`, `cargo-deny`, `air-formatter`, and more):

```sh
devenv shell

# Or auto-enable it
devenv allow
```

If you'd rather not use Nix, install a recent stable Rust toolchain and, for the
R-dependent tasks (roxygen oracles, corpus checks), a working `R` with
`roxygen2`. The core build, tests, and lint gates need only Rust.

## Building and running

```sh
cargo build                    # dev build
cargo build --release

cargo run -- format <file.R>   # format to stdout (stdin if omitted)
cargo run -- lint <path>       # lint; exits 1 on findings
cargo run -- parse <file.R>    # print the CST
cargo run -- lsp               # start the stdio language server
```

## Quality gates

Treat CI (`.github/workflows/`) as the source of truth. Before opening a pull
request, make sure the following all pass locally:

```sh
cargo test                                                 # all tests
cargo clippy --all-targets --all-features -- -D warnings   # warnings are errors
cargo fmt -- --check                                       # keep changes rustfmt-clean
```

The [`Taskfile.yml`](Taskfile.yml) wraps these and more: `task test`,
`task lint`, `task format`, `task audit`, `task deny`. Run `task --list` to see
everything.

Dependency changes must stay compatible with `cargo-audit` and `cargo-deny` (see
[`deny.toml`](deny.toml)).

## Test-driven development

**Write the test first, watch it fail, then make it pass.** For a bug, start by
adding a failing test that reproduces it (a new fixture case or snapshot) before
touching the fix.

- Integration tests live in `tests/*.rs`; fixtures in
  `tests/fixtures/{parser,formatter}/<case>/`. Parser fixtures hold `input.R`
  (the CST and diagnostics are snapshotted, losslessness asserted); formatter
  fixtures hold `input.R` plus `expected.R`.
- Snapshot tests use [`insta`](https://insta.rs). Review and accept snapshots
  with `cargo insta review` or `cargo insta accept`.
- Logging honors `RUST_LOG` (e.g. `RUST_LOG=debug cargo test`).

## Design tenets to keep in mind

These are the load-bearing invariants; see [`AGENTS.md`](AGENTS.md) for the full
statements.

1. **Deterministic, rule-based formatting.** Layout is decided solely by the
   formatter's rules and layout engine. Arity does *not* honor persistent line
   breaks; the input's existing line breaks never influence the result. Avoid
   hard-coding special cases for specific constructs.
2. **Incremental parsing is first-class.** Keep the `salsa`-based incremental
   reparse path (`src/incremental.rs`) viable.
3. **Parsing is the parser's job.** Don't paper over parser mistakes in the
   formatter, and don't let parsing logic creep into the formatter.
4. **Losslessness is the parser's job.** The parser preserves all text, so
   `reconstruct(text) == text` always. Formatter output must additionally be
   **idempotent**: `format(format(x)) == format(x)`.

We also track a *soft, one-directional* compatibility target with the `air`
formatter as a differential oracle. It is never a quality gate and is strictly
subordinate to Tenet 1. Deliberate divergences are recorded in
`tests/air_compat_allowlist.toml` with a rationale.

## Adding a lint rule

The linter ships a growing set of rules with generated per-rule docs. Adding one
touches the dispatch, the registry (`src/linter/rules.rs`), TDD fixtures, an
autofix-correctness (parse-clean) case, and the snapshot-pinned generated docs.
If you use Claude Code, the `add-lint-rule` skill walks through the whole
sequence.

Note the autofix-correctness bar: a fix is a textual edit that must leave code
that still parses and stays lossless; it does **not** owe line-width (the
formatter re-lays-out afterward). When an edit can't be made correct by
construction for some shape, withhold the fix for that shape rather than risk
broken output.

## Documentation

The docs site (`book/`) is an [mdBook](https://rust-lang.github.io/mdBook/).
Some reference pages are generated: `build.rs` writes the CLI reference from the
clap definitions, and `cargo run --example docgen` renders the per-rule pages by
running the real linter on each rule's examples. Regenerate with `task docs-gen`
and preview with `task docs-preview`. The rendered rule docs are pinned by
`tests/rule_docs.rs`, so they can't drift from behavior.

## Commits and pull requests

- Use [Conventional Commits](https://www.conventionalcommits.org)
  (`type(scope): subject`) and semantic versioning.
- Keep the subject in the imperative mood and short (aim for 60 chars or under,
  72 at most). Use the body for concise explanations when needed.
- Wrap code in backticks in commit messages, and avoid fancy Unicode characters.
- Reference issues in the body (e.g. `Fixes #123`) when applicable.
- **Never edit the changelog by hand**; it is generated by `versionary`.

Prefer atomic commits. Small fixes can go straight to `main`; branch first for
anything substantial. Please make sure the quality gates above pass before you
open the PR.

## Reporting issues

Bug reports and feature requests are welcome on the [issue
tracker](https://github.com/jolars/arity/issues). For a bug, a minimal R snippet
that reproduces the problem is the most helpful thing you can include.

## License

By contributing, you agree that your contributions are licensed under the
project's [MIT License](LICENSE).
