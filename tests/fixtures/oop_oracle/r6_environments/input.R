self <- "lexical self"
private <- "lexical private"
super <- "lexical super"
helper <- function() self
Base <- R6::R6Class(
  "Base",
  public = list(
    read = function(value = self) identical(value, self),
    nested = function() function() self,
    boxed = list(fn = function() self)
  )
)
Child <- R6::R6Class(
  "Child", inherit = Base,
  public = list(
    read = function(value = private$value) value,
    shadow = function(self) self,
    assign = function(value = self) {
      before <- self
      self <- "local self"
      paste(is.environment(before), value)
    }
  ),
  private = list(value = 2, private_read = function() self),
  active = list(doubled = function() private$value * 2)
)
Empty <- R6::R6Class(
  "Empty", private = list(),
  public = list(read = function() paste(private, super))
)
NonPortable <- R6::R6Class(
  "NonPortable", portable = FALSE,
  public = list(read = function() private$value),
  private = list(value = 3)
)
Aliased <- R6::R6Class("Aliased", public = list(read = helper))
base <- Base$new()
child <- Child$new()
empty <- Empty$new()
nonportable <- NonPortable$new()
aliased <- Aliased$new()
