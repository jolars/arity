class_name <- paste0("Dyn", "Counter")
factory <- R6::R6Class
Counter <- factory(class_name, public = list(value = 1))
member_name <- paste0("re", "ad")
Counter$set("public", member_name, function() self$value)
obj <- Counter$new()
print(obj$read())
