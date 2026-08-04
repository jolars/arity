# `if-always-true`

Flag an `if` whose condition is the literal `TRUE` or `FALSE`. The branch is decided statically, so the `if` is dead control flow. Only the bare literals are flagged—not folded constants (`if (1 == 1)`) or the rebindable symbols `T`/`F`.

This rule is **enabled by default**.

An `if` gated on a constant always takes the same branch:

```r
if (TRUE) {
  f()
} else {
  g()
}
```

```text
warning: if-always-true
 --> example.R:1:5
  |
1 | if (TRUE) {
  |     ^^^^ `if` condition is always `TRUE`
  = help: The condition always holds; the branch always runs.
```
