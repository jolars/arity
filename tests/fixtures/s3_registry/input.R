g <- function(x) UseMethod("g")
handler <- function(x) x
g.implicit <- function(x) x + 1
.S3method("g", "script.class", "handler")
.S3method("g", "implicit")
register <- base::.S3method
register("g", "alias", handler)
.onLoad <- function(lib, pkg) {
  base::registerS3method("g", "hook", handler)
}
