# `comparison-negation`

Flag a negated comparison — `!(a == b)`, `!x < y` — which reads more clearly as the opposite comparison (`a != b`, `x >= y`).

The fix is withheld when a comment in the operand would otherwise be lost.

Negating an equality test:

```r
if (!(a == b)) stop()
```

```text
warning: comparison-negation
 --> example.R:1:5
  |
1 | if (!(a == b)) stop()
  |     ^^^^^^^^^ negated comparison is clearer as the opposite operator
  = help: Flip the comparison instead of negating it.
```

After applying the fix:

```r
if (a != b) stop()
```
