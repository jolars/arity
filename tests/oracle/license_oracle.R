#!/usr/bin/env Rscript
# Exercise the live license database without requiring R in ordinary tests.
# arity-ignore-file internal-function: these are the database under test.
db <- tools:::R_license_db()
aliases <- tools:::.standardizable_license_specs_db
names <- unique(c(db$Name, db$Abbrev))
names <- names[nzchar(names)]
versioned <- db[nzchar(db$Version), ]
fixed <- c(
  db$SSS, paste0(versioned$Abbrev[nzchar(versioned$Abbrev)], "-", versioned$Version[nzchar(versioned$Abbrev)]),
  paste(versioned$Name, versioned$Version),
  paste(versioned$Name, "Version", versioned$Version),
  paste(versioned$Name, "version", versioned$Version)
)
constraints <- c(
  "(>= 0)", "(== 2)", "(== 2.0)", "(== 2-1)", "(>= 2, < 3)",
  "(!= 2)", "(<= 2)", "(> 2)", "(>= 99)", "(>= 3, < 3)",
  "(>= 2)(< 3)", "(=> 2)", "(>= latest)", "(>= 1..0)", "(>= 2,)"
)
cases <- unique(c(
  names, fixed[nzchar(fixed)], aliases$ispecs, aliases$ospecs,
  as.vector(outer(names, constraints, paste)),
  paste(names, "+ file LICENSE"), paste(names, "+ file LICENCE"),
  "file LICENSE", "file LICENCE", "Unlimited", "Unlimited + file LICENSE",
  "Public domain", "GPL-4", "Apache-2.0", "file license", "MIT + LICENSE"
))
for (spec in cases) {
  canonical <- isTRUE(suppressWarnings(tools::analyze_license(spec)$is_canonical))
  replacement <- aliases$ospecs[match(spec, aliases$ispecs)]
  if (is.na(replacement) || grepl("file ", replacement, fixed = TRUE) ||
      !isTRUE(suppressWarnings(tools::analyze_license(replacement)$is_canonical))) {
    replacement <- ""
  }
  cat(as.integer(canonical), replacement, spec, sep = "\t")
  cat("\n")
}
