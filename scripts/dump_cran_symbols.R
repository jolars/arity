#!/usr/bin/env Rscript
#
# Dump the exported names of the bundled CRAN packages to a single sectioned
# text file at src/semantic/cran/exports.txt. The Rust side `include_str!`s this
# to build the `BundledPackages` symbol provider, which lets `undefined-symbol`
# resolve names from `library()`-attached packages without those packages being
# installed locally.
#
# The package *set* comes from scripts/cran_top_packages.txt, the additive union
# of the top-N CRAN packages by download count (refresh it with
# scripts/rank_cran_downloads.sh). This script reads that list and writes one
# section per package, in sorted order. Confirmed archived packages are pruned
# from both the membership list and the generated exports.
#
# A single flaky install must not drop a package's exports and flicker the file.
# For each remaining member we therefore prefer a fresh dump from the installed
# namespace, replacing its entire prior section so removed exports disappear.
# When it can't be loaded this run, keep its previously generated section. A
# member with neither a fresh nor a prior section is omitted until a later run
# can dump it. Failed installs never count as evidence of archival.
#
# Names only (no formals, no help) - those stay with the local on-disk harvest,
# which is version-exact. This file is an approximate "latest" snapshot.
#
# This script runs by hand / in CI, not at build time:
#
#     scripts/rank_cran_downloads.sh    # refresh the package list (optional)
#     Rscript scripts/dump_cran_symbols.R
#
# Install the listed packages first for complete coverage. The CI workflow
# (.github/workflows/cran-symbols.yml) installs the set from binaries and opens
# a PR with the regenerated file.

list_file <- file.path("scripts", "cran_top_packages.txt")
out_file <- file.path("src", "semantic", "cran", "exports.txt")

cran_archived_packages <- function(packages) {
  cran <- "https://cran.r-project.org"
  # PPM binaries and the default R/OS filters can hide active packages. Only
  # CRAN's unfiltered source index is suitable for identifying candidates.
  current <- tryCatch(
    available.packages(
      repos = cran,
      type = "source",
      filters = list(),
      ignore_repo_cache = TRUE
    ),
    error = function(e) NULL,
    warning = function(w) NULL
  )
  if (is.null(current) || nrow(current) == 0L) {
    warning(
      "Could not read the current CRAN source index; skipping archive pruning.",
      call. = FALSE
    )
    return(character())
  }

  archived <- character()
  for (pkg in setdiff(packages, current[, "Package"])) {
    unverified <- function(e) {
      warning(
        sprintf(
          "Could not verify CRAN archive status for %s; preserving it: %s",
          pkg,
          conditionMessage(e)
        ),
        call. = FALSE
      )
      character()
    }
    page <- tryCatch(
      readLines(
        paste0(cran, "/web/packages/", pkg, "/index.html"),
        warn = FALSE
      ),
      error = unverified,
      warning = unverified
    )
    # Archive directories also contain old versions of active packages. Require
    # CRAN's explicit removal notice rather than inferring status from a link.
    if (
      any(grepl("was removed from the CRAN repository", page, fixed = TRUE)) &&
        any(grepl("Archived on [0-9]{4}-[0-9]{2}-[0-9]{2}", page))
    ) {
      archived <- c(archived, pkg)
    }
  }
  archived
}

list_lines <- readLines(list_file, warn = FALSE)
lines <- trimws(list_lines)
packages <- lines[nzchar(lines) & !startsWith(lines, "#")]
packages <- sort(unique(packages))

archived <- cran_archived_packages(packages)
if (length(archived) > 0L) {
  writeLines(list_lines[!lines %in% archived], list_file)
  packages <- setdiff(packages, archived)
  message("pruned archived CRAN packages: ", paste(archived, collapse = ", "))
}

# Parse the previously generated exports.txt into pkg -> names, so a package
# that can't be loaded this run can fall back to its last-known section. The
# format mirrors the writer below: a `[pkg]` line opens a section; subsequent
# non-empty, non-comment lines are that package's names; this is the same shape
# the Rust `parse_bundled` reader consumes.
parse_exports <- function(path) {
  prev <- new.env(parent = emptyenv())
  if (!file.exists(path)) {
    return(prev)
  }
  cur <- NULL
  for (line in readLines(path, warn = FALSE)) {
    s <- trimws(line)
    if (!nzchar(s) || startsWith(s, "#")) {
      next
    }
    m <- regmatches(s, regexec("^\\[(.*)\\]$", s))[[1]]
    if (length(m) == 2L) {
      cur <- m[[2L]]
      if (is.null(get0(cur, envir = prev, inherits = FALSE))) {
        assign(cur, character(0), envir = prev)
      }
    } else if (!is.null(cur)) {
      assign(cur, c(get0(cur, envir = prev, inherits = FALSE), s), envir = prev)
    }
  }
  prev
}

prev <- parse_exports(out_file)
prev_names <- function(pkg) {
  get0(pkg, envir = prev, inherits = FALSE, ifnotfound = NULL)
}

installed <- rownames(installed.packages())

dump_names <- function(pkg) {
  ns <- asNamespace(pkg)
  exports <- getNamespaceExports(ns)
  lazydata <- tryCatch(
    as.character(names(.getNamespaceInfo(ns, "lazydata"))),
    error = function(e) character(0)
  )
  out <- sort(unique(c(exports, lazydata)))
  out[nzchar(out)]
}

# Resolve each member to a section, preferring a fresh dump over the prior one.
# Collect into memory first (names are small) so the header can report counts.
sections <- list()
total <- 0L
fresh_n <- 0L
preserved_n <- 0L
no_data <- character(0)

for (pkg in packages) {
  syms <- NULL
  src <- NA_character_
  if (pkg %in% installed) {
    # `installed.packages()` can list a package whose namespace won't load (e.g.
    # unmet dependencies); treat that like a missing install and fall back.
    syms <- tryCatch(dump_names(pkg), error = function(e) NULL)
    if (!is.null(syms)) src <- "fresh"
  }
  if (is.null(syms)) {
    syms <- prev_names(pkg)
    if (!is.null(syms)) src <- "preserved"
  }
  if (is.null(syms)) {
    no_data <- c(no_data, pkg)
    next
  }
  sections[[pkg]] <- syms
  total <- total + length(syms)
  if (identical(src, "fresh")) {
    fresh_n <- fresh_n + 1L
  } else {
    preserved_n <- preserved_n + 1L
  }
  message(sprintf("%-9s %5d names  %s", src, length(syms), pkg))
}

dir.create(dirname(out_file), recursive = TRUE, showWarnings = FALSE)
con <- file(out_file, open = "wt")

writeLines(
  c(
    "# Generated by scripts/dump_cran_symbols.R - do not edit by hand.",
    sprintf(
      "# R %s; %d of %d bundled packages have exports.",
      getRversion(),
      length(sections),
      length(packages)
    ),
    ""
  ),
  con
)

for (pkg in names(sections)) {
  writeLines(c(sprintf("[%s]", pkg), sections[[pkg]], ""), con)
}
close(con)

if (length(no_data) > 0) {
  warning(
    sprintf(
      "%d package(s) have no exports (not installed and no prior dump): %s",
      length(no_data),
      paste(no_data, collapse = ", ")
    ),
    call. = FALSE,
    immediate. = TRUE
  )
}

message(sprintf(
  "wrote %d names across %d packages (%d fresh, %d preserved) to %s",
  total,
  length(sections),
  fresh_n,
  preserved_n,
  out_file
))
