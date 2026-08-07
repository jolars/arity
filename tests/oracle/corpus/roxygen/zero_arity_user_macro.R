#' Zero-arity R system Rd user macros
#'
#' @details
#' The macros \sspace and \LaTeX are defined with no placeholder, so parse_Rd
#' expands them on the name alone and never consumes a following group.
#'
#' A period.\sspace{}Then \LaTeX{} typesetting.
#'
#' A written group is a sibling list, not an argument: \sspace{x} and
#' \LaTeX{y} both leave their braces behind.
#'
#' They expand wherever they sit: \emph{\LaTeX{} in emphasis} and
#' \code{\sspace{}} in code.
#' @references
#' Typeset with \LaTeX.
#' @name zero_arity_user_macro
NULL

#' Zero-arity user macros under markdown
#'
#' @md
#' @details
#' The expansion is plain Rd in both modes: \LaTeX{} and \sspace{} still
#' expand, and *emphasis* around \LaTeX still resolves.
#' @name zero_arity_user_macro_md
NULL
