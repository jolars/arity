#' R system Rd user macros
#'
#' @details
#' R loads `share/Rd/macros/system.Rd` before every Rd file, so parse_Rd expands
#' these into a `USERMACRO` node carrying the raw definition plus the argument
#' text, immediately followed by the expansion itself.
#'
#' A DOI \doi{10.1000/182}, a CRAN package \CRANpkg{dplyr}, and a bug report
#' \PR{1234} each expand in prose.
#'
#' The identity macro \I{stays put} expands to its argument, which then
#' coalesces with the surrounding prose.
#'
#' The build-stage variants expand the same way: \packageTitle{stats} and
#' \packageDescription{stats}, as does a bibentry citation \bibcitep{key}.
#'
#' Expansion happens wherever the macro sits: nested in \emph{\doi{10.1/2}} and
#' in \code{\CRANpkg{utils}}.
#' @references
#' A paper with a DOI, \doi{10.1000/182}.
#' @name user_macro_expansion
NULL

#' User macros under markdown
#'
#' @md
#' @details
#' A user macro's argument is protected from cmark, so `*b*` stays literal:
#' \CRANpkg{a *b* c}. The macro still expands: \doi{10.1/2}.
#' @name user_macro_expansion_md
NULL
