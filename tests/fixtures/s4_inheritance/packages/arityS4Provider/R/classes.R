Marker <- methods::setClass("Marker", slots = c(marker = "logical"))
methods::setMethod("g", "Parent", function(x) "parent")
methods::setMethod("g", "numeric", function(x) "numeric")
methods::setMethod("length", "Parent", function(x) 1L)
