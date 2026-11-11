NewR6 <- R6::R6Class(
  "NewR6",
  public = list(
    initialize = function(id = NULL) {
      if (is.null(id)) {
        private$id <- "myid"
      }
    }
  ),
  private = list(id = NULL)
)
obj <- NewR6$new()
print(obj)
