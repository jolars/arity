#!/usr/bin/env Rscript
#
# `R CMD check` DESCRIPTION oracle driver for tests/description_oracle.rs.
# NOT part of the Cargo build.
#
# R's own checkers are the definition of what `R CMD check` will say about a
# DESCRIPTION, so arity's Packaging rules are checked against them rather than
# against a reading of `Writing R Extensions` (which paraphrases the code, and
# is looser than it). Four checkers are exposed:
#
#   tools:::.check_package_description(strict = TRUE)
#       what `R CMD check` enforces, including the strict-only Title and
#       Description clauses.
#   tools:::.check_package_description_authors_at_R_field(strict = 2L)
#       called again directly, because the outer checker invokes it at
#       strict = FALSE and so never reaches the per-person name, role, ORCID,
#       and ROR signals.
#   tools:::.check_package_description2()
#       only its `duplicates` component: a package named in more than one
#       dependency field. Its other components need installed packages and a
#       `src/` directory, which a text-only oracle has no business simulating.
#   tools:::.check_package_CRAN_incoming(localOnly = TRUE)
#       only its two version components, on the same grounds: the CRAN pretest
#       is mostly about files, URLs, and network state, but the version NOTEs
#       are functions of the DESCRIPTION text and nothing else covers them.
#
# Reads DESCRIPTION text from stdin, writes a line-oriented report to stdout.
# stderr is diagnostic noise; a non-zero exit means "could not process" and the
# Rust harness records the case as skipped, never a hard failure.
#
# Output grammar:
#
#   ERROR<TAB><escaped message>                could not read the input at all
#   SIGNAL<TAB><name><TAB><escaped detail>     one signal R raised
#
# A signal with no natural detail (a bare TRUE flag such as `missing_encoding`)
# emits an empty detail. A signal carrying several offenders emits one line
# each, so the harness can compare per-entry rather than per-file. Details are
# escaped (`\\`, `\n`, `\t`, `\r`) because the report is line-oriented and a
# DESCRIPTION value routinely contains newlines.

# arity-ignore-file internal-function: reaching into `tools:::` is the point —
# these checkers are unexported, and pinning arity against them is precisely
# what makes an R upgrade that changes them visible instead of silent.

escape <- function(x) {
  x <- gsub("\\", "\\\\", x, fixed = TRUE)
  x <- gsub("\n", "\\n", x, fixed = TRUE)
  x <- gsub("\t", "\\t", x, fixed = TRUE)
  x <- gsub("\r", "\\r", x, fixed = TRUE)
  x
}

# Signals accumulate rather than print, because the outer checker already
# merges the `Authors@R` results and the strict re-run below raises them again.
# The report is a *set* of signals, so the harness never has to care which
# checker happened to surface one.
seen <- character()

emit <- function(name, detail = "") {
  if (!length(detail)) {
    return(invisible(NULL))
  }
  for (d in detail) {
    if (is.na(d)) {
      d <- ""
    }
    line <- paste0("SIGNAL\t", name, "\t", escape(as.character(d)))
    if (!(line %in% seen)) {
      seen <<- c(seen, line)
    }
  }
  invisible(NULL)
}

# A logical TRUE flag has no offender to name; anything else is reported by
# value, which is what makes a per-entry comparison possible downstream.
emit_component <- function(name, value) {
  if (is.null(value)) {
    return(invisible(NULL))
  }
  if (is.logical(value)) {
    if (isTRUE(any(value))) {
      emit(name, "")
    }
    return(invisible(NULL))
  }
  emit(name, as.character(value))
}

text <- paste(readLines(file("stdin"), warn = FALSE), collapse = "\n")
if (nzchar(text)) {
  text <- paste0(text, "\n")
}

# The checkers take a path, and `.check_package_description2` additionally looks
# for a sibling `src/`, so the case needs to live in a directory of its own.
dir <- tempfile("arity-desc-oracle-")
dir.create(dir)
on.exit(unlink(dir, recursive = TRUE), add = TRUE)
dfile <- file.path(dir, "DESCRIPTION")
writeLines(text, dfile, useBytes = TRUE)

db <- tryCatch(
  tools:::.read_description(dfile),
  error = function(e) {
    structure(list(msg = conditionMessage(e)), class = "read_error")
  }
)
if (inherits(db, "read_error")) {
  cat("ERROR\t", escape(db$msg), "\n", sep = "")
  quit(status = 0)
}

out <- tryCatch(
  tools:::.check_package_description(dfile, strict = TRUE),
  error = function(e) {
    structure(list(msg = conditionMessage(e)), class = "check_error")
  }
)
if (inherits(out, "check_error")) {
  cat("ERROR\t", escape(out$msg), "\n", sep = "")
  quit(status = 0)
}

for (name in names(out)) {
  value <- out[[name]]
  # The dependency-field result is a nested list of three offender vectors.
  # Flattening it here is what lets the harness compare entry text directly.
  if (name == "bad_depends_or_suggests_or_imports" && is.list(value)) {
    for (sub in names(value)) {
      emit(sub, value[[sub]])
    }
  } else {
    emit_component(name, value)
  }
}

# The outer checker calls this at strict = FALSE, so its per-person signals are
# unreachable from there. Called again at strict = 2L for the full set.
aar <- db["Authors@R"]
if (!is.na(aar)) {
  strict_aar <- tryCatch(
    tools:::.check_package_description_authors_at_R_field(aar, strict = 2L),
    error = function(e) NULL
  )
  for (name in names(strict_aar)) {
    emit_component(name, strict_aar[[name]])
  }
}

out2 <- tryCatch(
  tools:::.check_package_description2(dfile),
  error = function(e) NULL
)
if (!is.null(out2)) {
  emit("duplicates", out2$duplicates)
}

# The CRAN pretest, for the two clauses that are functions of the DESCRIPTION
# text alone. Cherry-picked exactly as `.check_package_description2` is above,
# and for the same reason: of this checker's ~85 components, nearly all are
# about files, URLs, network state, or installed packages, and a text-only
# oracle has no business simulating those. What is left is the version half,
# which `.check_package_description` does not cover at all.
#
# Note it errors outright on a DESCRIPTION with no `Maintainer` — it compares
# one against "ORPHANED" before reaching anything else, and NA is not a
# condition — so a case that wants these signals has to carry one.
cran <- tryCatch(
  tools:::.check_package_CRAN_incoming(dir, localOnly = TRUE),
  error = function(e) NULL
)
if (!is.null(cran)) {
  emit("version_with_leading_zeroes", cran$version_with_leading_zeroes)
  emit("version_with_large_components", cran$version_with_large_components)
}

cat(seen, sep = "\n")
if (length(seen)) {
  cat("\n")
}
