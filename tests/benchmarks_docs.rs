//! Tests for the benchmark docs renderer (`arity::bench_docs`), the single
//! source of truth shared by the docs generator (`examples/docgen.rs`) that
//! writes the mdBook benchmark partials from `benches/benchmark_results.json`.

use arity::bench_docs::render_partials;

/// A representative artifact: two synthetic tiers, arity as the baseline, plus
/// two optional tools (`air` present, `styler` present) so the renderer's
/// tool-generic path is exercised.
const SAMPLE: &str = r#"{
  "schema_version": 1,
  "meta": {
    "generated_at": "2026-07-10T09:00:00Z",
    "host": {"os": "linux", "arch": "x86_64", "cpu": "Test CPU"},
    "backend": "hyperfine",
    "min_runs": 3,
    "tools": {
      "arity": {"version": "0.4.0"},
      "air": {"version": "0.10.0"},
      "styler": {"version": "1.11.0"}
    }
  },
  "documents": [
    {"id": "small", "name": "small", "file": "small.R", "size_bytes": 1000, "lines": 100, "iterations": 50},
    {"id": "large", "name": "large", "file": "large.R", "size_bytes": 12000, "lines": 1200, "iterations": 5}
  ],
  "results": [
    {"document": "small", "formatter": "arity", "mean_ms": 10.0, "stddev_ms": 1.0, "min_ms": 9.0, "max_ms": 12.0, "runs": 50},
    {"document": "small", "formatter": "air", "mean_ms": 20.0, "stddev_ms": 1.0, "min_ms": 18.0, "max_ms": 22.0, "runs": 50},
    {"document": "small", "formatter": "styler", "mean_ms": 200.0, "stddev_ms": 5.0, "min_ms": 190.0, "max_ms": 210.0, "runs": 5},
    {"document": "large", "formatter": "arity", "mean_ms": 100.0, "stddev_ms": 2.0, "min_ms": 96.0, "max_ms": 104.0, "runs": 5},
    {"document": "large", "formatter": "air", "mean_ms": 80.0, "stddev_ms": 2.0, "min_ms": 76.0, "max_ms": 84.0, "runs": 5}
  ]
}"#;

#[test]
fn meta_lists_every_tool_in_first_appearance_order() {
    let (meta, _) = render_partials(Some(SAMPLE));

    // arity is the baseline and must come first, then air, then styler — the
    // order tools first appear in `results`, not alphabetized.
    let arity = meta.find("arity").expect("arity in meta");
    let air = meta.find("air").expect("air in meta");
    let styler = meta.find("styler").expect("styler in meta");
    assert!(arity < air && air < styler, "tools out of order:\n{meta}");

    // Versions, backend, host, and generation date are all surfaced.
    assert!(meta.contains("0.4.0"), "arity version missing:\n{meta}");
    assert!(meta.contains("0.10.0"), "air version missing:\n{meta}");
    assert!(meta.contains("1.11.0"), "styler version missing:\n{meta}");
    assert!(meta.contains("hyperfine"), "backend missing:\n{meta}");
    assert!(meta.contains("Test CPU"), "host cpu missing:\n{meta}");
    assert!(
        meta.contains("2026-07-10T09:00:00Z"),
        "date missing:\n{meta}"
    );
}

#[test]
fn results_carry_chart_data_and_fallback_table() {
    let (_, results) = render_partials(Some(SAMPLE));

    // The Vega chart scaffolding and its inline data payload.
    assert!(results.contains("bench-chart-block"), "chart block missing");
    assert!(
        results.contains("class=\"bench-data\""),
        "inline data script missing"
    );

    // A no-JS/print fallback table with the raw numbers.
    assert!(results.contains("<table>"), "fallback table missing");
    assert!(results.contains("<details"), "collapsed table missing");

    // The inline JSON payload must contain the computed ratios: air on `small`
    // is 20/10 = 2.0 (2x slower), arity is the baseline at 1.0.
    assert!(results.contains("\"ratio\""), "ratio field missing");
    assert!(results.contains("baseline"), "baseline label missing");
    assert!(
        results.contains("slower"),
        "slower ratio label missing:\n{results}"
    );
    assert!(
        results.contains("faster"),
        "faster ratio label missing (air beats arity on large):\n{results}"
    );
}

#[test]
fn missing_or_bad_json_degrades_to_a_note() {
    let (meta_none, results_none) = render_partials(None);
    assert!(
        meta_none.contains("unavailable"),
        "expected note: {meta_none}"
    );
    assert!(
        results_none.contains("unavailable"),
        "expected note: {results_none}"
    );

    let (meta_bad, _) = render_partials(Some("{ not json"));
    assert!(
        meta_bad.contains("unavailable"),
        "bad json should degrade: {meta_bad}"
    );
}
