#!/usr/bin/env Rscript
# Pin the English title-case recommendation used by CRAN's incoming checker.
# R runs only in this test oracle, never in the linter.
#
# Rscript --vanilla tests/oracle/title_case_oracle.R [--refresh]
# Review every changed expected title after refreshing the pin.

args <- commandArgs(trailingOnly = TRUE)
stopifnot(length(args) == 0L || identical(args, "--refresh"))
invisible(Sys.setlocale("LC_CTYPE", "C.UTF-8"))
path <- "tests/oracle/description-title-case.json"
corpus <- jsonlite::fromJSON(path, simplifyVector = FALSE)

recommend <- function(title) {
  title <- gsub("[\n\t]", " ", title)
  result <- tools::toTitleCase(title)
  # This is CRAN's additional exemption for quoted software names, including
  # names containing spaces or hyphens that toTitleCase alone would change.
  pattern <- "(^|(?<=[ \t[:punct:]]))'[^']*'($|(?=[ \t[:punct:]]))"
  matches <- gregexpr(pattern, title, perl = TRUE)
  regmatches(result, matches) <- regmatches(title, matches)
  result
}

for (i in seq_along(corpus$cases)) {
  case <- corpus$cases[[i]]
  expected <- recommend(case$title)
  if (!length(args)) {
    if (!identical(case$expected, expected)) {
      stop(sprintf("%s: %s: pinned %s, R returned %s",
                   case$origin, case$title, case$expected, expected))
    }
  } else {
    corpus$cases[[i]]$expected <- expected
  }
}
if (length(args)) {
  corpus$r_version <- R.version.string
  jsonlite::write_json(corpus, path, pretty = TRUE, auto_unbox = TRUE)
}
cat(sprintf("title-case oracle: %d titles checked against %s\n",
            length(corpus$cases), R.version.string))
