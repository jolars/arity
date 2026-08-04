# `roxygen-return`

Flag an `@export`ed function documented without `@return`.

CRAN requires every exported function's documentation to describe its return value (the `.Rd` `\value` section); roxygen2 itself stays silent, so the omission otherwise surfaces only at submission time. `@returns` is accepted as an alias. `@noRd` blocks and merged or inherited topics (`@rdname`, `@inherit`, …) are skipped.

This rule is **enabled by default**.

An exported function with no `@return`:

```r
#' Add one
#' @param x A number.
#' @export
add_one <- function(x) x + 1
```

```text
warning: roxygen-return
 --> example.R:3:4
  |
3 | #' @export
  |    ^^^^^^^ exported function is documented without `@return`
  = help: Add `@return` (or `@returns`) describing the value.
```
