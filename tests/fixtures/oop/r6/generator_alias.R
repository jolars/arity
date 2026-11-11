Generator <- R6::R6Class("Display", public = list(value = 1))
Child <- R6::R6Class("Child", inherit = Generator)
obj <- Child$new()
print(obj$value)
