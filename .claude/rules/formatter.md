---
paths:
  - "crates/arity-formatter/**/*.rs"
  - "src/formatter.rs"
  - "src/formatter/**/*.rs"
---

# Formatter rules

Engine: `crates/arity-formatter/src/formatter/`. Two CLI-side concerns stay in
the **root crate** behind the `src/formatter.rs` bridge. Target style is the
tidyverse R style guide (vendored in `style/`, untracked).

## Hard invariants

- **The formatter is the sole authority on layout** (Tenet 1). Output is decided
  solely by the rules and the layout engine. **Push back on hard-coded special
  cases** for specific constructs.
- **No persistent line breaks.** Unlike air, the input's existing line breaks
  never influence the result. A rule that reads whether the author broke a line
  is a bug, not a feature.
- **Idempotence:** `format(format(x)) == format(x)`.
- **Losslessness is assumed, not enforced here.** The CST is lossless; focus on
  layout. **Never paper over a parser bug in the formatter** (Tenet 3) — fix it
  in the parser.
- Byte-identical output is the bar for a "behavior-preserving" refactor.

## Engine shape

- A Wadler/Prettier-style document IR (`ir.rs`) printed by a **single best-fit
  layout engine** (`printer.rs`) that makes *all* line-break decisions. `rules/`
  builds IR per construct; `core.rs` exposes `format`/`format_with_style`;
  `style.rs` is `FormatStyle`; `trivia.rs`/`context.rs`/`render.rs` support.
- **The native-IR migration is complete.** `Ir::verbatim`/`verbatim_forced` now
  carries only relocated comment text and rendered-flat `conditional_group`
  candidates. **Do not reintroduce whole-construct verbatim** to get a shape
  out — model it in IR, with comment relocation handled structurally.
- `roxygen.rs` reflows `ROXYGEN_BLOCK`s one `#'` line at a time. Layout is
  chosen by a tag's `TagClass` and **never** by its written form: a body written
  inline after `@details` and one written on the next line canonicalize to the
  same output (Tenet 1).
- The optional `serde`/`schema` features derive serde and schemars on
  `FormatStyle` for embedders (a dprint plugin). Keep them optional and keep the
  crate's dependency on `arity-parser` its only heavy one.

## The root-crate bridge

`src/formatter.rs` re-exports the engine and hosts what needs the filesystem:

- `check.rs` — the batch `check_paths*` API (needs `file_discovery`).
- `source.rs` — the per-file grammar branch, so the write path and `--check`
  cannot drift on which file is formatted how. The roxygen markdown probe is
  R-only; a `DESCRIPTION` decline is not a failure.
- `cache.rs` — the persistent already-formatted cache for `format --check`. It
  is a **disposable optimization that must never be a source of errors**; its
  cache key stays the CLI's version, so a formatter change can never hand back a
  stale "clean". `--no-cache` overrides per run. The key names the **grammar**
  first: a lone comment line is a fixed point of both, and a cross-grammar hit
  would report a dirty `DESCRIPTION` clean.

## Directives (`formatter/directive.rs`)

`# arity-format skip` / `off`…`on` / `skip-file` (and the `# arity` forms) are
the one place the engine declines to decide layout. Grammar in
`arity_parser::directive`, shared with the linter — never re-parse it here.

- **Byte for byte.** A skipped span is spliced from the source with its own
  column: `Ir::Skipped` clears the pending indent before writing. Emitting the
  structural indent instead would be the formatter deciding layout in the one
  place it was told not to. Keep `Ir::Skipped` distinct from `Ir::verbatim` —
  the latter is the comment/candidate bridge and must not grow into an escape
  hatch (see "Engine shape" above).
- **Statement lists only.** Honored in the three sequencers (`ir_statements`,
  `ir_block_statements`, `ir_block_expr_with_prefixed_comments`), which is why
  regions are list-local. `is_honored_position` is `pub` because the linter's
  `misplaced-suppression` reports exactly what this predicate denies.
- **`skip-file` short-circuits `format_node`** before anything else, returning
  the source unchanged — line ending included. `format_range` returns `None`.

## `DESCRIPTION`

`formatter/description/` formats the *other* grammar. It reuses none of the
layout engine — every break is decided by the field's class, and prose wants
first-fit, not the engine's all-or-nothing group.

- **`desc` is a style reference, never an oracle.** Field order and the
  four-space continuation indent come from `desc:::field_order` and
  `desc:::format.DescriptionField`, but `desc` drops every comment and we must
  not. Gauge it with `task desc-compat`; never gate on it.
- **The class table is closed, and its default is `Opaque`** — line structure
  preserved byte for byte. That is what makes formatting on by default
  defensible: an unrecognized field's value is identical to `read.dcf`.
- **Comments attach forward**, to the *next* field, matching
  `next_meaningful_dcf_sibling` in the linter's suppression map. Moving one
  relative to its anchor would silently retarget a directive.
- **Refuse rather than guess.** Duplicate fields, multiple records, whitespace
  before a colon, a non-UTF-8 `Encoding`: each is a case where restyling could
  change what R reads. A refusal is not an error.
- The continuation indent is a fixed four spaces, **not** `indent_width`, which
  configures R-code nesting.

## Air compatibility (soft gauge, never a gate)

A **soft, one-directional** target, strictly subordinate to Tenet 1. We do not
match air; we use its maturity as a free differential oracle.

- `crates/arity-formatter/tests/air_compat.rs` is `#[ignore]`d, so it never runs
  in `cargo test` and cannot fail CI. Run `task air-compat`; it regenerates
  `AIR_COMPAT.md`.
- It measures the **fixed point** `air(arity(x)) == arity(x)`, not a head-to-head
  diff — which cancels the persistent-line-break difference by construction and
  leaves only genuine rule divergences.
- Triage each divergence: **adopt** when air is simply more idiomatic and arity
  is being inconsistent (fix the rule), or **record** it in
  `tests/air_compat_allowlist.toml` with a rationale when it is a deliberate
  arity choice.
- Diverging is allowed but must **raise tension**: an unexplained divergence in
  `AIR_COMPAT.md` is an open question, never an excuse to fail a build.

## Testing

- Fixtures: `crates/arity-formatter/tests/fixtures/formatter/<case>/` holding
  `input.R` + `expected.R`. **Hand-registered** — add the name to
  `fixture_names()` in `tests/formatter.rs` or it does not run.
- Suites: `formatter.rs`, `range_format.rs`, `air_compat.rs`. Losslessness and
  idempotence are asserted there; keep both green.
- Roxygen formatting stability also has a root-crate suite
  (`tests/roxygen_format_stability.rs`).
