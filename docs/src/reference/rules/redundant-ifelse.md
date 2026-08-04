# `redundant-ifelse`

Flag `ifelse(c, TRUE, FALSE)` (which is just `c`) and `ifelse(c, FALSE, TRUE)` (which is `!c`).

This rule is **enabled by default**.

An `ifelse` that returns its own condition:

```r
flag <- ifelse(cond, TRUE, FALSE)
```

```text
warning: redundant-ifelse
 --> example.R:1:9
  |
1 | flag <- ifelse(cond, TRUE, FALSE)
  |         ^^^^^^^^^^^^^^^^^^^^^^^^^ `ifelse()` returning `TRUE`/`FALSE` is redundant
  = help: Use the condition directly, or negate it.
```

After applying the fix:

```r
flag <- cond
```
