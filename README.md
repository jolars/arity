# Arity <img src='https://raw.githubusercontent.com/jolars/arity/refs/heads/main/images/logo.png' align="right" width="139" />

[![Build and
Test](https://github.com/jolars/arity/actions/workflows/build-and-test.yml/badge.svg)](https://github.com/jolars/arity/actions/workflows/build-and-test.yml)
[![Crates.io](https://img.shields.io/crates/v/arity.svg?logo=rust)](https://crates.io/crates/arity)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Arity is a language server, formatter, and linter for the R programming
language, built in Rust on a lossless, incremental parser. It provides a fast,
deterministic development experience that integrates with popular code editors
and IDEs.

- **Formatter**: deterministic, rule-based formatting toward the tidyverse style
  guide, with idempotent output and roxygen support.
- **Linter**: a growing set of correctness, readability, and performance rules,
  many with safe autofixes.
- **Language server**: formatting, diagnostics with quick fixes, hover,
  completion, signature help, go-to-definition and references, rename, document
  and workspace symbols, semantic tokens, folding, and call hierarchy.

Runs on Linux, macOS, and Windows (x86_64 and arm64).

## Installation

Arity is available from several sources:

- **crates.io**: `cargo install arity`
- **npm**: `npm install -g arity-cli` (bundles a prebuilt binary)
- **PyPI**: `uv tool install arity`/`pipx install arity`
- **Prebuilt binaries**: from the [releases
  page](https://github.com/jolars/arity/releases)
- **VS Code/Open VSX**: the **Arity** extension (also works in Positron)

### From npm

Install with [npx](https://www.npmjs.com/package/npx) or `npm`:

```sh
# One-shot run, no install:
npx arity-cli format file.R

# Persistent install:
npm install -g arity-cli
```

The package detects your platform at install time and pulls in a prebuilt binary
via npm's optional dependencies; no Rust toolchain required.

### From crates.io

If you have Rust installed:

```sh
cargo install arity
```

## Formatter

To format your code, you can use:

- `arity format [file]`
- `arity format --verify [file]`
- `arity format --check <path> [<path> ...]`

## Linter

To lint your code, you can use:

- `arity lint <path> [<path> ...]`

`arity lint` reads from stdin when given no paths, and exits non-zero when it
reports any findings.

## Configuration

Arity reads an optional `arity.toml`, discovered by walking up from each file's
directory to the repository root. Run `arity init` to scaffold a commented
starter file. See the [configuration
reference](https://arity.cc/reference/configuration.html) for every key.

## Editor integration

`arity lsp` starts a stdio-based language server offering formatting,
diagnostics with quick fixes, hover, completion, signature help,
go-to-definition and references, rename, document and workspace symbols,
semantic tokens, folding, and call hierarchy.

The **Arity** extension for VS Code/Open VSX (and Positron) bundles the binary
and starts the server automatically. For Neovim, Helix, and other editors, see
the [editor setup guide](https://arity.cc/guide/editors.html).

## Documentation

Full documentation lives at [arity.cc](https://arity.cc):

- [Getting started](https://arity.cc/getting-started.html)
- [Editor setup](https://arity.cc/guide/editors.html)
- [Configuration](https://arity.cc/reference/configuration.html)
- [CLI reference](https://arity.cc/reference/cli.html)
- [Lint rules](https://arity.cc/reference/rules.html)
