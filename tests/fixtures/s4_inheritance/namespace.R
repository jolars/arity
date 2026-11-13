args <- commandArgs(trailingOnly = TRUE)
root <- if (length(args)) args[[1L]] else "."
local({
    library <- tempfile("arity-s4-namespace-")
    dir.create(library)
    on.exit(unlink(library, recursive = TRUE), add = TRUE)
    Sys.setenv(R_LIBS_USER = library)
    .libPaths(c(library, .libPaths()))
    for (package in c("arityS4Owner", "arityS4Provider", "arityS4Client", "arityS4ClassClient")) {
        output <- system2(file.path(R.home("bin"), "R"),
                          c("CMD", "INSTALL", "--no-byte-compile", "--no-test-load",
                            paste0("--library=", shQuote(library)),
                            shQuote(file.path(root, "packages", package))),
                          stdout = TRUE, stderr = TRUE)
        if (!is.null(attr(output, "status"))) stop(paste(output, collapse = "\n"))
    }
    emit <- function(...) cat(paste(..., sep = "\t"), "\n", sep = "")
    # Load the class-only client first: the provider's g methods are available
    # even though that client imports only Marker metadata from the provider.
    method <- arityS4ClassClient::probe()
    emit("loaded_method", methods::getPackageName(environment(method)), as.character(method@defined))
    method <- arityS4Client::probe()
    generic <- get("g", asNamespace("arityS4Client"))
    emit("generic_owner", generic@package)
    emit("method_owner", methods::getPackageName(environment(method)))
    emit("inherited", as.character(method@defined))
    emit("primitive_owner", methods::getGeneric("length")@package)
    emit("primitive_method", as.character(arityS4Client::probeLength()@defined))
    imports <- parent.env(asNamespace("arityS4Client"))
    emit("class_import", exists(methods::classMetaName("Parent"), imports, inherits = FALSE))
    emit("class_value_import", exists("Parent", imports, inherits = FALSE))
    emit("class_value_export", "Parent" %in% getNamespaceExports("arityS4Owner"))
    emit("generic_import", exists("g", imports, inherits = FALSE))
    emit("primitive_import", exists("length", imports, inherits = FALSE))
})
