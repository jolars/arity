# Agent Instructions

This file provides guidance to coding agents when working with code in this
repository.

## Project

Arity is a Rust CLI providing a language server, formatter, and linter for the R
language. It is a Cargo workspace (edition 2024) with a root package: the root
crate `arity` (both the binary and a library, published to crates.io) hosts the
CLI, LSP, linter, semantic model, project graph, and introspection index, and
builds on two independently published member crates:

- `crates/arity-parser` — `syntax` (SyntaxKind, node pointers), `ast` (typed
  wrappers), and `parser` (lossless CST parser + incremental reparse). Depends
  only on `rowan`, `serde`, `smol_str`.
- `crates/arity-formatter` — the formatting engine, for embedders such as a
  dprint plugin. Depends on `arity-parser`; optional `serde`/`schema` features
  derive serde and schemars on `FormatStyle`.

The root crate re-exports the parser crate's modules
(`pub use arity_parser::{ast, parser, syntax}` in `src/lib.rs`), and
`src/formatter.rs` is a bridge that re-exports the engine while hosting the
CLI-side batch `check` API and the persistent format `cache`—so `arity::parser`,
`arity::formatter`, etc. remain the paths everything uses. The member crates'
few low-level cross-crate helpers (`parser::expr`, `parser::roxygen`) are `pub`
but documented as semver-loose.

**Strategy (see `TODO.md`):** the parser + formatter foundation was brought to
near-completion *first*; the linter and LSP were then built out on top of it and
are now substantially complete. When in doubt about scope/priority, `TODO.md` is
the live roadmap and records known issues and follow-ups.

The dev environment is provided via `devenv`/Nix (`devenv.nix`, `devenv.yaml`,
`flake.nix`) and includes `R` (with `roxygen2`, `commonmark`, `styler`,
`languageserver`) plus the auxiliary tooling (`go-task`, `mdbook`,
`cargo-insta`, `cargo-audit`, `cargo-deny`, `air-formatter`, `jarl`,
`hyperfine`, `vsce`, …). `devenv.nix` also declares the git hooks that run on
commit: `clippy`, `rustfmt`, `eslint`, and `panache-format`.

Beyond the Rust crate, the repo ships the distribution surfaces: a VS Code
extension (`editors/code`), npm packages (`npm/`), a PyPI package (via
`pyproject.toml`/maturin), the docs site (`docs/`), and benchmarks
(`benches/`, `scripts/bench.sh`).

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

- The gauge lives in `crates/arity-formatter/tests/air_compat.rs`
  (`#[ignore]`d, so it never runs in `cargo test` and cannot fail CI). Run it
  with `task air-compat`; it regenerates `AIR_COMPAT.md` at the repo root.
- It measures the *fixed point* `air(arity(x)) == arity(x)`, not a head-to-head
  diff—this cancels out the persistent-line-break difference (which is the
  whole point of arity) by construction, leaving only genuine rule divergences.
- Divergences are triaged into two buckets. **Adopt** when air's output is
  simply more idiomatic and arity is being inconsistent (fix the rule).
  **Record** when the divergence is a deliberate arity choice—add it to
  `crates/arity-formatter/tests/air_compat_allowlist.toml` with a rationale.
- Diverging from air is allowed, but should **raise tension**: it is a
  conscious, documented decision (an allowlist entry), not a silent one. An
  unexplained divergence in `AIR_COMPAT.md` is an open question, never an excuse
  to fail a build.

## Commands

