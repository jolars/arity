# Formatting

Arity formats R source, roxygen comments, and package `DESCRIPTION` files. Start
with [installation](../getting-started.md#installation), then run the formatter
from your project directory.

## Format Files

Format one file in place, or pass a directory to format the files inside it:

```sh
arity format R/example.R
arity format .
```

You can pass several files or directories in one invocation. Directory walks
honor `.gitignore` and the exclusions in `arity.toml`. They include package-root
`DESCRIPTION` files as well as R source. See [Configuration](configuration.md)
to exclude generated or vendored files.

## Check Without Writing

Use `--check` to report files that need formatting without changing them:

```sh
arity format --check .
```

Arity prints a diff for each file that would change and exits nonzero if any
need formatting. Add `--quiet` to show the file list and summary without the
diffs. See [Integrations](integrations.md) to run this check in CI or
pre-commit.

## Format Standard Input

Pass `-` to read a buffer from standard input and write the formatted text to
standard output:

```sh
cat R/example.R | arity format -
```

Standard input is treated as R source unless you supply `--stdin-filename`. For
a package description, use:

```sh
cat DESCRIPTION | arity format --stdin-filename DESCRIPTION -
```

`--check` requires file paths. For an editor buffer, consume the formatted
standard output instead. [Editor Setup](editors.md) covers the language server
and RStudio integration.

## Choose Formatting Settings

Put shared settings in `arity.toml` so the CLI and language server use the same
style:

```toml
[format]
line-width = 100
indent-width = 2
```

For a single run, use `arity format --line-width 100 R/example.R`. The
[configuration reference](../reference/configuration.md#format) describes every
setting, including line endings and the switches for roxygen and `DESCRIPTION`
formatting. `DESCRIPTION` continuation lines use four spaces regardless of the R
indentation setting.

## Preserve Selected Layout

Use a formatter directive when a particular statement needs hand-written layout:

```r
# arity-format skip: the rows show the matrix layout
m <- matrix(c(1, 0,
              0, 1), nrow = 2)
```

`# arity-format off` and `# arity-format on` protect a region, and
`# arity-format skip-file` protects a whole file. These spellings leave linting
enabled. See [Directives](../reference/directives.md) for their scope and the
special rules for `DESCRIPTION` files.

Formatting changes layout. To apply code rewrites as well, follow the [linting
guide](linting.md#apply-fixes) and run the formatter after the fixes.
