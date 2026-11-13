Parent <- methods::setClass("Parent", slots = c(value = "numeric"))
methods::setGeneric("g", function(x) standardGeneric("g"))
