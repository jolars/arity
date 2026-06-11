//! Soft air-compatibility gauge (NOT a quality gate).
//!
//! This harness measures how much of arity's output the `air` formatter would
//! leave unchanged --- a one-directional "arity is air-compatible" signal in the
//! spirit of ruff's "% Black-compatible" number. It is **subordinate to Tenet 1**
//! (deterministic, rule-based formatting): a divergence from air is never a bug
//! by itself, and this test never fails the build. It is `#[ignore]`d so it does
//! not run in `cargo test`; invoke it explicitly:
//!
//! ```sh
//! task air-compat
//! # or
//! cargo test --test air_compat -- --ignored --nocapture
//! ```
//!
//! Methodology --- we measure the *fixed point*, `air(arity(x)) == arity(x)`, not
//! a head-to-head `air(x)` vs `arity(x)`. That is deliberate: arity ignores the
//! input's line breaks while air honors them ("persistent line breaks"), so a
//! head-to-head diff would be dominated by that intended philosophical difference
//! rather than genuine rule divergences. Because arity's collapsed output never
//! carries air's persistent-break trigger, the fixed-point check cancels the
//! line-break difference out by construction and surfaces only real rule gaps.
//!
//! Corpus --- defaults to the formatter fixtures' `expected.R` files (an
//! adversarial, edge-case set; the headline number will read low and is not
//! representative of real-world R). Point `AIR_COMPAT_CORPUS` at a directory of
//! real `.R` files for a meaningful headline.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use arity::formatter::format;

/// One corpus file's outcome.
enum Outcome {
    /// `air` would leave arity's output unchanged.
    Compatible,
    /// `air` would rewrite arity's output. Carries the per-file line similarity
    /// (Dice coefficient over lines, 0.0..=1.0).
    Divergent { line_similarity: f64 },
    /// arity could not format the input (parse error / unsupported).
    SkippedArity,
    /// `air` could not process arity's output (parse error in air).
    SkippedAir,
}

struct FileReport {
    key: String,
    outcome: Outcome,
}

#[test]
#[ignore = "soft air-compat gauge; run via `task air-compat`"]
fn air_compat_report() {
    let Some(air) = locate_air() else {
        eprintln!("air-compat: `air` binary not found on PATH; skipping (this is not a failure).");
        return;
    };

    let corpus = collect_corpus();
    if corpus.is_empty() {
        eprintln!("air-compat: empty corpus; nothing to measure.");
        return;
    }

    let allowlist = load_allowlist();
    let tmp = tempfile::tempdir().expect("create tempdir");

    let mut reports: Vec<FileReport> = Vec::new();
    // Aggregate line counts for the corpus-wide similarity index.
    let mut total_lcs2: usize = 0; // sum of 2 * LCS
    let mut total_lines: usize = 0; // sum of (arity_lines + air_lines) over divergent+compatible files

    for (key, path) in &corpus {
        let raw = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(_) => continue,
        };

        let arity_out = match format(&raw) {
            Ok(out) => out,
            Err(_) => {
                reports.push(FileReport {
                    key: key.clone(),
                    outcome: Outcome::SkippedArity,
                });
                continue;
            }
        };

        let Some(air_out) = air_format(&air, tmp.path(), &arity_out) else {
            reports.push(FileReport {
                key: key.clone(),
                outcome: Outcome::SkippedAir,
            });
            continue;
        };

        let arity_lines: Vec<&str> = arity_out.lines().collect();
        let air_lines: Vec<&str> = air_out.lines().collect();
        let lcs = lcs_len(&arity_lines, &air_lines);
        total_lcs2 += 2 * lcs;
        total_lines += arity_lines.len() + air_lines.len();

        if arity_out == air_out {
            reports.push(FileReport {
                key: key.clone(),
                outcome: Outcome::Compatible,
            });
        } else {
            let denom = arity_lines.len() + air_lines.len();
            let line_similarity = if denom == 0 {
                1.0
            } else {
                (2 * lcs) as f64 / denom as f64
            };
            reports.push(FileReport {
                key: key.clone(),
                outcome: Outcome::Divergent { line_similarity },
            });
        }
    }

    let report = render_report(
        &reports,
        &allowlist,
        total_lcs2,
        total_lines,
        &corpus_label(),
    );
    print!("{report}");

    let out_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("AIR_COMPAT.md");
    fs::write(&out_path, &report).expect("write AIR_COMPAT.md");
    eprintln!("air-compat: wrote {}", out_path.display());
}