```sh
cargo build                       # dev build
cargo build --release
cargo test --workspace            # all tests (bare `cargo test` runs only the root crate!)
cargo test --workspace <substring>              # run tests matching a name
cargo test -p arity-parser --test parser_snapshots   # one member-crate test file
cargo test --test lint            # one root-crate test file (see `ls tests/*.rs`)
cargo clippy --workspace --all-targets --all-features -- -D warnings   # lint; warnings are errors
cargo fmt --all -- --check        # rustfmt check (keep changes rustfmt-clean)
```

CLI usage:

```sh
cargo run -- parse <file.R>                  # print CST; stdin if no file
cat file.R | cargo run -- parse --verify --quiet   # losslessness round-trip check
cargo run -- format <file.R>                 # format to stdout (stdin if omitted)
cargo run -- format --check <path>           # check without writing (multi-path requires --check)
cargo run -- format --verify <file.R>        # check idempotence; does not write
cargo run -- lint <path>                     # lint (stdin if no path); exits 1 on findings
cargo run -- lint --fix <path>               # apply safe autofixes (--unsafe-fixes for the rest)
cargo run -- index                           # build/refresh the installed-package index
cargo run -- lsp                             # run the language server over stdio
cargo run -- init                            # write a starter `arity.toml`
cargo run -- completions <shell>             # shell completion script to stdout
```

All commands honor an `arity.toml` discovered by an ancestor walk
(`--config <path>` to force one, `--no-config` to ignore); the repo's own
`arity.toml` dogfoods that path and documents the defaults.

The documentation site (`docs/`) is an mdBook. Its reference pages are
generated: `build.rs` writes `docs/src/reference/cli.md` from the clap CLI, and
`cargo run --example docgen` renders the whole rule reference
(`docs/src/reference/rules.md`—one `###` section per rule keyed by its ID, plus
the index over them, by running the real linter on each rule's examples),
`version.md`, and the benchmark partials
(`benchmarks_meta.md`/`benchmarks_results.md`, rendered by `src/bench_docs.rs`
from the committed `benches/benchmark_results.json`). `mdbook build docs` then
builds the site; `examples/canonical.rs` and `examples/sitemap.rs` post-process
it, and `.github/workflows/docs.yml` deploys it to GitHub Pages. The rendered
rule docs are pinned by `tests/rule_docs.rs` and the benchmark partials by
`tests/benchmarks_docs.rs`, so neither can drift from behavior.

Snapshot tests use `insta`: review/accept with `cargo insta review` or
`cargo insta accept`. **Logging is currently inert**: `env_logger` is a
dependency but is never initialized, and the only log sites in the workspace
are three `log::error!`/`log::warn!` calls (LSP task pool, lint thread, format
cache)—all in the root crate. So `RUST_LOG` has no effect today, and
`task test-debug` emits nothing—wiring up a logger is an open task, not a
working facility. `task <name>` (Taskfile.yml)
wraps the above: `lint`, `format`, `test`, `test-debug`, `audit`, `deny`,
`docs-gen`, `docs-build`, `docs-preview`, `air-compat`, `corpus`, `bench`, and
the `roxygen-*` oracle/projector tasks. `task --list` shows them all.

## Architecture

Paths below are relative to the owning crate: `syntax`, `ast`, and `parser`
live in `crates/arity-parser/src/`, the formatter in
`crates/arity-formatter/src/`, and everything else in the root crate's `src/`.

**Parse pipeline** (`parser/`, public API `parse`/`reconstruct` re-exported
from `parser.rs`, in `crates/arity-parser`): lossless `rowan` CST built via an
event-based pipeline.

```
lex (lexer.rs) → Vec<Token>
parse_expr (expr.rs, Pratt) + structural.rs (recursive descent) → Vec<Event>
build_tree (tree_builder.rs) → rowan SyntaxNode (CST)
```

- `core::parse` drives the loop; `events.rs` defines `Event` (start node, token,
  and finish node); `cursor.rs`, `context.rs`, `recovery.rs`, `diagnostics.rs`
  support the parser. `syntax.rs` defines `SyntaxKind` (rowan-style
  `SCREAMING_SNAKE_CASE`).
- **Losslessness is the core invariant:** all whitespace, newlines, comments,
  and `%...%`/`[[`/`]]` tokens are preserved; `reconstruct(text)` must equal
  `text`. Parser work prioritizes stable, recoverable CST shape over early
  semantic precision. Semantics stay **static**—no R evaluation.
- The **AST-wrapper layer** (`ast/`, also in `crates/arity-parser`) is a
  zero-cost typed *navigation* view
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
  idempotence are unaffected; `crates/arity-parser/tests/ast_wrappers.rs` is
  its integration-test home.
- **Roxygen is parsed, not treated as opaque comments.** `parser/roxygen/`
  sub-tokenizes any `^#+'` line so its structure (marker, tags, arguments,
  prose, Rd macros, markdown blocks) lives in the CST: `lex.rs` (sub-lexing),
  `group.rs` (block grouping + section/paragraph skeleton), `build.rs`
  (block-level Rd/markdown constructs), `inline.rs`. The sub-tokens' texts tile
  the line's bytes exactly, so losslessness still holds.
- `parser/reparse.rs` implements the incremental reparse strategies
  (token → block → top-level statement → full reparse); `syntax/ptr.rs` holds
  position-independent node pointers. The salsa layer sits **above** the parser
  crate: the root crate's `src/incremental.rs` models file text → CST →
  semantic model as `salsa` queries (the parser crate itself is salsa-free).

**Semantic model** (`src/semantic/`, facade `src/semantic.rs`): strictly
*single-file* analysis — scope tree, bindings, identifier resolution, in-file
`library()` tracking (`builder.rs`, `scope.rs`, `binding.rs`), plus a per-region
control-flow graph (`cfg.rs`). `symbols.rs` resolves against package
namespaces: `StaticBaseR` covers R's seven default packages from `base_r/*.txt`
(generated by `scripts/dump_base_symbols.R`), and `BundledPackages` covers the
top-N CRAN packages by download count from `cran/exports.txt` (generated by
`scripts/dump_cran_symbols.R`, ranked by `scripts/rank_cran_downloads.sh`,
refreshed by `.github/workflows/cran-symbols.yml`).

**Project layer** (`src/project/`): the *cross-file* counterpart to
`semantic` — the `source()` dependency graph (`source.rs`, `sequence.rs`), the
implicit shared namespace of an R package, per-file export projection
(`exports.rs`), the OOP class/inheritance index for S4/R6/reference classes
(`classes.rs`), and `scope.rs`'s pure `ProjectScope::build`. `graph.rs` wires
that into salsa; the per-file projections are deliberately **range-free** so
they backdate across a body edit and the project graph's memo survives
(`tests/salsa_incremental.rs` guards this). `FileScope` keeps the three reasons
a top-level binding is "not unused" apart rather than merging them, because
different rules need different ones: `read_elsewhere` (a sibling reads it),
`exported_by_namespace` (public API), and `is_s3_method` (reached by dispatch).
`used_elsewhere` is the union of the first two, which is what `unused-binding`
asks; `unused-function` asks for each separately.

**R introspection index** (`src/rindex/`, CLI `arity index`): harvests exports,
formals, and help from *installed* packages **without an R runtime**, by reading
R's on-disk formats natively — `rds.rs` (RDS serialization), `lazyload.rs`
(`.rdb`/`.rdx` lazy-load databases), `rd.rs` (help). `discover.rs`/`libpaths.rs`
find libraries, `build.rs`/`harvest.rs` populate the cache (`schema.rs`,
`cache.rs`), `remote.rs` fetches a prebuilt index, and `provider.rs` exposes it
as a `SymbolProvider` for the linter and LSP.

**Roxygen analysis** (`src/roxygen/`): hosts the **CST → Rd-tree projector**
(`project_rd.rs` + submodules), a *test-only* faithful diagnostic behind the
projector-parity gate (`tests/roxygen_projector.rs`). It is not a roxygen2
reimplementation and must never be patched to make a case pass — a divergence
means the parser is wrong. See the `roxygen-parity` skill.

**Config** (`src/config.rs`): `arity.toml` schema, loading, and ancestor-walk
discovery. The library API still takes a fully-resolved `FormatStyle`; the CLI,
LSP, and `arity index` all resolve config so their walks honor the same
excludes. The schema:

- top-level `exclude` (gitignore-style; **replaces** `DEFAULT_EXCLUDE`, which
  mirrors air's defaults), `extend-exclude` (adds to it), and `cache` (the
  persistent already-formatted cache; `--no-cache` overrides per run). Excludes
  are top-level, not under `[format]`, because format and lint share one walk.
- `[format]`: `line-width`, `indent-width`, `line-ending`
  (`auto`/`lf`/`crlf`/`native`).
- `[lint]`: `select` (allowlist; when `Some`, only those run) and `ignore`
  (subtracted). Unknown rule IDs are reported at lint time, not parse time.
- `[index]`: `library-paths`, `cache-dir`, `auto-build`, `help`.

Conventions when extending it: every struct is
`#[serde(deny_unknown_fields, rename_all = "kebab-case")]`, so a user's typo is
an error rather than a silent no-op, and TOML keys are kebab-case. `remote_url`
is deliberately `#[serde(skip)]` and read from `ARITY_REMOTE_URL` instead—
network egress is a per-user consent decision, not a committed project setting.
Per-rule config lives in `[lint.rules.<id>]` tables, typed one struct per
configurable rule on `RulesConfig` (so, unlike an unknown ID in
`select`/`ignore`, a mistyped rule ID here is a *parse* error). It reaches rules
as `RuleContext::config`, carried on `ResolvedRules` rather than through
`run_rules`' parameter list. Only `undesirable-function` takes options today;
per-rule severity is still reserved (`TODO.md` §I4).

**Formatter** (`crates/arity-formatter`, engine in `src/formatter/` there):
consumes the CST and uses a Wadler/Prettier-style document IR (`ir.rs`) printed
by a single best-fit layout engine (`printer.rs`) that makes all line-break
decisions. `rules/` builds the IR per construct; `core.rs` exposes `format` and
`format_with_style`; `style.rs` is `FormatStyle` (with the optional
`serde`/`schema` derive features); `trivia.rs`/`context.rs`/`render.rs` are
support. `roxygen.rs`
formats `ROXYGEN_BLOCK`s — reflowed one `#'` line at a time, with layout chosen
by a tag's `TagClass` and **never** by its written form (a body written inline
after `@details` and one written on the next line canonicalize to the same
output; Tenet 1). Two CLI-side concerns stay in the **root crate's**
`src/formatter/` behind the `src/formatter.rs` bridge: `check.rs` (the batch
`check_paths*` API, which needs `file_discovery`) and `cache.rs` (the
persistent fixed-point cache for `format --check` — a disposable optimization
that must never be a source of errors; its cache key stays the CLI's version).
Target style is the tidyverse R style guide. The native-IR migration is
complete (subset/call/function arg-lists, curly-curly, parens, if/else including
comment-bearing chains, and external-body control flow all build native IR, with
comment relocation handled structurally). `Ir::verbatim`/`verbatim_forced` no
longer bridges a whole construct; it now only carries relocated comment text and
rendered-flat `conditional_group` candidates.

**Linter** (`src/linter/`): `check_paths` walks files, parses, and reports
`LintStatus` (`Clean`/`Findings`/`ParseDiagnostics`); parse diagnostics
block linting a file. The linter is **purely semantic**: anything the
formatter's `--check` mode can catch belongs to the formatter, not here. Ships
48 rules across six categories (12 correctness, 12 suspicious, 5 readability,
10 performance, 5 documentation, 4 meta) with autofixes, `# arity-ignore`
suppression, and a generated rule reference. The `meta` rules are the odd ones out:
they lint arity's own suppression directives rather than R code, reading the
parsed directive list off `RuleContext::suppressions`. `outdated-suppression`
runs on the `Rule::check_suppressions` post-pass, because "did this directive
match anything" is a fact about the driver's filtering step and does not exist
until every rule has emitted. `src/linter/rules.rs` is the **single source of
truth** registry (`rules_by_category`, from which `all_rules`, `all_rule_ids`,
and the reference page's category sections are all derived) and owns
the dispatch: rules declare the `SyntaxKind`s they care about via
`Rule::interests` and one shared CST walk calls `Rule::check`; whole-file rules
leave `interests` empty and override `Rule::check_file`. `run_rules` also owns
suppression filtering (it is the only place holding both the directive map and
the findings) and the post-suppression pass. The `add-lint-rule`
skill walks the full sequence; the `linter-investigation` skill triages the
linter against a real-world R codebase.

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
references, rename (with prepare) and `workspace/willRenameFiles`, document and
workspace symbols, document highlight, semantic tokens, folding and selection
ranges, document links, document color, and call and type hierarchy, plus
on-disk change detection via dynamically-registered
`workspace/didChangeWatchedFiles` watchers (`arity.toml`, `DESCRIPTION`,
`NAMESPACE`, `.R`; see `src/lsp/watched_files.rs`) and
`workspace/didChangeWorkspaceFolders`, backed by the introspection index and a
per-file semantic model. The main loop owns no salsa database: read-only
requests run on a purpose-built read `TaskPool` (not rayon's global pool), with
a separate single-thread index pool isolating the one unbounded-duration job
(background package indexing), and linting is serialized on a **dedicated
thread** that owns the persistent `IncrementalDatabase`. This is forced by salsa
being strictly single-writer (a `set_*` setter blocks until all other db handles
drop) combined with cross-file lint *writing* sibling files into the db—so lint
can't run on a shared read snapshot. Each lint splits into a cheap write-phase
(`prepare_document_in_project`, `&mut db`, on the lint thread) and an expensive
read-phase (`analyze_prepared`, `&db`, on the read pool), so a long analyze
doesn't block queued reads. The lint thread *coalesces* requests (latest version
per URI wins) in lieu of a debounce. See the module doc for the full rationale.

**File discovery** (`src/file_discovery.rs`): `collect_r_files` walks paths for
`.R` files (case-insensitive extension, via `ignore`) honoring the config's
`ExcludeFilter`; rejects non-`.R` explicit file paths unless force-excluded (so
a runner like pre-commit staging a non-R file is skipped, not an error).

## Invariants & conventions

- Treat CI as the source of truth for quality gates (`.github/workflows/`):
  cross-platform build/test (`build-and-test.yml`), `cargo-audit` +
  `cargo-deny`, and `lint.yml`'s clippy `-D warnings`, rustfmt check, and
  **panache** prose-formatting check (Markdown; `panache.toml` lists the
  excluded generated and non-prose files). `devenv.nix` runs clippy, rustfmt,
  eslint, and panache-format as git hooks locally.
- Formatter output must be **idempotent** (`format(format(x)) == format(x)`);
  the formatter and parser test suites guard losslessness +
  idempotence—byte-identical output is the bar for "behavior-preserving"
  refactors.
- Dependency changes must stay compatible with `cargo-audit` and `cargo-deny`
  (`deny.toml`).

## Performance & benchmarks

Speed is measured, not asserted, and the measurement is **opt-in and local**:
neither CI nor `cargo test` runs it, and it is never a quality gate.

- `task bench` runs `scripts/bench.sh`: formatter vs `air` (and, via
  `ARITY_BENCH_STYLER=1`, `styler`) and linter vs `jarl`, each at two
  scopes—synthetic single-file tiers built from the formatter fixtures, and a
  real R package (`tidyr`, shallow-cloned to a cache; override with
  `ARITY_BENCH_PROJECT`). Timing prefers `hyperfine` + `jq`, falling back to a
  shell loop. Tools missing from PATH are skipped silently.
- It rewrites the **tracked** artifact `benches/benchmark_results.json`, the
  sole source of the published benchmark page: `docgen` renders it into the
  generated partials, and the numbers are never re-measured at site-build time.
  Moving performance and wanting the docs to show it means re-running
  `task bench` and committing the artifact.
- `task bench-parse` is the criterion microbenchmark of parse + incremental
  reparse (`crates/arity-parser/benches/parse.rs`)—the right tool for
  parser-level work.
- It measures wall-clock speed only, never output equivalence (that is
  `task air-compat`). Report **ratios**, not milliseconds: the tools do
  different work behind different startup floors (`styler` is an R process).
- `src/bench_docs.rs`'s renderer is deliberately tool-generic—adding a
  comparison tool to the artifact needs no code change.

## Distribution & releases

Releases are fully automated off Conventional Commits; the commit type picks the
version. On a push to `main`, once test + `cargo-audit` + `cargo-deny` pass,
`versionary` (`versionary.jsonc`) opens/updates a release PR that bumps the
version, regenerates `CHANGELOG.md`, and propagates the version into
`npm/arity-cli/package.json` (its own version *and* every
`optionalDependencies` entry). The workspace has four versionary packages,
routed by path: the root CLI (bare `v*` tags), `crates/arity-parser` and
`crates/arity-formatter` (independently versioned, tagged
`arity-parser-v*`/`arity-formatter-v*`, each with its own `CHANGELOG.md`), and
`editors/code` (`arity-code`, which `follows` the CLI). Paths under `editors/`
and `crates/` are excluded from the CLI's version calculation—keep commits
atomic per area so path routing produces clean per-crate changelogs. Merging
the release PR tags it and fans
out to `packages.yml` (eight targets—Linux gnu/musl, macOS, Windows, each
x86_64 + aarch64—cross-built with `cargo-zigbuild`, glibc-floor checked, with
keyless provenance attestation), then the VS Code/Open VSX, crates.io, npm, and
PyPI publishes. The crates.io publish (`publish-cargo.yml`, on `v*` tags) runs
`cargo workspaces publish --from-git --skip-published`, which uploads every
workspace crate not yet on crates.io in dependency order—member-crate bumps
therefore publish on the next CLI tag. Because member tags are prefixed, the
`v*` tag filters in the workflows match only the CLI stream, and **only the CLI
stream carries GitHub release assets**.

**Never hand-edit `CHANGELOG.md` or any version field**—they are generated and
your edit is overwritten. The pre-1.0 config sets `bump-minor-pre-major`, so
breaking changes land as minor bumps.

The distribution surfaces themselves:

- `editors/code`—TypeScript VS Code extension, esbuild-bundled
  (`npm run compile`/`watch`/`package`), eslint-gated via the devenv git hook.
  At publish time a platform binary is downloaded from the GitHub release into
  `editors/code/server/` and packaged per-target; at runtime the client resolves
  the server via the `arity.executableStrategy` setting
  (`bundled`/`environment`/`path`), falling back to `arity` on PATH—which is
  also the NixOS path, where a downloaded binary would not run.
- `npm/arity-cli`—a launcher whose `optionalDependencies` pull one
  `@arity-cli/<platform>` package per target, generated from
  `npm/platform-template`.
- `pyproject.toml`—the PyPI package, built by maturin.

## Commits & versioning

- **Conventional Commits** (`type(scope): subject`) and **semantic versioning**.
- Subject line: aim for ≤ 60 chars, ≤ 72 is fine, longer only if truly needed.
- Bodies are short and to the point.
- **Never edit the changelog by hand**—`versionary` generates it.

## Testing layout

**Use test-driven development.** Write the test first, watch it fail, then make
it pass. For a bug, always start by adding a failing test that reproduces it
(typically a new fixture case or snapshot) before touching the fix.

- Integration tests live with their crate: parser suites
  (`parser_snapshots.rs`, `incremental_reparse.rs`, `line_endings.rs`,
  `ast_wrappers.rs`, `node_ptr.rs`, `air_parser_harness.rs`) in
  `crates/arity-parser/tests/` with fixtures in
  `crates/arity-parser/tests/fixtures/parser/<case>/`; formatter suites
  (`formatter.rs`, `range_format.rs`, `air_compat.rs`) in
  `crates/arity-formatter/tests/` with fixtures in
  `crates/arity-formatter/tests/fixtures/formatter/<case>/`; everything else
  (linter, LSP, salsa, roxygen oracles + projector, rindex, config, CLI-level
  format tests in `format_cli.rs`, corpus) in the root `tests/*.rs` with
  fixtures in `tests/fixtures/rindex/<case>/`. Parser fixtures hold `input.R`
  (snapshot the CST + diagnostics, assert losslessness); formatter fixtures
  hold `input.R` + `expected.R`. **Run `cargo test --workspace`**—a bare
  `cargo test` covers only the root crate.
- `insta` snapshots live in each crate's `tests/snapshots/`. Never accept a
  snapshot you have not read.
- Which suite to reach for: formatter bug → a
  `crates/arity-formatter/tests/fixtures/formatter/` case
  (`input.R` + `expected.R`), fixed in the formatter crate's
  `src/formatter/rules/` or `printer.rs`;
  parser bug → a `crates/arity-parser/tests/fixtures/parser/` case +
  `cargo insta review`; lint rule
  → a `#[test]` in `tests/lint.rs` (lint has no fixture dir) plus the rule's own
  `examples()`, which generate its docs page; cross-file/LSP work →
  `tests/lsp.rs`, `tests/lsp_protocol.rs`, `tests/salsa_incremental.rs` (guards
  that a body edit does *not* invalidate the project graph), and
  `crates/arity-parser/tests/incremental_reparse.rs`.
- Both fixture suites are **hand-registered**: a new case only runs once its
  name is added to `fixture_names()` in
  `crates/arity-parser/tests/parser_snapshots.rs` /
  `crates/arity-formatter/tests/formatter.rs`.
- The roxygen oracles live under `tests/oracle/` and are allowlist-gated.
  `tests/roxygen_projector.rs` is the pure-Rust, CI-safe conformance gate (no R:
  it diffs the projector against pinned `.rdtree` files); the `#[ignore]`d
  `tests/roxygen_oracle.rs` and `tests/roxygen_lint_oracle.rs` need R +
  `roxygen2`. See the `roxygen-parity` skill.
- `crates/arity-parser/tests/air_parser_harness.rs` compares against the
  `air_r_parser` crate (a git dev-dependency of `arity-parser` from
  posit-dev/air)—AIR snapshot cases are ported into the parser fixtures as
  hardening input.
- `tests/corpus.rs` is the Tier 0 corpus smoke test (`#[ignore]`d; run with
  `ARITY_CORPUS=<dir> task corpus`): losslessness + idempotence over a large
  body of real R sources, with unparseable files skipped rather than failed.
  `.github/workflows/smoke-test.yml` runs it weekly over cloned R package repos
  and files one deduped issue per (repo, failure category); triage those with
  the `smoke-test-triage` skill.

## Reference-only directories (not part of the build, untracked)

**A fresh clone does not have these**—nothing in `cargo build`/`cargo test`
needs them, so their absence is normal, not a broken checkout. Clone them into
the repo root when you need the corresponding oracle or reference:

```sh
git clone https://github.com/posit-dev/air
git clone https://github.com/tidyverse/style
git clone --branch "v$(cat tests/oracle/.roxygen2-source)" \
  https://github.com/r-lib/roxygen2 roxygen2-ref
```

- `air/`—a local checkout of posit-dev/air (tree-sitter-based R tooling)
  kept for reference/comparison. **Not** in the Cargo build and not exposed via
  this CLI. It has its own `air/CLAUDE.md` describing *that* project's
  conventions (e.g. `just test`, `air.toml`)—do not apply those to arity. Note
  that `task air-compat` needs the `air` **binary** on PATH (devenv provides
  it), not this checkout.
- `style/`—vendored copy of the tidyverse R style guide (the formatter's
  target style).
- `roxygen2-ref/`—a local checkout of r-lib/roxygen2, the reference
  implementation the roxygen oracles are measured against. Read by
  `scripts/harvest-roxygen-corpus.R`; the version it must be pinned to is
  recorded in `tests/oracle/.roxygen2-source`.
