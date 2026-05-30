#!/usr/bin/env Rscript

# Generative logo for ravel.
#
# A tangle of threads on the left unravels into a fan of curling strands
# that settle (deterministically) onto the silhouette of an R. The tangle
# is seeded-random; the resolved end is fixed --- which mirrors what the
# formatter does to source code.
#
# Usage:
#   Rscript scripts/logo.R                  # seed = 1, default output
#   Rscript scripts/logo.R 42               # different seed
#   Rscript scripts/logo.R 42 out.svg       # custom output path (.svg or .png)

# ---- Bezier helpers ------------------------------------------------------
bezier1d <- function(t, p) {
  (1 - t)^3 * p[1] + 3 * (1 - t)^2 * t * p[2] + 3 * (1 - t) * t^2 * p[3] +
    t^3 * p[4]
}

line2d <- function(n, x0, y0, x1, y1) {
  t <- seq(0, 1, length.out = n)
  cbind(x0 + t * (x1 - x0), y0 + t * (y1 - y0))
}

bezier2d <- function(n, xs, ys) {
  t <- seq(0, 1, length.out = n)
  cbind(bezier1d(t, xs), bezier1d(t, ys))
}

# ---- R logo geometry -----------------------------------------------------
# Outer outline and bowl counter of the official R-project logo (Rlogo.svg).
# Source coordinates are in the SVG viewBox (724 x 561, y-down). We flatten
# the path into a polygon and rejection-sample inside (outer minus counter)
# so threads land across the whole letter with visible weight --- the
# counter naturally reads as negative space.

# Flatten a sequence of cubic-bezier / line segments to a polygon. `segs` is
# a list of list(type = "L", end) or list(type = "C", c1, c2, end).
flatten_path <- function(start, segs, n_per_seg = 18) {
  pts <- matrix(start, nrow = 1)
  cur <- start
  for (s in segs) {
    if (s$type == "L") {
      seg <- line2d(n_per_seg, cur[1], cur[2], s$end[1], s$end[2])
    } else {
      seg <- bezier2d(
        n_per_seg,
        xs = c(cur[1], s$c1[1], s$c2[1], s$end[1]),
        ys = c(cur[2], s$c1[2], s$c2[2], s$end[2])
      )
    }
    pts <- rbind(pts, seg[-1, , drop = FALSE])
    cur <- s$end
  }
  pts
}

r_outer_start <- c(550.000, 377.000)
r_outer_segs <- list(
  list(
    type = "C",
    c1 = c(550.000, 377.000),
    c2 = c(571.822, 383.585),
    end = c(584.500, 390.000)
  ),
  list(
    type = "C",
    c1 = c(588.899, 392.226),
    c2 = c(596.510, 396.668),
    end = c(602.000, 402.500)
  ),
  list(
    type = "C",
    c1 = c(607.378, 408.212),
    c2 = c(610.000, 414.000),
    end = c(610.000, 414.000)
  ),
  list(type = "L", end = c(696.000, 559.000)),
  list(type = "L", end = c(557.000, 559.062)),
  list(type = "L", end = c(492.000, 437.000)),
  list(
    type = "C",
    c1 = c(492.000, 437.000),
    c2 = c(478.690, 414.131),
    end = c(470.500, 407.500)
  ),
  list(
    type = "C",
    c1 = c(463.668, 401.969),
    c2 = c(460.755, 400.000),
    end = c(454.000, 400.000)
  ),
  list(
    type = "C",
    c1 = c(449.298, 400.000),
    c2 = c(420.974, 400.000),
    end = c(420.974, 400.000)
  ),
  list(type = "L", end = c(421.000, 558.974)),
  list(type = "L", end = c(298.000, 559.026)),
  list(type = "L", end = c(298.000, 152.938)),
  list(type = "L", end = c(545.000, 152.938)),
  list(
    type = "C",
    c1 = c(545.000, 152.938),
    c2 = c(657.500, 154.967),
    end = c(657.500, 262.000)
  ),
  list(
    type = "C",
    c1 = c(657.500, 369.033),
    c2 = c(550.000, 377.000),
    end = c(550.000, 377.000)
  )
)

r_counter_start <- c(496.500, 241.024)
r_counter_segs <- list(
  list(type = "L", end = c(422.037, 240.976)),
  list(type = "L", end = c(422.000, 310.026)),
  list(type = "L", end = c(496.500, 310.002)),
  list(
    type = "C",
    c1 = c(496.500, 310.002),
    c2 = c(531.000, 309.895),
    end = c(531.000, 274.877)
  ),
  list(
    type = "C",
    c1 = c(531.000, 239.155),
    c2 = c(496.500, 241.024),
    end = c(496.500, 241.024)
  )
)

# Vectorized ray-casting point-in-polygon: loops over edges, vectorized
# over test points.
points_in_poly <- function(xs, ys, poly) {
  n <- nrow(poly)
  inside <- logical(length(xs))
  j <- n
  for (i in seq_len(n)) {
    xi <- poly[i, 1]
    yi <- poly[i, 2]
    xj <- poly[j, 1]
    yj <- poly[j, 2]
    cond <- ((yi > ys) != (yj > ys)) &
      (xs < (xj - xi) * (ys - yi) / (yj - yi) + xi)
    inside <- xor(inside, cond)
    j <- i
  }
  inside
}

