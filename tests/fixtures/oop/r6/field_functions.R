Container <- R6::R6Class(
  "Container",
  public = list(
    boxed = list(fn = function() self),
    evaluated = private,
    read = function() list(self, private, super, unknown),
    nested = function() function() self
  ),
  private = list()
)
ordinary <- function() list(self, private, super)
print(list(Container, ordinary))
