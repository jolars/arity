#' Invalid inline link destinations
#'
#' @details
#' A space in a bare destination [t](a\ b) is not a link, so [t] is a shortcut
#' and (a\ b) stays literal, whereas [u](a%20b) and [v](url "title") remain
#' inline links.
#' @md
#' @name md_link_invalid_dest
NULL
