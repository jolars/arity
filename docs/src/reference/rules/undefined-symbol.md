# `undefined-symbol`

Flag an identifier read that resolves to no in-scope binding and no known package export.

Gated for safety: the rule stays silent for a whole file unless every `library()`-attached package is indexed, since an un-indexed package could export the otherwise-unresolved name.

`subtotal` resolves to nothing:

```r
total <- subtotal
```

```text
warning: undefined-symbol
 --> example.R:1:10
  |
1 | total <- subtotal
  |          ^^^^^^^^ no in-scope binding or attached package exports `subtotal`
```
