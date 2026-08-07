#' An unrecognized macro carves name-only: \zzz{x} leaves its group behind,
#' \zzz{x}{y} leaves two, \zzz{} leaves an empty one, and the optional-argument
#' bracket is literal prose too: \zzz[a]{x} and \zzz[a] alone.
#'
#' Nested in an argument, \emph{\zzz{x} inside}, and in code, \code{\zzz{x}}.
#'
#' An unbalanced group is not a block macro either:
#' \zzz{
#'   spanning
#' }
#'
#' A defined system user macro still consumes its argument: \CRANpkg{utils},
#' \doi{10.1/2}, and the two-argument \manual{a}{b}. A known built-in written
#' brace-less stays literal prose: \emph z.
#' @name unknown_macro_group
NULL
