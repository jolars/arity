f <- function(centers, bb, what) {
	if (what == "centers")
		st_sfc(lapply(seq_len(nrow(centers)), function(i) mk_point(centers[i,])), crs = st_crs(bb))
	else # points:
		st_sfc(lapply(seq_len(nrow(centers)), function(i) mk_pol(centers[i,])), crs = st_crs(bb))
}
