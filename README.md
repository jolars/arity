# Arity <img src='https://raw.githubusercontent.com/jolars/arity/refs/heads/main/images/logo.png' align="right" width="139" />

Arity is a language server, formatter, and linter for the R programming
language. It is designed to provide a seamless development experience for R
programmers by integrating with popular code editors and IDEs.

## Installation

### From npm

Install with [npx](https://www.npmjs.com/package/npx) or `npm`:

```sh
# One-shot run, no install:
npx arity-cli format file.R

# Persistent install:
npm install -g arity-cli
```

The package detects your platform at install time and pulls in a prebuilt binary
via npm's optional dependencies --- no Rust toolchain required.

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

- `arity lint --check <path> [<path> ...]`

## Editor integration

`arity lsp` starts a stdio-based language server. It currently advertises only
formatting (`textDocument/formatting`); diagnostics and other capabilities are
not implemented yet. Configuration is read from `arity.toml` discovered from
each file's parent directory, matching the CLI.

Helix example (`~/.config/helix/languages.toml`):

```toml
[language-server.arity]
command = "arity"
args = ["lsp"]

[[language]]
name = "r"
language-servers = ["arity"]
formatter = { command = "arity", args = ["format"] }
```

Neovim (with `nvim-lspconfig` or a custom client) should launch `arity lsp` for
files with the `r` filetype and request formatting via `vim.lsp.buf.format()`.
