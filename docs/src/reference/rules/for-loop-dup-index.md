# `for-loop-dup-index`

Flag a nested `for` loop that reuses the index variable of an enclosing `for` loop. R loops introduce no scope, so the inner loop overwrites the outer index rather than shadowing it: the outer loop resumes with a corrupted counter and any later read of the name sees the inner loop's last value.

A loop nested inside a *function* defined in the outer body is not flagged—it runs in its own frame and leaves the outer index alone. No fix is offered, since the repair is to invent a new index name.

This rule is **enabled by default**.

The inner loop overwrites the outer loop's counter:

```r
for (i in 1:10) {
  for (i in 1:5) {
    print(i)
  }
}
```

```text
warning: for-loop-dup-index
 --> example.R:2:8
  |
2 |   for (i in 1:5) {
  |        ^^^^^^^^ loop index `i` is already the index of an enclosing `for` loop
  = help: Rename this loop index so it does not overwrite the enclosing loop's `i`.
```
