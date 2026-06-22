#!/usr/bin/env Rscript
#
# Harvest a corpus of standalone roxygen blocks from roxygen2's own test suite.
#
# roxygen2's tests embed complete `#'` blocks as the source argument of
# `roc_proc_text(rd_roclet(), "<src>")`. Each such block is a self-contained,
# roxygen2-processable unit --- exactly what arity's roxygen oracle consumes. This
# script walks every test file's AST (via R's own parser), pulls out those source
# strings, dedents them, keeps only the ones roxygen2 can render, dedups, and
# emits one `{"slug","input"}` record per line to a JSONL corpus.
#
# The slug is a content hash, so it is stable across re-harvests: the allowlist
# (`tests/oracle/roxygen-allowlist.txt`) keys on it and survives a corpus refresh
# as long as a case's text is unchanged.
#
#   Rscript scripts/harvest-roxygen-corpus.R [<roxygen2-source-dir>] [<out.jsonl>]
#
# Defaults: source `roxygen2-ref/`, output `tests/oracle/corpus/roxygen.jsonl`.
# Pin the harvested-from version in `tests/oracle/.roxygen2-source`.

suppressWarnings(suppressMessages({
  library(roxygen2)
  library(jsonlite)
  library(digest)
}))

args <- commandArgs(trailingOnly = TRUE)
src_dir <- if (length(args) >= 1) args[[1]] else "roxygen2-ref"
out_path <- if (length(args) >= 2) args[[2]] else "tests/oracle/corpus/roxygen.jsonl"

test_dir <- file.path(src_dir, "tests", "testthat")
if (!dir.exists(test_dir)) {
  stop(sprintf("test dir not found: %s (clone roxygen2 to %s)", test_dir, src_dir))
}

# --- extract roc_proc_text source strings via the R parser ------------------

raw_blocks <- character(0)

walk <- function(node) {
  if (!is.call(node)) return(invisible())
  fn <- node[[1]]
  if (is.symbol(fn) && as.character(fn) == "roc_proc_text") {
    for (a in as.list(node)[-1]) {
      is_str <- tryCatch(is.character(a) && length(a) == 1, error = function(e) FALSE)
      if (isTRUE(is_str)) {
        raw_blocks[[length(raw_blocks) + 1]] <<- a
        break
      }
    }
  }
  # Recurse into call children, guarding R's empty-symbol arg placeholder.
  for (i in seq_along(node)) {
    is_c <- tryCatch({ el <- node[[i]]; is.call(el) }, error = function(e) FALSE)
    if (isTRUE(is_c)) walk(node[[i]])
  }
}

files <- sort(list.files(test_dir, pattern = "\\.[Rr]$", full.names = TRUE))
for (f in files) {
  exprs <- tryCatch(parse(f, keep.source = FALSE), error = function(e) NULL)
  if (is.null(exprs)) next
  for (e in exprs) walk(e)
}

# --- dedent ----------------------------------------------------------------

dedent <- function(src) {
  lines <- strsplit(src, "\n", fixed = TRUE)[[1]]
  # Drop leading and trailing blank lines.
  nonblank <- which(nzchar(trimws(lines)))
  if (length(nonblank) == 0) return("")
  lines <- lines[seq(min(nonblank), max(nonblank))]
  indents <- vapply(lines, function(l) {
    if (!nzchar(trimws(l))) return(NA_integer_)
    nchar(sub("^([ \t]*).*$", "\\1", l))
  }, integer(1))
  common <- min(indents, na.rm = TRUE)
  if (common > 0) lines <- substring(lines, common + 1)
  paste0(paste(lines, collapse = "\n"), "\n")
}

# --- filter to renderable + dedup ------------------------------------------

renderable <- function(src) {
  # Renderable == the oracle driver can process it: roxygen2 returns >= 1 topic
  # without *error*. Warnings/messages (unresolved cross-package links, etc.) are
  # benign and deterministic --- both sides of the fixed point see them alike ---
  # so they stay in the corpus.
  ok <- tryCatch(
    suppressWarnings(suppressMessages(length(roc_proc_text(rd_roclet(), src)) >= 1)),
    error = function(e) FALSE
  )
  isTRUE(ok)
}

seen <- new.env(parent = emptyenv())
records <- list()
n_render_fail <- 0L
for (b in raw_blocks) {
  src <- dedent(b)
  if (!nzchar(trimws(src))) next
  if (exists(src, envir = seen, inherits = FALSE)) next
  assign(src, TRUE, envir = seen)
  if (!renderable(src)) {
    n_render_fail <- n_render_fail + 1L
    next
  }
  slug <- paste0("rx-", substr(digest(src, algo = "sha1"), 1, 8))
  records[[length(records) + 1]] <- list(slug = slug, input = src)
}

# Stable order by slug.
records <- records[order(vapply(records, function(r) r$slug, character(1)))]

con <- file(out_path, "w")
on.exit(close(con))
for (r in records) {
  writeLines(toJSON(r, auto_unbox = TRUE), con)
}

cat(sprintf(
  "harvested %d raw -> %d unique renderable blocks (%d unrenderable dropped) -> %s\n",
  length(raw_blocks), length(records), n_render_fail, out_path
), file = stderr())
