#!/usr/bin/env Rscript
# Regenerate `tests/fixtures/rindex/metatoy`: a toy meta-package whose
# namespace carries a `core` attach-set variable (the tidyverse convention)
# and an `.onAttach` hook, exercised by the harvest-time attach-capture
# tests. Its `core` names the two other checked-in fixture packages (R.oo,
# magrittr) so the installed-member validation passes against the fixture
# library itself. Run from the repo root:
#
#     Rscript scripts/make_metatoy_fixture.R

dest <- file.path("tests", "fixtures", "rindex", "metatoy")
if (!dir.exists(dirname(dest))) {
  stop("run from the repo root: ", dirname(dest), " not found")
}

src <- file.path(tempdir(), "metatoy")
dir.create(file.path(src, "R"), recursive = TRUE, showWarnings = FALSE)

writeLines(
  c(
    "Package: metatoy",
    "Version: 1.0.0",
    "Title: Toy Meta-Package Fixture for Arity",
    "Description: Checked-in fixture for arity's harvest-time attach capture.",
    "License: MIT + file LICENSE",
    "Encoding: UTF-8"
  ),
  file.path(src, "DESCRIPTION")
)

writeLines("export(metatoy_hello)", file.path(src, "NAMESPACE"))

writeLines(
  c(
    "core <- c(\"R.oo\", \"magrittr\")",
    "",
    ".onAttach <- function(libname, pkgname) {",
    "  for (pkg in core) {",
    "    library(pkg, character.only = TRUE)",
    "  }",
    "}",
    "",
    "metatoy_hello <- function() \"hello\""
  ),
  file.path(src, "R", "metatoy.R")
)

lib <- file.path(tempdir(), "metatoy-lib")
dir.create(lib, showWarnings = FALSE)
status <- system2(
  file.path(R.home("bin"), "R"),
  c("CMD", "INSTALL", "--no-docs", "-l", shQuote(lib), shQuote(src))
)
if (status != 0) {
  stop("R CMD INSTALL failed with status ", status)
}

installed <- file.path(lib, "metatoy")
unlink(dest, recursive = TRUE)
dir.create(file.path(dest, "R"), recursive = TRUE)
file.copy(file.path(installed, "DESCRIPTION"), dest)
file.copy(file.path(installed, "NAMESPACE"), dest)
file.copy(file.path(installed, "R", "metatoy.rdb"), file.path(dest, "R"))
file.copy(file.path(installed, "R", "metatoy.rdx"), file.path(dest, "R"))

cat("wrote", dest, "\n")
