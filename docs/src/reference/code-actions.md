# Code actions

Code actions are the editor's "do something here" menu, served by `arity lsp`
over `textDocument/codeAction` for the cursor position or selection. Arity
offers two families: **quick fixes**, which come from a lint finding, and
**refactors**, which are computed from the code under the cursor and need no
diagnostic.

In VS Code and Positron both families are gated by
`arity.languageFeatures.enable` (see [Editor Setup](../guide/editors.md)).

## Quick fixes

Every lint finding that carries a fix is offered as a `quickfix` action when the
cursor or selection overlaps the finding's range; a zero-width cursor touching
the edge of the range counts as overlapping. The action's title is the fix's own
description, and it is attached to the diagnostic, so clients that fix from the
lightbulb on a squiggle find it there.

Both safe and unsafe fixes appear. On the command line an unsafe fix is applied
only with `arity lint --fix --unsafe-fixes`, because the CLI edits in bulk; in
the editor you are approving one edit at a time with the diff in front of you,
so the distinction stops carrying its weight. Which rules have a fix, and
whether it is safe, is recorded per rule in the [lint rule reference](rules.md).

A fix is a textual edit and does not owe you layout: it may leave a line the
formatter would break differently, because layout is the formatter's job. The
intended sequence is fix, then format.

## Refactors

### Add/Update roxygen documentation

A `refactor` action that generates or extends the
[roxygen2](https://roxygen2.r-lib.org) block for the function under the cursor.
It is offered when the cursor sits anywhere in a function bound by a simple
assignment (`name <- function(...)`), including inside the body. The function
must be the direct value of the assignment, so a function nested in a call on
the right-hand side does not qualify.

**Add** --- when no roxygen block immediately precedes the function, insert a
skeleton above it: a title placeholder, one `@param` per formal in declaration
order, and `@return`, at the statement's own indentation. A blank line between a
block and the function detaches it, following roxygen2's own rule, so a detached
block counts as no block.

```r
add <- function(x, y = 1) {
  x + y
}
```

becomes

```r
#' Title
#'
#' @param x
#' @param y
#'
#' @return
add <- function(x, y = 1) {
  x + y
}
```

**Update** --- when a block is already attached but some formals are
undocumented, insert only the missing `@param` lines, in formal order, after the
last existing `@param` (or after the introductory prose if there is no `@param`
yet):

```r
#' Add two numbers
#'
#' @param x A number.
add <- function(x, y = 1) {
  x + y
}
```

becomes

```r
#' Add two numbers
#'
#' @param x A number.
#' @param y
add <- function(x, y = 1) {
  x + y
}
```

The action is non-destructive: existing prose and tags are never rewritten,
reordered, or removed. Nothing is offered when every formal is already
documented. Descriptions are left empty for you to fill in; arity does not
invent documentation text.
