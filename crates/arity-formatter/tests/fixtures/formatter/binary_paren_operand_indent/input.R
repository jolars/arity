if (!is.null(quantities$total) && (!is.numeric(quantities$total) || length(quantities$total) != 1 || is.na(quantities$total) || quantities$total <= 0)) {
  stop("`quantities$total` must be a single positive number.")
}
