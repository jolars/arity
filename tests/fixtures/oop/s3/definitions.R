speak <- function(x, ...) UseMethod("speak")
speak.point <- function(x, ...) x$value
speak.default <- function(x, ...) "default"
point <- structure(list(value = 1), class = "point")
speak(point)
