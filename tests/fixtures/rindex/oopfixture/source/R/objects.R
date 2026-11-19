setClass("Shape", slots = c(size = "numeric"))
setGeneric("area", function(x, scale = 1) standardGeneric("area"))
setMethod("area", "Shape", function(x, scale = 1) x@size * scale)
LocalBase <- R6::R6Class("BaseCounter", public = list(base_add = function(n = 2) n))
Counter <- R6::R6Class("Counter", inherit = LocalBase,
  public = list(count = 0, initialize = function(start = 0) self$count <- start,
    add = function(n = 1) n),
  private = list(secret = TRUE), active = list(value = function() 0))
RefCounter <- setRefClass("RefCounter", fields = list(count = "numeric"),
  methods = list(add = function(n = 1) n))
Point <- S7::new_class("Point", properties = list(x = S7::class_double))
measure <- S7::new_generic("measure", "x", function(x, scale = 1) S7::S7_dispatch())
S7::method(measure, Point) <- function(x, scale = 1) x@x * scale
describe <- function(x, ...) UseMethod("describe")
describe_impl <- function(x, ..., label = "fixture") label
malformed <- structure(new.env(), class = "R6ClassGenerator")
future <- structure(function(x) x, class = "FutureGenerator")
foreign_measure <- S7::new_external_generic("otherfixture", "foreign_measure", "x")
S7::method(foreign_measure, S7::new_union(Point, S7::class_double)) <- function(x) x
