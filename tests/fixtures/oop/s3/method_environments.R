g <- function(x) UseMethod("g")
g.C <- function(x) {
  .Generic
  nested <- function() .Generic
  nested()
}
g.D <- function(x) .Generic
g.E <- function(x, .Generic) .Generic
outside <- function() .Generic
print(list(g, outside))
