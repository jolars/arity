#' Link-reference poisoning inside a section body
#'
#' @md
#' @section My heading:
#' A shortcut like [before] resolves, but an escaped-close candidate like
#' [stop\] poisons the appended definition block, so a later [after] shortcut
#' is de-linked and both tail definitions leak after the colon.
#' @name md_linkref_poisoning_section
NULL
