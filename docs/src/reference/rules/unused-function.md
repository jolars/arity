# `unused-function`

Flag an exported function that nothing in the project calls. The complement of `unused-binding`, which stays quiet on public API: a function is reported here only when it is declared exported (a roxygen `@export`, or a NAMESPACE `export()`) *and* no file that can see it reads it. S3 methods are exempt — dispatch reaches them without a direct call, so having no caller says nothing about them. Disabled by default, since a library's exported functions are meant to be called from outside the project.

This rule is **disabled by default**; enable it with `select`.

`add_one` is exported but never called anywhere in the package:

```r
#' Add one
#'
#' @export
add_one <- function(x) {
  x + 1
}
```

```text
warning: unused-function
 --> example.R:4:1
  |
4 | add_one <- function(x) {
  | ^^^^^^^ exported function `add_one` is never called
  = help: Remove it, or stop exporting it if it is not part of the public API.
```
