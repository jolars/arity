# Getting started

Run Arity from the root of an existing R project. The commands below use the
default settings unless your project already has an `arity.toml`.

## Install Arity

Use the installer to download Arity for your platform:

::: code-group

```sh [macOS/Linux]
curl --proto '=https' --tlsv1.2 -sSf https://arity.cc/install | sh
```

```powershell [Windows]
powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://arity.cc/install.ps1 | iex"
```

:::

See [Installation](../installation.md) for package managers and other options,
or [Editor setup](editors.md) to use Arity directly in your editor.

## Check your project

Check formatting and report lint findings without changing files:

```sh
arity format --check .
arity lint .
```

The formatter prints a diff for files that need formatting. The linter reports
findings with file locations, rule names, and suggestions. Both commands exit
with a nonzero status when they report problems, so they work as CI checks too.

Directory walks honor `.gitignore` and the project's configured exclusions. You
can pass a file or subdirectory instead of `.` to narrow the scope.

## Apply changes

Apply available safe fixes, then format the result:

```sh
arity lint --fix .
arity format .
```

Some findings need manual changes. Review the diagnostics and the diff, then
rerun the checks above.

See [Linting](linting.md) for rule selection and suppressions, and
[Formatting](formatting.md) for standard input and other options.

## Share project settings

Arity works without configuration. To customize the defaults, generate an
`arity.toml` in a project that does not already have one:

```sh
arity init
```

Use it to choose rules, adjust formatting, and exclude generated files. The CLI
and language server use the same project configuration. See
[Configuration](configuration.md) for common settings and discovery rules.

## Use Arity in your workflow

Follow [Editor setup](editors.md) for formatting, diagnostics, and navigation
while editing. See [Integrations](integrations.md) to run the checks in CI or a
pre-commit hook.
