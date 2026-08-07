#' Unknown Rd macros never consume a group
#'
#' parse_Rd tags an unrecognized \zzz UNKNOWN and consumes nothing for it, so a
#' written group is a following sibling brace list.
#'
#' @details
#' One group: \zzz{x} stays a sibling. Two groups: \zzz{x}{y} are two siblings.
#' An empty one still counts: \zzz{} here.
#'
#' The optional-argument bracket is literal too: \zzz[a]{x} and a bracket with
#' no group at all, \zzz[a] here.
#'
#' Nested in a macro argument: \emph{\zzz{x} inside} and in R code,
#' \code{\zzz{x}} keeps its braces verbatim.
#'
#' A group may span lines:
#' \zzz{
#'   spanning
#' }
#'
#' A defined system user macro is the contrast --- \CRANpkg{utils} does consume
#' its argument, and so does the two-argument \manual{a}{b}.
#' @name unknown_macro_group
NULL

#' Unknown Rd macros under markdown
#'
#' @md
#' @details
#' The rule is mode-independent: \zzz{*x*} leaves the group behind (its markdown
#' still resolves), and a brace-less \zzz is the same UNKNOWN node.
#' @name unknown_macro_group_md
NULL
