# Test-only observations from R's namespace parser and registration table.
args <- commandArgs(trailingOnly = TRUE)
fixture <- normalizePath(args[[1L]])
lib <- tempfile("arity-s3-registry-")
dir.create(file.path(lib, "fixture"), recursive = TRUE)
invisible(file.copy(file.path(fixture, "NAMESPACE"), file.path(lib, "fixture", "NAMESPACE")))
namespace <- parseNamespaceFile("fixture", lib)
rows <- apply(namespace$S3methods, 1L, function(row) {
  method <- if (is.na(row[[3L]])) paste(row[[1L]], row[[2L]], sep = ".") else row[[3L]]
  paste("namespace", row[[1L]], row[[2L]], method,
        if (is.na(row[[4L]])) "-" else row[[4L]], sep = "\t")
})
owner <- new.env(parent = globalenv())
sys.source(file.path(fixture, "input.R"), owner)
owner$.onLoad(NULL, NULL)
table <- owner$.__S3MethodsTable__.
for (name in sort(ls(table))) {
  fun <- get(name, table)
  binding <- if (identical(fun, owner$handler)) "handler" else "g.implicit"
  rows <- c(rows, paste("script", "g", substring(name, 3L), binding, "-", sep = "\t"))
}
cat(paste(rows, collapse = "\n"), "\n", sep = "")
unlink(lib, recursive = TRUE)
