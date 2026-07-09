#' Inline link titles are dropped from the href
#'
#' @details
#' A titled link [a](https://ex.org/a "the title") drops its title, as do the
#' single-quote [b](https://ex.org/b 'quoted') and parenthesized
#' [c](https://ex.org/c (paren)) forms. An angle-bracketed
#' [d](<https://ex.org/d with space>) destination keeps its spaces, and an
#' entity [e](https://ex.org/e?x&amp;y) decodes.
#' @md
#' @name md_link_title
NULL