# Sample n target points uniformly inside the filled R region, mapped into
# the [0, 0.7] x [0, 1] box (y flipped, since SVG is y-down).
r_targets <- function(n = 200) {
  outer_poly <- flatten_path(r_outer_start, r_outer_segs)
  counter_poly <- flatten_path(r_counter_start, r_counter_segs)

  xr <- range(outer_poly[, 1])
  yr <- range(outer_poly[, 2])

  out_x <- numeric(0)
  out_y <- numeric(0)

  while (length(out_x) < n) {
    batch <- max(64L, as.integer((n - length(out_x)) * 3L))
    xs <- runif(batch, xr[1], xr[2])
    ys <- runif(batch, yr[1], yr[2])
    keep <- points_in_poly(xs, ys, outer_poly) &
      !points_in_poly(xs, ys, counter_poly)
    out_x <- c(out_x, xs[keep])
    out_y <- c(out_y, ys[keep])
  }

  out_x <- out_x[seq_len(n)]
  out_y <- out_y[seq_len(n)]

  nx <- (out_x - 298) / (696 - 298) * 0.7
  ny <- 1 - (out_y - 152.938) / (559.062 - 152.938)
  cbind(nx, ny)
}

# ---- Thread generator ----------------------------------------------------
smoothstep <- function(t) 3 * t^2 - 2 * t^3

# A single thread: cubic-Bezier baseline from `start` to `target`, with
# sinusoidal curl that decays smoothly toward `target`. Chaos on the left
# end, deterministic landing on the right.
make_thread <- function(
  start,
  target,
  n = 500,
  curl_amp = 0.10,
  curl_freq = c(4, 12)
) {
  t <- seq(0, 1, length.out = n)

  dx <- target[1] - start[1]
  c1x <- start[1] + dx * 0.35 + runif(1, -0.12, 0.12)
  c1y <- start[2] + runif(1, -0.35, 0.35)
  c2x <- target[1] - dx * 0.15
  c2y <- target[2] + runif(1, -0.04, 0.04)

  bx <- bezier1d(t, c(start[1], c1x, c2x, target[1]))
  by <- bezier1d(t, c(start[2], c1y, c2y, target[2]))

  # Soft decay: noise lingers across the path rather than pinning early ---
  # this is what gives the "uncombed hair flowing right" look.
  decay <- (1 - smoothstep(t))^1.2

  f1 <- runif(1, curl_freq[1], curl_freq[2])
  f2 <- runif(1, curl_freq[1], curl_freq[2]) * 0.5
  p1 <- runif(1, 0, 2 * pi)
  p2 <- runif(1, 0, 2 * pi)

  curl_y <- decay * curl_amp *
    (sin(f1 * 2 * pi * t + p1) + 0.5 * sin(f2 * 2 * pi * t + p2))
  curl_x <- decay * curl_amp * 0.5 * sin(f1 * 2 * pi * t + p1 + pi / 2)

  cbind(x = bx + curl_x, y = by + curl_y)
}

# ---- Renderer ------------------------------------------------------------
render_logo <- function(
  seed = 1,
  n_threads = 50,
  n_points = 500,
  col = "#111111",
  alpha = 0.45,
  lwd = 0.55,
  bg = "transparent",
  out = "images/logo-generated.svg",
  width = 6,
  height = 4
) {
  set.seed(seed)

  targets <- r_targets(n_threads)

  tangle_c <- c(-0.55, 0.5)

  is_svg <- grepl("\\.svg$", out, ignore.case = TRUE)
  if (is_svg) {
    svg(out, width = width, height = height, bg = bg)
  } else {
    png(out, width = width * 200, height = height * 200, bg = bg, res = 200)
  }
  on.exit(dev.off(), add = TRUE)

  par(mar = c(0, 0, 0, 0))
  plot(
    NA,
    xlim = c(-1.10, 0.85),
    ylim = c(-0.08, 1.08),
    asp = 1,
    axes = FALSE,
    xlab = "",
    ylab = ""
  )

  for (i in seq_len(n_threads)) {
    start <- tangle_c + c(runif(1, -0.08, 0.08), runif(1, -0.28, 0.38))
    pts <- make_thread(start, targets[i, ], n = n_points)
    lines(pts[, 1], pts[, 2], col = adjustcolor(col, alpha), lwd = lwd)
    points(
      pts[nrow(pts), 1],
      pts[nrow(pts), 2],
      col = adjustcolor(col, alpha * 1.2),
      pch = 16,
      cex = lwd * 1.5
    )
  }

  invisible(out)
}

# ---- CLI -----------------------------------------------------------------
if (!interactive()) {
  args <- commandArgs(trailingOnly = TRUE)
  seed <- if (length(args) >= 1) as.integer(args[1]) else 1L
  out <- if (length(args) >= 2) args[2] else "images/logo-generated.svg"
  path <- render_logo(seed = seed, out = out)
  cat(sprintf("wrote %s (seed = %d)\n", path, seed))
}
