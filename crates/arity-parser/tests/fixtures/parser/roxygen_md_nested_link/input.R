#' A nested inline [a [b] c](https://o.org) keeps the inner shortcut.
#' A nested shortcut [foo [bar] baz] does too, as does [[x]](https://e.com).
#' A nested inline-in-inline [a [b](https://i.org) c](https://o.org) link.
#' @md
f <- function(x) x
