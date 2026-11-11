generic_name <- paste0("spe", "ak")
class_name <- paste0("poi", "nt")
speak <- function(x) UseMethod(generic_name)
implementation <- function(x) x$value
registerS3method(generic_name, class_name, implementation)
obj <- structure(list(value = 1), class = class_name)
speak(obj)
