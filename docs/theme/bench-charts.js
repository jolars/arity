// Renders benchmark time ratios and memory bars with Vega-Lite.
//
// Data is injected by the docs generator (`arity::bench_docs`, via
// `examples/docgen.rs`) as an inline
// `<script type="application/json" class="bench-data">` next to a
// `<div class="bench-chart">`. The Vega runtime is vendored under docs/src/vendor/
// and loaded from the book's vendor/ directory only when charts are present.
//
// Chart: x = tool, y = time relative to arity (log scale, baseline = 1),
// color = document (corpus tier or project), one dot per (document, tool), with
// a hover tooltip. One such chart per (operation, scope): formatter/linter x
// single-files/projects. Memory uses grouped bars of absolute MiB on a linear axis.
(function () {
  "use strict";

  const vendorRoot = new URL("../vendor/", document.currentScript.src);

  function loadScript(name) {
    return new Promise(function (resolve, reject) {
      const script = document.createElement("script");
      script.src = new URL(name, vendorRoot).href;
      script.onload = resolve;
      script.onerror = reject;
      document.head.appendChild(script);
    });
  }

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

  // Whole decades keep the log scale easy to read; a small margin keeps dots
  // at the domain boundaries clear of the plot edges.
  function logDomain(points, field) {
    var values = points
      .map(function (p) {
        return p[field];
      })
      .filter(function (value) {
        return Number.isFinite(value) && value > 0;
      });
    var lo = Math.floor(Math.log10(Math.min.apply(null, values.concat([1]))));
    var hi = Math.ceil(Math.log10(Math.max.apply(null, values.concat([1]))));
    if (lo === hi) {
      lo--;
      hi++;
    }
    return [Math.pow(10, lo) / 1.1, Math.pow(10, hi) * 1.1];
  }

  // Match ggplot2's log ticks: long at powers of ten, medium at five, and
  // short at the other subdivisions. Only powers of ten receive labels.
  function logAxis(domain, color) {
    var ticks = [];
    var major = [];
    var middle = [];
    for (
      var e = Math.floor(Math.log10(domain[0]));
      e <= Math.ceil(Math.log10(domain[1]));
      e++
    ) {
      for (var m = 1; m < 10; m++) {
        var value = m * Math.pow(10, e);
        if (value >= domain[0] && value <= domain[1]) {
          ticks.push(value);
          if (m === 1) major.push(value);
          if (m === 5) middle.push(value);
        }
      }
    }
    var isMajor = "indexof(" + JSON.stringify(major) + ", datum.value) >= 0";
    var isMiddle = "indexof(" + JSON.stringify(middle) + ", datum.value) >= 0";
    return {
      values: ticks,
      labelExpr: isMajor + " ? format(datum.value, ',~g') : ''",
      labelOverlap: false,
      labelPadding: 4,
      tickColor: color,
      tickSize: {
        condition: [
          { test: isMajor, value: 9 },
          { test: isMiddle, value: 6 },
        ],
        value: 3,
      },
      grid: true,
      gridOpacity: {
        condition: { test: isMajor + " && datum.value !== 1", value: 1 },
        value: 0,
      },
    };
  }

  function spec(payload) {
    var lsp = !Array.isArray(payload);
    var points = lsp ? payload.points : payload;
    var dark = isDark();
    var fg = dark ? "#c8c9db" : "#333333";
    var grid = dark ? "#3b3f5c" : "#dddddd";
    var tools = orderedUnique(points, "tool");
    var documents = orderedUnique(points, "document");
    var domain = logDomain(points, "ratio");

    var chart = {
      $schema: "https://vega.github.io/schema/vega-lite/v5.json",
      description: lsp
        ? payload.ratio_title +
          ". Lower values use less time or memory. See the data table for measurements and sample details."
        : "Dot plot of speed relative to arity. Each dot is one input " +
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
          mark: { type: "rule", strokeDash: [4, 4], color: grid, aria: false },
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
              title: lsp ? payload.ratio_title : "Time relative to arity",
              scale: { type: "log", domain: domain, nice: false },
              axis: logAxis(domain, fg),
            },
            color: {
              field: "document",
              type: "nominal",
              title: lsp ? "Measurement" : "Input",
              sort: documents,
            },
            tooltip: lsp
              ? [
                  { field: "document", title: "Measurement" },
                  { field: "tool", title: "Server" },
                  { field: "value", title: payload.value_title, format: ".3f" },
                  { field: "ratio_label", title: "Relative" },
                  { field: "detail", title: "Details" },
                ]
              : [
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
    if (lsp && payload.kind === "bar") {
      chart.description =
        "Grouped bar chart of median resident memory in MiB at each session stage. " +
        "Each server has its own bar on a linear axis starting at zero. " +
        "See the data table for PSS, process counts, and relative memory use.";
      delete chart.layer;
      chart.mark = { type: "bar" };
      chart.encoding = {
        x: {
          field: "document",
          type: "nominal",
          title: "Stage",
          sort: documents,
          axis: { labelAngle: 0 },
        },
        xOffset: { field: "tool", type: "nominal", sort: tools },
        y: {
          field: "value",
          type: "quantitative",
          title: payload.value_title,
          scale: { type: "linear", zero: true },
        },
        color: { field: "tool", type: "nominal", title: "Server", sort: tools },
        tooltip: [
          { field: "document", title: "Stage" },
          { field: "tool", title: "Server" },
          { field: "value", title: payload.value_title, format: ".1f" },
          { field: "ratio_label", title: "Relative" },
          { field: "detail", title: "Details" },
        ],
      };
    }
    return chart;
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

  async function init() {
    var blocks = document.querySelectorAll(".bench-chart-block");
    if (!blocks.length) {
      return;
    }
    try {
      // The libraries depend on one another, so preserve their load order.
      await loadScript("vega.min.js");
      await loadScript("vega-lite.min.js");
      await loadScript("vega-embed.min.js");
    } catch (err) {
      // Measurements remain available if a runtime cannot be loaded.
      document.querySelectorAll(".bench-table").forEach(function (table) {
        table.open = true;
      });
      console.error("bench-charts: failed to load the chart runtime", err);
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
      var rows = Array.isArray(points) ? points : points.points;
      if (!Array.isArray(rows) || !rows.length) {
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
