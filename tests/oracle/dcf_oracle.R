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
# Values are escaped (`\\`, `\n`, `\t`, `\r`) because a folded DCF value
# routinely contains newlines and the report is line-oriented.

escape <- function(x) {
  x <- gsub("\\", "\\\\", x, fixed = TRUE)
  x <- gsub("\n", "\\n", x, fixed = TRUE)
  x <- gsub("\t", "\\t", x, fixed = TRUE)
  x <- gsub("\r", "\\r", x, fixed = TRUE)
  x
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
