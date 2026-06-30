#' Emphasis spanning a nested macro in a structural Rd argument
#'
#' @details
#' roxygen2 markdown-processes a structural argument as one cmark run, so an
#' emphasis span crosses a nested Rd macro.
#'
#' \describe{
#'   \item{x}{*a \strong{y} b*}
#' }
#'
#' \tabular{ll}{
#'   *a \tab b* \cr
#' }
#' @md
#' @name md_macro_arg_span
NULL
