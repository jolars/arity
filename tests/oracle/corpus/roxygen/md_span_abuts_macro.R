#' Markdown emphasis spans an inline Rd macro
#'
#' @details
#' An opener abutting a macro spans it: a*\code{x} y*.
#'
#' A span also crosses a macro over a soft break: *a \code{b}
#' c*.
#'
#' A closer abutting a macro stays literal: a*\code{z}*b.
#' @md
#' @name md_span_abuts_macro
NULL
