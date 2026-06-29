#' Undefined reference label in a nested bracket
#'
#' @details
#' A nested reference [a [b] c][ref] links only its inner shortcut: the inner
#' [b] is defined, while the outer ref label is not, so the surrounding brackets
#' stay literal.
#' @md
#' @name md_undefined_ref
NULL
