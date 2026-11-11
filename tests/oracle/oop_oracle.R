#!/usr/bin/env Rscript
# This driver is test-only. Each invocation gets a fresh R process and namespace.
args <- commandArgs(trailingOnly = TRUE)
if (length(args) != 5L) {
  stop("usage: oop_oracle.R FIXTURE SOURCE R_VERSION R6_VERSION S7_VERSION")
}

actual <- list(
  r = as.character(getRversion()),
  r6 = as.character(packageVersion("R6")),
  s7 = as.character(packageVersion("S7"))
)
if (!identical(unname(unlist(actual)), args[3:5])) {
  stop("OOP oracle requires R/R6/S7 ", paste(args[3:5], collapse = "/"),
       "; found ", paste(actual, collapse = "/"))
}

# Use base R to avoid making the observation format depend on a JSON package.
json <- function(x) {
  if (is.null(x)) return("null")
  if (is.list(x)) {
    if (is.null(names(x))) {
      return(paste0("[", paste(vapply(x, json, ""), collapse = ","), "]"))
    }
    entries <- vapply(seq_along(x), function(i) {
      paste0(json(names(x)[[i]]), ":", json(x[[i]]))
    }, "")
    return(paste0("{", paste(entries, collapse = ","), "}"))
  }
  stopifnot(length(x) == 1L, !is.na(x))
  if (is.logical(x)) return(if (x) "true" else "false")
  stopifnot(is.character(x))
  encoded <- vapply(as.integer(charToRaw(enc2utf8(x))), function(byte) {
    if (byte == 34L) return('\\"')
    if (byte == 92L) return("\\\\")
    if (byte < 32L) return(sprintf("\\u%04x", byte))
    rawToChar(as.raw(byte))
  }, "")
  paste0('"', paste0(encoded, collapse = ""), '"')
}

formal_facts <- function(fun) {
  values <- formals(fun)
  lapply(seq_along(values), function(i) {
    list(name = names(values)[[i]],
         default = if (identical(values[[i]], quote(expr = ))) {
           NULL
         } else {
           paste(deparse(values[[i]], width.cutoff = 500L), collapse = " ")
         })
  })
}

method_facts <- function(generic, signature, fun, owner) {
  enclosure <- environment(fun)
  frames <- list()
  # Addresses and generated package timestamps are not stable identities.
  # Record each frame's bindings and its distance to the lexical owner instead.
  while (!identical(enclosure, owner)) {
    if (identical(enclosure, emptyenv())) stop("method does not enclose the fixture")
    frames[[length(frames) + 1L]] <- as.list(sort(ls(enclosure, all.names = TRUE)))
    enclosure <- parent.env(enclosure)
  }
  list(
    generic = generic,
    signature = as.list(signature),
    formals = formal_facts(fun),
    environment = frames
  )
}

dispatch_facts <- function(expression, object, owner) {
  list(expression = expression, classes = as.list(class(object)),
       value = as.character(eval(parse(text = expression), owner)))
}

scope <- new.env(parent = globalenv())
scope$.packageName <- "arityOopOracle"
# Runtime printing and setGeneric's return value must not corrupt JSON output.
invisible(capture.output(sys.source(args[[2L]], envir = scope)))
invisible(capture.output(sys.source(file.path(args[[1L]], "observe.R"), envir = scope)))
cat(json(list(versions = actual, facts = scope$facts)), "\n", sep = "")
