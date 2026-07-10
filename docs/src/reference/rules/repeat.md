# `repeat`

Flag `while (TRUE)`, an unconditional loop better written as `repeat`.

`repeat` states the intent—loop until a `break`/`return`—without the dummy `TRUE` condition. Only the reserved literal `TRUE` is matched; the rebindable `T` is left to `true-false-symbol`.

An unconditional `while` loop:

```r
while (TRUE) {
  poll()
}
```

```text
warning: repeat
 --> example.R:1:1
  |
1 | while (TRUE) {
  | ^^^^^^^^^^^^ `while (TRUE)` is an unconditional loop; use `repeat`
  = help: Write `repeat` for a loop with no exit condition.
```

After applying the fix:

```r
repeat {
  poll()
}
```
