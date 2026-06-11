#!/usr/bin/env Rscript

# Generative logo for arity.
#
# Threads start jumbled on a ring around the letter, then fold into the R's
# outline (tangent-matched at the join, so there's no visible kink) and
# continue along the outline before settling flush. The chaos resolves
# into the letter shape itself --- which mirrors what the formatter does
# to source code.
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
# the paths into polygons; threads land along (and travel around) them.
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

# SVG-source -> normalized [0, 0.7] x [0, 1] (y flipped, since SVG is y-down).
to_norm <- function(xy) {
  cbind(
    (xy[, 1] - 298) / (696 - 298) * 0.7,
    1 - (xy[, 2] - 152.938) / (559.062 - 152.938)
  )
}

r_polys <- function() {
  list(
    outer = to_norm(flatten_path(r_outer_start, r_outer_segs)),
    counter = to_norm(flatten_path(r_counter_start, r_counter_segs))
  )
}

# ---- Polyline arc-length parameterization --------------------------------
# Treat each polygon as a closed polyline so threads can travel along it
# with a continuous arc-length parameter. Tangents are taken from the
# segment the sample lands in.
polyline_arclen <- function(poly) {
  closed <- rbind(poly, poly[1, , drop = FALSE])
  deltas <- diff(closed)
  seglens <- sqrt(deltas[, 1]^2 + deltas[, 2]^2)
  list(
    pts = closed,
    seglens = seglens,
    cum = c(0, cumsum(seglens)),
    total = sum(seglens)
  )
}

# Walk a closed polyline from arc length s0 in direction dir (+/-1) for
# length L, returning n sampled points and unit tangents in the walk
# direction.
polyline_walk <- function(al, s0, dir, L, n) {
  ss <- seq(0, L, length.out = n)
  pts <- matrix(0, nrow = n, ncol = 2)
  tans <- matrix(0, nrow = n, ncol = 2)
  for (i in seq_len(n)) {
    s <- (s0 + dir * ss[i]) %% al$total
    idx <- findInterval(s, al$cum, rightmost.closed = TRUE)
    idx <- max(1L, min(idx, length(al$seglens)))
    seg_start <- al$cum[idx]
    seg_len <- al$seglens[idx]
    t_local <- if (seg_len > 0) (s - seg_start) / seg_len else 0
    p0 <- al$pts[idx, ]
    p1 <- al$pts[idx + 1, ]
    pts[i, ] <- p0 + t_local * (p1 - p0)
    tan <- p1 - p0
    tlen <- sqrt(sum(tan^2))
    if (tlen > 0) tan <- tan / tlen
    tans[i, ] <- tan * dir
  }
  list(pts = pts, tans = tans)
}

# ---- Thread phases -------------------------------------------------------
smoothstep <- function(t) 3 * t^2 - 2 * t^3

# Phase 1: jumbled cubic-Bezier from `start` to `target`, with c2 pulled
# back along -target_tan so the bezier's tangent at t=1 lies along
# target_tan. Phase 2 then continues in the same direction --- positions
# and tangents both match at the join, no visible kink.
make_approach <- function(
  start,
  target,
  target_tan,
  n,
  curl_amp = 0.10,
  curl_freq = c(4, 12)
) {
  t <- seq(0, 1, length.out = n)
  dvec <- target - start
  dlen <- sqrt(sum(dvec^2))
  ux <- dvec[1] / dlen
  uy <- dvec[2] / dlen
  px <- -uy
  py <- ux

  bow1 <- runif(1, -0.3, 0.3) * dlen
  c1x <- start[1] + 0.35 * dvec[1] + bow1 * px
  c1y <- start[2] + 0.35 * dvec[2] + bow1 * py

  c2_dist <- 0.25 * dlen
  c2x <- target[1] - c2_dist * target_tan[1]
  c2y <- target[2] - c2_dist * target_tan[2]

  bx <- bezier1d(t, c(start[1], c1x, c2x, target[1]))
  by <- bezier1d(t, c(start[2], c1y, c2y, target[2]))

  # Curl decays to zero (and zero derivative) at t=1 so the bezier endpoint
  # is exactly target with tangent target_tan.
  decay <- (1 - smoothstep(t))^1.2
  f1 <- runif(1, curl_freq[1], curl_freq[2])
  f2 <- runif(1, curl_freq[1], curl_freq[2]) * 0.5
  p1 <- runif(1, 0, 2 * pi)
  p2 <- runif(1, 0, 2 * pi)
  curl_perp <- decay * curl_amp *
    (sin(f1 * 2 * pi * t + p1) + 0.5 * sin(f2 * 2 * pi * t + p2))
  curl_para <- decay * curl_amp * 0.5 * sin(f1 * 2 * pi * t + p1 + pi / 2)

  cbind(
    x = bx + curl_perp * px + curl_para * ux,
    y = by + curl_perp * py + curl_para * uy
  )
}

