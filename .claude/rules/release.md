---
paths:
  - ".github/workflows/*.yml"
  - "versionary.jsonc"
  - "editors/code/src/**/*.ts"
  - "editors/code/*.json*"
  - "npm/**/package.json"
  - "npm/**/*.js"
  - "pyproject.toml"
  - "Cargo.toml"
  - "crates/*/Cargo.toml"
---

# Distribution and release rules

Releases are fully automated off Conventional Commits; the commit type picks the
version.

## Never hand-edit

**`CHANGELOG.md` and every version field are generated** — `versionary`
(`versionary.jsonc`) overwrites your edit. That includes
`npm/arity-cli/package.json`'s own version *and* every `optionalDependencies`
entry, which the release PR propagates. Write a good conventional commit
instead. The pre-1.0 config sets `bump-minor-pre-major`, so breaking changes
land as **minor** bumps.

## Four packages, routed by path

| Package | Tag stream |
| --- | --- |
| root CLI (`arity`) | bare `v*` |
| `crates/arity-parser` | `arity-parser-v*` |
| `crates/arity-formatter` | `arity-formatter-v*` |
| `editors/code` (`arity-code`) | `follows` the CLI |

Paths under `editors/` and `crates/` are **excluded from the CLI's version
calculation**, so **keep commits atomic per area** — a commit spanning the root
crate and a member crate produces muddled per-crate changelogs.

## The pipeline

Push to `main` → test + `cargo-audit` + `cargo-deny` pass → `versionary` opens
or updates a release PR. Merging it tags and fans out to `packages.yml` (eight
targets: Linux gnu/musl, macOS, Windows, each x86_64 + aarch64, cross-built with
`cargo-zigbuild`, glibc-floor checked, keyless provenance attestation), then the
VS Code/Open VSX, crates.io, npm, and PyPI publishes.

- `publish-cargo.yml` runs on `v*` tags and publishes **every** workspace crate
  not yet on crates.io, in dependency order — so a member-crate bump ships on
  the next CLI tag.
- Member tags are prefixed, so the `v*` filters match only the CLI stream, and
  **only the CLI stream carries GitHub release assets**.

## Surfaces

- `editors/code` — TypeScript VS Code extension, esbuild-bundled (`npm run
  compile`/`watch`/`package`), **biome**-gated by the devenv git hook and
  `lint.yml`. At publish time a platform binary is downloaded into
  `editors/code/server/` and packaged per target; at runtime the client resolves
  the server via `arity.executableStrategy`
  (`bundled`/`environment`/`path`), falling back to `arity` on PATH — which is
  also the NixOS path, where a downloaded binary would not run. Don't make
  `bundled` load-bearing.
- `npm/arity-cli` — a launcher whose `optionalDependencies` pull one
  `@arity-cli/<platform>` package per target, generated from
  `npm/platform-template`.
- `pyproject.toml` — the PyPI package, built by maturin.

## Dependencies

A dependency change must stay clean under `cargo-audit` and `cargo-deny`
(`deny.toml`); both gate the release PR.
