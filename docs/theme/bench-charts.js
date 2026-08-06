// Renders the benchmark dot plot(s) on the Benchmarks page with Vega-Lite.
//
// Data is injected by the docs generator (`arity::bench_docs`, via
// `examples/docgen.rs`) as an inline
// `<script type="application/json" class="bench-data">` next to a
// `<div class="bench-chart">`. The Vega runtime is vendored under theme/vendor/
// and loaded before this file via book.toml's `additional-js`, so nothing is
// fetched at view time.
//
// Chart: x = tool, y = time relative to arity (log scale, baseline = 1),
// color = document (corpus tier or project), one dot per (document, tool), with
// a hover tooltip. One such chart per (operation, scope): formatter/linter x
// single-files/projects.
(function () {
  "use strict";

  // mdBook keeps the active theme as a class on <html>; these three are dark.
  function isDark() {
    var c = document.documentElement.classList;
    return c.contains("coal") || c.contains("navy") || c.contains("ayu");
  }

  // Unique values in first-appearance (results) order, so the axis and legend
  // read arity -> comparison tool rather than alphabetized.
  function orderedUnique(rows, key) {
    var seen = Object.create(null);
    var out = [];
    rows.forEach(function (r) {
      if (!(r[key] in seen)) {
        seen[r[key]] = true;
        out.push(r[key]);
      }
    });
    return out;
  }

  // Tick values for the log ratio axis, in decreasing order of preference:
  // exact powers of ten, then a 1-2-5 ladder. The first ladder placing at least
  // three ticks inside the data's range wins; a wide chart therefore reads
  // 1, 10, 100 and a narrow one 1, 2, 5 rather than either extreme.
  //
  // Candidates are clipped to the data range, not to a decade boundary, so
  // every tick returned is guaranteed to fall inside whatever domain the scale
  // ends up with. Returns undefined when even the finer ladder is too sparse
  // (all tools within a hair of arity); the scale then picks its own ticks,
  // which "~f" still renders as plain decimals.
  function logTicks(points) {
    var ratios = points
      .map(function (p) {
        return p.ratio;
      })
      .filter(function (r) {
        return r > 0;
      });
    if (!ratios.length) {
      return undefined;
    }
    var lo = Math.min.apply(null, ratios);
    var hi = Math.max.apply(null, ratios);
    var ladders = [[1], [1, 2, 5]];
    for (var i = 0; i < ladders.length; i++) {
      var ticks = [];
      for (
        var e = Math.floor(Math.log10(lo));
        e <= Math.ceil(Math.log10(hi));
        e++
      ) {
        for (var m = 0; m < ladders[i].length; m++) {
          var v = ladders[i][m] * Math.pow(10, e);
          if (v >= lo && v <= hi) {
            ticks.push(v);
          }
        }
      }
      if (ticks.length >= 3) {
        return ticks;
      }
    }
    return undefined;
  }

  function spec(points) {
    var dark = isDark();
    var fg = dark ? "#c8c9db" : "#333333";
    var grid = dark ? "#3b3f5c" : "#dddddd";
    var tools = orderedUnique(points, "tool");
    var documents = orderedUnique(points, "document");

    return {
      $schema: "https://vega.github.io/schema/vega-lite/v5.json",
      description:
        "Dot plot of speed relative to arity. Each dot is one input " +
        "processed by one tool; the vertical axis is mean time as a " +
        "ratio to arity on a log scale, with arity on a dashed baseline " +
        "at 1, faster tools below and slower tools above. See the data table " +
        "for the underlying numbers.",
      width: "container",
      height: 340,
      data: { values: points },
      layer: [
        // Baseline at 1.0 (arity); everything below is faster, above slower.
        {
          mark: { type: "rule", strokeDash: [4, 4], color: grid },
          encoding: { y: { datum: 1, type: "quantitative" } },
        },
        {
          mark: { type: "point", filled: true, size: 130, opacity: 0.9 },
          encoding: {
            x: {
              field: "tool",
              type: "nominal",
              title: "Tool",
              sort: tools,
              axis: { labelAngle: 0 },
            },
            // Dodge dots of different documents so same-ratio points (all the
            // arity dots sit at 1.0) don't stack on top of each other.
            xOffset: { field: "document", type: "nominal", sort: documents },
            y: {
              field: "ratio",
              type: "quantitative",
              title: "Time relative to arity",
              scale: { type: "log" },
              axis: {
                // Plain-decimal tick labels (100, 10, 1, 0.5, …); "~f" trims
                // trailing zeros. The default "~s" formatting turns sub-1
                // ratios into SI-prefixed labels ("500m", "200m") and large
                // ones into "1k", which read as units rather than ratios.
                values: logTicks(points),
                format: "~f",
              },
            },
            color: {
              field: "document",
              type: "nominal",
              title: "Input",
              sort: documents,
            },
            tooltip: [
              { field: "document", title: "Input" },
              { field: "tool", title: "Tool" },
              { field: "mean_ms", title: "Mean (ms)", format: ".3f" },
              { field: "ratio_label", title: "Relative" },
              { field: "min_ms", title: "Min (ms)", format: ".3f" },
              { field: "max_ms", title: "Max (ms)", format: ".3f" },
              { field: "stddev_ms", title: "Std dev (ms)", format: ".3f" },
            ],
          },
        },
      ],
      config: {
        background: null,
        view: { stroke: null },
        axis: {
          labelColor: fg,
          titleColor: fg,
          gridColor: grid,
          domainColor: grid,
          tickColor: grid,
        },
        legend: { labelColor: fg, titleColor: fg },
      },
    };
  }

  function renderInto(container, points) {
    if (!window.vegaEmbed) {
      return;
    }
    var vlSpec = spec(points);
    // Alt text on the container, mirroring the spec description Vega puts on the
    // rendered SVG, so the chart is labeled for assistive tech either way.
    container.setAttribute("role", "img");
    container.setAttribute("aria-label", vlSpec.description);
    window
      .vegaEmbed(container, vlSpec, { actions: false, renderer: "svg" })
      .catch(function (err) {
        // Leave the fallback table in place; surface the reason for debugging.
        console.error("bench-charts: failed to render", err);
      });
  }

  function init() {
    var blocks = document.querySelectorAll(".bench-chart-block");
    if (!blocks.length) {
      return;
    }
    blocks.forEach(function (block) {
      var container = block.querySelector(".bench-chart");
      var data = block.querySelector("script.bench-data");
      if (!container || !data) {
        return;
      }
      var points;
      try {
        points = JSON.parse(data.textContent);
      } catch (err) {
        console.error("bench-charts: bad data payload", err);
        return;
      }
      if (!Array.isArray(points) || !points.length) {
        return;
      }
      container.__benchPoints = points;
      renderInto(container, points);
    });

    // Re-render on light/dark toggle so axis and legend colors track the theme.
    var observer = new MutationObserver(function () {
      document.querySelectorAll(".bench-chart").forEach(function (container) {
        if (container.__benchPoints) {
          renderInto(container, container.__benchPoints);
        }
      });
    });
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    });
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