// --- corpus ---------------------------------------------------------------

fn corpus_label() -> String {
    match std::env::var("AIR_COMPAT_CORPUS") {
        Ok(dir) => format!("custom (`AIR_COMPAT_CORPUS={dir}`)"),
        Err(_) => "formatter fixtures (`tests/fixtures/formatter/*/expected.R`)".to_string(),
    }
}

/// Returns `(identifier, path)` pairs. The identifier is the allowlist key:
/// the fixture directory name for the default corpus, or the corpus-relative
/// path for a custom corpus.
fn collect_corpus() -> Vec<(String, PathBuf)> {
    if let Ok(dir) = std::env::var("AIR_COMPAT_CORPUS") {
        let root = PathBuf::from(&dir);
        let mut files = Vec::new();
        collect_r_files(&root, &root, &mut files);
        files.sort();
        return files;
    }

    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("formatter");
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(&fixtures) {
        for entry in entries.flatten() {
            let expected = entry.path().join("expected.R");
            if expected.is_file() {
                let key = entry.file_name().to_string_lossy().into_owned();
                files.push((key, expected));
            }
        }
    }
    files.sort();
    files
}

fn collect_r_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_r_files(root, &path, out);
        } else if path.extension().is_some_and(|e| e == "R" || e == "r") {
            let key = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            out.push((key, path));
        }
    }
}

// --- air invocation -------------------------------------------------------

fn locate_air() -> Option<PathBuf> {
    // Honor an explicit override, else trust PATH resolution by `air`.
    if let Ok(p) = std::env::var("AIR_BIN") {
        return Some(PathBuf::from(p));
    }
    let ok = Command::new("air")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    ok.then(|| PathBuf::from("air"))
}

/// Format `input` with `air` and return its output, or `None` if air failed.
/// `air format` rewrites in place, so we write to a temp `.R` file under a
/// directory with no `air.toml` (air defaults: line-width 80, persistent breaks
/// respected --- the realistic check).
fn air_format(air: &Path, tmp_dir: &Path, input: &str) -> Option<String> {
    let file = tmp_dir.join("air_compat_case.R");
    fs::write(&file, input).ok()?;
    let status = Command::new(air).arg("format").arg(&file).output().ok()?;
    if !status.status.success() {
        return None;
    }
    fs::read_to_string(&file).ok()
}

// --- allowlist ------------------------------------------------------------

/// Loads the deviations allowlist: `key = "reason"` lines (a simple TOML subset,
/// hand-parsed to avoid a dev-dependency). Comments start with `#`; a `[section]`
/// header line is ignored.
fn load_allowlist() -> BTreeMap<String, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("air_compat_allowlist.toml");
    let mut map = BTreeMap::new();
    let Ok(text) = fs::read_to_string(&path) else {
        return map;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().trim_matches('"').to_string();
        let value = value.trim().trim_matches('"').to_string();
        map.insert(key, value);
    }
    map
}

// --- line similarity ------------------------------------------------------

/// Length of the longest common subsequence of two line slices.
fn lcs_len(a: &[&str], b: &[&str]) -> usize {
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let mut prev = vec![0usize; b.len() + 1];
    for &line_a in a {
        let mut cur = vec![0usize; b.len() + 1];
        for (j, &line_b) in b.iter().enumerate() {
            cur[j + 1] = if line_a == line_b {
                prev[j] + 1
            } else {
                cur[j].max(prev[j + 1])
            };
        }
        prev = cur;
    }
    prev[b.len()]
}

