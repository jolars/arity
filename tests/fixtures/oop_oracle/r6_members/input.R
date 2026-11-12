read <- function(scale = 2) self$value * scale
ReadAlias <- read
Original <- R6::R6Class(
  "Display",
  public = list(value = 3, initialize = function(value = 3) self$value <- value,
                read = ReadAlias),
  private = list(secret = 4, hidden = function(x = 5) x),
  active = list(doubled = function(value) {
    if (missing(value)) self$value * 2 else self$value <- value / 2
  })
)
Alias <- Original
Child <- R6::R6Class(
  "Display", inherit = Alias, class = FALSE,
  public = list(read = function(scale = 6) super$read(scale),
                parent_private = function() super$hidden()),
  private = list(secret = 7)
)
Grandchild <- R6::R6Class(inherit = Child,
  public = list(read = function(scale = 8) super$read(scale)))
NoClone <- R6::R6Class(cloneable = FALSE)
ChildClone <- R6::R6Class(inherit = NoClone)
RestoredClone <- R6::R6Class(inherit = ChildClone)
DisabledClone <- R6::R6Class(inherit = RestoredClone, cloneable = FALSE)
base <- Original$new()
child <- Child$new()
grandchild <- Grandchild$new()
no_clone <- NoClone$new()
child_clone <- suppressMessages(ChildClone$new())
restored_clone <- suppressMessages(RestoredClone$new())
disabled_clone <- suppressMessages(DisabledClone$new())
child$doubled <- 10
