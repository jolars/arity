//! Render opt-in LSP measurements from the shared benchmark artifact.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use super::{Host, esc};

#[derive(Deserialize)]
struct Artifact {
    lsp: Measurements,
}

#[derive(Deserialize)]
struct Measurements {
    meta: Meta,
    corpus: Corpus,
    session: Session,
    servers: Vec<Server>,
}

#[derive(Deserialize)]
struct Meta {
    generated_at: String,
    host: Host,
    runs: usize,
    edit_count: usize,
    latency_runs: usize,
    latency_warmups: usize,
    quiet_seconds: f64,
    sample_interval_seconds: f64,
}

#[derive(Deserialize)]
struct Corpus {
    name: String,
    revision: String,
}

#[derive(Deserialize)]
struct Session {
    files: Vec<String>,
    opened_bytes: u64,
}

#[derive(Deserialize)]
struct Server {
    key: String,
    version: String,
    aggregate: Aggregate,
}

#[derive(Deserialize)]
struct Aggregate {
    timings: BTreeMap<String, f64>,
    milestones: BTreeMap<String, Memory>,
    request_latencies: Vec<Latency>,
}

#[derive(Deserialize)]
struct Memory {
    rss_mb: f64,
    pss_mb: f64,
    processes: usize,
}

#[derive(Deserialize)]
struct Latency {
    key: String,
    label: String,
    median_ms: f64,
    p95_ms: f64,
    samples: usize,
    empty_results: usize,
    result_count_min: usize,
    result_count_max: usize,
    payload_bytes_min: usize,
    payload_bytes_max: usize,
    #[serde(default)]
    stale_responses: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum ChartKind {
    Ratio,
    Bar,
}

#[derive(Serialize)]
struct Point {
    document: String,
    tool: String,
    value: f64,
    ratio: f64,
    ratio_label: String,
    detail: String,
}

/// Render the LSP partial independently of the CLI measurements' date and host.
/// Missing data does not prevent a documentation build.
pub fn render_lsp_partial(json: Option<&str>) -> String {
    let Some(artifact) = json.and_then(|s| serde_json::from_str::<Artifact>(s).ok()) else {
        return "_LSP benchmark data unavailable; run `task bench-lsp` and `task docs-gen`._\n"
            .to_string();
    };
    render(&artifact.lsp)
}

fn render(b: &Measurements) -> String {
    let mut out = String::from("### Measurement setup\n\n");
    let meta = &b.meta;
    let _ = writeln!(out, "- **Generated**: {}", esc(&meta.generated_at));
    let _ = writeln!(
        out,
        "- **Host**: {}/{}, {}",
        esc(&meta.host.os),
        esc(&meta.host.arch),
        esc(&meta.host.cpu)
    );
    let _ = writeln!(
        out,
        "- **Corpus**: {} at `{}`",
        esc(&b.corpus.name),
        esc(&b.corpus.revision)
    );
    let _ = writeln!(
        out,
        "- **Open buffers**: {} files, {} bytes including benchmark bindings",
        b.session.files.len(),
        b.session.opened_bytes
    );
    let _ = writeln!(
        out,
        "- **Workload**: {} fresh processes per server, {} edits per process",
        meta.runs, meta.edit_count
    );
    let _ = writeln!(
        out,
        "- **Warm requests**: {} warmups and {} measured rounds per target per process",
        meta.latency_warmups, meta.latency_runs
    );
    let _ = writeln!(
        out,
        "- **Settling**: {} seconds below 5% of one CPU core, sampled every {} seconds",
        meta.quiet_seconds, meta.sample_interval_seconds
    );
    for server in &b.servers {
        let _ = writeln!(
            out,
            "- **{}**: `{}`",
            esc(&server.key),
            esc(&server.version)
        );
    }
    out.push_str("\nOpened files: ");
    out.push_str(
        &b.session
            .files
            .iter()
            .map(|file| format!("`{}`", esc(file)))
            .collect::<Vec<_>>()
            .join(", "),
    );
    out.push_str(".\n\n");

    let Some(baseline) = b.servers.iter().find(|s| s.key == "arity") else {
        out.push_str("_Arity baseline unavailable._\n");
        return out;
    };
    let mut runtime = Vec::new();
    for (key, label) in [
        ("init_seconds", "Initialize"),
        ("workspace_ready_seconds", "Workspace ready"),
        ("documents_ready_seconds", "Open files ready"),
        ("edit_work_seconds", "Edit workload"),
    ] {
        for server in &b.servers {
            if let (Some(value), Some(base)) = (
                server.aggregate.timings.get(key),
                baseline.aggregate.timings.get(key),
            ) {
                push_point(
                    &mut runtime,
                    server,
                    label,
                    value * 1000.0,
                    base * 1000.0,
                    "Median across fresh processes".into(),
                );
            }
        }
    }
    out.push_str(&chart(
        "Startup and workload",
        "Median (ms)",
        "Time relative to arity",
        ChartKind::Ratio,
        runtime,
    ));

    let mut latency = Vec::new();
    for base in &baseline.aggregate.request_latencies {
        for server in &b.servers {
            if let Some(value) = server
                .aggregate
                .request_latencies
                .iter()
                .find(|r| r.key == base.key)
            {
                let detail = format!(
                    "p95: {:.3} ms; samples: {}; empty: {}; returned items: {}–{}; result bytes: {}–{}; stale responses: {}",
                    value.p95_ms,
                    value.samples,
                    value.empty_results,
                    value.result_count_min,
                    value.result_count_max,
                    value.payload_bytes_min,
                    value.payload_bytes_max,
                    value.stale_responses,
                );
                push_point(
                    &mut latency,
                    server,
                    &value.label,
                    value.median_ms,
                    base.median_ms,
                    detail,
                );
            }
        }
    }
    out.push_str(&chart(
        "Request latency",
        "Median (ms)",
        "Latency relative to arity",
        ChartKind::Ratio,
        latency,
    ));

    let mut memory = Vec::new();
    for (key, label) in [
        ("baseline", "Initialized"),
        ("settled", "Files open"),
        ("edited", "After edits"),
        ("peak", "Sampled peak"),
    ] {
        for server in &b.servers {
            if let (Some(value), Some(base)) = (
                server.aggregate.milestones.get(key),
                baseline.aggregate.milestones.get(key),
            ) {
                push_point(
                    &mut memory,
                    server,
                    label,
                    value.rss_mb,
                    base.rss_mb,
                    format!(
                        "PSS: {:.1} MiB; processes: {}",
                        value.pss_mb, value.processes
                    ),
                );
            }
        }
    }
    out.push_str(&chart(
        "Resident memory",
        "Median RSS (MiB)",
        "Memory relative to arity",
        ChartKind::Bar,
        memory,
    ));
    format!("{}\n", out.trim_end())
}

fn push_point(
    points: &mut Vec<Point>,
    server: &Server,
    label: &str,
    value: f64,
    base: f64,
    detail: String,
) {
    if base <= 0.0 || value <= 0.0 {
        return;
    }
    let ratio = value / base;
    points.push(Point {
        document: label.into(),
        tool: server.key.clone(),
        value,
        ratio,
        ratio_label: if server.key == "arity" {
            "baseline".into()
        } else {
            format!("{ratio:.2}× arity")
        },
        detail,
    });
}

fn chart(
    title: &str,
    value_title: &str,
    ratio_title: &str,
    kind: ChartKind,
    points: Vec<Point>,
) -> String {
    let payload = serde_json::json!({
        "points": points, "value_title": value_title, "ratio_title": ratio_title,
        "kind": kind,
    });
    let caption = match kind {
        ChartKind::Ratio => format!(
            "{ratio_title}. Hover a point for measurements and sample details. Lower values use less time."
        ),
        ChartKind::Bar => format!(
            "{value_title} at each session stage, with one bar per server on a linear scale starting at zero. Hover a bar for relative memory use, PSS, and process counts."
        ),
    };
    // Corpus and version strings can contain HTML; JSON lives in a script element.
    let json = payload.to_string().replace('<', "\\u003c");
    let mut out = format!(
        "### {title}\n\n<div class=\"bench-chart-block\">\n<figure class=\"bench-figure\">\n<div class=\"bench-chart\"></div>\n<script type=\"application/json\" class=\"bench-data\">{json}</script>\n<figcaption>{caption}</figcaption>\n</figure>\n<details class=\"bench-table\"><summary>Data table</summary>\n<table>\n<thead><tr><th>Measurement</th><th>Server</th><th>{value_title}</th><th>Relative</th><th>Details</th></tr></thead>\n<tbody>\n"
    );
    for point in points {
        let _ = writeln!(
            out,
            "<tr><td>{}</td><td>{}</td><td>{:.3}</td><td>{}</td><td>{}</td></tr>",
            esc(&point.document),
            esc(&point.tool),
            point.value,
            esc(&point.ratio_label),
            esc(&point.detail)
        );
    }
    out.push_str("</tbody>\n</table>\n</details>\n</div>\n\n");
    out
}
