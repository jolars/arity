Foo <- S7::new_class("Foo", properties = list(value = S7::class_numeric))
audit_get <- S7::new_generic("audit_get", "x")
S7::method(audit_get, Foo) <- function(x) x@value
obj <- Foo(value = 1)
stopifnot(identical(audit_get(obj), 1))
print(obj@value)
