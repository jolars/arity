#' Tab-stop expansion reaches CommonMark's indented-code threshold
#'
#' A leading tab expands to the next 4-column stop, so a tab line is
#' indented code; interior tabs stay verbatim in the code body.
#'
#' @md
#' @details
#' 	foo	baz		bim
#'     next
#' 	bar
#' @name md_tab_indent_code
NULL
