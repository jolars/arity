#' Title
#'
#' @md
#' @details <span>
#' folded into the block
#'
#' after the block
#' @name a
NULL

#' Title
#'
#' @md
#' @description <!-- note
#' still inside -->
#' after the closer
#' @name b
NULL

#' Title
#'
#' @md
#' @details <div>trailing text
#' gathered
#' @name c
NULL

#' Title
#'
#' @md
#' @details <pre>
#' verbatim body
#' </pre>
#' after the closer
#' @name d
NULL

#' Title
#'
#' @md
#' @details   <span>
#' ===
#' underline swallowed by the block, not setext
#' @name e
NULL

#' Title
#'
#' @md
#' @details some prose <span>
#' stays an inline tag value
#' @name f
NULL

#' Title
#'
#' @md
#' @details      <span>
#' deep indent stays an inline value (indented-code backlog)
#' @name g
NULL

#' Title
#'
#' @details <span>
#' no markdown mode, stays a literal prose value
#' @name h
NULL

#' Title
#'
#' @md
#' @name <span>
#'
#' a non-prose tag keeps its verbatim value
NULL
