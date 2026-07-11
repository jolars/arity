# `class-equals`

Flag comparisons of `class(x)` against a string literal—`class(x) == "cls"`, `class(x) != "cls"`, and the `%in%` membership spellings—which are `inherits(x, "cls")` (or its negation): `class()` returns the whole class vector, so the comparison is elementwise (an error in an `if ()` condition on a multi-class object) and misses subclasses, while `inherits()` asks the intended question directly without materializing the vector.

The rule fires only on the clean single-argument shape and only when `class` resolves to base R; a local redefinition is left alone. The fix is **unsafe**: on a multi-class object the comparison yields an elementwise vector where `inherits()` yields a scalar, and for S4 objects `inherits()` follows the formal inheritance chain.

Testing an object's class:

```r
if (class(x) == "factor") levels(x)
```

```text
warning: class-equals
 --> example.R:1:5
  |
1 | if (class(x) == "factor") levels(x)
  |     ^^^^^^^^^^^^^^^^^^^^ comparing `class(x)` to a string is fragile—`class()` returns a vector; `inherits()` asks directly
  = help: Use `inherits(x, "cls")` (or `!inherits(x, "cls")`).
```
