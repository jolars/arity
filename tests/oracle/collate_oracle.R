#!/usr/bin/env Rscript
# Pin R's installation checks and source selection for both target platforms.
# R runs only in this oracle, never in production analysis.
# arity-lint skip-file internal-function: these are the helpers under test.

args <- commandArgs(trailingOnly = TRUE)
stopifnot(length(args) == 0L || identical(args, "--refresh"))
path <- "tests/oracle/description-collate.json"
corpus <- jsonlite::fromJSON(path, simplifyVector = FALSE)
Sys.setenv(LANGUAGE = "en")
invisible(Sys.setlocale("LC_COLLATE", "C"))

check_case <- function(case, os) {
  root <- tempfile("arity-collate-")
  dir.create(file.path(root, "R"), recursive = TRUE)
  on.exit(unlink(root, recursive = TRUE))
  writeLines(
    paste0("Package: testpkg\nVersion: 0.1.0\n", case$description),
    file.path(root, "DESCRIPTION")
  )
  for (name in case$files) {
    file <- file.path(root, "R", name)
    dir.create(dirname(file), recursive = TRUE, showWarnings = FALSE)
    writeLines("NULL", file)
  }
  Sys.setenv(R_OSTYPE = os)
  db <- tools:::.read_description(file.path(root, "DESCRIPTION"))
  fields <- c(paste0("Collate.", os), "Collate")
  field <- fields[fields %in% names(db)][1L]
  entries <- if (is.na(field)) {
    character()
  } else {
    suppressWarnings(tools:::.read_collate_field(db[field]))
  }
  sources <- tools::list_files_with_type(
    file.path(root, "R"),
    "code",
    full.names = FALSE
  )
  failure <- tryCatch(
    {
      suppressWarnings(tools:::.install_package_code_files(
        root,
        file.path(root, "out")
      ))
      "ok"
    },
    error = function(error) {
      message <- conditionMessage(error)
      if (grepl("duplicated files in", message, fixed = TRUE)) {
        "duplicate"
      } else if (grepl("field missing from", message, fixed = TRUE)) {
        "ineligible"
      } else if (grepl("missing from", message, fixed = TRUE)) {
        "unlisted"
      } else {
        stop(message)
      }
    }
  )
  list(
    field = if (is.na(field)) "" else field,
    entries = as.list(entries),
    sources = as.list(sources),
    failure = failure
  )
}

for (i in seq_along(corpus$cases)) {
  for (os in c("unix", "windows")) {
    expected <- check_case(corpus$cases[[i]], os)
    if (!length(args)) {
      if (!identical(corpus$cases[[i]]$expected[[os]], expected)) {
        stop(sprintf(
          "%s (%s): the Collate pin differs from %s",
          corpus$cases[[i]]$name,
          os,
          R.version.string
        ))
      }
    } else {
      corpus$cases[[i]]$expected[[os]] <- expected
    }
  }
}
if (length(args)) {
  corpus$r_version <- R.version.string
  jsonlite::write_json(corpus, path, pretty = TRUE, auto_unbox = TRUE)
}
cat(sprintf(
  "collate oracle: %d cases checked on both platforms against %s\n",
  length(corpus$cases),
  R.version.string
))
