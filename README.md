# Arity <img src='https://raw.githubusercontent.com/jolars/arity/refs/heads/main/images/logo.png' align="right" width="139" />

[![Build and
Test](https://github.com/jolars/arity/actions/workflows/build-and-test.yml/badge.svg)](https://github.com/jolars/arity/actions/workflows/build-and-test.yml)
[![Crates.io](https://img.shields.io/crates/v/arity.svg?logo=rust)](https://crates.io/crates/arity)
[![Open
VSX](https://img.shields.io/open-vsx/v/jolars/arity?logo=vsix)](https://open-vsx.org/extension/jolars/arity)
[![VS
Code](https://vsmarketplacebadges.dev/version-short/jolars.arity.svg?logo=vsix)](https://marketplace.visualstudio.com/items?itemName=jolars.arity)
[![PyPI
version](https://badge.fury.io/py/arity.svg?icon=si%3Apython)](https://pypi.org/project/arity/)
[![npm
version](https://badge.fury.io/js/@arity-cli%2Farity-cli.svg?icon=si%3Anpm)](https://www.npmjs.com/package/arity-cli)

Arity is a language server, formatter, and linter for the R programming
language, built on a lossless, incremental parser. It provides a fast,
deterministic development experience that integrates with popular code editors
and IDEs.

- **Formatter**: deterministic, rule-based formatting based on the tidyverse
  style guide, with idempotent output and support for roxygen and `DESCRIPTION`
  (`.dcf`) files.
- **Linter**: project-aware linting, with correctness, readability, and
  performance rules, many with safe autofixes.
- **Language server**: formatting, diagnostics with quick fixes, hover,
  completion, signature help, go-to-definition and references, rename, document
  and workspace symbols, semantic tokens, folding, and call hierarchy.

## Installation

Arity is available from several sources:

- **crates.io**: `cargo install arity`
- **Homebrew**: `brew install jolars/tap/arity`
- **npm**: `npm install -g arity-cli` (bundles a prebuilt binary)
- **PyPI**: `uv tool install arity`/`pipx install arity`
- **Aqua**: `aqua install jolars/arity`
- **Prebuilt binaries**: from the [releases
  page](https://github.com/jolars/arity/releases)
- **VS Code/Open VSX**: the **Arity** extension (also works in Positron)
- **Arch Linux**: `pacman -S arity-bin` (or `arity`) (from the AUR:
  [`arity-bin`](https://aur.archlinux.org/packages/arity-bin/),
  [`arity`](https://aur.archlinux.org/packages/arity/))
- **NixOS**: the `arity` package is available in
  [nixpkgs](https://search.nixos.org/packages?channel=unstable&show=arity&from=0&size=50&sort=relevance&type=packages)

### Install Script

The installer scripts pick the right release artifact for your platform and
install to a user-local directory by default. They download the matching Arity
CLI release asset and verify its checksum. If you prefer, download and inspect
the script before running it.

For macOS and Linux:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://arity.cc/install | sh
```

For Windows PowerShell:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://arity.cc/install.ps1 | iex"
```

Set `ARITY_INSTALL_DIR` to change the destination, `ARITY_TAG` to pin a version,
`ARITY_LIBC` (`gnu` or `musl`) to override the detected libc on Linux, and
`ARITY_VERIFY_CHECKSUM=false` to skip checksum verification.

## Usage

```sh
# Format in place
arity format file.R

# Verify formatting without writing
arity format --check src/

# Lint a project directory
arity lint src/

# Fix lint findings in place
arity lint --fix file.R
```

`arity lint` reads from stdin when given `-` (or when piped with no paths), and
exits non-zero when it reports any findings.

## Configuration

Configure Arity with `arity.toml`, which is discovered by walking up from each
file's directory to the repository root. Run `arity init` to scaffold a
commented starter file. See the [configuration
reference](https://arity.cc/reference/configuration.html) for every key.

## Editor Integration

`arity lsp` starts a stdio-based language server offering formatting,
diagnostics with quick fixes, hover, completion, signature help,
go-to-definition and references, rename, document and workspace symbols,
semantic tokens, folding, and call hierarchy.

The Arity extension for VS Code/Open VSX (and Positron) bundles the binary and
starts the server automatically. For Neovim, Helix, and other editors, see the
[editor setup guide](https://arity.cc/guide/editors.html).

## Pre-Commit Hook

[arity-pre-commit](https://github.com/jolars/arity-pre-commit) provides
[pre-commit](https://pre-commit.com) hooks for linting and formatting. It
installs a prebuilt binary wheel from PyPI, so no Rust toolchain or R
installation is required:

```yaml
repos:
  - repo: https://github.com/jolars/arity-pre-commit
    # arity version
    rev: v0.18.0
    hooks:
      # Lint .R files
      - id: arity-lint
      # Format the same files in place
      - id: arity-format
```

## GitHub Actions

[arity-action](https://github.com/jolars/arity-action) installs Arity and runs
format and lint checks in CI:

```yaml
jobs:
  arity:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: jolars/arity-action@v1
```

## Crates

The parsing and formatting engines are published as standalone crates for use in
other tools: [`arity-parser`](https://crates.io/crates/arity-parser) (lossless
CST parser, typed AST wrappers, incremental reparser) and
[`arity-formatter`](https://crates.io/crates/arity-formatter) (the formatter,
with optional serde/schemars support for embedders). Both are versioned
independently of the CLI.

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
