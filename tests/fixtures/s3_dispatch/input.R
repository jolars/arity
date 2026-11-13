g <- function(x, ...) {
  x <- structure(list(), class = "changed")
  local <- 42
  UseMethod("g")
  stop("UseMethod returned")
}
g.C <- function(x, ...) .Method
g.D <- function(x, ...) .Method
g.default <- function(x, ...) .Method
g.changed <- function(x, ...) .Method
probe("ordered", g(structure(list(), class = c("C", "D"))))
probe("reversed", g(structure(list(), class = c("D", "C"))))
probe("default", g(structure(list(), class = "missing")))
probe("original-object", g(structure(list(), class = "C")))

as.data.frame.a.b <- function(x, ...) .Method
probe("dotted", as.data.frame(structure(list(), class = "a.b")))
length.C <- function(x) .Method
length.default <- function(x) .Method
probe("internal", length(structure(list(), class = "C")))
probe("internal-default", length(structure(list(), class = "absent")))

Math.C <- function(x, ...) .Method
abs.D <- function(x) .Method
abs.C <- function(x) .Method
probe("member-first", abs(structure(1, class = "C")))
probe("class-before-member", sqrt(structure(1, class = c("C", "D"))))
probe("group-parent", abs(structure(1, class = c("missing", "D"))))
Ops.C <- function(e1, e2) unique(.Method[nzchar(.Method)])
x <- structure(1, class = "C")
probe("operator-left", x + 1)
probe("operator-right", 1 + x)
probe("operator-unary", +x)
probe("operator-names", `+`(right = x, left = 1))

`[.C` <- function(x, ...) .Method
`[<-.C` <- function(x, ..., value) { attr(x, "method") <- .Method; x }
probe("subset", x[1])
probe("replacement", attr(`[<-`(x, 1, value = 2), "method"))

advance <- function(x, ...) UseMethod("advance")
advance.C <- function(x, ...) { class(x) <- "changed"; paste(.Method, NextMethod(), sep = ",") }
advance.D <- function(x, ...) .Method
probe("next-method", advance(structure(list(), class = c("C", "D"))))
