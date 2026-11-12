# Test-only observations from real R. Production dispatch never evaluates R.
args <- commandArgs(trailingOnly = TRUE)
fixture <- normalizePath(args[[1L]])
probe <- function(name, value) cat(name, "\t", value, "\n", sep = "")
sys.source(file.path(fixture, "input.R"), globalenv())

forward <- function(alpha, beta = "generic", ...) {
  alpha <- "changed"
  local <- "local"
  UseMethod("forward")
}
forward.C <- function(alpha, beta = "method", ...) {
  paste(alpha[[1]], beta, exists("local", inherits = FALSE), sep = ",")
}
probe("forwarding", forward(al = structure("original", class = "C")))
frame <- function(x, y, ...) UseMethod("frame")
frame.C <- function(x, y, ...) { y <- "current"; NextMethod(y = "extra", appended = "new") }
frame.D <- function(x, y, ...) paste(y, list(...)$kept, list(...)$appended, sep = ",")
probe("next-forwarding", frame(structure(1, class = c("C", "D")), "original", kept = "promise"))

nested <- function(x) UseMethod("nested")
nested.C <- function(x) (function() NextMethod())()
nested.D <- function(x) "nested.D"
probe("nested-next", tryCatch(nested(structure(1, class = c("C", "D"))), error = function(e) "error"))
