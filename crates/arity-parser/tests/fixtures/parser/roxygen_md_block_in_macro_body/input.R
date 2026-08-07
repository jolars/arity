#' @md
#' @details
#' \describe{
#'   \item{term}{a definition, then a list
#'
#'   - one
#'   - two
#'
#'   }
#'   \item{plain}{no block here}
#' }
#'
#' \describe{
#'   \item{lazy}{intro
#'   - alpha
#'   - beta
#'   }
#' }
#'
#' \describe{
#'   \item{code}{leading prose
#'
#'   ```
#'   mean(x)
#'   ```
#'   }
#' }
#'
#' \itemize{
#'   \item first
#'
#'   1. a
#'   2. b
#'
#' }
#'
#' A list line that would itself close the enclosing macro is withheld: the
#' macro's closing delimiter cannot live inside the list's node, so the marker
#' stays body prose.
#'
#' \describe{
#'   \item{shut}{intro
#'   - one}
#' }
#' @name md_block_in_macro_body
NULL

#' Markdown off: no block constructs are recognized in a macro body.
#'
#' @details
#' \describe{
#'   \item{term}{a definition
#'
#'   - one
#'   - two
#'
#'   }
#' }
#' @name md_block_in_macro_body_nomd
NULL
