owner <- environment()
class_facts <- function(binding, parents) {
  class <- get(binding, envir = owner)
  list(name = class@name, binding = binding, parents = as.list(parents),
       members = as.list(setNames(rep("property", length(class@properties)), names(class@properties))))
}
constructors <- c("Parent", "Child", "Custom", "Sub")
facts <- list(
  system = "s7_advanced",
  classes = list(class_facts("Parent", character()), class_facts("Child", "Parent"),
                 class_facts("Custom", character()), class_facts("Sub", "Custom")),
  constructors = lapply(constructors, function(binding) {
    list(binding = binding, formals = formal_facts(get(binding, envir = owner)))
  }),
  methods = list(method_facts("choose", c("Child", "ANY"), S7::method(choose, list(Child, S7::class_any)), owner)),
  dispatch = list(
    dispatch_facts("choose(Child(), 1L)", child, owner),
    dispatch_facts("choose(S7::super(Child(), Parent), 1L)", child, owner),
    dispatch_facts("choose(1L, 2L)", child, owner),
    dispatch_facts("choose(second = 2L)", child, owner)
  ),
  s7_observations = list(
    property_getter = as.character(S7::prop(child, "readonly")),
    property_replacement = S7::prop(child, "value"),
    s3_bridge = as.character(length(child)),
    s4_bridge = probeS4(Parent())
  )
)

# Real package namespaces exercise the load hooks and their delayed registrations.
development <- packageVersion("S7") >= "0.2.2.9000"
library_path <- tempfile("s7-packages-")
dir.create(library_path)
local({
  old_libraries <- Sys.getenv("R_LIBS", unset = NA_character_)
  on.exit({
    if (is.na(old_libraries)) Sys.unsetenv("R_LIBS") else Sys.setenv(R_LIBS = old_libraries)
    unlink(library_path, recursive = TRUE)
  })
  Sys.setenv(R_LIBS = paste(c(library_path, .libPaths()), collapse = .Platform$path.sep))
  package <- function(name, namespace, code, suggests = NULL) {
    path <- file.path(tempdir(), name)
    dir.create(file.path(path, "R"), recursive = TRUE)
    writeLines(c(
      paste0("Package: ", name), "Version: 1.0.0", "Title: S7 oracle namespace",
      'Author: Arity', 'Maintainer: Arity <arity@example.com>', 'Description: S7 oracle namespace.',
      "License: MIT", "Imports: S7", if (!is.null(suggests)) paste("Suggests:", suggests)
    ), file.path(path, "DESCRIPTION"))
    writeLines(namespace, file.path(path, "NAMESPACE"))
    writeLines(code, file.path(path, "R", "code.R"))
    output <- system2("R", c("CMD", "INSTALL", "--no-multiarch",
                            shQuote(paste0("--library=", library_path)), shQuote(path)),
                      stdout = TRUE, stderr = TRUE)
    if (!is.null(attr(output, "status"))) stop(paste(output, collapse = "\n"))
  }
  package("arityS7Provider", c("export(probe)", "export(Foreign)"), c(
    'probe <- S7::new_generic("probe", "x")',
    'Foreign <- S7::new_class("Foreign")'
  ))
  package("arityS7Extension", c("export(Extension)", if (development) "export(local_probe)"), c(
    'Extension <- S7::new_class("Extension")',
    'external_probe <- S7::new_external_generic("arityS7Provider", "probe", "x", "1.0.0")',
    'S7::method(external_probe, Extension) <- function(x) "extension"',
    if (development) c(
      'local_probe <- S7::new_generic("local_probe", "x")',
      'Foreign <- S7::new_external_class("arityS7Provider", "Foreign")',
      'S7::method(local_probe, Foreign) <- function(x) "external_class"',
      '.onLoad <- function(...) S7::S7_on_load()',
      '.onUnload <- function(...) S7::S7_on_unload()',
      'S7::S7_on_build()'
    ) else '.onLoad <- function(...) S7::methods_register()'
  ), suggests = "arityS7Provider")
  for (first in c("arityS7Provider", "arityS7Extension")) {
    second <- setdiff(c("arityS7Provider", "arityS7Extension"), first)
    code <- c(
      sprintf('loadNamespace("%s")', first),
      sprintf('loadNamespace("%s")', second),
      'cat("result=", arityS7Provider::probe(arityS7Extension::Extension()), "\n", sep = "")',
      if (development) c(
        'cat("result=", arityS7Extension::local_probe(arityS7Provider::Foreign()), "\n", sep = "")',
        'object <- arityS7Extension::Extension()',
        'stopifnot(!exists("external_probe", asNamespace("arityS7Extension"), inherits = FALSE))',
        'unloadNamespace("arityS7Extension")',
        'cat("result=", inherits(try(arityS7Provider::probe(object), silent = TRUE), "try-error"), "\n", sep = "")'
      )
    )
    script <- tempfile(fileext = ".R")
    writeLines(code, script)
    output <- system2("Rscript", c("--vanilla", shQuote(script)), stdout = TRUE, stderr = TRUE)
    if (!is.null(attr(output, "status"))) stop(paste(output, collapse = "\n"))
    result <- sub("^result=", "", grep("^result=", output, value = TRUE))
    stopifnot(length(result) == if (development) 3L else 1L)
    facts$s7_observations[[paste0("external_first_", first)]] <<- paste(result, collapse = "/")
  }
})
