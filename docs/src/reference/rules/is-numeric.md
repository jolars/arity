# `is-numeric`

Flag `is.numeric(x) || is.integer(x)` (and the vectorized `|` spelling), which is just `is.numeric(x)`: `is.numeric()` already returns `TRUE` for integer vectors, so the disjunction adds nothing and suggests a misreading of what `is.numeric()` tests.

The rule fires only when both operands are single-argument calls on the same argument and both callees resolve to base R; a local redefinition of either is left alone.

Testing for a numeric vector:

```r
if (is.numeric(x) || is.integer(x)) mean(x)
```

```text
warning: is-numeric
 --> example.R:1:5
  |
1 | if (is.numeric(x) || is.integer(x)) mean(x)
  |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ `is.numeric(x) || is.integer(x)` is redundant—`is.numeric()` is already `TRUE` for integer vectors
  = help: Use `is.numeric(x)`.
```

After applying the fix:

```r
if (is.numeric(x)) mean(x)
```
