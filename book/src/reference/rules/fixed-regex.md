# `fixed-regex`

Flag a base-R regex call (`grepl`, `grep`, `sub`, `gsub`, `regexpr`, `gregexpr`, `regexec`) whose pattern is a plain string literal with no regex metacharacter, and add `fixed = TRUE`—it skips regex compilation and states that the pattern is a literal.

The rule fires only when the callee resolves to base R and no `fixed`/`ignore.case`/`perl` argument is already present. Because a metacharacter-free pattern matches identically either way, the fix (inserting `, fixed = TRUE`) is safe.

A literal pattern matched as a regex:

```r
grepl("abc", x)
```

```text
warning: fixed-regex
 --> example.R:1:7
  |
1 | grepl("abc", x)
  |       ^^^^^ `grepl()` with a literal pattern should use `fixed = TRUE`
  = help: Add `fixed = TRUE`.
```

After applying the fix:

```r
grepl("abc", x, fixed = TRUE)
```
