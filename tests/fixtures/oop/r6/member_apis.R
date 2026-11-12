reader <- function(scale = 2) self$value * scale
Original <- R6::R6Class(
  "Display",
  public = list(value = 1, read = reader,
                initialize = function(value = 3) self$value <- value),
  private = list(secret = 4, hidden = function() private$secret),
  active = list(doubled = function() self$value * 2)
)
Alias <- Original
Child <- R6::R6Class(
  inherit = Alias, class = FALSE, cloneable = FALSE,
  public = list(read = function(scale = 6) super$read(scale),
                parent_private = function() super$hidden())
)
base <- Alias$new(value = 3)
obj <- Child$new()
print(list(base, obj$read(scale = 4), obj$doubled))
