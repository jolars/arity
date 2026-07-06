#' Multi-line inline HTML comment
#'
#' @details
#' a <!-- x
#' y --> b
#' @md
#' @name multiline_comment
NULL

#' Emphasis wraps a multi-line span; tag and declaration forms
#'
#' @details
#' *a <!-- x
#' y --> b*
#'
#' t <span
#' class="v"> u
#'
#' d <!A
#' y> e
#' @md
#' @name multiline_forms
NULL

#' CDATA, PI, and two spans in one paragraph
#'
#' @details
#' c <![CDATA[ x
#' y ]]> d
#'
#' a <!-- x
#' --> <?y
#' z ?> b
#' @md
#' @name multiline_cdata_pi
NULL

#' Dash at end of line still closes; interior markup literalizes
#'
#' @details
#' a <!-- x -
#' --> b
#'
#' a <!-- *x*
#' `y` --> *b*
#' @md
#' @name multiline_edges
NULL

#' Unterminated opener stays literal, later emphasis still resolves
#'
#' @details
#' a <!-- x
#' y *z* w
#' @md
#' @name multiline_unterminated
NULL

#' Same-line tag value opener
#'
#' @details a <!-- x
#' y --> b
#' @md
#' @name multiline_from_value
NULL

#' Not markdown: stays literal prose
#'
#' @details
#' a <!-- x
#' y --> b
#' @name multiline_nomd
NULL
