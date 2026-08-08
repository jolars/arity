---
paths:
  - "src/config.rs"
  - "arity.toml"
  - "tests/config.rs"
  - "docs/src/reference/configuration.md"
---

# Config rules

`src/config.rs`: the `arity.toml` schema, loading, and ancestor-walk discovery
(`--config <path>` forces one, `--no-config` ignores). Every command honors it,
so a schema change is a change to format, lint, LSP, and `arity index` at once.

## Conventions when extending the schema

- Every struct is `#[serde(deny_unknown_fields, rename_all = "kebab-case")]`, so
  a user's typo is an **error, not a silent no-op**. TOML keys are kebab-case.
- **A new key needs a reason for its level.** Excludes are top-level, not under
  `[format]`, because format and lint share one walk. `[compat]` is top-level
  because R and roxygen2 version floors are *project facts*, not lint options.
- **The library API takes a fully-resolved `FormatStyle`.** Config resolution is
  the caller's job (CLI, LSP, `arity index` each resolve it) so every walk
  honors the same excludes.
- `remote_url` is deliberately `#[serde(skip)]` and read from `ARITY_REMOTE_URL`
  instead: network egress is a per-user consent decision, never a committed
  project setting. Anything with the same character gets the same treatment.
- `LintConfig::compat` is a `#[serde(skip)]` mirror of the parsed `[compat]`
  table, so the CLI and the LSP lint thread ship the floors without a parallel
  plumbing path.

## The schema today

- top-level: `exclude` (gitignore-style; **replaces** `DEFAULT_EXCLUDE`, which
  mirrors air's defaults), `extend-exclude` (adds to it), `cache`.
- `[format]`: `line-width`, `indent-width`, `line-ending`
  (`auto`/`lf`/`crlf`/`native`).
- `[lint]`: `select` (allowlist; when `Some`, only those run), `ignore`
  (subtracted), and `[lint.rules.<id>]` per-rule tables. Unknown IDs in
  `select`/`ignore` are reported **at lint time**; a mistyped ID in
  `[lint.rules.<id>]` is a **parse** error, because those tables are typed.
- `[compat]`: `r`, `roxygen2` (MSRV-style plain version strings). Unset → derived
  per file from the enclosing `DESCRIPTION` (`src/project/description.rs`); no
  floor at all → the version-aware rules stay silent.
- `[index]`: `library-paths`, `cache-dir`, `auto-build`, `help`.

## When you change it

Update all three: the repo's own `arity.toml` (it dogfoods the discovery path
and documents the defaults), `docs/src/reference/configuration.md` (hand-written,
not generated), and `tests/config.rs`.
