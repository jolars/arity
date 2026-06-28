#' Link-reference poisoning inside a field definition
#'
#' @md
#' @field x A shortcut like [before] resolves, but an escaped-close candidate
#'   like [stop\] poisons the appended definition block, so a later [after]
#'   shortcut is de-linked and both tail definitions leak.
#' @name md_linkref_poisoning_field
NULL
