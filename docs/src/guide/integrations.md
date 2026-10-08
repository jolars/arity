# Integrations

Use Arity in CI, Git hooks, and other formatting tools.

For editor and language-server setup, see [Editor Setup](editors.md) instead.

## GitHub Actions

[arity-action](https://github.com/jolars/arity-action) installs arity and runs
the format and lint checks in CI. Create `.github/workflows/arity.yml`:

```yaml
name: Arity

on:
  pull_request:
  push:
    branches: [main]

permissions:
  contents: read

jobs:
  arity:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: jolars/arity-action@v1
        with:
          version: v0.25.0
```

By default this runs both `arity format --check` and `arity lint` over the whole
repository without changing files. It checks R files and package-root
`DESCRIPTION` files using the CLI's [configuration
discovery](../reference/configuration.md#discovery). Directory walks honor
`.gitignore` and the exclusions in `arity.toml`. Formatting differences or lint
findings fail the check.

The `@v1` tag selects the action version; `version` selects the Arity CLI
release. Keep the latter aligned with your local installation and pre-commit
revision. Omitting it selects `latest`. Common inputs:

  | Input             | Default  | Description                                            |
  | ----------------- | -------- | ------------------------------------------------------ |
  | `path`            | `.`      | File or directory to check                             |
  | `version`         | `latest` | Version to install (`latest` or `vX.Y.Z`)              |
  | `format`          | `true`   | Run `arity format --check`                             |
  | `lint`            | `true`   | Run `arity lint`                                       |
  | `config`          | *(none)* | Path to an `arity.toml` to use                         |
  | `verify-checksum` | `true`   | Verify the downloaded asset against its published hash |

It also exposes the installed version as the `version` output. To run only one
of the two checks, turn the other off:

```yaml
- uses: jolars/arity-action@v1
  with:
    version: v0.25.0
    path: R
    lint: "false"
```

Set `format: "false"` to run only linting. See the [action
reference](https://github.com/jolars/arity-action/tree/v1#inputs) for the inputs
and outputs supported by `@v1`.

## pre-commit

[arity-pre-commit](https://github.com/jolars/arity-pre-commit) provides
[pre-commit](https://pre-commit.com) hooks. They install a prebuilt binary wheel
from PyPI, so no separate Arity or Rust installation is needed.

[Install pre-commit](https://pre-commit.com/#installation), then add this entry
to `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/jolars/arity-pre-commit
    rev: v0.25.0
    hooks:
      - id: arity-lint
      - id: arity-format
```

Install the Git hook and run it once over all tracked files:

```sh
pre-commit install
pre-commit run --all-files
```

On subsequent commits, `arity-lint` reports findings for staged `.r` and `.R`
files. `arity-format` formats those files and staged `DESCRIPTION` files in
place. The lint hook does not select `DESCRIPTION` files by default, although
the CLI supports them. When a hook changes a file, review and stage the changes,
then commit again.

To apply safe fixes, add `--fix` to the lint hook and keep it before formatting:

```yaml
hooks:
  - id: arity-lint
    args: [--fix]
  - id: arity-format
```

To check formatting without changing files, add `args: [--check]` to
`arity-format`.

Both hooks run with `--force-exclude`. pre-commit passes staged files as
explicit arguments, and files named explicitly are normally always processed;
the flag applies the `exclude` patterns from your `arity.toml` to them anyway,
so a staged file you have excluded stays excluded. See
[Configuration](../reference/configuration.md) for those patterns.

The `rev` selects the Arity release: `v0.25.0` installs Arity 0.25.0. Run
`pre-commit autoupdate` to update hook revisions, then review and commit the
changes to `.pre-commit-config.yaml`.

## dprint

Format R files alongside other languages with [dprint](https://dprint.dev) and
[dprint-plugin-arity](https://github.com/jolars/dprint-plugin-arity). The plugin
bundles Arity's formatter as WebAssembly, so no Arity CLI or R installation is
needed.

[Install dprint](https://dprint.dev/install/). If your project has no
`dprint.json`, create one with `dprint init`, then add Arity:

```sh
dprint config add jolars/arity
```

Commit the versioned, checksummed plugin URL that the command adds to
`dprint.json`. Run `dprint fmt` to format files in place or `dprint check` to
check without changing files. The check fails when files need formatting. The
plugin handles `.r` and `.R` files; it provides formatting rather than linting.

Configure it under the `arity` key:

  | Key               | Values                         | Default                   |
  | ----------------- | ------------------------------ | ------------------------- |
  | `lineWidth`       | integer                        | dprint global, else `80`  |
  | `indentWidth`     | integer                        | dprint global, else `2`   |
  | `lineEnding`      | `auto`, `lf`, `crlf`, `native` | from global `newLineKind` |
  | `roxygen`         | boolean                        | `true`                    |
  | `roxygenMarkdown` | boolean                        | `false`                   |

These correspond to the [formatting settings](../reference/configuration.md).
Set `roxygen` to `false` to preserve roxygen comment layout while formatting the
surrounding R code.

The plugin reads its configuration from `dprint.json` and does not load
`arity.toml`. Use dprint's top-level `includes` and `excludes` for file
selection; Arity's TOML exclusions do not apply. dprint also respects
`.gitignore`. See [dprint's configuration guide](https://dprint.dev/config/).

The Arity CLI discovers whether roxygen comments use Markdown by reading the
package's `DESCRIPTION` and `man/roxygen/meta.R`. The plugin cannot read those
files. So a package whose `DESCRIPTION` sets `Roxygen: list(markdown = TRUE)`
needs `roxygenMarkdown` set explicitly:

```json
{
  "arity": { "roxygenMarkdown": true }
}
```

Per-block `@md` and `@noMd` tags still take precedence over that default,
exactly as they do in the CLI.

The plugin is released independently of the CLI. Run `dprint config update`,
then review and commit the configuration changes. Matching CLI output requires
the same `arity-formatter` version, equivalent settings, and an explicit
`roxygenMarkdown` setting when the CLI derives it from package files.

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

Use [mise or Aqua](../getting-started.md#mise-and-aqua) to install and pin the
Arity CLI for a project.
