#' Markdown blocks inside a block macro body
#'
#' @md
#' @details
#' A markdown block construct inside a block Rd macro's body is a block, not
#' prose: cmark sees the whole field flat, so the `\describe{`/`\item{` lines are
#' ordinary paragraph text and a list at the same column opens a real list.
#'
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
#' A list may also interrupt the definition's paragraph with no blank line:
#'
#' \describe{
#'   \item{lazy}{intro
#'   - alpha
#'   - beta
#'   }
#' }
#'
#' An item whose body is only a list takes no `GRP` wrapper:
#'
#' \describe{
#'   \item{bare}{
#'
#'   - solo
#'   }
#' }
#'
#' A fenced code block folds in the same way:
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
#' An ordered list inside an `\itemize` body, terminated by a blank line:
#'
#' \itemize{
#'   \item first
#'
#'   1. a
#'   2. b
#'
#' }
#' @name md_block_in_macro_body
NULL

#' Block macro bodies with markdown off
#'
#' @details
#' With markdown off the same text is literal Rd prose: a dash opens no list and
#' a fence is not a code block, so the definition stays one text run.
#'
#' \describe{
#'   \item{term}{a definition, then what looks like a list
#'
#'   - one
#'   - two
#'
#'   }
#' }
#' @name md_block_in_macro_body_nomd
NULL
