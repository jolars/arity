# Configuration

Put an `arity.toml` at the root of your project to share settings between the
command line and your editor. You only need to write the settings you want to
change. The [configuration reference](../reference/configuration.md) lists every
option and its default.

## Create a project config

Run `arity init` from the project root to create a commented starter file, or
create `arity.toml` yourself. For example, this config skips generated files,
sets a target line width of 100 columns, and enables an optional lint rule.

```toml
extend-exclude = ["generated/"]

[format]
line-width = 100

[lint]
extend-select = ["undesirable-function"]
```

Put project-wide keys such as `extend-exclude` before the first table heading.
Keys use hyphens, as in `line-width`. Arity reports unknown keys so you can
catch typos. Commit the file so other contributors use the same settings.

## Choose the configuration source

For ordinary file inputs, Arity starts in each file's directory and looks upward
for the nearest `arity.toml`. It checks the Git repository root too, then stops.
When you pipe content into Arity, the search starts in the current working
directory, even if you pass `--stdin-filename`.

For example, with a config at the project root and another in `examples/`,
`R/model.R` uses the root config while `examples/demo.R` uses
`examples/arity.toml`. The nested file supplies its own settings. Use
[`extend`](#share-settings-between-projects) to inherit the root settings.

Set the `ARITY_CONFIG` environment variable to use a personal config in projects
without their own. A project config takes precedence over the fallback. Use an
absolute path so the fallback works from any directory. With neither config,
Arity uses its built-in defaults. See [editor setup](editors.md#vs-codepositron)
for setting this variable in VS Code or Positron.

### Override settings for one run

Use `--config` to choose a specific file or `--no-config` to skip configuration
files and start from the defaults. Command options such as `--line-width`
override the corresponding setting for that run.

```sh
arity --config path/to/arity.toml format R/model.R
arity --no-config lint R/model.R
arity format --line-width 100 R/model.R
```

## Share settings between projects

Use `extend` to load a shared config first. This example keeps the shared
settings and changes just the target line width.

```toml
extend = "../shared/arity.toml"

[format]
line-width = 100
```

Relative paths start from the file that declares `extend`. Paths beginning with
`~/` start from your home directory. A shared config can extend another file in
turn.

Tables merge by key. If the shared config sets `indent-width`, the example above
keeps it. `extend-exclude` and `extend-select` append to inherited lists.
Setting `exclude`, `select`, or `ignore` replaces the corresponding inherited
list.

## Exclude files

Use `extend-exclude` to skip extra files during formatting and linting while
keeping the [built-in exclusions](../reference/configuration.md#top-level-keys).
Use `exclude` to set the entire list yourself.

```toml
extend-exclude = ["vendor/", "*.gen.R"]
```

These patterns follow gitignore syntax. In a project config or a file selected
with `--config`, paths are relative to that file's directory, including patterns
you inherited with `extend`. For an `ARITY_CONFIG` fallback, CLI walks match
paths relative to the working directory.

A directory scan skips excluded directories and any configs inside them. When
you name a file explicitly, Arity processes it by default. Add `--force-exclude`
to apply the exclusions to explicit files too.

```sh
arity format --force-exclude R/example.gen.R
```

## Ignore rules for selected files

Use `[lint.per-file-ignores]` when a rule should be quiet in some files but
other checks should still run. For example, this keeps the undefined-symbol rule
from flagging names supplied by a test framework.

```toml
[lint.per-file-ignores]
"inst/tinytest/**" = ["undefined-symbol"]
```

Arity adds together the ignores from every matching pattern. Other lint rules
and syntax errors still appear. The same settings apply in your editor and when
you pass filenames explicitly. See the [pattern
reference](../reference/configuration.md#lint-per-file-ignores) for matching
rules or use [directives](../reference/directives.md) for an exception within a
file.

## Adjust formatting and linting

Use `[format]` to adjust layout or preserve files that another tool formats. For
example, set `description = false` under `[format]` to leave DESCRIPTION
formatting to your package tooling while keeping its lint checks active.

Under `[lint]`, `extend-select` enables rules alongside the defaults. Use
`select` to choose the starting list yourself and `ignore` to turn off rules in
that list or in `extend-select`. Find rule names in the [lint rule
reference](../reference/rules.md).

Use `[compat]` when you need to declare the oldest R or roxygen2 version your
project supports. For example, setting `r = "4.1"` there lets Arity flag
features that require a newer R release. For an R package, Arity can usually
read the version requirement from DESCRIPTION. See the [compatibility
options](../reference/configuration.md#compat) for the fields it reads.

## Get configuration help in your editor

Add this comment on the first line of `arity.toml` to enable completion and
validation in editors that support TOML schema directives.

```toml
#:schema https://arity.cc/arity.schema.json
```

If your editor uses schema associations instead, associate `arity.toml` with
`https://arity.cc/arity.schema.json`.
