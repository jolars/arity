# Formatting

Arity formats R source, roxygen comments, and package `DESCRIPTION` files. Start
with [installation](../installation.md), then run the formatter from your
project directory.

## Format files

Format one file in place, or pass a directory to format the files inside it:

```sh
arity format R/example.R
arity format .
```

You can pass several files or directories in one invocation. Directory walks
honor `.gitignore` and the exclusions in `arity.toml`. They include package-root
`DESCRIPTION` files as well as R source. See [Configuration](configuration.md)
to exclude generated or vendored files.

## Check without writing

Use `--check` to report files that need formatting without changing them:

```sh
arity format --check .
```

Arity prints a diff for each file that would change and exits nonzero if any
need formatting. Add `--quiet` to show the file list and summary without the
diffs. See [Integrations](integrations.md) to run this check in CI or
pre-commit.

## Format standard input

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
standard output instead. [Editor setup](editors.md) covers the language server
and RStudio integration.

## Choose formatting settings

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

## Function bodies

Functions assigned to a name or target always use braces and an expanded body,
even when the body is a single expression:

```r
f <- function(x) {
  x + 1
}
```

Anonymous functions can stay compact when the whole definition fits on one line.
This applies to both `function(...)` callbacks and `\(...)` lambdas, including
callbacks passed through named arguments:

```r
map(xs, function(x) x + 1)
map(xs, \(x) x + 1)
map(xs, .f = function(x) x + 1)
```

These rules do not depend on input line breaks or existing braces. Anonymous
functions expand when needed for width or comments. Empty bodies stay `{}`.
Unlike Air, Arity always expands assigned functions and can collapse a short,
braced anonymous function to its compact form.

## Preserve selected layout

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
