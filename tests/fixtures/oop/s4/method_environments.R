methods::setGeneric("g", function(x) standardGeneric("g"))
methods::setMethod("g", "ANY", function(x) {
  .defined
  nested <- function() .defined
  nested()
})
methods::setMethod("length", "ANY", function(x) list(.Generic, .defined))
g(1)
