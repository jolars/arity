#' Without markdown an underline line is literal.
#' Title
#' ===
foo <- 1

#' With markdown these do not head a paragraph.
#'
#' @md
#' @details
#'
#' ---
#' A dash run after a blank is a thematic-break position, not setext.
#'
#' -
#' A single dash is an empty list bullet.
#'
#' =-=
#' A mixed run is literal prose.
bar <- 2
