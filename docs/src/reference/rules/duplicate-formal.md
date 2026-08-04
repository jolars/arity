# `duplicate-formal`

Flag a function defined with two parameters of the same name. R raises a runtime error (`repeated formal argument`); this catches it statically.

This rule is **enabled by default**.

Two parameters named `x`:

```r
f <- function(x, x) x
```

```text
error: duplicate-formal
 --> example.R:1:18
  |
1 | f <- function(x, x) x
  |                  ^ parameter `x` is declared more than once in this function
  = help: Rename one of the parameters.
```
