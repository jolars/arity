# vscode-R unused binding investigation

At Arity `5eeae42b`, the vscode-R adoption case produces 19 `unused-binding`
findings: 10 correct reports and nine confirmed analysis gaps. The appropriate
follow-up is to model specific execution relationships and declare the external
entry points that R source cannot reveal. Disabling the rule for the repository
would also hide unused helpers and unused assignment results.

The target is [vscode-R PR
#1946](https://github.com/REditorSupport/vscode-R/pull/1946) at
[`3c54a6e6`](https://github.com/REditorSupport/vscode-R/tree/3c54a6e6f732104ca8b56619771c9be342c6fdff).
The investigation covered the same 58 R files under `R/`, `sess/`, `scripts/`,
and `src/test/examples/`. Reduced cases were checked with R 4.6.1 on November
16, 2026. Production analysis remains static.

## Reproduction and comparison

Build the current release binary from the Arity checkout:

```sh
compute run -- cargo build --release
```

Then run these commands from the pinned vscode-R checkout, substituting the
absolute path to the binary:

```sh
compute run -- /path/to/arity/target/release/arity --no-config lint \
  --select unused-binding --output json R sess scripts src/test/examples
compute run -- /path/to/arity/target/release/arity --no-config lint \
  --select undefined-symbol --output json R sess scripts src/test/examples
```

`--no-config` bypasses the PR's global ignores for both semantic rules. Each
lint command exits 1 because it reports findings; the JSON goes to stdout. The
investigation run has 19 unused bindings and 10 undefined-symbol findings.

The locally installed Arity 0.29.0 also reproduces the PR's original 27 unused
bindings on the pinned checkout. Comparing those diagnostics with the current
run shows that these eight names no longer appear:

  | File               | Bindings no longer reported                                                                                                             |
  | ------------------ | --------------------------------------------------------------------------------------------------------------------------------------- |
  | `R/sess_source.R`  | `sess_managed_library`, `sess_installed_source_revision`, `sess_loaded_source_revision`, `sess_load_namespace`, `sess_install_required` |
  | `R/tool-consent.R` | `tool_library_prepare`                                                                                                                  |
  | `R/tool-library.R` | `tool_library_install`, `tool_library_load`                                                                                             |

These files have literal `source()` consumers in `R/tests/`. Resolving those
paths from the Git root supplies the previously missing cross-file uses. The
0.29.0 comparison reproduces the original count; it is not a rebuilt comparison
with the earlier investigation's `39014aa` commit.

## Qualified uses across package boundaries

The qualified-use follow-up is implemented. Package and member names remain
paired through semantic analysis and the range-free project projection, and
DESCRIPTION supplies the package identity. This counts external callers while
keeping unrelated bindings and bare-name visibility separate. Missing or
ambiguous package identities do not resolve to an arbitrary root.

Rerunning the pinned checkout with the updated binary produces 18 findings. Only
the `interactive_start` report disappears; the other findings below are
unchanged. Workspace tests, Clippy with all targets and features, and Rust
formatting checks pass. Regression tests cover quoted names, namespace
isolation, graph reuse, qualifier changes, and live DESCRIPTION renames.

## Correct reports

These 10 bindings have no consumer in the checked corpus. An unused assignment
result does not imply that evaluating its right-hand side is unnecessary. In
particular, the child-process launches and the template enumeration must retain
their side effects if their assignment targets are removed.

  | Definition                  | Binding             | Evidence                                                                                                                                                                                                                           |
  | --------------------------- | ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
  | `R/rmarkdown/templates.R:6` | `template_dirs`     | The `lapply()` result is discarded; its callback populates `templates`, which is consumed later.                                                                                                                                   |
  | `R/tests/attach_sess.R:509` | `managed_library`   | This binding inside `run_case()` is never read. Other bindings with this name belong to separate scopes or generated child scripts.                                                                                                |
  | `R/tests/prepare-jgd.R:437` | `real_process`      | The child runs asynchronously; the test waits on its result file and never reads the launch result.                                                                                                                                |
  | `R/tests/prepare-jgd.R:491` | `approved_process`  | The test observes files and consent requests, not the launch result.                                                                                                                                                               |
  | `R/tests/prepare-jgd.R:528` | `declined_process`  | The test observes files and consent requests, not the launch result.                                                                                                                                                               |
  | `R/tests/prepare-jgd.R:611` | `parallel_children` | The `lapply()` launches children, but its returned list is unused.                                                                                                                                                                 |
  | `R/tests/prepare-jgd.R:736` | `profile_result`    | This path is never consumed; the test waits on `profile_done` and reads `profile_log`.                                                                                                                                             |
  | `R/tests/prepare-jgd.R:737` | `profile_process`   | The test waits on files, not the launch result.                                                                                                                                                                                    |
  | `sess/R/rstudioapi.R:164`   | `page_viewer`       | No code reads this helper and NAMESPACE does not export it. The option named `page_viewer` is populated separately by `make_viewer("page_viewer")` in `sess/R/hooks.R:238`. The event and option strings do not read this binding. |
  | `sess/R/rstudioapi.R:363`   | `namespace_has`     | No caller, export, or registration references this helper.                                                                                                                                                                         |

## Confirmed analysis gaps

These nine bindings are consumed at runtime. The evidence identifies the
consumer, rather than inferring that every top-level helper is an entry point.

  | Definition                               | Binding                       | Consumer and missing relationship                                                                                                                                                                                            |
  | ---------------------------------------- | ----------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
  | `R/attach_sess.R:287`                    | `vscode_r_attach_sess`        | `src/session.ts:1608–1610` generates R that sources the helper and calls this function. The invocation originates outside the R files.                                                                                       |
  | `R/prepare-tools.R:2`                    | `vscode_r_connect_with_jgd`   | `R/attach_sess.R:46–47` calls `jgd_setup$vscode_r_connect_with_jgd()` after loading a computed helper path with `sys.source()` into a fresh environment.                                                                     |
  | `R/prepare-tools.R:139`                  | `vscode_r_prepare_rstudioapi` | `R/attach_sess.R:79–80` calls the member of the environment returned by the same loader.                                                                                                                                     |
  | `R/sess-package-install.R:51`            | `sess_install`                | `R/install_sess.R:69–71`, `R/attach_sess.R:267–269`, and `src/test/examples/sess-installer.R` load and call `installer$sess_install()`. The environment is not a package namespace or a modeled source closure.              |
  | `R/terminal-startup.R:63`                | `vscode_r_startup_register`   | `R/profile.R:84–85` loads a computed path with `sys.source(..., envir = environment())` inside nested control flow, then calls the injected function.                                                                        |
  | `sess/R/interactive.R:122`               | `interactive_start`           | `R/interactive-worker.R:37` contains the literal R call `sess:::interactive_start()`. Qualified reads currently count toward uses only among files sharing a package or source relationship. This script is outside `sess/`. |
  | `sess/R/interactive.R:246`               | `interactive_execute`         | `src/interactive/backends/arf.ts:134` generates the call to `sess:::interactive_execute()`. The R tests inspect its existence by computed name, but the production invocation comes from TypeScript.                         |
  | `sess/inst/tinytest/test-listview.R:209` | `outer`                       | Lines 211–214 generate and evaluate `utils::View(outer$custom$a)`. This is a runtime read through `eval(parse(text = ...))`.                                                                                                 |
  | `sess/inst/tinytest/test-listview.R:345` | `named_root`                  | Lines 346–347 generate and evaluate `utils::View(named_root$values)`. This is another runtime read through generated syntax.                                                                                                 |

The last two reports are test-local analysis gaps. They are unrelated to
tinytest's injected expectations, and an entry-point declaration for top-level
functions would not address them.

## Reduced R checks

Use an isolated directory for each source-loading case so an unrelated literal
source consumer cannot mark the helper used. Put this in `helper.R`:

```r
helper <- function() 42L
```

Each row supplies a separate `main.R`. The Arity results below describe the
investigation baseline, before the source-destination follow-up. R evaluates
every case successfully and obtains 42. Lint both files with
`--no-config --select unused-binding,undefined-symbol`.

  | Loading form                             | `main.R`                                                                                                              | Current Arity result                                                                              |
  | ---------------------------------------- | --------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
  | Direct literal source, control           | `source("helper.R"); stopifnot(helper() == 42L)`                                                                      | Neither finding.                                                                                  |
  | Source inside a block                    | `{ source("helper.R") }; stopifnot(helper() == 42L)`                                                                  | Reports `helper` as unused and undefined. A block does not create a new R evaluation environment. |
  | Source inside a function                 | `run <- function() { source("helper.R", local = TRUE); helper() }; stopifnot(run() == 42L)`                           | Reports `helper` as unused and undefined.                                                         |
  | Computed source path                     | `path <- "helper.R"; source(path); stopifnot(helper() == 42L)`                                                        | Reports `helper` as unused. Resolution is incomplete, so the undefined-symbol rule stays quiet.   |
  | Explicit environment with `sys.source()` | `helpers <- new.env(parent = baseenv()); sys.source("helper.R", envir = helpers); stopifnot(helpers$helper() == 42L)` | Reports `helper` as unused.                                                                       |
  | Explicit environment with `source()`     | `helpers <- new.env(parent = baseenv()); source("helper.R", local = helpers); stopifnot(helpers$helper() == 42L)`     | Reports `helper` as unused.                                                                       |

The `source(..., local = helpers)` case also passes an R assertion that `helper`
was not created in the caller's frame. An environment-valued `local` argument
needs its own representation: the current Boolean records only literal `TRUE` or
`T`. Treating every other value as the global environment would invent
visibility.

The test-local dynamic reads reduce to this runnable example:

```r
local({
  outer <- list(custom = list(a = 42L))
  expression <- "outer$custom$a"
  stopifnot(eval(parse(text = expression)) == 42L)
})
```

R passes; Arity reports `outer` unused. The two expression loops from the corpus
were also checked with ordinary lists and vectors, obtaining the expected values
through both the direct and nested spellings. No viewer or installed `sess`
package is needed to establish these reads.

A separate minimal package named `entryfixture`, with an empty NAMESPACE and
`entry <- function() 42L` under `R/`, establishes the qualified-use gap. After
installing that fixture into an isolated library, R passes
`stopifnot(entryfixture:::entry() == 42L)` from a script outside the package.
Arity reports `entry` unused when linting the package and the caller together.

For injected names, R passes a runner that creates an environment, supplies
`injected <- function() 42L` there, and sources this script into that
environment:

```r
helper <- function() injected()
stopifnot(helper() == 42L)
```

Arity reports `injected` undefined when linting that script independently.
Conversely, sourcing `entry <- function() 42L` into a runner environment and
calling `runner$entry()` establishes an external use while the standalone script
produces an unused-binding finding. These are two different inputs to analysis:
names available to a script and names consumed by its caller.

As a check on the real environment members, sourcing `R/prepare-tools.R` into a
fresh environment makes both reported functions available. Calling
`vscode_r_connect_with_jgd("httpgd", function(allow_jgd) allow_jgd)` returns
`TRUE`, and `vscode_r_prepare_rstudioapi(FALSE)` returns `FALSE`, without
running package setup. The ordinary unused-function control still produces an
unused-binding finding.

## Source destination follow-up

The literal-path loading cases above now have separate destination environments
and execution directories. File-scope blocks share their enclosing frame,
function-local loads stay local to that function, and explicit helper loads
track member reads without introducing bare global names. The model includes
`sys.source()`, simple environment aliases, and caller-local reads from sourced
code. Conditional branches retain their uncertainty, and all projected facts
remain range-free.

R 4.6.1 confirms the reduced destination, ordering, alias, caller-local, and
nested `chdir` cases. Computed paths in the adoption checkout remain outside
this literal-path model; the external contracts described below still need
explicit declarations. Rename declines when it cannot rewrite all environment or
member reads.

## Implementation boundaries

1. **Qualified uses across package boundaries.** Implemented: each qualified
   read retains its package name and targets the corresponding local package's
   definition. A same-name member of a different package does not count. This
   addresses `interactive_start` without configuration or R evaluation.
2. **Source execution environments.** Implemented for literal paths: the target
   path, execution directory, and destination environment stay separate across
   blocks, function bodies, and explicit helper environments. The model includes
   `sys.source()` and preserves ordering and conditional uncertainty. A block
   does not create a new environment, and a helper load does not introduce bare
   globals. Computed targets remain unresolved.
3. **Add narrowly scoped semantic declarations for external contracts.** An
   entry-point declaration should name exact top-level bindings in matched
   files. A runner-environment declaration should name the bindings injected
   into those files, or identify a declared helper provider. Entry points affect
   use tracking; injected names affect resolution. Neither should mark all
   bindings used, attach every export of a framework, or suppress unrelated
   diagnostics. Keep these facts range-free and share them between CLI and LSP.
   Configuration syntax remains a design decision.
4. **Keep generated local reads separate.** The two `eval(parse(...))` cases
   need scope-specific handling. Existing statement-level
   `# arity-lint skip unused-binding: ...` directives provide a narrow
   workaround. An arbitrary `eval()` call must not silence unused-binding for
   the whole file, and the production analyzer must not evaluate R. Any future
   static handling of generated syntax must remain conservative about which
   variables it consumes.

The 10 remaining undefined-symbol findings fit the source-environment work: one
`sess_source_revision` read in `R/install_sess.R`, five injected helper reads in
`R/profile.R`, and four tool-library reads in `R/tool-consent.R`. The two
`tool_library_check` occurrences are separate findings. They do not establish
additional unused bindings or a need to attach all helper names to every R
script.
