R6Generator <- R6::R6Class("MutationR6", lock_class = TRUE,
  public = list(value = 2, read = function(before = 1) self$value * before),
  active = list(doubled = function(value) {
    if (missing(value)) self$value * 2 else self$value <- value / 2
  }))
old <- R6Generator$new()
locked_error <- inherits(try(R6Generator$set("public", "blocked", 1), silent = TRUE), "try-error")
R6Generator$unlock()
Alias <- R6Generator
Alias$set("public", "read", function(after = 3) self$value * after, overwrite = TRUE)
Alias$set("private", "secret", function(scale = 4) scale)
Alias$set("public", "added", NULL)
Alias$set("public", "clone", function(other) other, overwrite = TRUE)
R6Generator$lock()
fresh <- R6Generator$new()
fresh$doubled <- 10

ReferenceGenerator <- methods::setRefClass("MutationReference",
  fields = list(value = "numeric", doubled = function(new_value) {
    if (missing(new_value)) value * 2 else value <<- new_value / 2
  }), methods = list(read = function(before = 1) value * before,
                    removed = function() 1))
reference_old <- ReferenceGenerator(value = 2)
reference_cached <- ReferenceGenerator(value = 2)
cached <- reference_cached$read
ReferenceAlias <- ReferenceGenerator
ReferenceAlias$methods(read = function(after = 3) value * after,
                       added = function(scale = 4) value * scale)
ReferenceGenerator$methods(removed = NULL)
ReferenceGenerator$lock("value")
ReferenceGenerator$methods(after_lock = function() value)
reference_fresh <- ReferenceGenerator(value = 2)
field_lock_error <- inherits(try(reference_fresh$value <- 3, silent = TRUE), "try-error")
GlobalLockedGenerator <- methods::setRefClass("MutationLockedGlobal", where = globalenv())
lockBinding(".__C__MutationLockedGlobal", globalenv())
class_lock_error <- inherits(try(GlobalLockedGenerator$methods(blocked = function() 1), silent = TRUE), "try-error")
