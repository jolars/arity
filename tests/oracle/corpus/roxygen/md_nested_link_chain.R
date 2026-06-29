#' Nested bracket deactivation chains
#'
#' @details
#' Multiple inner shortcuts each win over the outer inline link, as in
#' [a [b] c [d] e](https://example.org), and a three-level nest
#' [w [x [y] z] v](https://example.org) deactivates every enclosing bracket so
#' only the innermost shortcut resolves.
#' @md
#' @name md_nested_link_chain
NULL
