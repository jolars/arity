#' Markdown inside structural Rd macro arguments
#'
#' @details
#' A structural two-arg macro markdown-processes each argument. See the
#' \href{http://x.org}{*the* site} link, whose URL stays verbatim while its
#' display resolves emphasis and wraps in a group.
#'
#' \describe{
#'   \item{*term*}{a \strong{bold} def}
#'   \item{x}{a \code{*y*} b}
#' }
#'
#' \tabular{ll}{
#'   *a* \tab **b** \cr
#'   c \tab d \cr
#' }
#' @md
#' @name md_macro_arg_structural
NULL
