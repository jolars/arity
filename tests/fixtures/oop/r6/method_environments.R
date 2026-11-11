self <- 1
private <- 2
super <- 3
Base <- R6::R6Class("Base", private = list(value = 1))
Child <- R6::R6Class(
  "Child", inherit = Base,
  public = list(
    read = function(value = private$value) {
      nested <- function() list(self, private, super, value)
      nested()
    },
    shadow = function(self) self,
    construct = function() {
      self$fn <- function() private$value
      self$fn
    }
  ),
  private = list(private_read = function() list(self, private, super)),
  active = list(doubled = function() private$value * 2)
)
Plain <- R6::R6Class(
  "Plain", portable = FALSE,
  public = list(read = function() self)
)
ordinary <- function() unrelated
print(list(Base, Child, Plain, ordinary))
