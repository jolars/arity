f <- function() {
  switch(coord, x = {
    if (is.null(x)) {
      x <- with(
        ranges,
        if (lower) {
          x[1] - 0.075 * line * diff(x)
        } else {
          x[2] + 0.075 * line * diff(x)
        }
      )
    }
  })
}
