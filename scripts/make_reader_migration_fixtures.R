#!/usr/bin/env Rscript
# Regenerate the synthetic reader contracts without using installed packages.
# R supplies the serialization and independently checks each compiled record.
stopifnot(getRversion() == "4.6.1")
dest <- "tests/fixtures/rindex/migration"
dir.create(dest, recursive = TRUE, showWarnings = FALSE)

write_db <- function(stem, values, labels = names(values)) {
  bytes <- raw()
  locations <- list()
  for (value in values) {
    stream <- serialize(value, NULL, version = 3, xdr = TRUE)
    record <- c(
      writeBin(as.integer(length(stream)), raw(), size = 4, endian = "big"),
      memCompress(stream, "gzip")
    )
    locations <- c(
      locations,
      list(as.integer(c(length(bytes), length(record))))
    )
    bytes <- c(bytes, record)
  }
  names(locations) <- labels
  writeBin(bytes, paste0(stem, ".rdb"))
  index <- list(variables = locations, references = list(), compressed = TRUE)
  saveRDS(index, paste0(stem, ".rdx"), version = 3)
  for (i in seq_along(values)) {
    restored <- lazyLoadDBfetch(
      locations[[i]],
      paste0(stem, ".rdb"),
      TRUE,
      function(x) x
    )
    stopifnot(identical(restored, values[[i]]))
  }
  invisible(index)
}

index <- write_db(
  file.path(dest, "duplicates"),
  list(1L, 2L, 3L),
  c("z", "a", "a")
)
bad <- index
bad$variables[[1]][1] <- -1L
saveRDS(bad, file.path(dest, "negative.rdx"), version = 3)
bad$variables[[1]][1] <- NA_integer_
saveRDS(bad, file.path(dest, "na-offset.rdx"), version = 3)
saveRDS(
  list(variables = list(), references = list(), compressed = TRUE),
  file.path(dest, "empty.rdx"),
  version = 3
)

pkg <- file.path(dest, "readerfixture")
for (folder in c("R", "help", "Meta", "data")) {
  dir.create(file.path(pkg, folder), recursive = TRUE, showWarnings = FALSE)
}
writeLines(
  c(
    "Package: readerfixture",
    "Version: 1.0.0",
    "Title: Reader fixture",
    "Description: Synthetic reader migration contracts.",
    "License: MIT",
    "Encoding: UTF-8",
    "Built: R 4.6.1; ; ;"
  ),
  file.path(pkg, "DESCRIPTION")
)
writeLines("exportPattern(\"^[a-z]\")", file.path(pkg, "NAMESPACE"))
values <- list(
  zero = function() NULL,
  defaults = function(
    x,
    flag = TRUE,
    count = 1L,
    label = "text",
    nothing = NULL
  ) {
    x
  },
  primitive = `+`,
  number = 42L,
  vector = c("one", "two"),
  core = c("R.oo", "magrittr"),
  .onAttach = function(...) NULL
)
write_db(file.path(pkg, "R", "readerfixture"), values)
write_db(file.path(pkg, "data", "Rdata"), list(number = 99L, dataset = 1:3))

page <- tools::parse_Rd(textConnection(paste0(
  "\\name{zero}\n\\alias{zero}\n\\title{Page title}\n",
  "\\description{A \\code{value} with \\emph{emphasis}.}\n",
  "\\usage{zero()}\n\\arguments{\\item{x, y}{Grouped values.}}\n"
)))
# Source environments have identity rather than value equality after decoding.
strip_source <- function(x) {
  attr(x, "srcref") <- NULL
  attr(x, "Rdfile") <- NULL
  attr(x, "macros") <- NULL
  if (is.list(x)) for (i in seq_along(x)) {
    x[[i]] <- strip_source(x[[i]])
  }
  x
}
page <- strip_source(page)
write_db(
  file.path(pkg, "help", "readerfixture"),
  list(zero = page, other = page)
)
description <- which(vapply(
  page,
  function(x) identical(attr(x, "Rd_tag"), "\\description"),
  logical(1)
))
opaque_page <- page
attr(opaque_page[[description]], "unexpected") <- "opaque attribute"
write_db(file.path(dest, "opaque-help"), list(zero = opaque_page, other = page))
invalid_utf8 <- rawToChar(as.raw(255))
Encoding(invalid_utf8) <- "UTF-8"
encoded_page <- page
encoded_page[[description]][[1]] <- structure(invalid_utf8, Rd_tag = "TEXT")
write_db(
  file.path(dest, "invalid-encoding-help"),
  list(zero = encoded_page, other = page)
)
metadata <- data.frame(
  Name = c("zero", "other", "dataset"),
  File = c("zero.Rd", "other.Rd", NA_character_),
  Title = c("Metadata title", "Other title", "Dataset title"),
  Aliases = I(list(
    c("zero", "defaults", "shared", NA_character_),
    c("shared", "number"),
    "dataset"
  ))
)
saveRDS(metadata, file.path(pkg, "Meta", "Rd.rds"), version = 3)
encoded_metadata <- metadata
encoded_metadata$Title[1] <- invalid_utf8
saveRDS(
  encoded_metadata,
  file.path(dest, "invalid-encoding-title.rds"),
  version = 3
)
normalized <- metadata
normalized$File[1] <- "man/zero.rd"
saveRDS(normalized, file.path(dest, "normalized.rds"), version = 3)
stopifnot(sub("\\.[Rr]d$", "", basename(normalized$File[1])) == "zero")
optional <- metadata
optional$Title <- NULL
optional$File[1] <- NA_character_
saveRDS(optional, file.path(dest, "optional.rds"), version = 3)
invalid <- metadata
invalid$Title <- 1:3
saveRDS(invalid, file.path(dest, "altrep-title.rds"), version = 3)
invalid$Title <- c(1L, 2L, 3L)
saveRDS(invalid, file.path(dest, "invalid-title.rds"), version = 3)
invalid <- metadata
invalid$Aliases[[1]] <- 1L
saveRDS(invalid, file.path(dest, "invalid-aliases.rds"), version = 3)
saveRDS(metadata[FALSE, ], file.path(dest, "empty.rds"), version = 3)
writeLines(
  c(
    R.version.string,
    paste("LC_CTYPE:", Sys.getlocale("LC_CTYPE")),
    "Serialization: XDR version 3; gzip RDS indexes; zlib records.",
    "Source: scripts/make_reader_migration_fixtures.R; synthetic MIT fixtures.",
    "Every intact record is checked with R's lazyLoadDBfetch."
  ),
  file.path(dest, "PROVENANCE.txt")
)
