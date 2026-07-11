# Arity <img src='https://raw.githubusercontent.com/jolars/arity/refs/heads/main/images/logo.png' align="right" width="139" />

[![Build and
Test](https://github.com/jolars/arity/actions/workflows/build-and-test.yml/badge.svg)](https://github.com/jolars/arity/actions/workflows/build-and-test.yml)
[![Crates.io](https://img.shields.io/crates/v/arity.svg?logo=rust)](https://crates.io/crates/arity)
[![Open
VSX](https://img.shields.io/open-vsx/v/jolars/arity?logo=vsix)](https://open-vsx.org/extension/jolars/arity)
[![VS
Code](https://vsmarketplacebadges.dev/version-short/jolars.arity.svg?logo=vsix)](https://marketplace.visualstudio.com/items?itemName=jolars.arity)
[![PyPI
version](https://badge.fury.io/py/arity.svg?icon=si%3Apython)](https://badge.fury.io/py/arity)
[![npm version](https://badge.fury.io/js/@arity-cli%2Farity-cli.svg?icon=si%3Anpm)](https://www.npmjs.com/package/arity-cli)

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
- **Arch Linux**: `pacman -S arity-bin` (or `arity`) (from the AUR:
  [`arity-bin`](https://aur.archlinux.org/packages/arity-bin/),
  [`arity`](https://aur.archlinux.org/packages/arity/))

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

## Acknowledgements

The core architecture of Arity is entirely based on
[rust-analyzer](https://github.com/rust-lang/rust-analyzer), using salsa for
incremental computation and rowan for lossless syntax trees. Arity also owes a
great debt to [air](https://posit-dev.github.io/air/), on which it is heavily
inspired and from which it has borrowed tests, rules, and formatting style. It
is also inspired by [jarl](https://jarl.etiennebacher.com/) and has borrowed
rules from it as well as some of its architecture.

## Documentation

Full documentation lives at [arity.cc](https://arity.cc):

- [Getting started](https://arity.cc/getting-started.html)
- [Editor setup](https://arity.cc/guide/editors.html)
- [Configuration](https://arity.cc/reference/configuration.html)
- [CLI reference](https://arity.cc/reference/cli.html)
- [Lint rules](https://arity.cc/reference/rules.html)
