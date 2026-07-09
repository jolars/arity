#' Invalid inline link destinations fall back to shortcuts
#'
#' @details
#' A bare destination with an interior space [a](x\ y) is not an inline link, so
#' the [a] resolves as a shortcut and the (x\ y) stays literal, as do the
#' multi-space [b](p q r) and the still-spaced [c](url ok) forms. A valid
#' destination keeps its link: [d](x%20y) percent-encodes, [e](<x y>) uses angle
#' brackets, and [f](url "t") carries a title.
#' @md
#' @name md_link_invalid_dest
NULL
