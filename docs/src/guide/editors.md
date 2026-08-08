# Editor Setup

Arity ships a language server, started with `arity lsp` (stdio, JSON-RPC). It
offers formatting, diagnostics with quick fixes, hover, completion, signature
help, go-to-definition and find-references, rename, document and workspace
symbols, semantic tokens, folding and selection ranges, document links, document
color swatches, and call and type hierarchy.

Renaming reaches beyond symbols: moving or renaming an `.R` file, or a folder of
them, rewrites the `source()` paths that referred to it, and rebases the moved
files' own `source()` paths to their new location.

Configuration is read from an `arity.toml` discovered from each file's directory
(see the [configuration reference](../reference/configuration.md)).

## VS Code/Positron

Install the **Arity** extension from the [VS Code
Marketplace](https://marketplace.visualstudio.com/) or [Open
VSX](https://open-vsx.org/). It bundles the `arity` binary (falling back to a
download) and starts the language server automatically for R files. Editors that
support VS Code extensions, such as Positron, work the same way.

### Using only some features

The formatter, linter, and language features share one server but can be turned
off independently, so you can adopt just the parts you want:

- `arity.formatting.enable` — use arity as a formatter.
- `arity.diagnostics.enable` — show arity diagnostics (the linter).
- `arity.languageFeatures.enable` — hover, completion, navigation, symbols,
  rename, code actions, semantic tokens, and the rest.

All three default to `true`. They are client-side gates, so the server keeps
running and the toggles take effect without a restart or reinstall. For a
formatter-only setup, turn off the other two:

```json
{
  "arity.diagnostics.enable": false,
  "arity.languageFeatures.enable": false
}
```

Turning off `arity.diagnostics.enable` this way suppresses **every** diagnostic,
including the syntax/parse errors that an `arity.toml` `[lint]` selection
[cannot silence](../reference/configuration.md#lint). The `arity.toml` route
stays the right tool when you want to keep parse errors but mute specific lint
rules across every editor and the CLI.

## Neovim

With [`nvim-lspconfig`](https://github.com/neovim/nvim-lspconfig) installed,
register arity as a server for R files:

```lua
vim.lsp.config("arity", {
  cmd = { "arity", "lsp" },
  filetypes = { "r" },
  root_markers = { "arity.toml", "DESCRIPTION", ".git" },
})
vim.lsp.enable("arity")
```

Format on save (optional):

```lua
vim.api.nvim_create_autocmd("BufWritePre", {
  pattern = "*.R",
  callback = function() vim.lsp.buf.format() end,
})
```

## Helix

In `~/.config/helix/languages.toml`:

```toml
[language-server.arity]
command = "arity"
args = ["lsp"]

[[language]]
name = "r"
language-servers = ["arity"]
formatter = { command = "arity", args = ["format"] }
auto-format = true
```

## Other editors

Any LSP-capable editor can use arity by launching `arity lsp` over stdio for the
`r` language. Point your client's R language-server command at `arity` with the
`lsp` argument.
