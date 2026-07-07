#' Bare brace groups in markdown prose
#'
#' @md
#' @details
#' A bare group is a list: a {b c} d.
#' Groups nest: e {f {g} h} i.
#' A group spans emphasis: j {k *x* l} m.
#' An empty group is a bare list: n {} o.
#' Escaped braces stay literal: p \{q\} r.
#' An even run opens a group: s \\{t} u.
#' A literal percent keeps a group: v % {w} x.
#' A comment percent hides one: y \% {z} gone
#' but the next line groups: {aa} bb.
#' @name md_brace_group
NULL
