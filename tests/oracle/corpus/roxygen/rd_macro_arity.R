#' Per-macro Rd argument arity
#'
#' @details
#' parse_Rd gives each Rd macro its own argument count, so \ifelse{html}{yes}{no}
#' consumes three groups while a one-argument macro leaves the rest as prose.
#'
#' An argument with several atoms wraps: \ifelse{html}{yes \emph{em} branch}{plain
#' no}. A fourth group is not an argument: \ifelse{a}{b}{c}{d} ends the call.
#'
#' The two-argument conditionals and dispatch forms behave the same way:
#' \if{latex}{only in LaTeX}, \method{print}{foo}, \S3method{format}{bar}, and
#' \S4method{show}{baz}.
#'
#' Encoding fallbacks take two groups too: \enc{Jöreskog}{Joreskog}.
#'
#' A single-line \subsection{Inline heading}{with a body} nests in place.
#' @name rd_macro_arity
NULL

#' Multi-argument system user macros
#'
#' @details
#' Now that three-argument macros parse, the system macros that expand through
#' a conditional work: a language name \proglang{C++} in prose.
#'
#' A manual citation \manual{R-exts}{Writing R Extensions} takes two arguments,
#' and \bibinfo{author}{Smith}{extra} takes three.
#'
#' They expand nested as well: \emph{\proglang{Rust}} and
#' \code{\manual{R-lang}{R Language Definition}}.
#' @name rd_macro_arity_user
NULL

#' Arity under markdown
#'
#' @md
#' @details
#' A fragile multi-argument macro keeps every argument literal:
#' \ifelse{html}{*yes*}{*no*} and \method{print}{*foo*}.
#'
#' A non-fragile one has each argument markdown-processed:
#' \subsection{A *title*}{a **body** with `code`} and \enc{*a*}{*b*}.
#' @name rd_macro_arity_md
NULL

#' Block-form multi-argument macros
#'
#' @details
#' \subsection{Spanning heading}{
#'   The body of a two-argument macro may span lines, exactly as a
#'   \code{\link{describe}} definition does.
#' }
#'
#' \ifelse{html}{
#'   The html branch spans lines.
#' }{
#'   The fallback branch does too.
#' }
#' @name rd_macro_arity_block
NULL
