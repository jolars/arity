#' Link displays whose macro argument carries active markdown
#'
#' @details
#' A shortcut [a\emph{*x*}] is dropped because its macro argument is active
#' markdown, but [a\emph{x}] survives (literal argument) and [a\code{*z*}] too
#' (fragile macro, body protected).
#' @md
#' @name md_link_macro_arg_drop
NULL
