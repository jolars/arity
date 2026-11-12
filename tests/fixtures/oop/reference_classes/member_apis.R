Base <- methods::setRefClass(
  "Display", fields = list(value = "numeric", doubled = function(value) .self$value * 2),
  methods = list(read = function(scale = 2) value * scale,
                initialize = function(value = 3, ...) initFields(value = value, ...))
)
Alias <- Base
Child <- methods::setRefClass(
  "Child", contains = "Display", fields = c("label"),
  methods = list(read = function(scale = 4) callSuper(scale),
                external = function(.self, increment = 1) .self$value + increment)
)
obj <- getRefClass("Child")$new(value = 5)
direct <- Alias(value = 6)
print(list(obj$read(scale = 8), obj$external(), direct$copy(), obj$doubled))
