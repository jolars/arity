#' Bare brace groups in literal Rd prose
#'
#' @details
#' A bare group is a list: a {b c} d.
#' Groups nest: e {f {g} h} i.
#' A group spans a macro: j {k \emph{x} l} m.
#' An empty group is a bare list: n {} o.
#' Escaped braces stay literal: p \{q\} r.
#' An even run opens a group: s \\{t} u.
#' A group can lead a line: {v w} x.
#' A group spans a
#' soft break: {y
#' z} end.
#' @name rd_brace_group
NULL
