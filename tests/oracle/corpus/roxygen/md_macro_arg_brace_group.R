#' Bare brace groups inside a macro arg under md
#'
#' @md
#' @details
#' A latexlike arg groups: \emph{a {b} c}.
#' Groups nest: \strong{d {e {f} g} h}.
#' A group spans a macro: \emph{i {j \strong{k} l} m}.
#' An empty group is a list: \emph{n {} o}.
#' A structural display GRP-wraps: \href{http://x.org}{s {t} u}.
#' A verbatim arg never groups: \code{v {w} x}.
#' @name md_macro_arg_brace_group
NULL
