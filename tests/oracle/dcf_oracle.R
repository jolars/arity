#!/usr/bin/env Rscript
#
# `read.dcf` oracle driver for arity's DCF harness (tests/dcf_oracle.rs).
# NOT part of the Cargo build.
#
# R's `read.dcf` is the definition of what a DESCRIPTION means. This script
# exposes it as a differential oracle so arity's DCF parser is checked against
# R's behavior rather than against comments claiming what R does.
#
# Reads DCF text from stdin, writes a line-oriented report to stdout. Anything
# on stderr is diagnostic noise; a non-zero exit means "could not process" and
# the Rust harness records the case as skipped, never a hard failure.
#
# Output grammar (one record per `RECORD`, fields in R's column order):
#
#   ERROR<TAB><escaped message>      read.dcf refused the input entirely
#   RECORD                           starts a record
#   F<TAB><name><TAB><escaped value> one field of the current record
#
# With the `authors` argument, reads an `Authors@R` field *value* instead and
# reports what R itself derives from it:
#
#   AAR-ERROR<TAB><escaped message>       the field did not read
#   AAR-AUTHOR<TAB><escaped string>       the `Author:` field `R CMD build` writes
#   AAR-MAINTAINER<TAB><escaped string>   the `Maintainer:` field it writes
#   AAR-DEPARSE<TAB><escaped string>      the deparsed person vector
#
# The first two are the exact bytes that land in a built tarball, which is the
# sharpest available statement of "this still describes the same people". The
# deparse is what catches a dropped `comment = c(ORCID = ...)`, which `format()`
# would happily hide.
#
# Values are escaped (`\\`, `\n`, `\t`, `\r`) because a folded DCF value
# routinely contains newlines and the report is line-oriented.

# arity-ignore-file internal-function: reaching into `utils:::` is the point —
# these readers and formatters are unexported, and they are exactly what
# `R CMD build` calls, so pinning arity against them is what makes an R upgrade
# that changes them visible instead of silent.

escape <- function(x) {
  x <- gsub("\\", "\\\\", x, fixed = TRUE)
  x <- gsub("\n", "\\n", x, fixed = TRUE)
  x <- gsub("\t", "\\t", x, fixed = TRUE)
  x <- gsub("\r", "\\r", x, fixed = TRUE)
  x
}

read_stdin <- function() {
  paste(readLines(file("stdin"), warn = FALSE), collapse = "\n")
}

# `Authors@R` mode. Uses R's own reader rather than `eval(parse(...))`: this is
# what `tools:::.check_package_description_authors_at_R_field` and `R CMD build`
# call, so agreement here is agreement with the thing that actually matters.
if (identical(commandArgs(trailingOnly = TRUE)[1], "authors")) {
  value <- read_stdin()
  aar <- tryCatch(
    utils:::.read_authors_at_R_field(value, TRUE),
    error = function(e) structure(list(msg = conditionMessage(e)), class = "aar_error"),
    warning = function(w) structure(list(msg = conditionMessage(w)), class = "aar_error")
  )
  if (inherits(aar, "aar_error")) {
    cat("AAR-ERROR\t", escape(aar$msg), "\n", sep = "")
    quit(status = 0)
  }
  cat("AAR-AUTHOR\t", escape(utils:::.format_authors_at_R_field_for_author(aar)), "\n", sep = "")
  cat(
    "AAR-MAINTAINER\t",
    escape(utils:::.format_authors_at_R_field_for_maintainer(aar)),
    "\n",
    sep = ""
  )
  cat(
    "AAR-DEPARSE\t",
    escape(paste(deparse(unclass(aar)), collapse = "\n")),
    "\n",
    sep = ""
  )
  quit(status = 0)
}

text <- paste(readLines(file("stdin"), warn = FALSE), collapse = "\n")
# `readLines` drops a trailing newline; DCF's last field does not care, and
# re-adding one keeps a final field from looking truncated to `read.dcf`.
if (nzchar(text)) {
  text <- paste0(text, "\n")
}

parsed <- tryCatch(
  read.dcf(textConnection(text)),
  error = function(e) structure(list(msg = conditionMessage(e)), class = "dcf_error")
)

if (inherits(parsed, "dcf_error")) {
  cat("ERROR\t", escape(parsed$msg), "\n", sep = "")
  quit(status = 0)
}

if (is.null(nrow(parsed)) || nrow(parsed) == 0L) {
  quit(status = 0)
}

names <- colnames(parsed)
for (i in seq_len(nrow(parsed))) {
  cat("RECORD\n")
  for (j in seq_along(names)) {
    value <- parsed[i, j]
    # A field absent from *this* record is NA: the matrix is the union of every
    # record's fields, so a multi-record file has holes.
    if (!is.na(value)) {
      # The *name* needs escaping too: `read.dcf` does not trim it, so
      # `Version\t: 1` yields a name containing a literal tab, which would
      # otherwise split this line in the wrong place.
      cat("F\t", escape(names[j]), "\t", escape(value), "\n", sep = "")
    }
  }
}
