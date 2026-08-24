# Agent Instructions

This file is the complete instruction source for agents in this repository. It
combines repository-wide and subsystem rules so tools that only discover
`AGENTS.md` receive the full guidance.

## Project and architecture

Arity is a Rust CLI for R with formatter, linter, parser, language-server,
semantic/project, and package-index capabilities.

- Root crate (`arity`): CLI, LSP, lint, semantic/project, roxygen projector, and
  rindex.
- `crates/arity-parser`: lossless R CST parser, `DESCRIPTION` DCF grammar, and
  typed AST wrappers.
- `crates/arity-formatter`: format engine. `src/formatter.rs` is its root-crate
  filesystem bridge.
- Other distributed surfaces: `editors/code`, `npm/`, `pyproject.toml`, and
  `docs/`.

## Repository-wide tenets

1. The formatter is the sole layout authority. Autofixes are textual rewrites;
   the pipeline is fix-then-format.
2. Incremental parsing is first-class. Parser/CST changes must preserve the
   reparse path in `src/incremental.rs`.
3. Parsing belongs in the parser. Never patch parser mistakes downstream.
4. Parser reconstruction is byte-lossless: `reconstruct(text) == text`.
5. Formatting is idempotent: `format(format(x)) == format(x)`.
6. Semantics are static. Do not evaluate R or introduce an R runtime into
   production behavior.

Air compatibility is a soft, one-directional formatter gauge, never a quality
gate. CI workflows in `.github/workflows/` are the quality-gate source of truth.
Dependency changes must pass `cargo-audit` and `cargo-deny`.

## Working and testing conventions

- Use TDD: reproduce with a failing test first, then fix.
- Run `cargo test --workspace`; bare `cargo test` covers only the root crate.
- Fixture suites are hand-registered through `fixture_names()`; merely adding a
  directory does not run it.
- Review every `insta` snapshot with `cargo insta review`; never accept an
  unread snapshot.
- Preserve losslessness and idempotence. Benchmarks/profiles are opt-in local
  measurements, not release gates.
- Recurring workflows use the `add-lint-rule`, `linter-investigation`,
  `roxygen-parity`, `smoke-test-triage`, and `perf-investigation` skills.
- Optional untracked oracle/reference checkouts (`air/`, `style/`,
  `roxygen2-ref/`) may be absent in a fresh clone.

Primary checks:

