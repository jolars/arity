Point <- methods::setClass("Point", slots = c(value = "numeric"))
ColoredPoint <- methods::setClass(
  "ColoredPoint", contains = "Point", slots = c(color = "character")
)
methods::setGeneric("describe", function(x) standardGeneric("describe"))
methods::setMethod("describe", "Point", function(x) paste("point", x@value))
methods::setMethod("describe", "ColoredPoint", function(x) {
  paste(x@color, callNextMethod())
})
methods::setGeneric("combine", function(x, y) standardGeneric("combine"))
methods::setMethod("combine", c("Point", "ColoredPoint"), function(x, y) {
  x@value + y@value
})
base <- Point(value = 1)
child <- ColoredPoint(value = 2, color = "red")
