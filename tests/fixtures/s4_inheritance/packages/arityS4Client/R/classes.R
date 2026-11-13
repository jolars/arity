methods::setClass("Child", contains = "Parent")
probe <- function() methods::selectMethod("g", "Child")
probeLength <- function() methods::selectMethod("length", "Child")
