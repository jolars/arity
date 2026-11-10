#!/usr/bin/env Rscript
# Run with Rscript --vanilla tests/cran_symbols.R. All CRAN responses and
# namespaces are local fixtures so these tests need no network or packages.

script <- normalizePath(file.path("scripts", "dump_cran_symbols.R"))

refresh <- function(
  packages,
  previous,
  fresh = list(),
  active = character(),
  pages = list(),
  index_error = FALSE,
  index_warning = FALSE,
  lazydata = list()
) {
  root <- tempfile("cran-symbols-")
  dir.create(root)
  old_wd <- setwd(root)
  on.exit({
    setwd(old_wd)
    unlink(root, recursive = TRUE)
  })
  dir.create("scripts")
  dir.create(file.path("src", "semantic", "cran"), recursive = TRUE)
  list_file <- file.path("scripts", "cran_top_packages.txt")
  out_file <- file.path("src", "semantic", "cran", "exports.txt")
  list_lines <- c("# Keep this membership header.", "", sort(packages))
  writeLines(list_lines, list_file)
  writeLines(previous, out_file)

  env <- new.env(parent = globalenv())
  env$available.packages <- function(repos, type, filters, ignore_repo_cache) {
    stopifnot(
      identical(repos, "https://cran.r-project.org"),
      identical(type, "source"),
      identical(filters, list()),
      isTRUE(ignore_repo_cache)
    )
    if (index_error) {
      stop("CRAN is unreachable")
    }
    if (index_warning) {
      warning("cannot open CRAN index")
    }
    cbind(Package = active)
  }
  checked_pages <- character()
  env$readLines <- function(con, ...) {
    prefix <- "https://cran.r-project.org/web/packages/"
    if (startsWith(con, prefix)) {
      pkg <- sub("/index.html$", "", substring(con, nchar(prefix) + 1L))
      checked_pages <<- c(checked_pages, pkg)
      page <- pages[[pkg]]
      if (is.null(page)) {
        stop("package page is unreachable")
      }
      return(page)
    }
    base::readLines(con, ...)
  }
  env$installed.packages <- function() {
    installed <- cbind(Package = names(fresh))
    rownames(installed) <- names(fresh)
    installed
  }
  env$asNamespace <- identity
  env$getNamespaceExports <- function(pkg) {
    exports <- fresh[[pkg]]
    if (inherits(exports, "error")) {
      stop(exports)
    }
    exports
  }
  env$.getNamespaceInfo <- function(pkg, which) {
    setNames(as.list(lazydata[[pkg]]), lazydata[[pkg]])
  }
  warnings <- character()
  withCallingHandlers(
    eval(parse(file = script), envir = env),
    warning = function(w) {
      warnings <<- c(warnings, conditionMessage(w))
      invokeRestart("muffleWarning")
    }
  )
  list(
    members = readLines(list_file),
    original_members = list_lines,
    exports = readLines(out_file),
    checked_pages = checked_pages,
    warnings = warnings
  )
}

archived_page <- c(
  "<p>Package &lsquo;retired&rsquo; was removed from the CRAN repository.</p>",
  "<p>Archived on 2026-10-01 at the request of the maintainer.</p>"
)

result <- refresh(
  packages = c("active", "retired", "unknown"),
  previous = c("[active]", "keep", "[retired]", "stale", "[unknown]", "retain"),
  fresh = list(retired = "still_installed"),
  active = "active",
  pages = list(retired = archived_page)
)
stopifnot(
  identical(
    result$members,
    c("# Keep this membership header.", "", "active", "unknown")
  ),
  !any(result$exports %in% c("[retired]", "stale", "still_installed")),
  all(c("[active]", "keep", "[unknown]", "retain") %in% result$exports),
  setequal(result$checked_pages, c("retired", "unknown"))
)
cat("PASS: only confirmed archived packages lose membership and exports\n")

result <- refresh(
  packages = c("active", "unlisted"),
  previous = c("[active]", "keep", "[unlisted]", "retain"),
  active = "active",
  pages = list(
    active = archived_page,
    unlisted = '<a href="../../src/contrib/Archive/unlisted/">Old sources</a>'
  )
)
stopifnot(
  identical(result$members, result$original_members),
  all(c("[active]", "keep", "[unlisted]", "retain") %in% result$exports),
  identical(result$checked_pages, "unlisted")
)
cat("PASS: active packages and archive links alone never trigger pruning\n")

for (failure in c("error", "warning", "empty")) {
  result <- refresh(
    packages = "retired",
    previous = c("[retired]", "preserved"),
    active = character(),
    pages = list(retired = archived_page),
    index_error = failure == "error",
    index_warning = failure == "warning"
  )
  stopifnot(
    identical(result$members, result$original_members),
    all(c("[retired]", "preserved") %in% result$exports),
    length(result$checked_pages) == 0L,
    length(result$warnings) > 0L
  )
}
cat("PASS: failed or empty CRAN indexes preserve all members\n")

result <- refresh(
  packages = c("active", "broken", "empty", "missing"),
  previous = c(
    "[active]",
    "removed",
    "surviving",
    "old_dataset",
    "[broken]",
    "fallback",
    "[empty]",
    "removed_too",
    "[missing]",
    "last_known"
  ),
  fresh = list(
    active = c("surviving", "new"),
    broken = simpleError("namespace cannot load"),
    empty = character()
  ),
  active = c("active", "broken", "empty", "missing"),
  lazydata = list(active = "new_dataset")
)
stopifnot(
  identical(result$members, result$original_members),
  all(c("[active]", "surviving", "new", "new_dataset") %in% result$exports),
  !any(c("removed", "old_dataset", "removed_too") %in% result$exports),
  all(
    c("[broken]", "fallback", "[empty]", "[missing]", "last_known") %in%
      result$exports
  ),
  length(result$checked_pages) == 0L
)
cat("PASS: fresh exports replace stale symbols, including empty export sets\n")
