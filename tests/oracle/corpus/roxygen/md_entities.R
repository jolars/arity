#' HTML character references in markdown prose
#'
#' @details
#' Named entities like &amp; and &copy; decode, as do numeric ones: &#65; and
#' &#x41;. A null &#0; becomes the replacement character. An unknown &nope; and a
#' bare &amp without a semicolon stay literal.
#' @md
#' @name md_entities
NULL
