# `assignment-in-condition`

Flag an assignment (`<-`, `=`, `<<-`, `:=`) used as the direct condition of an `if`/`while`. The bare `=` form (often a `==` typo) is autofixed to `==`; the others are reported without a fix.

`=` where `==` was meant:

```r
if (x = 5) print(x)
```

```text
warning: assignment-in-condition
 --> example.R:1:5
  |
1 | if (x = 5) print(x)
  |     ^^^^^ assignment used as a condition; did you mean `==`?
  = help: Replace `=` with `==` or move the assignment out.
```

After applying the fix:

```r
if (x == 5) print(x)
```
