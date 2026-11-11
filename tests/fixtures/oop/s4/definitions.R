Point <- methods::setClass("Point", slots = c(value = "numeric"))
ColoredPoint <- methods::setClass(
  "ColoredPoint", contains = "Point", slots = c(color = "character")
)
methods::setGeneric("describe", function(x) standardGeneric("describe"))
methods::setMethod("describe", "Point", function(x) x@value)
obj <- Point(value = 1)
describe(obj)
