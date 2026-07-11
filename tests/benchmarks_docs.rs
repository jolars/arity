//! Tests for the benchmark docs renderer (`arity::bench_docs`), the single
//! source of truth shared by the docs generator (`examples/docgen.rs`) that
//! writes the mdBook benchmark partials from `benches/benchmark_results.json`.

use arity::bench_docs::render_partials;

/// A representative v2 artifact: a formatter section (single-file tiers vs
/// `air`/`styler`, plus a project) and a linter section (single files vs
/// `jarl`), with `arity` the baseline throughout, so the renderer's
/// tool-generic, multi-section, multi-chart path is exercised.
const SAMPLE: &str = r#"{
  "schema_version": 2,
  "meta": {
    "generated_at": "2026-07-10T09:00:00Z",
    "host": {"os": "linux", "arch": "x86_64", "cpu": "Test CPU"},
    "backend": "hyperfine",
    "min_runs": 3,
    "tools": {
      "arity": {"version": "0.4.0"},
      "air": {"version": "0.10.0"},
      "styler": {"version": "1.11.0"},
      "jarl": {"version": "0.5.0"}
    }
  },
  "sections": [
    {
      "id": "formatter",
      "title": "Formatter",
      "charts": [
        {
          "id": "formatter-files",
          "title": "Single files",
          "caption": "Formatting speed relative to arity on single files.",
          "documents": [
            {"id": "small", "name": "small", "size_bytes": 1000, "lines": 100},
            {"id": "large", "name": "large", "size_bytes": 12000, "lines": 1200}
          ],
          "results": [
            {"document": "small", "tool": "arity", "mean_ms": 10.0, "stddev_ms": 1.0, "min_ms": 9.0, "max_ms": 12.0, "runs": 50},
            {"document": "small", "tool": "air", "mean_ms": 20.0, "stddev_ms": 1.0, "min_ms": 18.0, "max_ms": 22.0, "runs": 50},
            {"document": "small", "tool": "styler", "mean_ms": 200.0, "stddev_ms": 5.0, "min_ms": 190.0, "max_ms": 210.0, "runs": 5},
            {"document": "large", "tool": "arity", "mean_ms": 100.0, "stddev_ms": 2.0, "min_ms": 96.0, "max_ms": 104.0, "runs": 5},
            {"document": "large", "tool": "air", "mean_ms": 80.0, "stddev_ms": 2.0, "min_ms": 76.0, "max_ms": 84.0, "runs": 5}
          ]
        },
        {
          "id": "formatter-projects",
          "title": "Projects",
          "caption": "Formatting speed relative to arity on a real R package.",
          "documents": [
            {"id": "tidyr", "name": "tidyr", "size_bytes": 500000, "lines": 20000}
          ],
          "results": [
            {"document": "tidyr", "tool": "arity", "mean_ms": 300.0, "stddev_ms": 5.0, "min_ms": 290.0, "max_ms": 310.0, "runs": 10},
            {"document": "tidyr", "tool": "air", "mean_ms": 250.0, "stddev_ms": 5.0, "min_ms": 240.0, "max_ms": 260.0, "runs": 10}
          ]
        }
      ]
    },
    {
      "id": "linter",
      "title": "Linter",
      "charts": [
        {
          "id": "linter-files",
          "title": "Single files",
          "caption": "Linting speed relative to arity on single files.",
          "documents": [
            {"id": "small", "name": "small", "size_bytes": 1000, "lines": 100}
          ],
          "results": [
            {"document": "small", "tool": "arity", "mean_ms": 15.0, "stddev_ms": 1.0, "min_ms": 14.0, "max_ms": 16.0, "runs": 50},
            {"document": "small", "tool": "jarl", "mean_ms": 30.0, "stddev_ms": 1.0, "min_ms": 28.0, "max_ms": 32.0, "runs": 50}
          ]
        }
      ]
    }
  ]
}"#;

#[test]
fn meta_lists_every_tool_baseline_first() {
    let (meta, _) = render_partials(Some(SAMPLE));

    // arity is the baseline and must come first; the rest fall back to sorted
    // key order (air, jarl, styler).
    let arity = meta.find("arity").expect("arity in meta");
    let air = meta.find("air").expect("air in meta");
    assert!(arity < air, "arity must lead the meta list:\n{meta}");

    // Every tool version, the backend, host, and generation date are surfaced.
    assert!(meta.contains("0.4.0"), "arity version missing:\n{meta}");
    assert!(meta.contains("0.10.0"), "air version missing:\n{meta}");
    assert!(meta.contains("1.11.0"), "styler version missing:\n{meta}");
    assert!(meta.contains("0.5.0"), "jarl version missing:\n{meta}");
    assert!(meta.contains("hyperfine"), "backend missing:\n{meta}");
    assert!(meta.contains("Test CPU"), "host cpu missing:\n{meta}");
    assert!(
        meta.contains("2026-07-10T09:00:00Z"),
        "date missing:\n{meta}"
    );
}

#[test]
fn body_has_a_section_per_operation_and_a_chart_per_scope() {
    let (_, body) = render_partials(Some(SAMPLE));

    // Section headers (### ) and chart headers (#### ) for both operations;
    // the body nests under the page's `## Results` heading.
    assert!(body.contains("### Formatter"), "formatter section missing");
    assert!(body.contains("### Linter"), "linter section missing");
    assert!(body.contains("#### Single files"), "files chart missing");
    assert!(body.contains("#### Projects"), "projects chart missing");

    // Three charts total (formatter x2 + linter x1) => three chart blocks.
    assert_eq!(
        body.matches("bench-chart-block").count(),
        3,
        "expected three chart blocks:\n{body}"
    );

    // Per-chart caption prose distinguishes the operations.
    assert!(
        body.contains("Formatting speed"),
        "formatter caption missing"
    );
    assert!(body.contains("Linting speed"), "linter caption missing");
}

#[test]
fn charts_carry_data_and_fallback_tables() {
    let (_, body) = render_partials(Some(SAMPLE));

    assert!(
        body.contains("class=\"bench-data\""),
        "inline data script missing"
    );
    assert!(body.contains("<table>"), "fallback table missing");
    assert!(body.contains("<details"), "collapsed table missing");
    assert!(body.contains("tidyr"), "project document missing");

    // Ratios: air on `small` is 20/10 = 2x slower, arity is the baseline, and
    // air beats arity on the `large` tier (80/100).
    assert!(body.contains("\"ratio\""), "ratio field missing");
    assert!(body.contains("baseline"), "baseline label missing");
    assert!(body.contains("slower"), "slower label missing:\n{body}");
    assert!(body.contains("faster"), "faster label missing:\n{body}");

    // jarl appears as the linter comparison tool.
    assert!(
        body.contains("jarl"),
        "jarl missing from linter chart:\n{body}"
    );
}

#[test]
fn missing_or_bad_json_degrades_to_a_note() {
    let (meta_none, body_none) = render_partials(None);
    assert!(
        meta_none.contains("unavailable"),
        "expected note: {meta_none}"
    );
    assert!(
        body_none.contains("unavailable"),
        "expected note: {body_none}"
    );

    let (meta_bad, _) = render_partials(Some("{ not json"));
    assert!(
        meta_bad.contains("unavailable"),
        "bad json should degrade: {meta_bad}"
    );
}
