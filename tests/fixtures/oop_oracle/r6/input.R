Base <- R6::R6Class(
  "Base",
  public = list(
    value = NULL,
    initialize = function(value = 1) self$value <- value,
    read = function(scale = 1) self$value * scale
  )
)
Child <- R6::R6Class(
  "Child", inherit = Base,
  public = list(
    initialize = function(value = 2, label = "child") {
      super$initialize(value)
      private$label <- label
    },
    read = function(scale = 2) paste(private$label, super$read(scale))
  ),
  private = list(label = NULL),
  active = list(doubled = function() self$value * 2)
)
base <- Base$new()
child <- Child$new()
