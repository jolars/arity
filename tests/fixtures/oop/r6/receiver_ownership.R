A <- R6::R6Class("A", public = list(read = function() self$alpha))
B <- R6::R6Class("B", public = list(read = function() self$beta))
C <- R6::R6Class(
  "C", public = list(gamma = 1, read = function() self$gamma)
)
print(list(A, B, C))
