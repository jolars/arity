#' HTML block case sensitivity
#'
#' @details
#' A lowercase CDATA opener still starts condition 5:
#'
#' <![cdata[
#' hidden ]]>
#' Prose after the block.
#'
#' <!doctype html>
#' A lowercase declaration is not a block; this stays one paragraph.
#' @md
#' @name md_html_block_case
NULL
