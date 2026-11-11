Point <- S7::new_class("Point", properties = list(value = S7::class_numeric))
ColoredPoint <- S7::new_class(
  "ColoredPoint", parent = Point, properties = list(color = S7::class_character)
)
describe <- S7::new_generic("describe", "x")
S7::method(describe, Point) <- function(x) paste("point", x@value)
combine <- S7::new_generic("combine", c("x", "y"))
S7::method(combine, list(Point, ColoredPoint)) <- function(x, y) x@value + y@value
base <- Point(value = 1)
child <- ColoredPoint(value = 2, color = "red")