```sh
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Useful focused checks:

```sh
cat file.R | cargo run -- parse --verify --quiet
cargo run -- format --check <path>
cargo run -- format --verify <file.R>
cargo run -- lint --fix <path>
```

`task <name>` wraps common workflows including `lint`, `format`, `test`,
`docs-gen`, `air-compat`, `corpus`, `bench`, `profile`, and roxygen oracles.

## CLI

Scope: `src/cli.rs`, `src/main.rs`, and CLI integration tests such as
`tests/format_cli.rs`, `tests/lint_cli.rs`, and `tests/config.rs`.

- Command syntax and help come from clap. Regenerate the pinned CLI reference
  with `task docs-gen` after changing them; never edit it directly.
- Stdout/stderr placement and exit codes are user-facing integration surfaces.
  Preserve them unless intentionally changing the contract, and pin changes in
  CLI integration tests.
- The stdin contract is stable. `-` as the only path makes `format`, `lint`, and
  `parse` read one buffer from stdin; `format -` writes only the formatted buffer
  to stdout, the working directory anchors config discovery, and a pathless
  buffer is R unless `--stdin-filename` says otherwise. Changing any of those is
  breaking.

## Parser and typed AST

Scope: `crates/arity-parser/**` and the parsing parts of `src/incremental.rs`.
The root crate re-exports this crate as `arity::{syntax, ast, parser, dcf}`;
intra-repo users should continue using `crate::parser::…`.

### Hard constraints

- Preserve every byte: whitespace, comments, `%...%`, brackets, and line
  endings. Every parser feature needs a losslessness assertion.
- Errors never abort parsing. Diagnostics are a side channel and a recoverable
  CST is always produced.
- Recognize lexical/structural shape only; meaning belongs in semantic code.
- Keep the parser dependency-thin and salsa-free (`rowan`, `serde`, `smol_str`).
  Salsa belongs above it in `src/incremental.rs`.
- Pipeline: lexer tokens → Pratt expression plus recursive structural parsing →
  events → rowan CST. `SyntaxKind` uses `SCREAMING_SNAKE_CASE`.
- `parser::expr` and `parser::roxygen` are public but semver-loose; do not grow
  their low-level surface casually.

### AST wrappers

`ast/` is a zero-cost, read-only navigation view, not a re-model. R atomic
operands are bare tokens, so operand accessors return `SyntaxElement` and
`Expr::cast` accepts node expressions and token atoms. Prefer `HasArgList` and
shared predicates in `kinds.rs` over bespoke CST walking. Linter, semantic, and
LSP consumers navigate through wrappers; the formatter intentionally uses raw
CST for byte-level layout.

### DCF grammar

`dcf/` independently parses `DESCRIPTION` with the same losslessness and static
constraints.

- R and DCF have distinct `SyntaxKind` types; never glob-import both. Outside
  `dcf/`, use `dcf::Document`, `Field`, and other typed wrappers.
- Every physical line is one line node, or the first `VALUE_LINE` owned by a
  `FIELD`. `COMMENT_LINE` may be a field child because R resumes continuations
  after comments.
- DCF deliberately has no event or incremental pipeline. Never use
  `str::lines()`, which destroys CRLF fidelity.
- Report only lexical facts matching `read.dcf` hard errors. Duplicate/required
  field policy is lint; indentation style is format.
- DCF fixtures live under `crates/arity-parser/tests/fixtures/dcf/`, registered
  in `tests/dcf_snapshots.rs`. Protect CRLF fixtures with `.gitattributes`.
- `task dcf-oracle` is an ignored differential against `read.dcf`. A new
  disagreement must be fixed or deliberately normalized in its recorded
  divergence table, never explained away.

### Incremental parsing

- Preserve the reparse ladder: token → block → top-level statement → full parse
  (`parser/reparse.rs`).
- All reparse entry points are total for caller-provided `Edit`: invalid ranges
  return `None`, never panic. Staged reparsing verifies with `Edit::produces`
  before parsing and must not allocate the final document via apply-and-compare.
- Syntax node pointers are position-independent. Store green nodes in salsa,
  never red `SyntaxNode`s (`SyntaxNode` is not `Send`/`Eq`).

Parser fixtures are under `crates/arity-parser/tests/fixtures/parser/` and are
registered in `tests/parser_snapshots.rs`. The air parser harness is a hardening
oracle, not a gate; port useful cases into fixtures. Use `task bench-parse` for
parser performance.

## Roxygen parser and projector

Parsing belongs in parser roxygen modules, formatting in formatter `roxygen.rs`,
and `src/roxygen/` contains only the test-time CST→Rd projector. Use the
`roxygen-parity` skill for parity gaps.

- Roxygen `#'` sub-tokens must tile every input byte. Marker, tag, arguments,
  prose, Rd macros, and markdown structure live in CST and are never re-lexed
  downstream.
- The projector is a faithful diagnostic, not a roxygen2 implementation or a
  place to fix divergence. It emits parser-owned Rd section subtrees; fix CST or
  encoding translation, never patch projector output to pass.
- `tests/roxygen_projector.rs` is the pure-Rust pinned/allowlisted CI gate. When
  a case starts passing, ratchet it into the allowlist.
- R-backed oracle tests are ignored and run through `task roxygen-oracle` and
  `task roxygen-lint-oracle`. They use optional `roxygen2-ref/`, pinned by
  `tests/oracle/.roxygen2-source`.

## Formatter

Scope: `crates/arity-formatter/**`, `src/formatter.rs`, and `src/formatter/**`.
The engine is `crates/arity-formatter/src/formatter/`.

### Engine constraints

- Output depends only on rules and the single best-fit Wadler/Prettier-style
  layout engine. Reject construct-specific hard-coded layout escapes.
- Input line breaks never influence output. Persistent line breaks are a bug.
- A behavior-preserving refactor means byte-identical output.
- Never hide a parser defect or reintroduce whole-construct verbatim output.
  `Ir::verbatim`/`verbatim_forced` is limited to relocated comment text and
  rendered-flat conditional-group candidates; model constructs in native IR.
- `roxygen.rs` formats by semantic `TagClass`, never by whether a tag body was
  originally inline or on the next line.
- Keep `FormatStyle`'s optional `serde`/`schema` features optional and keep
  `arity-parser` as the formatter crate's only heavy dependency.

The root bridge owns filesystem concerns: `check.rs` batch discovery,
`source.rs` grammar selection, and `cache.rs`. The format-check cache is a
disposable optimization and never an error source. Its key includes CLI version
and grammar; `--no-cache` overrides it. Roxygen probing is R-only, and declining
to format a `DESCRIPTION` is not an error.

### Format directives

Directive grammar comes only from `arity_parser::directive`.
`# arity-format skip`, `off`…`on`, `skip-file` and `# arity` forms are the sole
layout opt-out.

- Splice skipped spans byte-for-byte at their own column. `Ir::Skipped` clears
  pending indentation and stays distinct from `Ir::verbatim`.
- Directives are honored only in the three statement-list sequencers, making
  regions list-local. Keep `is_honored_position` public because the linter uses
  it as the behavioral authority.
- `skip-file` short-circuits `format_node`, preserving even line endings;
  `format_range` returns `None`.

### Table layout

`rules/table.rs` lays a `tribble()` call out as a header line plus one line per
row. It is the one sanctioned construct-specific layout, and it stays narrow:
`tribble` alone, bare or `::`-qualified, with no directive and no configurable
name list.

- Rows come from the **header count**, never from input line breaks. Air's
  `fmt: table` decides rows from the source layout; arity must not.
- Cells are rendered flat through `Printer::render_flat` and emitted as measured
  text, because alignment only holds if a cell is exactly one line.
- Declining the table is total: it returns `Option`, never an error, and every
  ambiguous shape (ragged rows, holes, named arguments, `...`/`!!`/`!!!`, a cell
  with a forced break, any comment in the call) falls back to ordinary call
  formatting rather than guessing a row shape.
- Where a call already is a well-formed table the output is byte-identical to
  air, which makes air a real oracle for the alignment arithmetic. The recorded
  deviations are the fallback fixtures, where air tables a call that is not one.

### DESCRIPTION formatting

`formatter/description/` is a separate first-fit formatter, not the R layout
engine. `desc` is a style reference, never an oracle, because it drops comments.
Use `task desc-compat` only as a gauge.

- The field-class table is closed and defaults to `Opaque`, preserving unknown
  value line structure byte-for-byte.
- Comments attach forward to the next field, matching the linter's
  `next_meaningful_dcf_sibling`; do not retarget directives.
- Refuse formatting when interpretation is unsafe: duplicate fields, multiple
  records, whitespace before colon, or non-UTF-8 `Encoding`. Refusal is not an
  error.
- Continuation indentation is always four spaces, independent of R
  `indent_width`.

### Compatibility and tests

`task air-compat` runs the ignored fixed-point test `air(arity(x)) == arity(x)`
and regenerates `AIR_COMPAT.md`. Adopt idiomatic rules or record deliberate
differences with rationale in `tests/air_compat_allowlist.toml`; an unexplained
difference is an open question but never a build failure.

Formatter cases use `input.R`/`expected.R` directories under
`crates/arity-formatter/tests/fixtures/formatter/`, registered in
`tests/formatter.rs`. Keep formatter, range, losslessness, idempotence, and
`tests/roxygen_format_stability.rs` green.

## Linter

Scope: `src/linter.rs` and `src/linter/**`. Use `add-lint-rule` when adding a
rule and `linter-investigation` for corpus triage.

### Scope and dispatch

- Lint is semantic. Layout detectable by formatter `--check` belongs there.
- Keep false positives rare. Make rules precise and conservative or opt-in.
- Parse diagnostics block lint for R and DCF; `check_paths` reports `Clean`,
  `Findings`, or `ParseDiagnostics`.
- Inputs include `.R` and package-root `DESCRIPTION`. During walks, exclude
  nested fake packages; explicitly named descriptions are always linted. Reading
  is never conditional on selected rules: `syntax-error` is not a rule.
- Rules never walk independently. Declare `Rule::interests` and join the shared
  walk; whole-file rules override `check_file` with empty interests.
- `src/linter/rules.rs::rules_by_category` is the single catalogue for all rule
  IDs, lists, and docs. DCF rules implement `DcfRule` and register as
  `AnyRule::Dcf` in that same catalogue. `ResolvedRules::with_config` splits R
  and DCF dispatch exactly once; never create a second registry.
- `run_rules` alone owns suppression filtering and the post-pass.
  `check_suppressions` handles facts known only after filtering. Meta rules read
  parsed directives from `RuleContext`; `misplaced-suppression` asks the
  formatter's public predicate rather than recreating its behavior.

### Identity, fixes, and configuration

- IDs are stable user-facing kebab-case and must not equal directive verbs
  `skip`, `skip-file`, `off`, or `on`. Renaming is breaking.
- Every rule has a description and executable `examples()`. Package-level
  examples declare `doc_package` so docs execute in a synthetic package.
- Fixes are textual and must leave parseable, lossless code without dropped
  trivia. They do not owe formatting or line width and never invoke formatter.
  Use tight/atom-guarded spans or withhold the fix for unsafe shapes while still
  reporting. `Safe` applies with `--fix`; others need `--unsafe-fixes`.
- `[lint.rules.<id>]` maps to typed structs on `RulesConfig`; unknown keys there
  are parse errors. Unknown `select`/`ignore` IDs are lint-time errors. Options
  travel through `ResolvedRules` to `RuleContext::config`, not through
  `run_rules` parameters.
- Version rules use resolved R/roxygen2 floors and remain silent if neither
  config nor `DESCRIPTION` supplies a floor.

### Suppressions

Parse suppression syntax only with `arity_parser::directive`. Deprecated
`# arity-ignore` aliases behave like skip directives but stay out of docs.
`Coverage::{File, Range, Nothing}` drives a single suppression predicate. Lint
regions are byte ranges, so unclosed `off` reaches EOF (unlike formatter
list-local regions). A blanket directive cannot silence a finding whose span is
the directive comment itself; an explicitly named meta rule can.

Add tests in `tests/lint.rs` or `tests/lint_description.rs` plus examples; use a
complete `TEST_DESCRIPTION` for package fixtures. Fixed-output tests must parse,
and curated width-safe cases remain format-clean. Rules docs are generated by
`task docs-gen`; never edit them directly.

## Semantic and project layers

Scope: `src/semantic/**`, `src/project/**`, and their incremental queries.
`semantic` is strictly single-file; cross-file logic belongs in `project`.

### Single-file semantics

- Never evaluate R. Semantic owns scopes, bindings, in-file resolution,
  `library()` tracking, and per-region CFGs.
- `StaticBaseR` and `BundledPackages` symbol lists are generated by the scripts
  and `.github/workflows/cran-symbols.yml`; never hand-edit them.

### Project graph

- Project owns source dependencies, package shared namespaces, export
  projections, class inheritance, native registrations, and pure
  `ProjectScope::build`.
- Keep `FileScope` reasons separate: `read_elsewhere`, `exported_by_namespace`,
  and `is_s3_method`. `used_elsewhere` unions only the first two.
- Parse all description facts together: package name, dependencies, compat
  floors, Roxygen, and Collate. `R` names the language and is never a package
  dependency.
- Only `Depends` attaches bare names. `Imports` requires `pkg::` or NAMESPACE
  import declarations.
- Native registration scanning reads `src/` only for `.registration = TRUE`.
  Harvest actual routines; never blanket-suppress unresolved names.
- `ProjectScope::build` remains pure. Record wholesale imports there; resolve
  them against `LibraryIndex` later in `external_resolution`. Its
  `resolution_incomplete` flag means only dynamic/unanalyzed `source()`.

### Incrementality

- Per-file projections and `DescriptionFacts` stay range-free so body/prose
  edits backdate without rebuilding the graph. `tests/salsa_incremental.rs`
  guards this.
- Salsa models text → CST → semantics above the parser and is single-writer.
  Store green nodes. `DescriptionFile` tracks text while `description_facts` is
  its range-free `Eq` projection.

## Language server

Scope: `src/lsp.rs` and `src/lsp/**`. Read the module documentation's full
threading rationale before changing the main loop. Transport is synchronous
stdio JSON-RPC via `lsp-server`; salsa cancellation is a synchronous unwind.

### Threading and scheduling

- The main loop owns no salsa DB. The dedicated lint thread owns the persistent
  DB and is the sole writer.
- Split lint into cheap mutable `prepare_document_in_project` on the lint thread
  and expensive immutable `analyze_prepared` on the read pool. Long analysis
  must not block queued writes/reads.
- Use purpose-built pools: latency-sensitive reads and a separate single-thread
  pool for unbounded package indexing. Never put unbounded work on the read
  pool.
- Coalesce requests by newest URI version. At most one analysis is in flight; a
  newer edit cancels analysis of the same URI, but a different URI waits and is
  never cross-canceled.
- Salsa cancellation or cache miss falls back to fresh parsing. Reads must stay
  correct even when not warm.

### R and DESCRIPTION

- Decide `DocumentKind` once at open; filename overrides `languageId`.
- `r_doc_snapshot` rejects descriptions; `doc_snapshot_any` serves both. Every
  new handler must deliberately choose one.
- Existing read methods branch on kind inside `*_via_db`, not by adding
  `ReadJob` variants. A genuinely new method adds a variant and exhaustive arms
  in both `run_read` and state drain; no wildcard matches.
- Full-document formatting serves both grammars; range formatting is R-only.
- Pathless URIs use `kind.placeholder_file_name()`, never a literal filename.
- An open description is authoritative: seed it before `upsert_description`
  (graph refresh reads disk), and emit `RelintAll` only if facts changed.

### Buffers and edits

- Open documents use `Arc<TextBuffer>` containing text and incrementally spliced
  `LineIndex`. The main loop mutates only via `Arc::make_mut`; version stays
  outside the Arc.
- Share one `Arc<str>` among buffer, `SourceFile`, and `PrevParse`. Never add
  `.text().to_string()` on dispatch; use `text_arc()`.
- In equality checks, `Arc::ptr_eq` may precede content comparison but never
  replace it. Equal content in fresh allocations must not invalidate queries.
- Reads clone Arcs and use the buffer's index; do not rebuild a live-buffer
  `LineIndex` per request. Salsa independently rebuilds its own index, and both
  must agree (`apply_edit_matches_rebuild_exhaustively`).
- Keep public `compute_*(&str)` APIs; hot paths use buffer-taking `*_in` and
  `*_via_db` forms.
- Formatting returns line-diff edits to preserve cursor/folds. Only a diff over
  more than half the span falls back to one replacement. Full/range and R/DCF
  output must reproduce formatter bytes exactly.

Convert URIs only through `src/lsp/uri.rs::{to_path,from_path}`; tests must not
assume slash direction. Watch workspace changes for `arity.toml`, DESCRIPTION,
NAMESPACE, `.R`, and workspace folders. Project facts belong in `arity.toml`;
machine facts belong in editor settings. Rename supports symbols, files, and
folders and drops targets leaving workspace scope.

Test behavior in `tests/lsp.rs`, wire protocol in `tests/lsp_protocol.rs`, and
incremental graph preservation in `tests/salsa_incremental.rs`.

## R package introspection index

Scope: `src/rindex/**` and `tests/rindex.rs`; CLI command `arity index`.

- Never invoke R/Rscript. Read RDS, lazy-load `.rdb`/`.rdx`, and Rd natively.
- Network fetches require per-user consent through `ARITY_REMOTE_URL`; keep the
  corresponding config field `#[serde(skip)]`, never project-configurable.
- Malformed installed files degrade to no symbols, never panic. A stale/missing
  cache may reduce precision but cannot change correctness.
- Discovery/library paths feed build/harvest, cache/schema, then
  `SymbolProvider`. Indexing honors the same `arity.toml` excludes and `[index]`
  settings as other commands.
- LSP indexing stays on its isolated one-thread pool.
- Tests use `tests/fixtures/rindex/`; never assert against packages installed on
  the current machine.

## Configuration and discovery

Scope: `src/config.rs`, `arity.toml`, config tests, and configuration docs.
Every command honors discovered config; `--config` forces a path and
`--no-config` ignores config.

- Config structs use `#[serde(deny_unknown_fields, rename_all = "kebab-case")]`.
  Typos are errors. Place keys according to ownership: shared excludes and
  project compatibility facts are top-level.
- Library APIs take a fully resolved `FormatStyle`; CLI, LSP, and index resolve
  config so all walks honor the same excludes.
- Secrets/egress choices like remote index URL come from environment and use
  `#[serde(skip)]`, never committed project config.
- `LintConfig::compat` is a skipped mirror of top-level parsed compatibility so
  CLI and LSP share one plumbing path.
- Schema: top-level `exclude` (replaces defaults), `extend-exclude`, `cache`;
  `[format]` line/indent width and line endings; `[lint]` selection, ignores,
  and typed rule tables; `[compat]` R and roxygen2 version floors; `[index]`
  library paths, cache dir, auto-build, and help.
- Missing compat floors derive per file from enclosing DESCRIPTION; with no
  floor, version-aware rules are silent.
- Schema changes update the dogfood `arity.toml`, hand-written configuration
  docs, and `tests/config.rs`.

## Documentation, benchmarks, and profiles

`docs/` is mdBook. Never hand-edit generated pages:

  | Generated page                                       | Source                                |
  | ---------------------------------------------------- | ------------------------------------- |
  | `docs/src/reference/cli.md`                          | `build.rs` from clap                  |
  | `docs/src/reference/rules.md`, `docs/src/version.md` | `docgen`                              |
  | benchmark meta/results pages                         | `src/bench_docs.rs` from tracked JSON |

Regenerate all with `task docs-gen`; pinning-test failures mean regenerate, not
edit snapshots. Other prose is hand-written and panache-formatted. New pages
need `SUMMARY.md` entries. Canonical/sitemap examples skip mdBook redirect
stubs.

Benchmarks are measured, never asserted. `task bench` compares formatter and
linter tools at synthetic and real-package scopes and rewrites tracked
`benches/benchmark_results.json`, the sole published performance source. Report
ratios, not milliseconds; it measures wall time, not equivalence. Use
`task air-compat` for output comparison and `task bench-parse` for parser work.
The renderer is tool-generic.

`task profile` samples `benches/profile.rs` and writes only under
`target/profile/`. Preserve `scripts/profile.sh` as flag/profile authority,
including frame-pointer call graphs and `[profile.profiling]`. Use the
`perf-investigation` workflow for measured performance changes.

## Editor and packaging

Scope: `editors/code/**`, `npm/**`, `pyproject.toml`, and their release
workflows.

- VS Code is TypeScript/esbuild and Biome-gated. Run `npm run check-types` from
  `editors/code` and `biome ci` from the repository root for focused validation.
- Extension packaging stages a target server, but runtime supports
  bundled/environment/path and PATH fallback; bundled must not become mandatory
  (notably on NixOS).
- The npm CLI selects generated platform packages through optional dependencies.
- PyPI uses maturin.

## Releases

Releases derive from Conventional Commits; use `type(scope): subject`. Never
hand-edit `CHANGELOG.md` or any version field: versionary owns Cargo, npm,
editor, and other versions. Pre-1.0 breaking changes produce minor bumps.

Release streams are root CLI `v*`, parser `arity-parser-v*`, formatter
`arity-formatter-v*`, VS Code following CLI, and Zed `arity-zed-v*`. Only the
root stream may carry assets: the Zed extension resolves its download with
`latest_github_release(require_assets: true)`, which cannot filter by tag
prefix, so an asset on any sibling stream would shadow the CLI release.

Editor/member paths are excluded from CLI version calculation, so commits
spanning root, member crates, or `editors/` must be split atomically. The VS
Code bundled binary must not be load-bearing—PATH fallback remains necessary
for NixOS. `editors/zed` obeys the same rule by resolving PATH before
downloading. It is a language-server-only extension, deliberately outside the
root workspace (its own `[workspace]`, edition 2021, `wasm32-wasip2`), so the
`zed` CI job is the only thing that compiles it; a version bump must also
refresh its `Cargo.lock`. Its registry entry in `zed-industries/extensions` is
submitted by hand, which is why it does not follow the CLI version.

Main pushes run tests/audit/deny and versionary opens a release PR. Merging tags
and fans out builds/publishing. `publish-cargo.yml` publishes unpublished
workspace crates in dependency order on CLI tags; only the CLI stream owns
GitHub assets.
