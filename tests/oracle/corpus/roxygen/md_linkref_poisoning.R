#' Mixed link-reference poisoning
#'
#' @details
#' A shortcut like [before] resolves, but once an escaped-close candidate
#' like [stop\] appears, its unclosed synthesized definition poisons the rest
#' of the appended block: every following definition leaks, and a later
#' shortcut like [after] is de-linked into literal text.
#' @md
#' @name md_linkref_poisoning
NULL
