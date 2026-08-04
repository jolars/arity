# `unused-binding`

Flag a local binding that is never read in the same file. Function parameters, `for`-loop variables, and names beginning with `.` are exempt, since those are meaningful even when unused.

This rule is **enabled by default**.

`x` is assigned but never used:

```r
x <- 1
y <- 2
print(y)
```

```text
warning: unused-binding
 --> example.R:1:1
  |
1 | x <- 1
  | ^ local binding `x` is assigned but never read
  = help: Remove the assignment, or prefix the name with `.` to mark it intentional.
```
