#!/usr/bin/env Rscript
#
# Build a roxygen `@md` corpus from the vendored CommonMark spec test set.
#
# arity's markdown layer targets full CommonMark parity, but the **oracle is
# roxygen2, not the spec** (roxygen2 parses via cmark/cmark-gfm, then translates
# a subset to Rd and validates). So we take only the spec's markdown *inputs*:
# each example in the named section is wrapped into a single self-contained `@md`
# roxygen block and emitted as a `{slug, input}` JSONL record. The expected Rd is
# minted separately from roxygen2 by the `projector-pins` op --- the spec's
# `expected_html` is ignored entirely.
#
#   Rscript scripts/build-commonmark-corpus.R <spec.txt> <section-substr> <out.jsonl>
#
# `section-substr` matches an ATX heading (e.g. "Emphasis and strong emphasis").
# Slug = `cm-<NNN>`, the spec's canonical *global* example number (stable across
# sections, so later slices --- links, code spans --- extend the same keyspace).

suppressWarnings(suppressMessages(library(jsonlite)))

args <- commandArgs(trailingOnly = TRUE)
if (length(args) < 3) {
  stop("usage: build-commonmark-corpus.R <spec.txt> <section-substr> <out.jsonl>")
}
spec_path <- args[[1]]
section_match <- args[[2]]
out_path <- args[[3]]

lines <- readLines(spec_path, warn = FALSE)
fence_re <- "^`{32,} example"     # an example opener (32+ backticks, ` example`)
close_re <- "^`{32,}$"            # the matching closer

records <- list()
example_no <- 0L                  # global counter over *every* example fence
in_section <- FALSE
i <- 1L
n <- length(lines)
while (i <= n) {
  line <- lines[[i]]
  # ATX heading: a section boundary. Examples are numbered globally regardless.
  if (grepl("^#+ ", line)) {
    in_section <- grepl(section_match, line, fixed = TRUE)
    i <- i + 1L
    next
  }
  if (grepl(fence_re, line)) {
    example_no <- example_no + 1L
    # Collect the markdown half (up to the lone `.` separator).
    md <- character(0)
    i <- i + 1L
    while (i <= n && lines[[i]] != ".") {
      md <- c(md, lines[[i]])
      i <- i + 1L
    }
    # Skip the `.`, then the expected-HTML half, then the closing fence.
    i <- i + 1L
    while (i <= n && !grepl(close_re, lines[[i]])) i <- i + 1L
    i <- i + 1L
    if (in_section) {
      # The spec writes literal tabs as U+2192 RIGHTWARDS ARROW.
      md <- gsub("→", "\t", md)
      # Wrap into an `@md` block: markdown is the `\details` body so emphasis
      # rendering is isolated from intro title/description splitting. A blank
      # markdown line is a bare `#'` marker (a paragraph break).
      body <- ifelse(nzchar(md), paste0("#' ", md), "#'")
      input <- paste0(
        paste(c("#' @md", "#' @title T", "#' @details", body, "#' @name spec",
                "NULL"), collapse = "\n"),
        "\n"
      )
      slug <- sprintf("cm-%03d", example_no)
      records[[length(records) + 1L]] <- list(slug = slug, input = input)
    }
    next
  }
  i <- i + 1L
}

con <- file(out_path, "w")
on.exit(close(con))
for (r in records) writeLines(toJSON(r, auto_unbox = TRUE), con)
cat(sprintf(
  "wrote %d example(s) from section '%s' -> %s\n",
  length(records), section_match, out_path
), file = stderr())
