#' Backslash runs before a letter under markdown
#'
#' @details
#' Odd runs re-form a macro: a \y b and c \\\y d.
#' Even runs are literal text: e \\y f and g \\\\y h.
#' Known zero-arg macros carve: i \dots j but even k \\dots l stays text.
#' @md
#' @name md_backslash_letter
NULL
