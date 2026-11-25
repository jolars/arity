// Keep the existing measured chart specifications and lazy-load their runtimes.
let runtime;
function loadRuntime() {
  runtime ??= (async () => {
    for (const name of [
      "vega.min.js",
      "vega-lite.min.js",
      "vega-embed.min.js",
    ]) {
      await new Promise((resolve, reject) => {
        const script = document.createElement("script");
        script.src = `/vendor/${name}`;
        script.onload = resolve;
        script.onerror = reject;
        document.head.appendChild(script);
      });
    }
  })();
  return runtime;
}

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

// Label 1, 2, and 5 in each decade, matching Panache's timing charts.
function timeScale(points, field) {
  const values = points.map((point) => point[field]);
  const low = values.length ? Math.floor(Math.log10(Math.min(...values))) : 0;
  const high = values.length ? Math.ceil(Math.log10(Math.max(...values))) : 1;
  const ticks = [];
  for (let exponent = low; exponent < high; exponent++) {
    for (const multiple of [1, 2, 5]) {
      ticks.push(multiple * 10 ** exponent);
    }
  }
  ticks.push(10 ** high);
  return { domain: [10 ** low / 1.1, 10 ** high * 1.1], ticks };
}

function spec(payload) {
  const lsp = !Array.isArray(payload);
  const memory = lsp && payload.kind === "bar";
  const field = lsp ? "value" : "mean_ms";
  const rows = lsp ? payload.points : payload;
  const points = memory ? rows : rows.filter((point) => point[field] > 0);
  const dark = document.documentElement.classList.contains("dark");
  const fg = dark ? "#c8c9db" : "#333333";
  const axisGray = dark ? "#40434d" : "#aaaaaa";
  const gridGray = dark ? "#535764" : "#eeeeee";
  const tools = orderedUnique(points, "tool");
  const documents = orderedUnique(points, "document");
  const valueTitle = lsp ? payload.value_title : "Mean time (ms)";

  const chart = {
    $schema: "https://vega.github.io/schema/vega-lite/v5.json",
    width: "container",
    height: 340,
    data: { values: points },
    config: {
      background: null,
      view: { stroke: null },
      axis: {
        labelColor: fg,
        titleColor: fg,
        gridColor: gridGray,
        domainColor: axisGray,
        tickColor: axisGray,
      },
      legend: { labelColor: fg, titleColor: fg },
    },
  };
  if (memory) {
    return {
      ...chart,
      description:
        "Grouped bar chart of median resident memory in MiB at each session stage. " +
        "Each server has its own bar on a linear axis starting at zero. " +
        "See the data table for PSS and process counts.",
      mark: { type: "bar" },
      encoding: {
        y: {
          field: "document",
          type: "nominal",
          title: "Stage",
          sort: documents,
        },
        yOffset: { field: "tool", type: "nominal", sort: tools },
        x: {
          field,
          type: "quantitative",
          title: valueTitle,
          scale: { type: "linear", zero: true },
        },
        color: { field: "tool", type: "nominal", title: "Server", sort: tools },
        tooltip: [
          { field: "document", title: "Stage" },
          { field: "tool", title: "Server" },
          { field, title: valueTitle, format: ".1f" },
          { field: "detail", title: "Details" },
        ],
      },
    };
  }

  const inputTitle = lsp ? "Measurement" : "Input";
  const toolTitle = lsp ? "Server" : "Tool";
  const { domain, ticks } = timeScale(points, field);
  return {
    ...chart,
    description:
      `${valueTitle} on a logarithmic scale. Lower values mean shorter waits. ` +
      "See the data table for measurements.",
    mark: { type: "point", filled: true, size: 130, opacity: 0.9 },
    encoding: {
      y: {
        field: "document",
        type: "nominal",
        title: inputTitle,
        sort: documents,
        axis: { grid: true },
      },
      x: {
        field,
        type: "quantitative",
        title: valueTitle,
        scale: { type: "log", domain, nice: false },
        axis: {
          values: ticks,
          labelExpr: "format(datum.value, ',~g')",
          labelOverlap: "greedy",
          grid: false,
        },
      },
      color: {
        field: "tool",
        type: "nominal",
        title: toolTitle,
        sort: tools,
        legend: { orient: "top" },
      },
      tooltip: [
        { field: "document", title: inputTitle },
        { field: "tool", title: toolTitle },
        { field, title: valueTitle, format: ".3f" },
        ...(lsp
          ? [{ field: "detail", title: "Details" }]
          : [
              { field: "min_ms", title: "Min (ms)", format: ".3f" },
              { field: "max_ms", title: "Max (ms)", format: ".3f" },
              { field: "stddev_ms", title: "Std dev (ms)", format: ".3f" },
            ]),
      ],
    },
  };
}

export function mountCharts() {
  let active = true;
  const views = [];
  const blocks = [...document.querySelectorAll(".bench-chart-block")];
  const resize = () => views.forEach((view) => view.resize().runAsync());
  if (blocks.length) {
    loadRuntime()
      .then(async () => {
        for (const block of blocks) {
          if (!active) return;
          const container = block.querySelector(".bench-chart");
          const data = block.querySelector(".bench-data");
          if (!container || !data) continue;
          const payload = JSON.parse(data.textContent);
          const chart = spec(payload);
          container.setAttribute("role", "img");
          container.setAttribute("aria-label", chart.description);
          const { view } = await window.vegaEmbed(container, chart, {
            actions: false,
            renderer: "svg",
          });
          if (active) views.push(view);
          else view.finalize();
        }
      })
      .catch((error) => {
        if (!active) return;
        blocks.forEach((block) => {
          const table = block.querySelector(".bench-table");
          if (table) table.open = true;
        });
        console.error("bench-charts: failed to render", error);
      });
    window.addEventListener("resize", resize);
  }
  return () => {
    active = false;
    window.removeEventListener("resize", resize);
    views.forEach((view) => view.finalize());
  };
}
