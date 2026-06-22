# Editor setup

Arity ships a language server, started with `arity lsp` (stdio, JSON-RPC). It
offers formatting, diagnostics with quick fixes, hover, completion, signature
help, go-to-definition and find-references, rename, document and workspace
symbols, semantic tokens, folding ranges, and call hierarchy.

Configuration is read from an `arity.toml` discovered from each file's directory
(see the [configuration reference](../reference/configuration.md)).

## VS Code / Positron

Install the **Arity** extension from the [VS Code
Marketplace](https://marketplace.visualstudio.com/) or [Open
VSX](https://open-vsx.org/). It bundles the `arity` binary (falling back to a
download) and starts the language server automatically for R files. Editors that
support VS Code extensions, such as Positron, work the same way.

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
