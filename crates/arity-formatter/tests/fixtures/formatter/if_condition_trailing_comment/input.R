if (
  isFALSE(getOption(
    "dplyr.show_progress",
    default = TRUE
  )) || # user specifies no progress
    !interactive() || # not an interactive session
    !is.null(getOption("knitr.in.progress")) # dplyr used within knitr document
) {
  return(invisible(self))
}