# Phase 2: walk the outline with a perpendicular curl whose envelope is
# zero (and has zero derivative) at u=0 --- so the join with phase 1 is
# tangent-continuous --- and decays to zero by u=1, so the thread settles
# flush onto the outline.
make_travel <- function(walk, curl_amp = 0.025, curl_freq = c(4, 12)) {
  n <- nrow(walk$pts)
  u <- seq(0, 1, length.out = n)

  ramp <- smoothstep(pmin(u / 0.15, 1))
  decay <- 1 - smoothstep(u)

  f1 <- runif(1, curl_freq[1], curl_freq[2])
  p1 <- runif(1, 0, 2 * pi)
  offset <- ramp * decay * curl_amp * sin(2 * pi * f1 * u + p1)

  perp <- cbind(-walk$tans[, 2], walk$tans[, 1])
  cbind(
    x = walk$pts[, 1] + offset * perp[, 1],
    y = walk$pts[, 2] + offset * perp[, 2]
  )
}

# ---- Renderer ------------------------------------------------------------
render_logo <- function(
  seed = 1,
  n_threads = 34,
  n_points = 1000,
  ring_center = c(0.35, 0.5),
  ring_radius = 1.15,
  ring_jitter = 0.10,
  approach_curl_amp = 0.05,
  travel_curl_amp = 0.035,
  counter_frac = 0.05,
  max_arc_frac = 0.4,
  col = "#111111",
  alpha = 0.7,
  lwd = 1.05,
  bg = "transparent",
  out = "images/logo-generated.svg",
  width = 6,
  height = 6
) {
  set.seed(seed)

  polys <- r_polys()
  outer_al <- polyline_arclen(polys$outer)
  counter_al <- polyline_arclen(polys$counter)

  is_svg <- grepl("\\.svg$", out, ignore.case = TRUE)
  if (is_svg) {
    svg(out, width = width, height = height, bg = bg)
  } else {
    png(out, width = width * 200, height = height * 200, bg = bg, res = 200)
  }
  on.exit(dev.off(), add = TRUE)

  par(mar = c(0, 0, 0, 0))
  half <- ring_radius + ring_jitter + 0.10
  plot(
    NA,
    xlim = c(ring_center[1] - half, ring_center[1] + half),
    ylim = c(ring_center[2] - half, ring_center[2] + half),
    asp = 1,
    axes = FALSE,
    xlab = "",
    ylab = ""
  )

  for (i in seq_len(n_threads)) {
    angle <- runif(1, 0, 2 * pi)
    r <- ring_radius + runif(1, -ring_jitter, ring_jitter)
    start <- ring_center + r * c(cos(angle), sin(angle))

    al <- if (runif(1) < counter_frac) counter_al else outer_al

    s0 <- runif(1, 0, al$total)
    dir <- sample(c(-1, 1), 1)
    # Heavy-tailed travel length: most threads barely wrap, a few wrap a
    # lot --- that's where the "arity" reading comes from.
    L <- rbeta(1, 1.5, 4) * max_arc_frac * al$total

    entry <- polyline_walk(al, s0, dir, 0, 1)
    target <- entry$pts[1, ]
    target_tan <- entry$tans[1, ]

    # Split n_points across phases by approximate arc length (phase 1's
    # length is approximated by its straight-line distance).
    dlen <- sqrt(sum((target - start)^2))
    total_len <- dlen + L
    n1 <- max(2L, round(n_points * dlen / total_len))
    n2 <- max(2L, n_points - n1)

    ap <- make_approach(start, target, target_tan, n1, approach_curl_amp)
    walk <- polyline_walk(al, s0, dir, L, n2)
    tr <- make_travel(walk, travel_curl_amp)

    pts <- rbind(ap, tr[-1, , drop = FALSE])

    lines(pts[, 1], pts[, 2], col = adjustcolor(col, alpha), lwd = lwd)
  }

  invisible(out)
}

# ---- CLI -----------------------------------------------------------------
if (!interactive()) {
  args <- commandArgs(trailingOnly = TRUE)
  seed <- if (length(args) >= 1) as.integer(args[1]) else 1L
  out <- if (length(args) >= 2) args[2] else "images/logo-generated.svg"
  path <- render_logo(out = out)
  cat(sprintf("wrote %s (seed = %d)\n", path, seed))
}
