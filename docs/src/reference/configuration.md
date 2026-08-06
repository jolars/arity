# Configuration

Arity is configured with a TOML file named `arity.toml`. All keys are optional;
omitting a key uses its default. Keys are **kebab-case**, and unknown keys are
rejected with an error (so a typo never silently falls back to a default).

Run [`arity init`](cli.md#arity-init) to write a commented starter file.

## Discovery

For a given file, arity looks for `arity.toml` by walking up from the file's
directory through its ancestors, stopping at the first `arity.toml` it finds or
at a directory containing a `.git` entry (the repository root), whichever comes
first.

On the command line:

- `--config <PATH>` loads an explicit file and skips discovery.
- `--no-config` ignores any discovered file and uses the built-in defaults.

## Top-level keys

These apply to **both** `format` and `lint` (the first two govern the shared
file walk).

  | Key              | Type             | Default      | Description                                                                                                                                                                                                      |
  | ---------------- | ---------------- | ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
  | `exclude`        | array of strings | built-in set | [gitignore-style](https://git-scm.com/docs/gitignore) patterns to skip, resolved relative to the directory containing `arity.toml`. Setting it **replaces** the built-in set (below).                            |
  | `extend-exclude` | array of strings | `[]`         | Like `exclude`, but **added to** `exclude` rather than replacing it. Use this to skip extra paths while keeping the built-in defaults.                                                                           |
  | `cache`          | boolean          | `true`       | Enable the persistent result cache (currently the `format --check` already-formatted cache; the cache directory follows `[index] cache-dir`/`$ARITY_CACHE_DIR`). The `--no-cache` CLI flag overrides it per run. |

The built-in default exclude set (the default value of `exclude`; generated or
vendored files that should not be reformatted or linted) is:

```
.git/
renv/
revdep/
cpp11.R
RcppExports.R
extendr-wrappers.R
import-standalone-*.R
```

Excludes apply only to directory walks. A file named **explicitly** on the
command line is always processed, even if it matches an exclude pattern. Pass
`--force-exclude` (on `format` and `lint`) to apply the patterns to explicitly
named files too---useful for runners like pre-commit that pass staged files as
arguments. The CLI flag `--exclude <PATTERN>` (on `format` and `lint`) adds to
the configured `exclude`/`extend-exclude` for a single run.

```toml
# Keep the built-in defaults and also skip these:
extend-exclude = ["vendor/", "*.gen.R"]

# Or replace the built-in defaults entirely:
# exclude = ["vendor/", "*.gen.R"]
```

## `[format]`

  | Key            | Type             | Default  | Description                                                           |
  | -------------- | ---------------- | -------- | --------------------------------------------------------------------- |
  | `line-width`   | integer (1–1000) | `80`     | The width the formatter tries to keep lines within. Not a hard cap.   |
  | `indent-width` | integer (1–1000) | `2`      | Number of spaces per indentation level.                               |
  | `line-ending`  | string           | `"auto"` | Newline style: `"auto"`, `"lf"`, `"crlf"`, or `"native"` (see below). |

`line-ending = "auto"` mirrors the source file's first line ending (defaulting
to `lf` when the file has none); `"native"` is `crlf` on Windows and `lf`
elsewhere; `"lf"` and `"crlf"` force that ending.

```toml
[format]
line-width = 80
indent-width = 2
line-ending = "auto"
```

`line-width` and `indent-width` can be overridden per run with the
`--line-width`/`--indent-width` flags on `arity format`.

## `[lint]`

  | Key      | Type             | Default | Description                                                          |
  | -------- | ---------------- | ------- | -------------------------------------------------------------------- |
  | `select` | array of strings | unset   | If set, only these rule IDs run.                                     |
  | `ignore` | array of strings | `[]`    | Rule IDs to disable (applied on top of `select` or the default set). |

Rule IDs are the kebab-case names from the [rule reference](rules.md). Unknown
IDs are reported when linting runs, not when the config is parsed. The
`--select`/`--ignore` flags on `arity lint` override these for a single run.

```toml
[lint]
select = ["undefined-symbol", "equals-na"]
ignore = ["unused-binding"]
```

### `[lint.rules.<id>]`

A few rules take options of their own, set in a table named after the rule ID.
Rules that take no options have no table.

Unlike `select`/`ignore`---where rule IDs are data, checked when linting
runs---a rule ID here is part of the schema, so a mistyped one is reported when
the config is *parsed*, alongside any other unknown key.

#### `[lint.rules.undesirable-function]`

The function-name policy for
[`undesirable-function`](rules.md#undesirable-function).

  | Key                | Type                 | Default      | Description                                                      |
  | ------------------ | -------------------- | ------------ | ---------------------------------------------------------------- |
  | `functions`        | table of name → hint | built-in set | Flagged functions. **Replaces** the built-in set.                |
  | `extend-functions` | table of name → hint | `{}`         | Entries added on top of `functions`, overriding same-named ones. |

The value is the advice shown as the diagnostic's suggestion; an empty string
means "no alternative, just don't call this". The `functions`/`extend-functions`
split works like `exclude`/`extend-exclude`: reach for `extend-functions` unless
you really mean to discard the built-in set. Setting `functions = {}` silences
the rule entirely.

The built-in set covers base-R functions that mutate global state (`attach`,
`detach`, `.libPaths`, `install.packages`, `setwd`, `sink`, `source`, `options`,
`par`, `Sys.setenv`, `Sys.setlocale`) and the debugging entry points (`debug`,
`debugonce`, `undebug`, `trace`, `untrace`). `browser()` is deliberately absent:
it has its own [`browser`](rules.md#browser) rule.

```toml
[lint]
select = ["undesirable-function"]

[lint.rules.undesirable-function]
extend-functions = { sapply = "use `vapply()` for a stable return type" }
```

## `[index]`

Controls the R-package symbol index used by the language server (and by
namespace-aware lint rules) to resolve names.

  | Key             | Type           | Default | Description                                                             |
  | --------------- | -------------- | ------- | ----------------------------------------------------------------------- |
  | `library-paths` | array of paths | `[]`    | Explicit R library directories, used when automatic discovery misses.   |
  | `cache-dir`     | path           | unset   | Override the index cache directory (otherwise XDG/`$ARITY_CACHE_DIR`).  |
  | `auto-build`    | boolean        | `true`  | Let the language server lazily index referenced-but-unindexed packages. |
  | `help`          | boolean        | `true`  | Harvest help titles while indexing. `false` stores names only (faster). |

> **Note:** the downloadable CRAN symbol sidecar is *not* configured here.
> Enabling network access is a per-user decision set via the `ARITY_REMOTE_URL`
> environment variable, never committed in a shared `arity.toml`.

> **Note:** the same applies to the attach probe (`arity index --attach-probe`),
> which observes what a meta-package attaches by running `library()` in a fresh
> R session. Because that executes package attach hooks, it is enabled per run
> by the flag or per user via the `ARITY_ATTACH_PROBE` environment variable,
> never from `arity.toml`. Without it, attach sets are still captured for
> packages following the tidyverse `core` convention, with a built-in table as
> the offline fallback.

## Reserved for future use

The following are **not yet implemented** but are reserved so the schema can
grow without breaking changes (adding a key is always backward-compatible under
the strict unknown-key check):

- `[format].indent-style` (`"space"` or `"tab"`)---tab indentation.
- `[format].skip` and a `# fmt: skip` comment---opt specific calls out of
  formatting.
- `severity` in a `[lint.rules.<id>]` table---overriding a rule's severity.
- Category names (e.g. `"correctness"`) in `select`/`ignore`.
