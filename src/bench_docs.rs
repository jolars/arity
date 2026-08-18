//! Rendering the mdBook benchmark page from the committed benchmark artifact.
//!
//! [`render_partials`] is the single source of truth shared by the docs
//! generator (`examples/docgen.rs`, which writes the generated partials into
//! the mdBook source tree) and its test (`tests/benchmarks_docs.rs`). It reads
//! the machine-readable `benches/benchmark_results.json` produced by
//! `scripts/bench.sh` and renders two Markdown partials:
//!
//! - a **meta** bullet list (tool versions, timing backend, host, run date);
//! - a **body**: one `###` section per benchmarked operation (formatter,
//!   linter), each holding one `####` chart per scope (single files, projects).
//!   Every chart is an interactive Vega-Lite dot plot (driven by
//!   `docs/theme/bench-charts.js`) plus a collapsed HTML fallback table. The
//!   page includes it under a `## Results` heading, so sections nest as `###`.
//!
//! The renderer is deliberately **tool-generic**: within each chart it treats
//! `arity` as the baseline and every other tool as a comparison point, deriving
//! the tool order from the results themselves. Adding a new comparison tool
//! (e.g. `styler`, `jarl`) to the benchmark artifact therefore needs no change
//! here; nor does adding a section or chart.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// The tool treated as the baseline (ratio `1.0`) within every chart; each
/// other tool's timing is reported relative to it. The benchmark script always
/// lists arity first, so this is also the first tool to appear in the results.
const BASELINE: &str = "arity";

/// The committed benchmark artifact, deserialized straight from
/// `benches/benchmark_results.json`. See `scripts/bench.sh` for the producer and
/// the schema.
#[derive(Deserialize)]
struct Benchmarks {
    meta: Meta,
    /// One section per benchmarked operation (formatter, linter), in the order
    /// they should appear on the page.
    sections: Vec<Section>,
}

#[derive(Deserialize)]
struct Meta {
    generated_at: String,
    host: Host,
    backend: String,
    min_runs: u32,
    /// Version per tool, keyed by tool name. Kept as a free-form map so a new
    /// tool needs no code change; display order hoists the baseline first.
    tools: std::collections::BTreeMap<String, Tool>,
}

#[derive(Deserialize)]
struct Host {
    os: String,
    arch: String,
    cpu: String,
}

#[derive(Deserialize)]
struct Tool {
    version: String,
}

/// One `##` section on the page (e.g. the formatter), holding one chart per
/// scope.
#[derive(Deserialize)]
struct Section {
    title: String,
    charts: Vec<Chart>,
}

/// One `###` chart within a section (e.g. single files, or projects). Each chart
/// is self-contained: its own documents, its own results, and its own arity
/// baseline, so ratios never cross charts.
#[derive(Deserialize)]
struct Chart {
    title: String,
    /// The `<figcaption>` prose (verb differs by operation: "Formatting speed…"
    /// vs "Linting speed…"), supplied by the producing script.
    caption: String,
    documents: Vec<Document>,
    results: Vec<BenchResult>,
}

#[derive(Deserialize)]
struct Document {
    id: String,
    name: String,
    size_bytes: u64,
    lines: u64,
}

#[derive(Deserialize)]
struct BenchResult {
    document: String,
    tool: String,
    mean_ms: f64,
    stddev_ms: Option<f64>,
    min_ms: Option<f64>,
    max_ms: Option<f64>,
}

/// One dot in a chart: a (document, tool) timing, its ratio to the arity
/// baseline, and the numbers the tooltip shows. Serialized inline into the page
/// for `docs/theme/bench-charts.js` to plot with Vega-Lite.
#[derive(Serialize)]
struct ChartPoint {
    document: String,
    tool: String,
    mean_ms: f64,
    ratio: f64,
    ratio_label: String,
    stddev_ms: Option<f64>,
    min_ms: Option<f64>,
    max_ms: Option<f64>,
}

