new_class <- function(name, properties) properties
Fake <- new_class("Pretend", properties = list(value = 1))
new_generic <- function(name, dispatch_args) identity
describe <- new_generic("describe", "x")
`method<-` <- function(generic, signature, value) generic
method(describe, Fake) <- function(x) x
print(describe(Fake))
