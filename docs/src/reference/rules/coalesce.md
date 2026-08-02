# `coalesce`

Flag `if (is.null(x)) y else x` (and its mirror `if (!is.null(x)) x else y`), which is the null-coalescing `x %||% y`—shorter, and it evaluates `x` once instead of twice.

The rule fires only when `is.null` resolves to base R; a local redefinition is left alone. The fix is unsafe: `%||%` needs R >= 4.4 (or rlang), and collapsing the two evaluations of `x` changes behavior when `x` has side effects.

Falling back to a default when a value is `NULL`:

```r
y <- if (is.null(x)) default else x
```

```text
warning: coalesce
 --> example.R:1:6
  |
1 | y <- if (is.null(x)) default else x
  |      ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ `if (is.null(x)) y else x` is the null-coalescing `x %||% y`
  = help: Use `x %||% y`.
```
