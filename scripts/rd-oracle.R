#!/usr/bin/env Rscript
# Dedicated source-parser oracle. Uses base R and tools, without roxygen2.
# Invocation: Rscript --vanilla scripts/rd-oracle.R INPUT.Rd
# Refresh:    Rscript --vanilla scripts/rd-oracle.R --refresh FIXTURE_ROOT

args <- commandArgs(trailingOnly = TRUE)

json_string <- function(value) {
  bytes <- as.integer(charToRaw(enc2utf8(value)))
  escaped <- vapply(bytes, function(byte) {
    if (byte == 34L) return('\\"')
    if (byte == 92L) return("\\\\")
    if (byte < 32L) return(sprintf("\\u%04x", byte))
    rawToChar(as.raw(byte))
  }, "")
  paste0('"', paste0(escaped, collapse = ""), '"')
}

array <- function(values) paste0("[", paste(values, collapse = ","), "]")

# Leaf segmentation is not syntax. Coalesce adjacent leaves of the same kind,
# preserving their complete contents, and retain every comment separately.
children <- function(nodes) {
  out <- character()
  buffered <- ""
  buffered_tag <- NULL
  flush <- function() {
    if (!is.null(buffered_tag)) {
      out <<- c(out, array(c(json_string(buffered_tag), json_string(buffered))))
    }
    buffered <<- ""
    buffered_tag <<- NULL
  }
  for (node in nodes) {
    tag <- attr(node, "Rd_tag")
    if (!is.list(node) && tag %in% c("TEXT", "RCODE", "VERB")) {
      if (!identical(tag, buffered_tag)) flush()
      buffered_tag <- tag
      buffered <- paste0(buffered, paste0(node, collapse = ""))
    } else {
      flush()
      out <- c(out, serialize_node(node))
    }
  }
  flush()
  array(out)
}

serialize_node <- function(node) {
  tag <- attr(node, "Rd_tag")
  if (is.list(node)) {
    if (is.null(tag) || identical(tag, "TEXT")) tag <- "ARG"
    option <- attr(node, "Rd_option")
    option <- if (is.null(option)) "null" else json_string(paste0(option, collapse = ""))
    return(array(c(json_string(tag), option, children(node))))
  }
  array(c(json_string(tag), json_string(paste0(node, collapse = ""))))
}

parse_file <- function(path) {
  # Any warning fails pin generation. Recovery trees are not an oracle for a
  # lossless parser because R may discard malformed source during recovery.
  parsed <- withCallingHandlers(
    tools::parse_Rd(path, encoding = "UTF-8", macros = FALSE,
                    fragment = FALSE, permissive = FALSE, warningCalls = FALSE),
    warning = function(w) stop(conditionMessage(w), call. = FALSE)
  )
  paste0('{"r_version":', json_string(R.version.string),
         ',"invocation":', json_string('tools::parse_Rd(encoding="UTF-8", macros=FALSE, fragment=FALSE, permissive=FALSE)'), ',',
         '"tree":', children(parsed), '}')
}

system_shapes <- function() {
  macros <- tools::loadRdMacros(file.path(R.home("share"), "Rd", "macros", "system.Rd"),
                               macros = TRUE)
  out <- character()
  for (name in sort(ls(macros, all.names = TRUE))) {
    definition <- attr(get(name, envir = macros, inherits = FALSE), "definition")
    if (is.null(definition)) next
    placeholders <- regmatches(definition, gregexpr("#[0-9]", definition))[[1L]]
    arity <- max(c(0L, as.integer(sub("#", "", placeholders, fixed = TRUE))))
    out <- c(out, array(c(json_string(substring(name, 2L)), as.character(arity))))
  }
  paste0('{"r_version":', json_string(R.version.string), ',"shapes":', array(out), '}')
}

if (length(args) == 2L && identical(args[[1L]], "--refresh")) {
  paths <- sort(list.files(args[[2L]], pattern = "^input\\.Rd$", recursive = TRUE,
                           full.names = TRUE))
  stopifnot(length(paths) > 0L)
  # Parse the entire corpus first, so an oracle failure cannot leave partial pins.
  pins <- lapply(paths, parse_file)
  shapes <- system_shapes()
  for (i in seq_along(paths)) {
    destination <- file.path(dirname(paths[[i]]), "expected.json")
    writeLines(pins[[i]], destination, useBytes = TRUE)
    cat("minted ", destination, "\n", sep = "")
  }
  writeLines(shapes, file.path(args[[2L]], "system-macros.json"), useBytes = TRUE)
} else if (length(args) == 1L && identical(args[[1L]], "--system-macros")) {
  cat(system_shapes(), "\n", sep = "")
} else if (length(args) == 1L) {
  cat(parse_file(args[[1L]]), "\n", sep = "")
} else {
  stop("expected INPUT.Rd or --refresh FIXTURE_ROOT", call. = FALSE)
}
