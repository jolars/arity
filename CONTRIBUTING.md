# Contributing to Arity

Thanks for your interest in improving Arity, a language server, formatter, and
linter for R. This guide covers how to set up a development environment, the
quality bar for changes, and the conventions we follow.

For a deeper tour of the architecture and the design tenets, read
[`AGENTS.md`](AGENTS.md); [`TODO.md`](TODO.md) is the live roadmap and records
known issues and follow-ups.

## Getting started

Arity is a Cargo workspace (edition 2024): the root `arity` package is the CLI,
language server, and linter, and it builds on two independently published member
crates, [`crates/arity-parser`](crates/arity-parser) (lossless CST parser, AST
wrappers, incremental reparser) and
[`crates/arity-formatter`](crates/arity-formatter) (the formatting engine). The
MSRV is declared as `rust-version` in [`Cargo.toml`](Cargo.toml).
[`rust-toolchain.toml`](rust-toolchain.toml) selects the `stable` channel, so
`rustup` picks up a suitable toolchain automatically.

The recommended setup uses [`devenv`](https://devenv.sh)/Nix, which provisions
the toolchain, `R` (with `roxygen2`, `commonmark`, `styler`), and every
auxiliary tool (`go-task`, `mdbook`, `cargo-insta`, `cargo-audit`, `cargo-deny`,
`air-formatter`, `jarl`, `hyperfine`, and more):

```sh
devenv shell
```

It also installs git hooks that run `clippy`, `rustfmt`, and `biome` on commit.

If you'd rather not use Nix, install a recent stable Rust toolchain and, for the
R-dependent tasks (the roxygen oracles that regenerate pins, corpus checks), a
working `R` with `roxygen2` and `commonmark`. The core build, tests, and lint
gates need only Rust.

## Building and running

```sh
cargo build                    # dev build
cargo build --release

cargo run -- format <file.R>   # format to stdout (stdin if omitted)
cargo run -- lint <path>       # lint; exits 1 on findings
cargo run -- lint --fix <path> # apply safe autofixes in place
cargo run -- parse <file.R>    # print the CST
cargo run -- index             # build the installed-package introspection index
cargo run -- lsp               # start the stdio language server
```

Every command reads an [`arity.toml`](arity.toml) found by walking up from the
target path (`--config` forces one, `--no-config` ignores it). The repo's own
`arity.toml` documents the defaults.

## Repository layout

The root crate lives in `src/` with its integration tests in `tests/`; the
parser and formatter live in `crates/arity-parser` and `crates/arity-formatter`
with their own `tests/` (see [`AGENTS.md`](AGENTS.md) for the module tour). The
repo also carries the distribution and documentation surfaces: `editors/code`
(VS Code extension), `npm/` and `pyproject.toml` (npm and PyPI wrappers around
the binary), `docs/` (the mdBook site), `benches/` plus `scripts/bench.sh`, and
`scripts/` (the R helpers that regenerate the bundled base-R and CRAN symbol
lists).

## Quality gates

Treat CI (`.github/workflows/`) as the source of truth. Before opening a pull
request, make sure the following all pass locally:

```sh
cargo test --workspace                       # all tests (bare `cargo test` skips the member crates!)
cargo clippy --workspace --all-targets --all-features -- -D warnings   # warnings are errors
cargo fmt --all -- --check                   # keep changes rustfmt-clean
```

Markdown prose is formatted by [`panache`](https://github.com/jolars/panache),
which CI checks; [`panache.toml`](panache.toml) excludes the generated and
non-prose files.

JavaScript and TypeScript are linted and formatted by
[`biome`](https://biomejs.dev), on both a devenv git hook and a CI job.
[`biome.jsonc`](biome.jsonc) scopes it to the repo's first-party sources ---
[`editors/code/src`](editors/code/src) and [`npm`](npm) --- and documents what
is deliberately left out.

The [`Taskfile.yml`](Taskfile.yml) wraps these and more: `task test`,
`task lint`, `task format`, `task audit`, `task deny`. Run `task --list` to see
everything, including the heavier `#[ignore]`d suites: `task air-compat`,
`task corpus` (needs `ARITY_CORPUS=<dir>`), `task bench`, and the `roxygen-*`
oracle tasks.

Dependency changes must stay compatible with `cargo-audit` and `cargo-deny` (see
[`deny.toml`](deny.toml)).

## Test-driven development

**Write the test first, watch it fail, then make it pass.** For a bug, start by
adding a failing test that reproduces it (a new fixture case or snapshot) before
touching the fix.

- Integration tests live in each crate's `tests/*.rs`: parser suites in
  `crates/arity-parser/tests/` (fixtures in
  `crates/arity-parser/tests/fixtures/parser/<case>/`, holding `input.R`; the
  CST and diagnostics are snapshotted, losslessness asserted), formatter suites
  in `crates/arity-formatter/tests/` (fixtures in
  `crates/arity-formatter/tests/fixtures/formatter/<case>/`, holding `input.R`
  plus `expected.R`), and everything else (linter, LSP, salsa, roxygen oracles,
  rindex) in the root `tests/`.
- **Both fixture suites are hand-registered.** A new case does not run until its
  directory name is added to `fixture_names()` in
  `crates/arity-parser/tests/parser_snapshots.rs` or
  `crates/arity-formatter/tests/formatter.rs`. This is the most common way a new
  test silently does nothing.
- Snapshot tests use [`insta`](https://insta.rs). Review and accept snapshots
  with `cargo insta review` or `cargo insta accept`.
- Logging is currently inert: `env_logger` is a dependency but nothing
  initializes it, so `RUST_LOG` (and `task test-debug`) has no effect. Reach for
  `dbg!` or a test-local print for now.

## Your first change

The loop differs a little per subsystem. Pick the one your change lands in.

**A formatter bug** (output is wrong, ugly, or unstable):

```sh
cd crates/arity-formatter
mkdir tests/fixtures/formatter/my_case
$EDITOR tests/fixtures/formatter/my_case/input.R      # the offending code
$EDITOR tests/fixtures/formatter/my_case/expected.R   # what it should become
$EDITOR tests/formatter.rs                            # add "my_case" to fixture_names()
cargo test -p arity-formatter --test formatter        # watch it fail, then fix
```

The fix belongs in `crates/arity-formatter/src/formatter/rules/` (the IR built
per construct) or `crates/arity-formatter/src/formatter/printer.rs` (how the
layout engine breaks lines) --- never a special case for one construct, and
never a parser workaround (Tenets 1 and 3). The suite also asserts idempotence
and losslessness, so a fix that formats your case correctly but destabilizes
another will fail loudly.

**A parser bug** (wrong tree shape, a lost byte, a bad diagnostic):

```sh
cd crates/arity-parser
mkdir tests/fixtures/parser/my_case
$EDITOR tests/fixtures/parser/my_case/input.R
$EDITOR tests/parser_snapshots.rs                     # add "my_case" to fixture_names()
cargo test -p arity-parser --test parser_snapshots    # the snapshot starts out missing
cargo insta review                                    # inspect the CST, accept if right
```

The CST is snapshotted and losslessness asserted automatically. If the tree
shape is wrong, fix `crates/arity-parser/src/parser/` and re-review the snapshot ---
do not accept a snapshot you have not read.

**A lint rule** (new rule, false positive, bad autofix): rules live in
`src/linter/rules/<category>/<id>.rs`, tests are plain `#[test]` functions in
`tests/lint.rs`, and the per-rule documentation page is generated from the
rule's own `examples()`. See [Adding a lint rule](#adding-a-lint-rule) below.

**Anything cross-file** (LSP, incremental, project graph): `tests/lsp.rs` and
`tests/lsp_protocol.rs` cover the server, `tests/salsa_incremental.rs` guards
that a function-body edit does *not* invalidate the project graph, and
`tests/incremental_reparse.rs` covers the reparse strategies. These invariants
are easy to break by accident: if you touch what a per-file salsa query returns,
run `cargo test --test salsa_incremental`.

## Configuration

Arity reads an [`arity.toml`](arity.toml), discovered by walking up from the
target path. `arity init` writes a starter file, and the repo's own `arity.toml`
dogfoods discovery while documenting the defaults. The schema lives in
[`src/config.rs`](src/config.rs) and is rendered at
[arity.cc](https://arity.cc/reference/configuration.html):

- `exclude` --- gitignore-style patterns; setting it **replaces** the built-in
  default set. `extend-exclude` adds to the defaults instead, which is usually
  what you want.
- `cache` --- enable the persistent already-formatted cache (`--no-cache`
  overrides it for one run).
- `[format]` --- `line-width`, `indent-width`, `line-ending`
  (`auto`/`lf`/`crlf`/`native`).
- `[lint]` --- `select` (an allowlist; when set, only those rules run) and
  `ignore` (subtracted from whichever set is active).
- `[index]` --- `library-paths`, `cache-dir`, `auto-build`, `help`.

Two conventions worth knowing when extending the schema. The structs are
`#[serde(deny_unknown_fields, rename_all = "kebab-case")]`, so a typo in a
user's config is an error rather than a silent no-op, and TOML keys are
kebab-case even though the Rust fields are snake_case. And not everything
belongs in the file: `index.remote_url` is deliberately `#[serde(skip)]` and
read from the `ARITY_REMOTE_URL` environment variable instead, because enabling
network egress is a per-user consent decision, not a committed project setting.

A rule that needs options of its own gets a `[lint.rules.<id>]` table, typed as
its own struct and added as a field on `RulesConfig`. That keeps the
unknown-field check working, at the price of one asymmetry worth knowing: a rule
ID in `select`/`ignore` is data and so an unknown one is reported when linting
runs, whereas a rule ID under `[lint.rules]` is schema and an unknown one fails
at parse time. Rules read their table off `RuleContext::config`.

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
`crates/arity-formatter/tests/air_compat_allowlist.toml` with a rationale.

## Reference checkouts

A few directories are referred to by tooling and documentation but are
deliberately **untracked**, so a fresh clone does not have them. Nothing in
`cargo build` or `cargo test` needs them; you only need a given one if you are
running the corresponding oracle or reading the reference. Clone them into the
repo root:

```sh
# posit-dev/air: the R formatter arity is measured against. Read-only
# reference; it has its own conventions in air/CLAUDE.md that do NOT apply here.
git clone https://github.com/posit-dev/air

# The tidyverse style guide: the formatter's target style.
git clone https://github.com/tidyverse/style

# r-lib/roxygen2: the reference implementation behind the roxygen oracles.
# Pin the version recorded in tests/oracle/.roxygen2-source.
git clone --branch "v$(cat tests/oracle/.roxygen2-source)" \
  https://github.com/r-lib/roxygen2 roxygen2-ref
```

`scripts/harvest-roxygen-corpus.R` reads `roxygen2-ref/`; the air comparison in
`crates/arity-formatter/tests/air_compat.rs` needs the `air` **binary** on PATH
(devenv provides it), not the checkout.

## Adding a lint rule

The linter ships 47 rules across six categories (correctness, suspicious,
readability, performance, documentation, meta) with generated per-rule docs. A
rule is a module under `src/linter/rules/<category>/<id>.rs` implementing the
`Rule` trait --- it either subscribes to `SyntaxKind`s via `interests` and gets
called during the one shared CST walk, or leaves `interests` empty and overrides
`check_file` for a whole-file pass. Adding one touches the registry (`all_rules`
in [`src/linter/rules.rs`](src/linter/rules.rs), the single source of truth),
TDD fixtures, an autofix-correctness (parse-clean) case, and the snapshot-pinned
generated docs. If you use Claude Code, the `add-lint-rule` skill walks through
the whole sequence.

Note the autofix-correctness bar: a fix is a textual edit that must leave code
that still parses and stays lossless; it does **not** owe line-width (the
formatter re-lays-out afterward). When an edit can't be made correct by
construction for some shape, withhold the fix for that shape rather than risk
broken output.

## Documentation

The docs site (`docs/`) is an [mdBook](https://rust-lang.github.io/mdBook/).
Some reference pages are generated: `build.rs` writes the CLI reference from the
clap definitions, and `cargo run --example docgen` renders the per-rule pages
(by running the real linter on each rule's examples), the version stamp, and the
benchmark partials. Regenerate with `task docs-gen` and preview with
`task docs-preview`. Don't hand-edit generated pages --- the rendered rule docs
are pinned by `tests/rule_docs.rs` and the benchmark partials by
`tests/benchmarks_docs.rs`, so they can't drift from behavior.

## Performance

Speed is a feature here, and it is measured rather than asserted.

- `task bench` runs [`scripts/bench.sh`](scripts/bench.sh): the formatter
  against `air` (and, opt-in, `styler`) and the linter against `jarl`, each at
  two scopes --- synthetic single-file tiers built from the formatter fixtures,
  and a real R package (`tidyr`, cloned once into a cache). It rewrites the
  tracked artifact `benches/benchmark_results.json`.
- That artifact is the *only* source of the published benchmark page.
  `task   docs-gen` renders it into the generated partials; the numbers are
  never re-measured at site-build time or in CI. So if you change something that
  moves performance and want the docs to reflect it, re-run `task bench` and
  commit the artifact.
- `task bench-parse` is a criterion microbenchmark of parse and incremental
  reparse (`crates/arity-parser/benches/parse.rs`) --- the right tool for a
  parser-level change.
- For profiling, devenv provides `perf`, `cargo-flamegraph`, `hyperfine`, and
  `cargo-llvm-cov`.

The benchmark is a **visibility tool, not a quality gate**. It measures
wall-clock speed only, never output equivalence (that is what `task air-compat`
is for), and the tools do genuinely different work behind different startup
floors --- read the ratios, not the milliseconds. A PR is never blocked on it.

## Editor extension and packaging

Beyond the crate, the repo builds several distribution artifacts. You only need
these if your change touches them.

- **VS Code extension** ([`editors/code`](editors/code)) --- TypeScript, bundled
  with esbuild. `npm run compile` (type-check plus bundle), `npm run watch`
  while developing, `npm run package` for a VSIX. It is linted and formatted by
  `biome` (a git hook and a CI job), and versioned separately from the crate
  (`arity-code`, following the crate's releases). At publish time a
  platform-specific `arity` binary is downloaded from the GitHub release into
  `editors/code/server/` and packaged into a per-target VSIX; at runtime the
  extension resolves the server via `arity.executableStrategy`
  (`bundled`/`environment`/`path`), falling back to `arity` on PATH --- which is
  also what it does on NixOS, where a downloaded binary would not run.
- **npm** ([`npm/`](npm)) --- `arity-cli` is a thin launcher whose
  `optionalDependencies` pull in one `@arity-cli/<platform>` package per target;
  `npm/platform-template` is the template those are generated from. The versions
  are bumped automatically (see below), so do not hand-edit them.
- **PyPI** --- built by maturin from [`pyproject.toml`](pyproject.toml).
- **Release binaries** --- eight targets (Linux gnu/musl, macOS, and Windows,
  each x86_64 and aarch64), cross-built with `cargo-zigbuild` where needed and
  checked against a glibc floor.

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

## How a release happens

Releases are automated, which is why commit messages matter: your Conventional
Commit type is what decides the next version number.

On every push to `main`, once the test, `cargo-audit`, and `cargo-deny` jobs go
green, [`versionary`](https://github.com/jolars/versionary) (configured in
[`versionary.jsonc`](versionary.jsonc)) opens or updates a release PR that bumps
the version, regenerates [`CHANGELOG.md`](CHANGELOG.md), and propagates the
version into `npm/arity-cli/package.json` (both its own version and every
`optionalDependencies` entry). The member crates `arity-parser` and
`arity-formatter` are versioned **independently**, routed by path: a commit
touching `crates/arity-parser` bumps only that crate, and its releases are
tagged `arity-parser-v<version>` (likewise `arity-formatter-v<version>`), while
the CLI keeps the bare `v<version>` stream. The VS Code extension is another
versioned package that follows the CLI. Merging the release PR tags the release,
which then fans out: release binaries for all eight targets (with keyless
provenance attestation), the VS Code and Open VSX extensions, and the crates.io
(all workspace crates, in dependency order), npm, and PyPI publishes. Only the
CLI's `v*` stream carries GitHub release assets.

Practical consequences:

- **Never hand-edit `CHANGELOG.md` or any of the version fields.** They are
  generated, and your edit will be overwritten.
- `feat:` bumps the minor version, `fix:` the patch; a `!` or a
  `BREAKING CHANGE:` footer bumps the major. The project is pre-1.0 with
  `bump-minor-pre-major`, so breaking changes currently land as minor bumps.
- A commit that only touches `editors/`, `crates/arity-parser`, or
  `crates/arity-formatter` is excluded from the CLI's version calculation, so
  keep commits atomic per area --- a commit that mixes parser and CLI changes
  bumps both.

## Reporting issues

Bug reports and feature requests are welcome on the [issue
tracker](https://github.com/jolars/arity/issues). For a bug, a minimal R snippet
that reproduces the problem is the most helpful thing you can include.

## License

By contributing, you agree that your contributions are licensed under the
project's [MIT License](LICENSE).
