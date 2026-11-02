---
layout: home
hero:
  name: Arity
  tagline: A fast formatter, linter, and language server for R. Keep your code
    consistent and get feedback as you write.
  image:
    src: /images/logo.png
    alt: Arity logo
    width: 192
    height: 192
  actions:
    - theme: brand
      text: Get started
      link: /guide/getting-started.html
    - theme: alt
      text: Set up your editor
      link: /guide/editors.html
    - theme: alt
      text: View on GitHub
      link: https://github.com/jolars/arity
features:
  - title: Format consistently
    details: Get consistent formatting for R code, roxygen documentation, and package descriptions.
    link: /guide/formatting.html
  - title: Find problems early
    details: Catch common mistakes, apply automatic fixes, and run the same checks locally and in CI.
    link: /guide/linting.html
  - title: Work in your editor
    details: Get completions, diagnostics, navigation, and refactoring through the language server, with formatting on save.
    link: /guide/editors.html
---

<span id="arity"></span> <span id="where-to-go-next"></span>

## Quick start

Install Arity for your platform, then format and lint a file:

::: code-group

```sh [macOS/Linux]
curl --proto '=https' --tlsv1.2 -sSf https://arity.cc/install | sh
```

```powershell [Windows]
powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://arity.cc/install.ps1 | iex"
```

:::

See the [installation guide](installation.md) for Homebrew, npm, uv, Cargo, and
other installation options.

After installation, you can run Arity from the command line:

```sh
arity format file.R
arity lint file.R
```

Using VS Code or Positron? Install the [Arity
extension](guide/editors.md#vs-codepositron), which includes Arity. For RStudio
and other editors, see the [editor setup guide](guide/editors.md).

## Built on your code

Arity parses your code into a lossless syntax tree and updates its analysis as
you type. Formatting is deterministic, and formatting an already-formatted file
changes nothing. The same parser powers the formatter, linter, and language
server.

See the [performance comparisons](guide/performance.md), or [configure
Arity](guide/configuration.md) for your project.
