# Overview

<span id="arity"></span>

Arity is a formatter, linter, and language server for R. It keeps your code
consistent and gives you feedback as you write.

## What Arity does

- **Format consistently.** Format R code, roxygen documentation, and package
  `DESCRIPTION` files.
- **Find problems early.** Catch common mistakes, apply safe fixes, and run the
  same checks locally and in CI.
- **Work in your editor.** Get completions, diagnostics, navigation, and
  refactoring through the language server, with formatting on save.

## Built on your code

Arity parses your code into a lossless syntax tree and updates its analysis as
you type. Formatting is deterministic, and formatting an already-formatted file
changes nothing. The same parser powers the formatter, linter, and language
server.

See the [performance comparisons](guide/performance.md) for measurements of the
formatter, linter, and language server.

## Choose your setup {#quick-start}

- **VS Code or Positron:** install the [Arity
  extension](guide/editors.md#vs-code-positron). It includes the binary and
  starts the language server for you.
- **RStudio:** use the [R package](guide/editors.md#rstudio) to format code
  inside RStudio or from the R console.
- **Other editors:** follow [Editor setup](guide/editors.md) to connect your
  editor to the language server. The guide includes Neovim, Zed, and Helix.
- **Terminal or CI:** [install the CLI](installation.md), then follow [Getting
  started](guide/getting-started.md) to check and format an existing R project.

The [Installation](installation.md) page covers the standalone installer,
package managers, and source builds.

## Where to go next

- [Getting started](guide/getting-started.md): a workflow for an existing R
  project.
- [Formatting](guide/formatting.md) and [Linting](guide/linting.md): use each
  tool on files, projects, and standard input.
- [Configuration](guide/configuration.md): share project settings in
  `arity.toml`.
- [Integrations](guide/integrations.md): run checks in CI or a pre-commit hook.

For individual commands, settings, and diagnostics, see the [CLI
reference](reference/cli.md), [Configuration
reference](reference/configuration.md), and [Lint rules](reference/rules.md).
