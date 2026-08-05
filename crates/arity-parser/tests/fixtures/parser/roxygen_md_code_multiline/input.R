#' Multi-line code span
#'
#' @details
#' a `code
#' span` b
#' @md
#' @name multiline_basic
NULL

#' Emphasis wraps a multi-line code span
#'
#' @details
#' *em `code
#' span` end*
#' @md
#' @name multiline_wrapped
NULL

#' An earlier opener re-splits the line-scoped carve
#'
#' @details
#' a `open
#' b` and `closed`
#' @md
#' @name multiline_recarve
NULL

#' Unterminated opener stays literal, later emphasis still resolves
#'
#' @details
#' a ` open
#' no closer *em*
#' @md
#' @name multiline_unterminated
NULL

#' Double-backtick span crosses the break; edge spaces trim
#'
#' @details
#' a ``x `y`
#' z`` b
#'
#' a ` code
#' span ` b
#' @md
#' @name multiline_doubletick
NULL

#' A code span opener consumes a would-be HTML opener
#'
#' @details
#' a `x <!-- y
#' z` w -->
#' @md
#' @name multiline_codefirst
NULL

#' Same-line tag value opener
#'
#' @details start `code
#' span` end
#' @md
#' @name multiline_from_value
NULL

#' Not markdown: stays literal prose
#'
#' @details
#' a `code
#' span` b
#' @name multiline_nomd
NULL
