#!/usr/bin/env Rscript
#
# `desc` style-reference driver for arity's DESCRIPTION formatting gauge
# (tests/desc_compat.rs). NOT part of the Cargo build.
#
# `desc::desc_normalize()` is where arity's canonical field order and four-space
# continuation indent come from, so measuring how often `desc` would leave
# arity's output alone is a free differential signal on the *rules*.
#
# It is a style reference, never an oracle: `desc` silently drops every comment,
# and arity must not. The Rust harness normalizes that and the other recorded
# deviations before comparing, and never fails a build on the result.
#
# Reads DCF text from stdin, writes `desc`'s normalized rendering to stdout. A
# non-zero exit means "could not process"; the harness skips the case.

if (!requireNamespace("desc", quietly = TRUE)) {
  quit(status = 1)
}

text <- readLines(file("stdin"), warn = FALSE)

out <- tryCatch(
  {
    d <- desc::desc(text = text)
    d$normalize()
    d$str(normalize = FALSE)
  },
  error = function(e) NULL,
  warning = function(w) NULL
)

if (is.null(out)) {
  quit(status = 1)
}

cat(out, "\n", sep = "")
