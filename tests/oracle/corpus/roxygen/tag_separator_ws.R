#' Title
#'
#' roxygen2 consumes at most one whitespace character after the marker before
#' looking for `@`, so the next line is description prose, not a tag.
#'
#'  @param x Two spaces makes this prose.
#'
#' @param x A real tag.
#'	@return A tab separator is a real tag.
#'@details No separator at all is a real tag.
#' @name tag_separator_ws
NULL
