# `equals-na`

Flag `x == NA`, which is always `NA` rather than `TRUE`/`FALSE` — almost always a mistake for `is.na(x)`, which is the autofix.

Comparing to `NA` with `==`:

```r
x == NA
```

```text
warning: equals-na
 --> example.R:1:1
  |
1 | x == NA
  | ^^^^^^^ comparison with `NA` is always `NA`; use `is.na()`
  = help: Use `is.na(x)`.
```

After applying the fix:

```r
is.na(x)
```
