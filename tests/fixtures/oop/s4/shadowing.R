setClass <- function(Class, ...) Class
setGeneric <- function(name, def) def
fake <- setClass("Pretend", contains = "Parent")
describe <- setGeneric("describe", function(x) x)
print(describe(fake))
