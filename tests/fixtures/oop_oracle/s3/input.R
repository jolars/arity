point <- function(value = 1) {
  structure(list(value = value), class = "point")
}
colored_point <- function(value = 2, color = "red") {
  structure(list(value = value, color = color), class = c("colored_point", "point"))
}
describe <- function(x, ...) UseMethod("describe")
describe.default <- function(x, ...) "default"
describe.point <- function(x, ...) paste("point", x$value)
describe.colored_point <- function(x, ...) paste(x$color, NextMethod())
`+.point` <- function(e1, e2) point(e1$value + e2)
base <- point()
child <- colored_point()
