value <- 100
Base <- methods::setRefClass(
  "Base", fields = list(value = "numeric"),
  methods = list(read = function() value)
)
Child <- methods::setRefClass(
  "Child", contains = "Base", fields = c(label = "character"),
  methods = list(
    initialize = function(...) initFields(...),
    read = function() paste(label, callSuper()),
    update = function() { value <<- value + 1; value },
    nested = function() { f <- function() value; f() },
    definition = function() .refClassDef@className,
    external = function(.self) paste(.self$value, value),
    probe = function(.self) paste(exists(".refClassDef"), exists("initFields"))
  )
)
base <- Base$new(value = 1)
child <- Child$new(value = 2, label = "child")
