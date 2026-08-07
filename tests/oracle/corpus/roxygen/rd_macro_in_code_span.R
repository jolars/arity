#' Rd macros inside literal backticks
#'
#' @details
#' Without markdown a backtick is nothing to Rd, so parse_Rd sees straight
#' through it: `\emph{x}` and `\code{f()}` are real macros between two literal
#' backtick characters.
#'
#' The arity rule is unchanged inside them: `\href{https://e.org}{disp}` takes
#' both groups, and `\code{\link{y}}` nests.
#'
#' A zero-arity name stops at itself, so `\dots{}` leaves its group behind.
#'
#' An escaped backslash still forms no macro: `\\emph{x}` stays a plain span,
#' just like `f(x)` with no backslash at all.
#'
#' A double-backtick span behaves the same: ``\emph{y}`` and ``a `b` c``.
#' @references
#' See `\link{stats}` for the rest.
#' @name rd_macro_in_code_span
NULL

#' Backticks under markdown are a code span
#'
#' @md
#' @details
#' With markdown on the same text is a code span, so `\emph{x}` is protected
#' content rather than a macro, and *emphasis* around it still resolves.
#' @name rd_macro_in_code_span_md
NULL
