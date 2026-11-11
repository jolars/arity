Base <- R6::R6Class("Base", public = list(read = function() 1))
Child <- R6::R6Class("Child", inherit = Base, public = list(value = 2))
obj <- Child$new()
print(obj$read())
