#' Block quote paragraph state
#'
#' @md
#' @details
#' > bar
#' >
#' baz
#'
#' Indented code inside a quote is not a paragraph:
#' >     code
#'     outside
#'
#' A fence opener inside a quote blocks laziness:
#' > ```
#' outside
#'
#' Over-indented prose is lazy while the paragraph is open:
#' > foo
#'     - still lazy text
#' @name x
NULL
