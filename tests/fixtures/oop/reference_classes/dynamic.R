class_name <- paste0("Dyn", "Counter")
field_name <- paste0("val", "ue")
Counter <- methods::setRefClass(
  class_name, fields = setNames(list("numeric"), field_name)
)
Counter$methods(read = function() .self$field(field_name))
obj <- Counter$new(value = 1)
print(obj$read())
