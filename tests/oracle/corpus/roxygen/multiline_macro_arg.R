#' Multi-line Rd macro arguments
#'
#' @details
#' A two-argument macro's last argument may span `#'` lines, so the whole call
#' is a block macro even though its earlier groups closed on the opening line.
#'
#' \describe{
#'   \item{a}{a definition long enough that it
#'   continues onto a second line}
#'   \item{b}{a definition with a \code{nested}
#'   inline macro and a bare \{group\} across lines}
#'   \item{\code{term}}{a nested macro in the term argument, whose
#'   definition also spans lines}
#'   \item{c}{one line} and trailing prose
#'   \item{d}{a definition that closes here} \item{e}{and another follows}
#' }
#'
#' A link display can also span lines: \href{http://example.org}{the
#' example site} sits mid-prose.
#'
#' \tabular{ll}{
#'   a \tab b \cr
#' }
#' @name multiline_macro_arg
NULL
