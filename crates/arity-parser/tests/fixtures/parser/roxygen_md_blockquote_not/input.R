#' Without markdown a leading angle is literal prose.
#' > not a quote
foo <- 1

#' With markdown a bare angle mid-text is not a quote opener.
#'
#' @md
#' @details
#' Prose with a > sign inside stays prose.
#' @name x
bar <- 2
