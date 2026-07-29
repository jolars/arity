vec_ptype2_impl <- function(x, y) {
  if (identical(class(x), class(y))) {
    x
  } else {
    # return empty for mixed types
    st_sfc(crs = st_crs(x))
  }
}
