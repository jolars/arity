#' Link-reference definition with a trailing macro stays prose
#'
#' @details
#' A definition line whose destination is followed by more content (here an
#' `\emph{}` macro) is not a valid definition, so the shortcut self-defines and
#' the line stays literal prose: see [foo] here.
#'
#' [foo]: https://example.com \emph{bar}
#' @md
#' @name md_url_reference_trailing_macro
NULL
