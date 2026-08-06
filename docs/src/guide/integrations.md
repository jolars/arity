# Integrations

Beyond running `arity` directly, several integrations wire it into version
control, CI, and other tooling. Each installs a prebuilt binary, so none of them
need a Rust toolchain or an R installation.

For editor and language-server setup, see [Editor Setup](editors.md) instead.

## GitHub Actions

[arity-action](https://github.com/jolars/arity-action) installs arity and runs
the format and lint checks in CI:

```yaml
jobs:
  arity:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: jolars/arity-action@v1
```

By default this runs both `arity format --check` and `arity lint` over the whole
repository. The inputs:

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
    path: R
    lint: false
```

Resolving `latest` picks the newest release that actually carries an asset for
the runner's platform, rather than the newest release outright, so a release
whose binaries are still uploading does not break the job.

## pre-commit

[arity-pre-commit](https://github.com/jolars/arity-pre-commit) provides
[pre-commit](https://pre-commit.com) hooks. It installs a prebuilt binary wheel
from PyPI.

```yaml
repos:
  - repo: https://github.com/jolars/arity-pre-commit
    # tracks the arity release it installs
    rev: v0.15.0
    hooks:
      # Lint .R files
      - id: arity-lint
      # Format the same files in place
      - id: arity-format
```

To apply safe autofixes as part of linting, pass the flag through:

```yaml
      - id: arity-lint
        args: [--fix]
```

Both hooks run with `--force-exclude`. pre-commit passes staged files as
explicit arguments, and files named explicitly are normally always processed;
the flag applies the `exclude` patterns from your `arity.toml` to them anyway,
so a staged file you have excluded stays excluded. See
[Configuration](../reference/configuration.md) for those patterns.

## mise-en-place

Arity is in the [aqua registry](https://github.com/aquaproj/aqua-registry) as
`jolars/arity`, so [mise](https://mise.jdx.dev) can install it through its aqua
backend:

```sh
mise use aqua:jolars/arity
```

Or in `mise.toml`:

```toml
[tools]
"aqua:jolars/arity" = "latest"
```

Replace `latest` with a released version to pin it.

The same registry entry works with [aqua](https://aquaproj.github.io) directly:

```sh
aqua g -i jolars/arity
```

## dprint

[dprint-plugin-arity](https://github.com/jolars/dprint-plugin-arity) is a
[dprint](https://dprint.dev) plugin that runs the arity **formatter** (not the
linter) inside dprint, so R files are formatted alongside the rest of a
project's languages. Add it with:

```sh
dprint config add jolars/arity
```

That writes a versioned, checksummed entry into your `dprint.json`:

```jsonc
{
  "arity": {},
  "plugins": [
    "https://plugins.dprint.dev/jolars/arity-x.x.x.wasm@<checksum>"
  ]
}
```

Configure it under the `arity` key:

  | Key               | Values                         | Default                   |
  | ----------------- | ------------------------------ | ------------------------- |
  | `lineWidth`       | integer                        | dprint global, else `80`  |
  | `indentWidth`     | integer                        | dprint global, else `2`   |
  | `lineEnding`      | `auto`, `lf`, `crlf`, `native` | from global `newLineKind` |
  | `roxygenMarkdown` | boolean                        | `false`                   |

These mirror the `[format]` keys in
[`arity.toml`](../reference/configuration.md), and the plugin's output is
byte-identical to `arity format` for equivalent settings.

Note that the plugin reads its configuration from `dprint.json`, **not** from
`arity.toml`. One setting has no equivalent on the dprint side: the `arity` CLI
discovers whether roxygen comments are markdown by default by reading the
package's `DESCRIPTION`, but a dprint plugin is a WebAssembly module with no
filesystem access and cannot. So a package whose `DESCRIPTION` sets
`Roxygen: list(markdown = TRUE)` needs `roxygenMarkdown` set explicitly:

```json
{
  "arity": { "roxygenMarkdown": true }
}
```

Per-block `@md` and `@noMd` tags still take precedence over that default,
exactly as they do in the CLI.
