# Integrations

Use Arity in CI, Git hooks, and other formatting tools.

For editor and language-server setup, see [Editor setup](editors.md) instead.

## GitHub Actions

Add `.github/workflows/arity.yml` to run Arity's formatting and lint checks on
pull requests.

```yaml
name: Arity

on: pull_request

permissions:
  contents: read

jobs:
  arity:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: jolars/arity-action@v1
```

See the [arity-action README](https://github.com/jolars/arity-action#readme) for
version pinning and other options.

## pre-commit

Use `arity-lint` to report findings and `arity-format` to format staged files in
place. Copy the hook configuration into `.pre-commit-config.yaml` from the
[arity-pre-commit README](https://github.com/jolars/arity-pre-commit#readme),
which also covers version pins and hook options.

After [installing pre-commit](https://pre-commit.com/#installation), enable the
Git hook and run it once over all tracked files.

```sh
pre-commit install
pre-commit run --all-files
```

If a hook rewrites files during a commit, review and stage the changes before
committing again.

## dprint

You can format R files alongside other languages with dprint. After [installing
dprint](https://dprint.dev/install/), run `dprint init` if your project needs a
configuration file. Then add Arity and format your files.

```sh
dprint config add jolars/arity
dprint fmt
```

Commit the updated `dprint.json` to share the setup with your project. See the
[dprint-plugin-arity
README](https://github.com/jolars/dprint-plugin-arity#readme) for plugin
options.

## Using with Panache

[Panache](https://panache.bz) can format and lint R code blocks inside Markdown,
Quarto, and R Markdown documents. Install both CLIs on your `PATH`, then add
this to the document project's `panache.toml`:

```toml
[formatters]
r = "arity"

[linters]
r = "arity"
```

Run `panache format document.qmd` to format the document and its R blocks, or
`panache lint document.qmd` to report findings without changing files. The
linter also runs through Panache's language server. To apply safe fixes before
formatting, run `panache lint --fix document.qmd`, then format the document.

Panache's Arity linter preset passes `--no-config`, so it does not load
`arity.toml` and uses Arity's default lint configuration. The formatter preset
runs separately and retains its CLI configuration behavior. See Panache's
[formatter preset](https://panache.bz/reference/formatter-presets.html#arity),
[linter preset](https://panache.bz/reference/linter-presets.html#arity), and
[external-tool
configuration](https://panache.bz/reference/configuration.html#external-code-linters).

## mise-en-place

Use mise to share an exact Arity CLI version with your team. From the project
root, install Arity and pin the version mise resolves.

```sh
mise use --pin aqua:jolars/arity
```

Commit the updated mise configuration, usually `mise.toml`. After cloning the
project, teammates can run `mise install` to install the pinned tools and
`mise exec -- arity --version` to check which Arity version they are using.

For standalone Aqua setup, see [Aqua](../installation.md#aqua).
