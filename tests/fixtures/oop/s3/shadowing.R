UseMethod <- function(generic, object) object
generic <- function(x) UseMethod("generic", x)
print(generic(1))
outside <- function() .Generic
print(outside)
