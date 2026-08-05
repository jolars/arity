g <- function(a = if (NROW(x) < NCOL(x)) 1e-2 else 1e-4) {
  check(a)
  a
}