/// Render the two benchmark partials `(meta, body)` from the artifact JSON.
///
/// `json` is the contents of `benches/benchmark_results.json`, or `None` when
/// the artifact is missing. On a missing or unparseable artifact both partials
/// degrade to an "unavailable" note, so `mdbook build` never breaks on a fresh
/// checkout that has not run `task bench` yet.
pub fn render_partials(json: Option<&str>) -> (String, String) {
    match json.and_then(|s| serde_json::from_str::<Benchmarks>(s).ok()) {
        Some(b) => (render_meta(&b.meta), render_body(&b)),
        None => {
            let note = "_Benchmark data unavailable \
                        (`benches/benchmark_results.json` missing or unreadable; \
                        run `task bench`)._\n"
                .to_string();
            (note.clone(), note)
        }
    }
}

/// A Markdown bullet list of tool versions, timing backend, host, and run date.
fn render_meta(meta: &Meta) -> String {
    let mut out = String::new();
    // Order arity-first, then alphabetically. The per-chart appearance order
    // isn't available from `meta` alone, so hoist the baseline and fall back to
    // the map's own (sorted) key order for the rest.
    let mut names: Vec<&String> = meta.tools.keys().collect();
    names.sort_by_key(|n| (*n != BASELINE, (*n).clone()));
    for name in names {
        let version = &meta.tools[name].version;
        let _ = writeln!(out, "- **{name}**: `{version}`");
    }
    let _ = writeln!(
        out,
        "- **backend**: {} (min runs: {})",
        meta.backend, meta.min_runs
    );
    let _ = writeln!(
        out,
        "- **host**: {}/{}, {}",
        meta.host.os, meta.host.arch, meta.host.cpu
    );
    let _ = writeln!(out, "- **generated**: {}", meta.generated_at);
    out
}

/// The page body: one `###` section per operation, each with one `####` chart
/// per scope. It is included under the page's `## Results` heading, so sections
/// nest one level below it. A section or chart carrying no results is skipped so
/// a partial run (e.g. jarl absent) leaves no empty headers.
fn render_body(b: &Benchmarks) -> String {
    let mut out = String::new();
    for section in &b.sections {
        let charts: Vec<&Chart> = section
            .charts
            .iter()
            .filter(|c| !c.results.is_empty())
            .collect();
        if charts.is_empty() {
            continue;
        }
        let _ = writeln!(out, "### {}\n", section.title);
        for chart in charts {
            let _ = writeln!(out, "#### {}\n", chart.title);
            out.push_str(&render_chart(chart));
            out.push('\n');
        }
    }
    if out.is_empty() {
        out.push_str("_No benchmark results recorded._\n");
    }
    out
}

/// One chart: an interactive Vega-Lite dot plot (driven by
/// `docs/theme/bench-charts.js`, wired via `book.toml`'s `additional-js`) plus a
/// collapsed HTML table with the same numbers as a no-JS/print fallback.
///
/// The chart data rides inline in a `<script type="application/json">`; the JS
/// plots time-relative-to-arity on a log axis, one dot per (document, tool).
/// Kept as raw HTML (not a Markdown pipe table) so the fallback renders inside
/// `<details>`.
fn render_chart(chart: &Chart) -> String {
    let points = chart_points(chart);
    let data_json = serde_json::to_string(&points).unwrap_or_else(|_| "[]".to_string());

    let mut out = String::new();
    out.push_str("<div class=\"bench-chart-block\">\n");
    out.push_str("<figure class=\"bench-figure\">\n");
    out.push_str("<div class=\"bench-chart\"></div>\n");
    out.push_str("<script type=\"application/json\" class=\"bench-data\">");
    out.push_str(&data_json);
    out.push_str("</script>\n");
    let _ = writeln!(out, "<figcaption>{}</figcaption>", esc(&chart.caption));
    out.push_str("</figure>\n");
    out.push_str(
        "<noscript>Enable JavaScript for the interactive chart; \
         the data table below has the same numbers.</noscript>\n",
    );
    out.push_str("<details class=\"bench-table\">\n<summary>Data table</summary>\n");
    out.push_str(&render_chart_tables_html(chart));
    out.push_str("</details>\n");
    out.push_str("</div>\n");
    out
}

