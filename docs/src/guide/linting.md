# Linting

Arity reports problems in R source and package metadata. It checks code without
running R. [Install Arity](../getting-started.md#installation), then run the
linter from your project directory.

## Check Files

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

## Choose Rules

The [Lint Rules](../reference/rules.md) reference explains each rule and its
default behavior. To check a particular rule for one run:

```sh
arity lint --select equals-na .
```

To disable a rule for the project, put it in `arity.toml`:

```toml
[lint]
ignore = ["unused-binding"]
```

`select` replaces the default rule selection; `ignore` removes rules from that
selection. The [configuration guide](configuration.md) covers shared settings,
and the [reference](../reference/configuration.md#lint) lists rule-specific
options.

## Apply Fixes

Apply safe fixes in place, then format the rewritten code:

```sh
arity lint --fix .
arity format .
```

The linter reports findings that remain after fixing. Some findings need manual
changes, so review that output even when files were updated. Fixes change source
code; the formatter handles its layout in the second command.

To include fixes that may change behavior, explicitly opt in with
`arity lint --fix --unsafe-fixes .` and review the resulting diff. The rule
reference describes the available fixes.

## Suppress a Finding

When a rule is inappropriate at one site, put a directive before that statement:

```r
# arity-lint skip browser: interactive debugging entry point
browser()
```

Prefer a named rule and a reason. For a whole file, use
`# arity-lint skip-file <rule>: <reason>`. Region directives, package metadata,
and the rules that check suppression comments are covered in
[Directives](../reference/directives.md). Parse errors cannot be suppressed.

Use [Editor Setup](editors.md) for diagnostics and fixes while editing, or
[Integrations](integrations.md) to run lint checks in CI and pre-commit.
