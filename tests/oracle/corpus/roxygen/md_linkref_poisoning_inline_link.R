#' Inline-link poisoning
#'
#' @details
#' A shortcut like [before] resolves, but an escaped-close candidate like
#' [stop\] poisons the appended definition block, so a later inline link
#' [after](https://example.org) survives as a link yet still leaks a
#' synthesized reference definition.
#' @md
#' @name md_linkref_poisoning_inline_link
NULL
