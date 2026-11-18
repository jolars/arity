Parent <- S7::new_class("Parent", properties = list(
  id = S7::class_integer,
  value = S7::new_property(S7::class_character, default = "parent")
))
Child <- S7::new_class("Child", parent = Parent, properties = list(
  value = S7::new_property(S7::class_character, default = "child"),
  S7::new_property(S7::class_integer, name = "readonly", getter = function(self) self@id)
))
Custom <- S7::new_class("Custom", properties = list(id = S7::class_integer), constructor = function(seed = 1L) {
  S7::new_object(S7::S7_object(), id = seed)
})
Sub <- S7::new_class("Sub", parent = Custom, properties = list(flag = S7::class_logical))
choose <- S7::new_generic("choose", c("first", "second"))
left <- function(first, second) "left"
numeric <- function(first, second) "numeric"
missing_first <- function(first, second) "missing_first"
specific <- function(first, second) "specific"
S7::method(choose, list(Parent, S7::class_any)) <- left
S7::method(choose, list(S7::class_any, S7::new_union(S7::class_integer, S7::class_double))) <- numeric
S7::method(choose, list(S7::class_missing, S7::class_any)) <- missing_first
S7::method(choose, list(Child, S7::class_any)) <- specific
child <- Child(id = 3L)
S7::prop(child, "value") <- "set"
S7::props(child) <- list(id = 4L)
length_generic <- base::length
S7::method(length_generic, Parent) <- function(x) 23L
S7::S4_register(Parent)
methods::setGeneric("probeS4", function(x) methods::standardGeneric("probeS4"))
S7::method(probeS4, Parent) <- function(x) "s4"
