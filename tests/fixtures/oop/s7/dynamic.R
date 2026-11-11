class_name <- paste0("Dyn", "Point")
Point <- S7::new_class(class_name, properties = list(value = S7::class_numeric))
create_generic <- S7::new_generic
describe <- create_generic(paste0("des", "cribe"), "x")
signature <- identity(Point)
S7::method(describe, signature) <- function(x) x@value
obj <- Point(value = 1)
print(describe(obj))
print(obj@value)
