# `any-duplicated`

Flag `any(duplicated(x))`, which is the purpose-built `anyDuplicated(x) > 0`—faster (it short-circuits and builds no intermediate logical vector) and clearer.

The rule fires only on the clean single-argument shape and only when both `any` and `duplicated` resolve to base R; a local redefinition of either is left alone. Because the replacement is a comparison, the fix is withheld in a context that binds tighter than a comparison, where the bare rewrite would need parentheses.

Testing for any duplicate value:

```r
if (any(duplicated(x))) stop()
```

```text
warning: any-duplicated
 --> example.R:1:5
  |
1 | if (any(duplicated(x))) stop()
  |     ^^^^^^^^^^^^^^^^^^ `any(duplicated(x))` is the faster, clearer `anyDuplicated(x) > 0`
  = help: Use `anyDuplicated(x) > 0`.
```

After applying the fix:

```r
if (anyDuplicated(x) > 0) stop()
```
