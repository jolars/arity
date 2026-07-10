# `roxygen-unknown-tag`

Flag roxygen tags that roxygen2 does not understand.

roxygen2 warns on an unknown tag and drops it from the generated `.Rd`, so a misspelled tag (`@exprot`, `@parma`) silently loses documentation—or worse, an intended `@export` never reaches the `NAMESPACE`. Custom tags from extension roclets can be suppressed with `# arity-ignore roxygen-unknown-tag`.

A misspelled `@export`:

```r
#' Add one
#' @exprot
add_one <- function(x) x + 1
```

```text
warning: roxygen-unknown-tag
 --> example.R:2:4
  |
2 | #' @exprot
  |    ^^^^^^^ `@exprot` is not a tag roxygen2 understands
  = help: Check the spelling against the roxygen2 tag index; suppress with `# arity-ignore roxygen-unknown-tag` for extension-roclet tags.
```
