read_impl <- function(scale = 2) value * scale
Base <- methods::setRefClass(
  "Base", fields = list(value = "numeric", doubled = function(value) {
    if (missing(value)) .self$value * 2 else .self$value <- value / 2
  }), methods = list(
    initialize = function(value = 3, ...) callSuper(value = value, ...),
    read = read_impl,
    external = function(.self, increment = 1) .self$value + increment
  )
)
Alias <- Base
Child <- methods::setRefClass(
  "Child", contains = "Base", fields = c(label = "character"),
  methods = list(read = function(scale = 4) callSuper(scale))
)
Grandchild <- methods::setRefClass("Grandchild", contains = "Child")
Other <- methods::setRefClass("Other", fields = c("first", "second"),
  methods = list(read = function(other = 5) other))
Multiple <- methods::setRefClass("Multiple", contains = c("Child", "Other"))
Formal <- methods::setClass("Formal", slots = c(slot = "numeric"))
Mixed <- methods::setRefClass("Mixed", contains = c("Formal", "Base"),
  methods = list(read = function(scale = 6) callSuper(scale)))
Builtin <- methods::setRefClass("Builtin", contains = "envRefClass",
  methods = list(copy = function() callSuper(), show = function() callSuper()))
CustomInit <- methods::setRefClass("CustomInit", methods = list(
  initFields = function(x = 7) .self,
  initialize = function(...) callSuper(...)))
base <- Alias()
child <- Child$new(label = "child")
grandchild <- getRefClass("Grandchild")()
other <- Other()
multiple <- Multiple()
mixed <- Mixed()
builtin <- Builtin()
custom_init <- CustomInit()
