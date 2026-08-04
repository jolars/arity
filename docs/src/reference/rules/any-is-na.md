# `any-is-na`

Flag `any(is.na(x))`, which is the purpose-built `anyNA(x)`—faster (it short-circuits and builds no intermediate logical vector) and clearer.

The rule fires only on the clean single-argument shape and only when both `any` and `is.na` resolve to base R; a local redefinition of either is left alone.

This rule is **enabled by default**.

Testing for any missing value:

```r
if (any(is.na(x))) stop()
```

```text
warning: any-is-na
 --> example.R:1:5
  |
1 | if (any(is.na(x))) stop()
  |     ^^^^^^^^^^^^^ `any(is.na(x))` is the faster, clearer `anyNA(x)`
  = help: Use `anyNA(x)`.
```

After applying the fix:

```r
if (anyNA(x)) stop()
```
