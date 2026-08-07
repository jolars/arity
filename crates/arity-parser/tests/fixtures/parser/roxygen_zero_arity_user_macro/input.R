#' Zero-arity Rd user macros
#'
#' @details
#' Brace-less \sspace and \LaTeX are complete calls.
#'
#' A written group is not an argument: \sspace{} and \LaTeX{x}.
#'
#' Nested: \emph{\LaTeX{} here} and \code{\sspace{}}.
#'
#' An argument-taking user macro still consumes its group: \doi{10.1/2}.
#' @name zero_arity
NULL

#' Under markdown
#'
#' @md
#' @details
#' Still zero-arity: \LaTeX{} and *emphasis* around \sspace here.
#' @name zero_arity_md
NULL
