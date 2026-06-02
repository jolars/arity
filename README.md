# Ravel <img src='https://raw.githubusercontent.com/jolars/ravel/refs/heads/main/images/logo.svg' align="right" width="139" />

Ravel is a language server, formatter, and linter for the R programming
language. It is designed to provide a seamless development experience for R
programmers by integrating with popular code editors and IDEs.

## Formatter

To format your code, you can use:

- `ravel format [file]`
- `ravel format --verify [file]`
- `ravel format --check <path> [<path> ...]`

## Linter

To lint your code, you can use:

- `ravel lint --check <path> [<path> ...]`

## Editor integration

`ravel lsp` starts a stdio-based language server. It currently advertises only
formatting (`textDocument/formatting`); diagnostics and other capabilities are
not implemented yet. Configuration is read from `ravel.toml` discovered from
each file's parent directory, matching the CLI.

Helix example (`~/.config/helix/languages.toml`):

```toml
[language-server.ravel]
command = "ravel"
args = ["lsp"]

[[language]]
name = "r"
language-servers = ["ravel"]
formatter = { command = "ravel", args = ["format"] }
```

Neovim (with `nvim-lspconfig` or a custom client) should launch `ravel lsp` for
files with the `r` filetype and request formatting via `vim.lsp.buf.format()`.
