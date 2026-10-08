# Configuration

Put an `arity.toml` at the root of your project to share formatting and linting
settings between the CLI and language server. Every key is optional. This guide
covers common setup tasks; the [configuration
reference](../reference/configuration.md) lists all keys, defaults, and
resolution rules.

## Create a Project Config

Run this in a project that does not yet have an `arity.toml`:

```sh
arity init
```

The generated file contains commented settings. A small active configuration
might look like this:

```toml
extend-exclude = ["vendor/", "*.gen.R"]

[format]
line-width = 100

[lint]
ignore = ["unused-binding"]
```

Commit the file so contributors use the same settings. Keys use kebab-case;
unknown keys are errors. Add `#:schema https://arity.cc/arity.schema.json` at
the top for completion and validation in editors that support TOML schema
directives. See [editor support](../reference/configuration.md#editor-support)
for other ways to associate the schema.

## Choose the Configuration Source

For each input file, Arity uses the nearest `arity.toml`, searching upward to
the repository root. When there is no project config, `ARITY_CONFIG` can point
to a fallback file. Discovery selects one file; it does not automatically merge
project and user settings.

To choose a file explicitly or try the built-in defaults:

```sh
arity --config path/to/arity.toml format R/example.R
arity --no-config lint R/example.R
```

CLI options such as `--line-width` override the corresponding setting for one
run. See [discovery](../reference/configuration.md#discovery) for fallback paths
and language-server behavior.

## Share Settings Between Projects

Use `extend` when a project should inherit another file:

```toml
extend = "../shared/arity.toml"

[format]
line-width = 100
```

The path is relative to the file declaring it. The project's values override
inherited values. Tables merge by key, while `extend-exclude` adds patterns to
the inherited exclusions.

## Exclude Files

Use `extend-exclude` to add gitignore-style patterns while retaining Arity's
built-in exclusions:

```toml
extend-exclude = ["vendor/", "generated/"]
```

These patterns apply to directory walks by both the formatter and linter. A file
named explicitly is still processed; add `--force-exclude` when a runner passes
filenames that should respect the exclusions:

```sh
arity format --force-exclude generated/example.R
```

Set `exclude` only when you want to replace the default list. For a local
exception within a source file, use [directives](../reference/directives.md).

## Adjust Formatting and Linting

Use `[format]` for layout settings and `[lint]` for rule selection. Start with
the [formatting](formatting.md) and [linting](linting.md) guides, then consult
the reference for [formatting options](../reference/configuration.md#format) and
[lint settings](../reference/configuration.md#lint).

The [dprint plugin](integrations.md#dprint) has its own configuration in
`dprint.json`; it does not load `arity.toml`.
