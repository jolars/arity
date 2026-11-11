Base <- R6::R6Class("Base", public = list(read = function() 1))
Child <- R6::R6Class(
  "Child", inherit = Base,
  public = list(value = 1, read = function() {
    c(self$value, private$id, super$read())
  }),
  private = list(id = 2),
  active = list(double = function() self$value * 2)
)
obj <- Child$new()
print(obj$read())
print(obj$double)
