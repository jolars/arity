Point <- R6::R6Class(classname = "Point", public = list(value = 1))
obj <- Point$new()
print(obj$value)
