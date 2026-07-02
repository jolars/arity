#' Without markdown a hash line is literal.
#' # Not a heading here
foo <- 1

#' With markdown these are still not headings.
#'
#' @md
#' @details
#' #hashtag is not a heading
#' ####### seven hashes is not a heading
bar <- 2
