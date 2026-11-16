# Linting

Arity reports problems in R source and package metadata. It checks code without
running R. [Install Arity](../installation.md), then run the linter from your
project directory.

## Check files

Check one file or a whole project:

```sh
arity lint R/example.R
arity lint .
```

Directory walks include R files and package-root `DESCRIPTION` files, honoring
`.gitignore` and configured exclusions. Explicitly named files are processed
even if excluded unless you add `--force-exclude`. See
[Configuration](configuration.md#exclude-files).

The linter reports findings with source locations and rule IDs. It exits with
status 0 when there are no findings, 1 for findings or parse diagnostics, and 2
for usage or I/O errors. A file with parse errors must be repaired before its
lint rules can run.

To lint a buffer from standard input, use:

```sh
cat R/example.R | arity lint --stdin-filename R/example.R -
```

For compact output or a JSON array of diagnostics:

```sh
arity lint --output concise .
arity lint --output json .
```

JSON diagnostics go to stdout, including `[]` when there are no findings.
Human-readable diagnostics and informational messages go to stderr.

## Resolve script dependencies

Arity follows literal `source()` and `sys.source()` calls between analyzed
files. For relative source paths, it assumes R starts in the nearest enclosing
package root (a directory with `DESCRIPTION` and `R/`). Outside a package, it
uses the nearest Git root, then the nearest directory containing `arity.toml`,
then the script's own directory. Nested configs inside a Git repository do not
change this working directory.

Sourced files inherit their caller's working directory. With `chdir = TRUE`, the
target executes in its containing directory, and the caller's directory is
restored afterward. The CLI and language server use the same defaults.

The destination environment is independent of this directory. Plain `{}` blocks
share the enclosing environment. `source(..., local = TRUE)` loads into the
current frame, including a function's local frame. An explicit destination such
as `source(..., local = helpers)` or `sys.source(..., envir = helpers)` loads
members into that environment; it does not introduce bare global names. Arity
tracks environments created with `new.env()`, their simple aliases, and literal
member reads such as `helpers$helper` and `helpers[["helper"]]`.

Reads respect loading order. Function analysis keeps its loading effects within
that function. Conditional loads retain possible bindings without treating them
as unconditional definitions. Arity does not evaluate R, resolve computed paths,
or infer calls from external programs. Explicit entry-point and helper-provider
declarations remain separate work.

## Choose rules

The [Lint rules](../reference/rules.md) reference explains each rule and its
default behavior. To check a particular rule for one run:

```sh
arity lint --select equals-na .
```

To disable a rule for the project, put it in `arity.toml`:

```toml
[lint]
ignore = ["unused-binding"]
```

To enable an opt-in rule while keeping the defaults, use `extend-select`:

```toml
[lint]
extend-select = ["undesirable-function"]
```

For one run, use `arity lint --extend-select undesirable-function .`.

To disable a rule only in matching files, use a glob in `arity.toml`:

```toml
[lint.per-file-ignores]
"sess/inst/tinytest/**" = ["undefined-symbol"]
```

Other rules and parse-error reporting remain active. See the [configuration
reference](../reference/configuration.md#lint-per-file-ignores) for matching and
inheritance rules.

## Apply fixes

Apply safe fixes in place, then format the rewritten code:

```sh
arity lint --fix .
arity format .
```

The linter reports findings that remain after fixing. Some findings need manual
changes, so review that output even when files were updated. Fixes change source
code; the formatter handles its layout in the second command.

With stdin, `--fix` writes the fixed source to stdout and reports remaining
findings to stderr. Combining stdin `--fix` with `--output json` is a usage
error because both outputs require stdout. For files, `--fix --output json`
reports the diagnostics that remain after fixing, including `[]` when clean.

To include fixes that may change behavior, explicitly opt in with
`arity lint --fix --unsafe-fixes .` and review the resulting diff. The rule
reference describes the available fixes.

## Suppress a finding

When a rule is inappropriate at one site, put a directive before that statement:

```r
# arity-lint skip browser: interactive debugging entry point
browser()
```

Prefer a named rule and a reason. For a whole file, use
`# arity-lint skip-file <rule>: <reason>`. Region directives, package metadata,
and the rules that check suppression comments are covered in
[Directives](../reference/directives.md). Parse errors cannot be suppressed.

Use [Editor setup](editors.md) for diagnostics and fixes while editing, or
[Integrations](integrations.md) to run lint checks in CI and pre-commit.
