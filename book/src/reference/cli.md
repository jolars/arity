# Command-Line Help for `arity`

This document contains the help content for the `arity` command-line program.

## `arity`

Arity: a language server, formatter, and linter for R

**Usage:** `arity [OPTIONS] <COMMAND>`

###### **Subcommands:**

* `parse` — Parse and display the CST tree for debugging
* `format` — Format .R files
* `lint` — Lint .R files
* `index` — Build or refresh the installed-package introspection index
* `lsp` — Run the language server over stdio (formatting only)

###### **Options:**

* `--config <PATH>` — Path to an explicit `arity.toml` (skips discovery)
* `--no-config` — Ignore any discovered `arity.toml` and use built-in defaults



## `arity parse`

Parse and display the CST tree for debugging

**Usage:** `arity parse [OPTIONS] [FILE]`

###### **Arguments:**

* `<FILE>` — Input file (stdin if not provided)

###### **Options:**

* `--quiet` — Suppress CST output to stdout
* `--verify` — Verify parser losslessness (input must equal CST text)



## `arity format`

Format .R files

**Usage:** `arity format [OPTIONS] [PATH]...`

###### **Arguments:**

* `<PATH>` — Input file(s) or path(s) (stdin if omitted)

###### **Options:**

* `--verify` — Verify formatting idempotence for supported inputs (does not write files)
* `--check` — Check formatting without writing changes; prints a diff for each file that would be reformatted and exits non-zero if any differ
* `--line-width <N>` — Override the configured line width
* `--indent-width <N>` — Override the configured indent width
* `--exclude <PATTERN>` — Additional gitignore-style exclude patterns (repeatable or comma-separated); augments the configured `exclude`



## `arity lint`

Lint .R files

Reads stdin when no paths are given. Exit codes: 0 = no findings, 1 = findings (or files blocked by parse errors), 2 = usage/IO error.

**Usage:** `arity lint [OPTIONS] [PATH]...`

###### **Arguments:**

* `<PATH>` — Input file(s) or path(s) (stdin if omitted)

###### **Options:**

* `--stdin-filename <PATH>` — Filename to report for stdin input (for diagnostics)
* `--fix` — Apply safe autofixes in place and report what remains
* `--unsafe-fixes` — Also apply fixes that may change behavior (requires --fix)
* `--select <RULE_ID>` — Only run these rules (overrides config `select`); repeatable or comma-separated
* `--ignore <RULE_ID>` — Disable these rules (overrides config `ignore`); repeatable or comma-separated
* `--exclude <PATTERN>` — Additional gitignore-style exclude patterns (repeatable or comma-separated); augments the configured `exclude`
* `--output <OUTPUT>` — Output format

  Default value: `pretty`

  Possible values:
  - `pretty`:
    Annotated multi-line snippets (default; matches jarl/rustc-style output)
  - `concise`:
    One finding per line (`path:line:col: severity [rule] message`)
  - `json`:
    JSON array of diagnostics, for editor integration




## `arity index`

Build or refresh the installed-package introspection index

**Usage:** `arity index [OPTIONS] [PATH]...`

###### **Arguments:**

* `<PATH>` — Project path(s) to scan for referenced packages (default: ".")

###### **Options:**

* `--force` — Re-harvest even when the installed version is already indexed
* `--no-help` — Skip harvesting help (names only; faster)
* `--cache-dir <DIR>` — Override the cache directory
* `--quiet` — Suppress per-package progress output



## `arity lsp`

Run the language server over stdio (formatting only)

**Usage:** `arity lsp`



