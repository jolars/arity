#' HTML block start conditions 2 through 5 under markdown
#'
#' @md
#' @details
#' A comment block runs to its closer, through a blank line:
#' <!-- comment line one
#'
#' comment line two -->
#' Prose after the comment.
#'
#' A processing instruction:
#' <?php echo 1; ?>
#' A declaration closes on the same line:
#' <!DOCTYPE html>
#' A CDATA section:
#' <![CDATA[ raw < & data ]]>
#' Prose after everything.
#' @name md_html_conditions
NULL
