# `shadowed-builtin`

Flag a local binding whose name is exported by a default R package when that name is later used as a call in the same scope (`c <- 1; c(2, 3)`). The two-step trigger keeps false positives down.

Shadowing base `c()` and then calling it:

```r
c <- 1
c(2, 3)
```

```text
warning: shadowed-builtin
 --> example.R:1:1
  |
1 | c <- 1
  | ^ local binding `c` shadows a base-R name later used in this scope
  = help: Rename the local, or fully qualify the base call (e.g. `base::c`).
```
