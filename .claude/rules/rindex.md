---
paths:
  - "src/rindex.rs"
  - "src/rindex/**/*.rs"
  - "tests/rindex.rs"
---

# R introspection index rules

`src/rindex/`, CLI `arity index`. Harvests exports, formals, and help from
*installed* packages and exposes them as a `SymbolProvider` (`provider.rs`) for
the linter and the LSP.

## Hard invariants

- **No R runtime, ever.** Read R's on-disk formats natively: `rds.rs` (RDS
  serialization), `lazyload.rs` (`.rdb`/`.rdx` lazy-load databases), `rd.rs`
  (help). Shelling out to `R`/`Rscript` to answer a question here is not an
  option, it is the thing this module exists to avoid.
- **Network egress is a per-user consent decision.** `remote.rs` fetches a
  prebuilt index only from `ARITY_REMOTE_URL`; the field is deliberately
  `#[serde(skip)]` on the config so it can never become a committed project
  setting.
- **A malformed or truncated on-disk file is input, not a bug report.** Degrade
  to "no symbols for this package" rather than panicking — these files come from
  whatever the user happens to have installed.
- The index is a **cache**: a stale or missing entry must only cost precision
  (fewer known symbols), never correctness of a lint or a format.

## Layout

`discover.rs`/`libpaths.rs` find libraries, `build.rs`/`harvest.rs` populate the
cache (`schema.rs`, `cache.rs`), `provider.rs` serves it. `arity index` resolves
`arity.toml` so its walk honors the same excludes as format and lint;
`[index]` carries `library-paths`, `cache-dir`, `auto-build`, and `help`.

Under the LSP, indexing is the one **unbounded-duration** job and runs on the
isolated single-thread index pool — never move it onto the read pool
(`.claude/rules/lsp.md`).

## Testing

`tests/rindex.rs`, with fixtures in `tests/fixtures/rindex/<case>/`. Add a
fixture for each new on-disk shape rather than asserting against whatever
happens to be installed on the machine.
