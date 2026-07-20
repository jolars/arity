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

These govern file discovery for **both** `format` and `lint`, which share one
file walk.

  | Key              | Type             | Default      | Description                                                                                                                                                                           |
  | ---------------- | ---------------- | ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
  | `exclude`        | array of strings | built-in set | [gitignore-style](https://git-scm.com/docs/gitignore) patterns to skip, resolved relative to the directory containing `arity.toml`. Setting it **replaces** the built-in set (below). |
  | `extend-exclude` | array of strings | `[]`         | Like `exclude`, but **added to** `exclude` rather than replacing it. Use this to skip extra paths while keeping the built-in defaults.                                                |

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

## Reserved for future use

The following are **not yet implemented** but are reserved so the schema can
grow without breaking changes (adding a key is always backward-compatible under
the strict unknown-key check):

- `[format].indent-style` (`"space"` or `"tab"`)---tab indentation.
- `[format].skip` and a `# fmt: skip` comment---opt specific calls out of
  formatting.
- `[lint.rules.<id>]`---per-rule configuration tables (including per-rule
  severity).
- Category names (e.g. `"correctness"`) in `select`/`ignore`.
