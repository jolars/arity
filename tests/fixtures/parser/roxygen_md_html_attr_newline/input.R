#' Newline ends an unquoted attribute value; the bad attribute kills the tag
#'
#' @details
#' <foo bar=baz
#' bim!bop />
#' @md
#' @name attr_newline_invalid
NULL

#' Valid bare attribute after the soft break: the tag resolves across lines
#'
#' @details
#' a <foo bar=baz
#' bim /> b
#' @md
#' @name attr_newline_valid
NULL

#' cm-623 whole shape: none of the candidates are tags
#'
#' @details
#' < a><
#' foo><bar/ >
#' <foo bar=baz
#' bim!bop />
#' @md
#' @name attr_newline_spec
NULL
