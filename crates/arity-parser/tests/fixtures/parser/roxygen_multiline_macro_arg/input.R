#' A two-argument Rd macro whose last argument spans lines
#'
#' @details
#' \describe{
#'   \item{a}{a definition that
#'   continues}
#'   \item{\code{t}}{a nested macro in the term, definition
#'   across lines} and trailing prose
#'   \item{b}{one line}
#' }
#'
#' Mid-prose too: \href{http://example.org}{a display
#' across lines} resumes here.
#'
#' Unclosed mid-prose stays literal prose: \href{http://example.org}{never
#' closed.
#'
#' @details
#' \describe{
#'   \item{a}{unterminated to the block end
#' @name multiline_macro_arg
NULL
