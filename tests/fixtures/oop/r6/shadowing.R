R6Class <- function(classname, public) public
fake <- R6Class("Pretend", public = list(read = function() private$id))
outside <- function() self$id
print(fake)
print(outside)
