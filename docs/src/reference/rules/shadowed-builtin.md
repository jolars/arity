# `shadowed-builtin`

Flag a local binding to a function whose name is exported by a default R package when that name is later called in the same scope (`c <- function(...) ...; c(2, 3)`). A value binding (`names <- names(x)`) is exempt: R's call-position lookup skips non-function locals, so it is not a hazard.

This rule is **enabled by default**.

Binding a function over base `c()` and then calling it:

```r
c <- function(x, y) x
c(2, 3)
```

```text
warning: shadowed-builtin
 --> example.R:1:1
  |
1 | c <- function(x, y) x
  | ^ local binding `c` shadows a base-R name later used in this scope
  = help: Rename the local, or fully qualify the base call (e.g. `base::c`).
```
