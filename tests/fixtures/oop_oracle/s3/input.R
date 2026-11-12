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

assigned_point <- function(value) {
  object <- list(value = value)
  class(object) <- c("assigned", "point", "assigned")
  return(object)
}
legacy_point <- function(value) {
  object <- list(value = value)
  oldClass(object) <- c("legacy", "point")
  object
}
assigned <- assigned_point(3)
legacy <- legacy_point(4)
copied <- assigned
class(assigned) <- "changed"
removed <- structure(list(), class = "removed")
oldClass(removed) <- NULL
empty_class <- structure(list(), class = character())
explicit_numeric <- structure(1, class = "numeric")
implicit_numeric <- 1
class(implicit_numeric) <- "numeric"
legacy_numeric <- `oldClass<-`(1, "numeric")
matrix_class <- structure(1:4, dim = c(2L, 2L))
class(matrix_class) <- c("matrix", "array")
last_class <- structure(list(), class = "first", class = "last")
copied_class <- structure(list(), class = oldClass(child))
