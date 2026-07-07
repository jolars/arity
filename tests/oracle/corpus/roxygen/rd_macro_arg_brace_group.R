#' Bare brace groups inside a macro arg
#'
#' @details
#' A latexlike arg groups: \emph{a {b} c}.
#' Groups nest: \sQuote{d {e {f} g} h}.
#' A group spans a macro: \emph{i {j \strong{k} l} m}.
#' An empty group is a list: \emph{n {} o}.
#' Escaped braces stay literal: \emph{p \{q\} r}.
#' A group can be the sole arg: \emph{{sole}}.
#' A structural display GRP-wraps: \href{http://x.org}{s {t} u}.
#' A verbatim arg never groups: \code{v {w} x}.
#' @name rd_macro_arg_brace_group
NULL
