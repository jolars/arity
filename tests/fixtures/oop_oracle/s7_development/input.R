library(S7)
Point := new_class(properties = list(id = class_integer))
describe := new_generic("x")
registered <- function(x) "registered"
method(describe, list(Point)) <- registered
point <- Point(id = 2L)
S4_register(Point)
methods::setClass("S4Point", contains = S4_contains(Point), slots = c(label = "character"))
methods::setClass("Plain", slots = c(seed = "integer"))
Colored := new_class(parent = methods::getClass("Plain"), properties = list(color = class_character))
wrapped <- methods::new("S4Point", id = 5L, label = "wrapped")
colored <- Colored(seed = 8L, color = "red")
