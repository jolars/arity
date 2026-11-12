metadata <- function(frame) {
  names <- c(
    ".Generic", ".Method", ".Class", ".Group", ".GenericCallEnv",
    ".GenericDefEnv", ".Methods", ".defined", ".target"
  )
  present <- vapply(names, exists, logical(1), envir = frame, inherits = TRUE)
  paste(names[present], collapse = ",")
}
Dispatch <- function(value = 1) structure(value, class = "Dispatch")
g <- function(x, ...) UseMethod("g")
g.Dispatch <- function(x, meta = .Generic, ...) {
  nested <- function() paste(meta, .Generic, sep = ":")
  paste(metadata(environment()), nested(), sep = ";")
}
Ops.Dispatch <- function(e1, e2) metadata(environment())
`[.Dispatch` <- function(x, ...) metadata(environment())
register <- base::registerS3method
register("g", "Registered", function(x, ...) metadata(environment()))
shadow.Dispatch <- function(x, .Generic = "formal") .Generic
shadow <- function(x, ...) UseMethod("shadow")
Formal <- methods::setClass("DispatchFormal", slots = c(value = "numeric"))
methods::setClass("HiddenDispatchClass", slots = c(value = "numeric"))
methods::setGeneric("readDispatch", function(x) standardGeneric("readDispatch"))
methods::setMethod("readDispatch", "DispatchFormal", function(x) {
  metadata(environment())
})
methods::setMethod("length", "DispatchFormal", function(x) metadata(environment()))
methods::setMethod("Math", "DispatchFormal", function(x) metadata(environment()))
`replaceDispatch<-` <- function(x, value) x
methods::setReplaceMethod("replaceDispatch", "DispatchFormal", function(x, value) {
  metadata(environment())
})
