#' Rd macros with more than two argument groups
#'
#' @details
#' Three groups: \ifelse{html}{yes}{no} and a fourth stays prose:
#' \ifelse{a}{b}{c}{d}.
#'
#' Two groups each: \if{latex}{x}, \method{print}{foo}, \S3method{format}{bar},
#' \S4method{show}{baz}, \enc{Jöreskog}{Joreskog}, and
#' \subsection{Inline}{body}.
#'
#' A one-argument macro still stops at its first group: \code{x}{y}.
#'
#' \subsection{Spanning heading}{
#'   A two-argument macro's body across lines.
#' }
#'
#' \ifelse{html}{
#'   The html branch spans lines.
#' }{
#'   The fallback branch does too.
#' }
#'
#' Mid-prose, a three-argument macro's last group spans lines:
#' \ifelse{html}{yes}{a fallback
#' across lines} resumes here.
#'
#' @details
#' \ifelse{html}{unterminated to the block end
#' @name rd_macro_arity
NULL
