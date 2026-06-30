#' Invalid link-reference destination stays prose
#'
#' @details
#' A destination with an unescaped space is not a valid definition, so the
#' shortcut self-defines and the line stays literal prose: see [ref] here.
#'
#' [ref]: https://example.com/a b
#' @md
#' @name md_url_reference_invalid_dest
NULL
