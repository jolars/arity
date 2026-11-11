setRefClass <- function(Class, ...) list()
Fake <- setRefClass("Pretend", fields = list(value = "numeric"))
outside <- function() .self$value
print(Fake)
print(outside)
