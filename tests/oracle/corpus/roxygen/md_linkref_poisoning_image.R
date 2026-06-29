#' Image poisoning
#'
#' @details
#' A shortcut like [before] resolves, but an escaped-close candidate like
#' [stop\] poisons the appended definition block, so a later image
#' ![alt](https://example.org/x.png) survives as a figure yet still leaks a
#' synthesized reference definition.
#' @md
#' @name md_linkref_poisoning_image
NULL
