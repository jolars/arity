#' Backslash pairing in literal Rd prose
#'
#' @details
#' An even run before a word is literal: a \\y b and c \\\\y d.
#' An odd run re-forms the macro: e \\\y f.
#' A lone backslash before a space stays: g \ h and i \\\ j.
#' Rd escapes resolve: k \% l and m \{x\} n.
#' An even run leaves the percent bare: o \\% comments out.
#' @name rd_backslash_parity
NULL
