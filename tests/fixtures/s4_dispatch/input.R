methods::setClass("S4A")
methods::setClassUnion("S4Either", c("S4A", "NULL"))
methods::setGeneric("s4Pair", function(x, y, context = 1, ...) standardGeneric("s4Pair"), signature = c("y", "x"))
methods::setMethod("s4Pair", "numeric", function(x, y, context = 1, ...) "first")
methods::setMethod("s4Pair", methods::signature(x = "numeric", y = "missing"), function(x, y, context = 1, ...) "missing")
methods::setMethod("s4Pair", c("numeric", "character"), function(x, y, context = 1, ...) "positional")
methods::setMethod("s4Pair", c("numeric", x = "character"), function(x, y, context = 1, ...) "mixed")
methods::setMethod("s4Pair", definition = function(x, y, context = 1, ...) "any")
methods::setMethod("s4Pair", character(), function(x, y, context = 1, ...) "empty")
methods::setMethod("s4Pair", methods::signature(), function(x, y, context = 1, ...) "empty_signature")
methods::setGeneric("s4Value<-", function(object, value) standardGeneric("s4Value<-"))
methods::setReplaceMethod("s4Value", c("S4Either", "missing"), function(object, value) object)
s4Fallback <- function(x, y = 1) "fallback"
methods::setGeneric("s4Default", function(x, y = 1) "default")
methods::setGeneric("s4Explicit", function(x, y = 1) standardGeneric("s4Explicit"), useAsDefault = s4Fallback)
s4Existing <- function(x) "existing"
methods::setGeneric("s4Existing")
s4Previous <- function(x) "previous"
methods::setGeneric("s4Previous", function(x) standardGeneric("s4Previous"))
methods::setGeneric("s4Disabled", function(x) "disabled", useAsDefault = FALSE)
methods::setGeneric("s4Grouped", function(x) standardGeneric("s4Grouped"), group = "Math")
methods::setMethod("Arith", c(e2 = "S4A"), function(e1, e2) e2)
methods::setGeneric("s4Next", function(x, y) "default")
methods::setMethod("s4Next", c(x = "numeric"), function(x, y) "numeric")
methods::setMethod("s4Next", c("numeric", "numeric"), function(x, y) {
    x <- "changed"
    methods::callNextMethod("replacement", y)
})
s4Promoted <- function(x, y = 1) "original"
methods::setMethod("s4Promoted", "numeric", function(x, y = 1) "promoted")
methods::setGeneric("s4Qualified", function(x) base::standardGeneric("s4Qualified"))
methods::setGeneric("s4Partial", function(object, other) standardGeneric("s4Partial"))
methods::setMethod("s4Partial", methods::signature(obj = "numeric", oth = "character"), function(object, other) "positional")
methods::setMethod("s4Partial", c("character", "numeric"), function(object, other) "reversed")
