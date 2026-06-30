#' Markdown inside non-fragile Rd macro arguments
#'
#' @details
#' A non-fragile macro has its argument markdown-processed: \emph{*x*} nests an
#' emphasis, \strong{*y*} too, and \emph{a *b* c} resolves a span mid-argument.
#' A fragile \code{*z*} keeps its body literal.
#' @md
#' @name md_macro_arg_emphasis
NULL
