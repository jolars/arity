#' Backslash pairing in prose
#'
#' @details
#' Even runs stay literal: a \\y b and c \\\\y d.
#' Odd runs carve the macro: e \y f and g \\\y h.
#' Zero-arg known macros carve: \dots and \ldots and \R and \cr and \tab.
#' Parity gates them too: even \\dots literal, odd \\\dots carves.
#' Other known names brace-less stay literal: \emph z and \code z.
#' @name backslash_parity
NULL

#' Markdown mode shares the parity gate
#'
#' @details
#' Even \\y literal, odd \\\y carves, \dots carves, \\dots literal.
#' @md
#' @name backslash_parity_md
NULL
