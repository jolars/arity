#' Basic loose merge: one list across single and double blanks
#'
#' @md
#' @details
#' - a
#'
#' - b
#'
#'
#' - c
#' @name a
NULL

#' Ordered loose merge: start numbers are irrelevant
#'
#' @md
#' @details
#' 1. one
#'
#' 5. five
#' @name b
NULL

#' Type changes split: bullet char and ordered delimiter
#'
#' @md
#' @details
#' - a
#'
#' * star
#'
#' 1. one
#'
#' 2) paren
#' @name c
NULL

#' Nested across blanks: deeper nests, shallower is a sibling
#'
#' @md
#' @details
#' - x
#'
#'   - deep
#'
#'   - deep two
#'
#' - y
#' @name d
NULL

#' From-value loose merge; blank then prose ends the list
#'
#' @md
#' @note - value a
#'
#' - value b
#'
#' prose after
#' @name e
NULL
