# `outer-negation`

Flag `any(!x)` / `all(!x)`, which by De Morgan's law read more clearly with the negation pulled outside: `!all(x)` and `!any(x)`.

The rule fires only when *every* positional argument is negated (a `na.rm` argument is allowed and preserved). The fix is withheld when the call sits in a context that binds tighter than `!`, where the rewrite would need parentheses.

Negating every element of an aggregation:

```r
if (any(!ok)) stop()
```

```text
warning: outer-negation
 --> example.R:1:5
  |
1 | if (any(!ok)) stop()
  |     ^^^^^^^^ negating an aggregation is clearer with the negation outside
  = help: `any(!x)` is `!all(x)`; `all(!x)` is `!any(x)`.
```

After applying the fix:

```r
if (!all(ok)) stop()
```
