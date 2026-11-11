Counter <- methods::setRefClass(
  "AuditCounter", fields = list(value = "numeric"),
  methods = list(read = function() value, get = function() .self$read())
)
obj <- Counter$new(value = 1)
stopifnot(identical(obj$read(), 1), identical(obj$get(), 1))
