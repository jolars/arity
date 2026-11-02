# Overview

<span id="arity"></span>

Arity is a formatter, linter, and language server for R. It keeps formatting
consistent, finds common mistakes, and provides diagnostics and navigation in
your editor. It also understands roxygen documentation and package `DESCRIPTION`
files.

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
