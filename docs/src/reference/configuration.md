# Configuration reference

<span id="editor-support" aria-hidden="true"></span>
<span id="discovery" aria-hidden="true"></span>
<span id="reserved-for-future-use" aria-hidden="true"></span>

All settings in `arity.toml` are optional. For help choosing a configuration
file, sharing settings, or setting up editor completion, see the [configuration
guide](../guide/configuration.md).

## Top-level keys

  | Key              | Type            | Default             | What it controls                                                                                                             |
  | ---------------- | --------------- | ------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
  | `extend`         | Path            | Unset               | Load settings from another file before applying this file's settings. Relative paths start from the file declaring `extend`. |
  | `exclude`        | List of strings | Built-in exclusions | Replace the default gitignore-style patterns for skipping files and directories.                                             |
  | `extend-exclude` | List of strings | `[]`                | Add patterns to `exclude`, keeping the defaults if `exclude` is unset.                                                       |
  | `cache`          | Boolean         | `true`              | Reuse results from earlier `arity format --check` runs. Use `--no-cache` for a single run.                                   |

The built-in exclusions are:

```text
.git/
renv/
revdep/
cpp11.R
RcppExports.R
extendr-wrappers.R
import-standalone-*.R
```

## `[format]`

  | Key            | Type            | Default  | What it controls                                                                                               |
  | -------------- | --------------- | -------- | -------------------------------------------------------------------------------------------------------------- |
  | `line-width`   | Integer, 1–1000 | `80`     | Target line width in columns. Some lines may still be longer.                                                  |
  | `indent-width` | Integer, 1–1000 | `2`      | Spaces per level of R indentation. DESCRIPTION continuation lines always use four spaces.                      |
  | `line-ending`  | String          | `"auto"` | Line endings written by the formatter.                                                                         |
  | `roxygen`      | Boolean         | `true`   | Format `#'` blocks. Set to `false` to preserve their layout while formatting the surrounding R code.           |
  | `description`  | Boolean         | `true`   | Format package DESCRIPTION files. Set to `false` to leave the file untouched and keep its lint checks enabled. |

The values for `line-ending` are:

- `"auto"` uses the first line ending in the input, or LF for input without a
  line ending.
- `"native"` uses CRLF on Windows and LF elsewhere.
- `"lf"` and `"crlf"` always use the specified ending.

## `[lint]`

  | Key                | Type             | Default       | What it controls                                                     |
  | ------------------ | ---------------- | ------------- | -------------------------------------------------------------------- |
  | `select`           | List of rule IDs | Unset         | Replace the default rule selection. Use `[]` to start with no rules. |
  | `extend-select`    | List of rule IDs | `[]`          | Enable rules in addition to `select` or the defaults.                |
  | `ignore`           | List of rule IDs | `[]`          | Disable rules even if they appear in `select` or `extend-select`.    |
  | `per-file-ignores` | Table            | `{}`          | Disable rules in matching files.                                     |
  | `rules`            | Table            | Rule defaults | Set options for individual rules.                                    |

Use exact IDs from the [lint rule reference](rules.md).

### `[lint.per-file-ignores]`

Each key is a file pattern and each value is a list of rule IDs. Use forward
slashes on all platforms.

  | Pattern     | Meaning                                                |
  | ----------- | ------------------------------------------------------ |
  | `helpers.R` | Match this filename in any directory.                  |
  | `tests/*.R` | Match R files directly inside `tests/`.                |
  | `tests/**`  | Match files inside `tests/`, including subdirectories. |
  | `!R/**`     | Match files outside `R/`.                              |

`*` and `?` match within one path component. `**` can cross directories.
Relative patterns containing a slash start from the selected config file's
directory, including when the file is selected by `--config` or `ARITY_CONFIG`.
When several patterns match, their rule lists are combined. With `extend`, a
list for the same pattern replaces the inherited list. Use `[]` to clear it.

Applies to R and DESCRIPTION lint checks, including fixes. For stdin,
`--stdin-filename` provides the path to match. Syntax errors are always
reported.

### `[lint.rules.<id>]`

Only rules with options have a table here. Set rule options here and enable the
rules separately with `select` or `extend-select` when needed.

#### `[lint.rules.cyclomatic-complexity]`

  | Key              | Type             | Default | What it controls                                                                                   |
  | ---------------- | ---------------- | ------- | -------------------------------------------------------------------------------------------------- |
  | `max-complexity` | Positive integer | `20`    | Report functions whose [cyclomatic complexity](rules.md#cyclomatic-complexity) exceeds this value. |

#### `[lint.rules.roxygen-unknown-tag]`

  | Key            | Type            | Default | What it controls                                                                                                     |
  | -------------- | --------------- | ------- | -------------------------------------------------------------------------------------------------------------------- |
  | `allowed-tags` | List of strings | `[]`    | Accept extra [roxygen tags](rules.md#roxygen-unknown-tag). Use the exact spelling and case, without the leading `@`. |

#### `[lint.rules.undesirable-function]`

  | Key                | Type                                | Default            | What it controls                                                                                  |
  | ------------------ | ----------------------------------- | ------------------ | ------------------------------------------------------------------------------------------------- |
  | `functions`        | Table of function names to messages | Built-in functions | Replace the list of functions flagged by [`undesirable-function`](rules.md#undesirable-function). |
  | `extend-functions` | Table of function names to messages | `{}`               | Add functions to the list or replace the message for an existing function.                        |

Each message gives advice in the diagnostic. An empty string flags the function
without suggesting an alternative. The built-in list includes `.libPaths`,
`attach`, `debug`, `debugonce`, `detach`, `install.packages`, `options`, `par`,
`setwd`, `sink`, `source`, `Sys.setenv`, `Sys.setlocale`, `trace`, `undebug`,
and `untrace`.

## `[compat]`

Write minimum versions as plain strings such as `"4.1"`.

  | Key        | Type           | When unset                                                                 | What it controls                              |
  | ---------- | -------------- | -------------------------------------------------------------------------- | --------------------------------------------- |
  | `r`        | Version string | Minimum R version in DESCRIPTION's `Depends`                               | Oldest R version the project supports.        |
  | `roxygen2` | Version string | DESCRIPTION's `Config/roxygen2/version`, then `RoxygenNote`                | Oldest roxygen2 version the project supports. |
  | `s7`       | Version string | Minimum S7 version in DESCRIPTION, or the S7 0.2.2 API if none is declared | Which S7 APIs Arity recognizes.               |

If Arity has no minimum R or roxygen2 version for a file, the corresponding
compatibility rule stays quiet.

## `[index]`

The R package index helps Arity resolve names from installed packages.

  | Key             | Type          | Default | What it controls                                                                                     |
  | --------------- | ------------- | ------- | ---------------------------------------------------------------------------------------------------- |
  | `library-paths` | List of paths | `[]`    | R library directories to look in for installed packages.                                             |
  | `cache-dir`     | Path          | Unset   | Where Arity stores the index. If unset, Arity uses `ARITY_CACHE_DIR` or its standard cache location. |
  | `auto-build`    | Boolean       | `true`  | Let the language server index packages as your code refers to them.                                  |
  | `help`          | Boolean       | `true`  | Include help topics when building the index. Set to `false` for a smaller index of package symbols.  |
