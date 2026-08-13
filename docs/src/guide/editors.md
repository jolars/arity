# Editor Setup

Arity ships a language server, started with `arity lsp` (stdio, JSON-RPC). It
offers formatting, diagnostics with quick fixes, hover, completion, signature
help, go-to-definition and find-references, rename, document and workspace
symbols, semantic tokens, folding and selection ranges, document links, document
color swatches, and call and type hierarchy.

Renaming reaches beyond symbols: moving or renaming an `.R` file, or a folder of
them, rewrites the `source()` paths that referred to it, and rebases the moved
files' own `source()` paths to their new location.

Beyond the quick fixes attached to lint diagnostics, the server offers a
cursor-context refactor that writes a roxygen2 skeleton for the function you are
on; see the [code action reference](../reference/code-actions.md).

Configuration is read from an `arity.toml` discovered from each file's directory
(see the [configuration reference](../reference/configuration.md)).

## DESCRIPTION files

The server also serves a package's `DESCRIPTION`, which is a different grammar
from R. There it offers the packaging diagnostics, completion of package names
in `Depends`, `Imports`, `Suggests`, `LinkingTo`, and `Enhances`, and hover
showing a dependency's installed version and title. An unsaved edit counts
immediately, so adding a package to `Imports` clears the
[`undeclared-dependency`](../reference/rules.md#undeclared-dependency) findings
in the R files that use it without saving first.

Diagnostics are reported only for a `DESCRIPTION` at a package root of its own,
matching what `arity lint` walks. A complete miniature package under
`tests/testthat/` is fixture data for a test, so it stays quiet.

A `DESCRIPTION` is formatted too, by default, so format-on-save canonicalizes it
the same way it canonicalizes your `.R` files. Set `description = false` under
`[format]` in `arity.toml` to leave it alone.

Only whole-document formatting is offered: canonical field order is a property
of the whole file, so `editor.formatOnSaveMode: "modifications"` (and
format-selection generally) will not touch a `DESCRIPTION`.

Editors need to be told to send the file, since most do not recognize
`DESCRIPTION` on their own. The VS Code extension does this for you; for the
rest, see the sections below.

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
-- Neovim ships no filetype for DESCRIPTION, so give it one. The name is yours
-- to choose: arity routes on the file name, not on what the client calls it.
vim.filetype.add({ filename = { DESCRIPTION = "r-description" } })

vim.lsp.config("arity", {
  cmd = { "arity", "lsp" },
  filetypes = { "r", "r-description" },
  root_markers = { "arity.toml", "DESCRIPTION", ".git" },
})
vim.lsp.enable("arity")
```

Drop the `vim.filetype.add` line and the second entry in `filetypes` if you only
want arity on `.R` files.

Format on save (optional):

```lua
vim.api.nvim_create_autocmd("BufWritePre", {
  pattern = { "*.R", "DESCRIPTION" },
  callback = function() vim.lsp.buf.format() end,
})
```

The `DESCRIPTION` entry matters: without it, the file is attached to the server
and linted but never formatted on save.

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

To get the `DESCRIPTION` features too, add the file to whatever the client uses
to decide which documents to send. Arity decides the grammar from the file name,
so the language id the client reports does not matter, and one that says `r` is
still handled as DCF.
