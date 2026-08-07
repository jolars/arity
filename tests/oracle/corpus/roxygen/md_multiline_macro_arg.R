#' Markdown in a multi-line Rd macro argument
#'
#' @details
#' Under `@md` a structural macro's argument is markdown, and an inline span may
#' cross the argument's line break.
#'
#' \describe{
#'   \item{a}{*emphasis across
#'   the line* and \emph{an Rd macro}}
#'   \item{**b**}{a [link](https://example.org) and
#'   a `code span`}
#' }
#' @md
#' @name md_multiline_macro_arg
NULL
