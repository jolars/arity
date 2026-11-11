Base <- methods::setRefClass(
  "Base", fields = list(value = "numeric"),
  methods = list(read = function(scale = 1) .self$value * scale)
)
Child <- methods::setRefClass(
  "Child", contains = "Base", fields = list(label = "character"),
  methods = list(read = function(scale = 2) paste(label, callSuper(scale)))
)
base <- Base$new(value = 1)
child <- Child$new(value = 2, label = "child")
