describe <- function(x) UseMethod("describe")
describe.point <- function(x) {
  list(.Generic, .Method, .Class, .GenericCallEnv, .GenericDefEnv)
}
obj <- structure(list(value = 1), class = "point")
obj$value
describe(obj)
