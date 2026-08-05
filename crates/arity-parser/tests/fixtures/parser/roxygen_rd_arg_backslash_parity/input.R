#' Backslash pairing inside a macro argument
#'
#' @details
#' Paired runs stay literal text: a \emph{\\y} b and c \emph{\\dots} d.
#' Odd runs carve the nested macro: e \emph{\y} f and g \emph{\\\dots} h.
#' Genuine nesting still carves: \emph{p \strong{q} r} and \emph{x \dots y}.
#' @name rd_arg_backslash_parity
NULL
