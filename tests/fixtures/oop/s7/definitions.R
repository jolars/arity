Point <- S7::new_class("Point", properties = list(value = S7::class_numeric))
ColoredPoint <- S7::new_class(
  "ColoredPoint", parent = Point, properties = list(color = S7::class_character)
)
describe <- S7::new_generic("describe", "x")
S7::method(describe, Point) <- function(x) x@value
obj <- ColoredPoint(value = 1, color = "red")
print(describe(obj))
