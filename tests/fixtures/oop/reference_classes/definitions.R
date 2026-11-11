Base <- methods::setRefClass("Base", fields = list(value = "numeric"))
Child <- methods::setRefClass(
  "Child", contains = "Base", methods = list(read = function() 1)
)
obj <- Child$new(value = 1)
print(obj$read())
