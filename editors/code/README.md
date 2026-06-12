# Arity

A language server, formatter, and linter for R.

## Quick start

1. Install the **Arity** extension.
2. Open an R file (`.R`, `.r`, `.Rprofile`).
3. The extension starts `arity lsp` automatically.

By default, the extension uses an `arity` binary that ships inside the extension
(one platform-specific VSIX per OS/architecture), falling back to downloading a
matching binary from GitHub releases when none is bundled.

## Features

- Starts `arity lsp` automatically when you open R files.
- Formats R code using Arity's deterministic, tidyverse-style formatter.
- Surfaces Arity diagnostics and quick-fix code actions in the editor.
- Registers itself as the default formatter for `[r]` files.

## Commands

- `Arity: Restart Server`: stops and restarts the Arity language server
  (re-reads settings and re-resolves the binary). Useful if the LSP gets wedged
  or after changing settings such as `arity.version` or `arity.executablePath`.

## Binary installation

By default, the extension uses an `arity` binary that ships inside the extension
itself (one platform-specific VSIX per OS/architecture). No download, no GitHub
round-trip, and the language server starts on first activation even on
restricted or offline networks. Behavior is controlled by
`arity.executableStrategy`:

- `bundled` (default): use the binary that ships inside the extension. If you're
  on a platform without a platform-specific build (or you've installed the
  universal VSIX), the extension falls back to downloading a matching binary
  from GitHub releases.
- `environment`: look for `arity` on the system `PATH`.
- `path`: use the binary at `arity.executablePath`.

If you set `arity.version` or `arity.releaseTag` explicitly, the bundled binary
is skipped and the requested version is downloaded from GitHub. When
`arity.version` is `latest`, the extension selects the most recent stable
release that contains a matching platform asset.

## Common setup examples

Use a local binary at a fixed path:

```json
{
  "arity.executableStrategy": "path",
  "arity.executablePath": "/usr/local/bin/arity"
}
```

Use whatever `arity` is on your `PATH`:

```json
{
  "arity.executableStrategy": "environment"
}
```

Pin to a specific release:

```json
{
  "arity.version": "0.2.0",
  "arity.githubRepo": "jolars/arity"
}
```

Use `arity.releaseTag` only if you need an exact tag override:

```json
{
  "arity.releaseTag": "v0.2.0"
}
```

## Requirements and troubleshooting

- **NixOS**: the bundled binary won't run because of the dynamic loader path.
  The extension detects NixOS and skips the download, using `arity` on your
  `PATH` instead; install `arity` (for example via the `arity` package) or set
  `arity.executableStrategy` to `path` with `arity.executablePath`.
- **Offline / restricted networks / proxies**: the bundled-binary default works
  without network access. Only the explicit-version download paths
  (`arity.version` / `arity.releaseTag`) require GitHub connectivity.
- If a download fall-through fails, the extension shows a warning and falls back
  to looking up `arity` on the system `PATH`.

## Settings

Arity registers itself as the default formatter for `[r]` files.

- `arity.executableStrategy`: how to locate the `arity` binary: `bundled`
  (default), `environment`, or `path`.
- `arity.executablePath`: path to the binary, used only when
  `executableStrategy` is `path`.
- `arity.version`: version to install (default: `"latest"`).
- `arity.releaseTag`: advanced exact tag override (takes precedence if
  explicitly set).
- `arity.githubRepo`: GitHub repo for downloads (default: `"jolars/arity"`).
- `arity.serverArgs`: extra args after `arity lsp`.
- `arity.serverEnv`: extra environment variables.
- `arity.extraPath`: extra PATH entries prepended for the language server
  process.
- `arity.logLevel`: log level for the language server, mapped to `RUST_LOG`
  (`off`, `error`, `warn`, `info`, `debug`, `trace`; unset by default).
  `arity.serverEnv.RUST_LOG` overrides this if both are set.
- `arity.trace.server`: LSP trace level (`off`, `messages`, `verbose`).

## Security and trust

When `arity.executableStrategy` is `bundled` (the default), the extension
prefers the binary that shipped inside the VSIX. If no bundled binary is
available, or `arity.version` / `arity.releaseTag` is set explicitly, it
downloads from GitHub releases configured by `arity.githubRepo` (default
`jolars/arity`).
