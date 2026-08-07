#' Brace-less system Rd user macros in prose
#'
#' @details A one-argument system macro swallows its tail: before \doi b after.
#' @seealso An identity macro does too: a \I b c.
#' @note A two-argument one as well: x \manual y z.
#' @source Soft-wrapped past the trigger: p \CRANpkg q
#'   and a continued line.
#' @references A zero-arity one expands instead: r \sspace s t.
#' @author An unknown name is a node: u \zzz v w.
#' @name braceless_system_macro
NULL

#' Brace-less system macros under markdown
#'
#' @md
#' @details Markdown mode swallows too: before \doi b after.
#' @seealso Wrapped under markdown: p \proglang q
#'   and a continued line.
#' @note A written group still binds: see \doi{10.1/x} here.
#' @name md_braceless_system_macro
NULL
