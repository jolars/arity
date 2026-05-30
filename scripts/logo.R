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

# ---- R outline -----------------------------------------------------------
# Target endpoints along the silhouette of an upper-case R inside the box
# [0, 0.7] x [0, 1]. The "R" emerges from where threads converge --- it is
# never drawn explicitly.
r_outline <- function(n = 200) {
  bar_len <- 1.0
  bowl_len <- 1.25
  leg_len <- sqrt(0.7^2 + 0.55^2)
  total <- bar_len + bowl_len + leg_len
  n_bar <- max(2, round(n * bar_len / total))
  n_bowl <- max(2, round(n * bowl_len / total))
  n_leg <- n - n_bar - n_bowl

  bar <- line2d(n_bar, 0, 0, 0, 1)
  bowl <- bezier2d(
    n_bowl,
    xs = c(0, 0.95, 0.95, 0),
    ys = c(1.00, 1.00, 0.55, 0.55)
  )
  leg <- line2d(n_leg, 0, 0.55, 0.7, 0)

  rbind(bar, bowl, leg)
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
  n_threads = 80,
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

  targets <- r_outline(n_threads)
  # Shuffle so adjacent threads don't fan out neatly --- gives crossings
  # in the middle, which reads as "unraveling".
  targets <- targets[sample.int(nrow(targets)), , drop = FALSE]

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
    start <- tangle_c + c(runif(1, -0.08, 0.08), runif(1, -0.18, 0.18))
    pts <- make_thread(start, targets[i, ], n = n_points)
    lines(pts[, 1], pts[, 2], col = adjustcolor(col, alpha), lwd = lwd)
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