/// One dot per (document, tool): its mean time as a ratio to that document's
/// arity baseline (arity itself is `1.0`), in document order. Documents whose
/// baseline is missing or non-positive are skipped (they carry no meaningful
/// ratio); they still appear in the fallback table.
fn chart_points(chart: &Chart) -> Vec<ChartPoint> {
    let mut points = Vec::new();
    for doc in &chart.documents {
        let base = baseline_mean(chart, &doc.id);
        let Some(base) = base.filter(|&b| b > 0.0) else {
            continue;
        };
        for r in chart.results.iter().filter(|r| r.document == doc.id) {
            let ratio_label = if r.tool == BASELINE {
                "baseline".to_string()
            } else {
                relative_cell(r.mean_ms, Some(base))
            };
            points.push(ChartPoint {
                document: doc.name.clone(),
                tool: r.tool.clone(),
                mean_ms: r.mean_ms,
                ratio: r.mean_ms / base,
                ratio_label,
                stddev_ms: r.stddev_ms,
                min_ms: r.min_ms,
                max_ms: r.max_ms,
            });
        }
    }
    points
}

/// One `<h5>` + HTML `<table>` per document in a chart, in document order; rows
/// follow the order tools appear in `results`. `arity` is the baseline and every
/// other tool's `Relative` cell is its mean ratio to it.
fn render_chart_tables_html(chart: &Chart) -> String {
    let order = tool_order(&chart.results);
    let mut out = String::new();
    for doc in &chart.documents {
        let base = baseline_mean(chart, &doc.id);

        let _ = writeln!(
            out,
            "<h5>{} ({} bytes, {} lines)</h5>",
            esc(&doc.name),
            doc.size_bytes,
            doc.lines
        );
        out.push_str(
            "<table>\n<thead><tr><th>Tool</th><th>Mean (ms)</th>\
             <th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>\n<tbody>\n",
        );
        // Walk tools in artifact order so rows read arity-first, skipping any a
        // tool did not run on this document.
        for tool in &order {
            let Some(r) = chart
                .results
                .iter()
                .find(|r| r.document == doc.id && &r.tool == tool)
            else {
                continue;
            };
            let relative = if r.tool == BASELINE {
                "baseline".to_string()
            } else {
                relative_cell(r.mean_ms, base)
            };
            let _ = writeln!(
                out,
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                esc(&r.tool),
                fmt_ms(Some(r.mean_ms)),
                fmt_ms(r.min_ms),
                fmt_ms(r.max_ms),
                esc(&relative),
            );
        }
        out.push_str("</tbody>\n</table>\n");
    }
    out
}

/// Tools in the order they first appear across a chart's `results` (arity
/// first). This is the row order for the fallback table, matching the chart's
/// axis order.
fn tool_order(results: &[BenchResult]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut order = Vec::new();
    for r in results {
        if seen.insert(r.tool.clone()) {
            order.push(r.tool.clone());
        }
    }
    order
}

/// The arity baseline mean for a document within a chart, if arity was measured
/// on it.
fn baseline_mean(chart: &Chart, doc_id: &str) -> Option<f64> {
    chart
        .results
        .iter()
        .find(|r| r.document == doc_id && r.tool == BASELINE)
        .map(|r| r.mean_ms)
}

/// Minimal HTML text escaping for the fallback table's cell text.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Format a millisecond figure to four decimals, or an em dash when absent
/// (the shell-loop fallback reports no min/max).
fn fmt_ms(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{x:.4}"),
        None => "—".to_string(),
    }
}

/// Human ratio of a tool's mean to the arity baseline.
fn relative_cell(tool_mean: f64, base: Option<f64>) -> String {
    match base {
        Some(b) if b > 0.0 && tool_mean > 0.0 => {
            let r = tool_mean / b;
            if r >= 1.0 {
                format!("{r:.1}x slower")
            } else {
                format!("{:.1}x faster", 1.0 / r)
            }
        }
        _ => "—".to_string(),
    }
}
