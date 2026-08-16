#' Wrapped inline Rd macros are not block macros
#'
#' An inline macro the author soft-wrapped stays inline markup wherever its
#' opener lands: mid-prose \code{f(a,
#' b)} here, and at a line start
#' \href{http://example.org}{over
#' two lines} there.
#'
#' @details \eqn{
#'   x^2
#' } opened on the tag's own line.
#' @param x prose that continues onto the next line, where the opener sits
#'   mid-prose: \code{c("top",
#'   "bottom")}
#' @param y prose whose continuation line *starts* with the opener,
#'   \code{c("left",
#'   "right")}
#' @return A block macro still owns its lines:
#' \itemize{
#'   \item one
#'   \item two
#' }
#' @name wrapped_inline_macro
NULL
