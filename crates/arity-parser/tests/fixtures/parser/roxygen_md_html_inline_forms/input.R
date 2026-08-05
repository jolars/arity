#' Inline HTML comment, PI, declaration, and CDATA
#'
#' @details
#' before <!-- hidden -- ok --> after, empty <!--> and <!---> too
#'
#' pi <?php echo x ?> here, empty <??> too
#'
#' decl <!DOCTYPE html> and single-letter <!D x> here
#'
#' cdata <![CDATA[raw <b> ]] text]]> and lowercase <![cdata[y]]> here
#'
#' not a comment: a <!-- dashy ---> b, unterminated <!-- x -- > c
#'
#' not a declaration: <!doctype html> and <!DOCTYPE> stay literal
#' @md
#' @name md_html_inline_forms
NULL