// --- reporting ------------------------------------------------------------

fn render_report(
    reports: &[FileReport],
    allowlist: &BTreeMap<String, String>,
    total_lcs2: usize,
    total_lines: usize,
    corpus_label: &str,
) -> String {
    let mut compatible = 0usize;
    let mut skipped_arity = 0usize;
    let mut skipped_air = 0usize;
    let mut intentional: Vec<(&str, &str, f64)> = Vec::new();
    let mut unexplained: Vec<(&str, f64)> = Vec::new();

    for r in reports {
        match r.outcome {
            Outcome::Compatible => compatible += 1,
            Outcome::SkippedArity => skipped_arity += 1,
            Outcome::SkippedAir => skipped_air += 1,
            Outcome::Divergent { line_similarity } => {
                if let Some(reason) = allowlist.get(&r.key) {
                    intentional.push((&r.key, reason, line_similarity));
                } else {
                    unexplained.push((&r.key, line_similarity));
                }
            }
        }
    }

    let measured = compatible + intentional.len() + unexplained.len();
    let file_compat = if measured == 0 {
        100.0
    } else {
        compatible as f64 / measured as f64 * 100.0
    };
    let line_sim = if total_lines == 0 {
        100.0
    } else {
        total_lcs2 as f64 / total_lines as f64 * 100.0
    };

    let mut s = String::new();
    s.push_str("# Air compatibility (soft target)\n\n");
    s.push_str(
        "_Generated by `task air-compat` (`tests/air_compat.rs`). Do not edit by hand._\n\n",
    );
    s.push_str(
        "This is a **soft gauge, not a quality gate**, and is subordinate to Tenet 1 \
         (deterministic, rule-based formatting). It measures the one-directional fixed point \
         `air(arity(x)) == arity(x)`: how much of arity's output the `air` formatter would \
         leave untouched. A divergence is never a bug by itself --- it is either a deliberate, \
         recorded deviation or an open question.\n\n",
    );
    s.push_str(&format!("- **Corpus:** {corpus_label}\n"));
    s.push_str(&format!(
        "- **Line similarity:** {line_sim:.1}%  _(Dice coefficient over lines)_\n"
    ));
    s.push_str(&format!(
        "- **File compatibility:** {file_compat:.1}%  ({compatible}/{measured} files unchanged by air)\n"
    ));
    s.push_str(&format!(
        "- **Intentional deviations:** {}  ·  **Unexplained divergences:** {}\n",
        intentional.len(),
        unexplained.len()
    ));
    if skipped_arity + skipped_air > 0 {
        s.push_str(&format!(
            "- **Skipped:** {skipped_arity} (arity could not format) + {skipped_air} (air could not parse)\n"
        ));
    }
    s.push('\n');

    if !unexplained.is_empty() {
        unexplained.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        s.push_str("## Unexplained divergences (triage queue)\n\n");
        s.push_str(
            "Each of these is **either a bug to fix** (air is more idiomatic --- adopt it) **or a \
             deliberate deviation to record** (add it to `tests/air_compat_allowlist.toml` with a \
             reason). Leaving it here is the \"tension\": diverging from air should be a conscious, \
             documented choice.\n\n",
        );
        s.push_str("| File | Line similarity |\n|---|---|\n");
        for (key, sim) in &unexplained {
            s.push_str(&format!("| `{key}` | {:.1}% |\n", sim * 100.0));
        }
        s.push('\n');
    }

    if !intentional.is_empty() {
        intentional.sort_by(|a, b| a.0.cmp(b.0));
        s.push_str("## Recorded intentional deviations\n\n");
        s.push_str(
            "Listed in `tests/air_compat_allowlist.toml`. These diverge from air on purpose.\n\n",
        );
        s.push_str("| File | Line similarity | Reason |\n|---|---|---|\n");
        for (key, reason, sim) in &intentional {
            s.push_str(&format!("| `{key}` | {:.1}% | {reason} |\n", sim * 100.0));
        }
        s.push('\n');
    }

    s
}
