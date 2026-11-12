value <- 100
Base <- methods::setRefClass("Base", fields = list(value = "numeric"))
Child <- methods::setRefClass(
  "Child", contains = "Base", methods = list(
    read = function() { value <<- value + 1; list(value, .self, .refClassDef) },
    initialize = function(...) initFields(...),
    external = function(.self) list(.self$value, value)
  )
)
print(Child)
