# `seq`

Flag colon ranges from `1` up to a length—`1:length(x)`, `1:nrow(x)`, or `1:n`—which silently count *down* when that length is zero (`1:0` is `c(1L, 0L)`, not an empty sequence), a classic off-by-one bug for loops over possibly-empty input. `seq_along(x)` and `seq_len(n)` return a zero-length sequence instead, and agree with the colon form everywhere else.

The `length`/`nrow`/`ncol`/`NROW`/`NCOL` forms fire only when the callee resolves to base R; a redefinition is left alone. Literal ranges (`1:10`) and computed bounds (`1:(n - 1)`) are not flagged.

This rule is **enabled by default**.

Ranges over a vector's indices and up to a count:

```r
for (i in 1:length(x)) print(x[i])
for (j in 1:n) f(j)
```

```text
warning: seq
 --> example.R:1:11
  |
1 | for (i in 1:length(x)) print(x[i])
  |           ^^^^^^^^^^^ `1:length(x)` counts down (`1:0`) when the length is zero; `seq_along(x)` handles empty input
  = help: Use `seq_along(x)`.
warning: seq
 --> example.R:2:11
  |
2 | for (j in 1:n) f(j)
  |           ^^^ `1:n` counts down (`1:0`) when the length is zero; `seq_len(n)` handles empty input
  = help: Use `seq_len(n)`.
```

After applying the fix:

```r
for (i in seq_along(x)) print(x[i])
for (j in seq_len(n)) f(j)
```
