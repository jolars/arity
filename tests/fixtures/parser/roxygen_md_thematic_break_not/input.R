#' Without markdown a rule line is literal prose.
#' ***
#' ---
foo <- 1

#' With markdown, look-alikes that are not thematic breaks stay prose.
#'
#' @md
#' @details
#' Two stars are emphasis delimiters, not a break: **x**.
#'
#' A `***` run inside a code span is not a break.
#'
#' A two-dash run is too short to break:
#'
#' --
#' @name mtb_not
bar <- function() NULL
