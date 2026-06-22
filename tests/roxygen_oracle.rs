//! roxygen2 differential oracle (a strict correctness check).
//!
//! roxygen2 turns `#'` blocks into `.Rd`. This harness uses that as a one-
//! directional oracle for arity's roxygen handling. Unlike the air-compat gauge
//! (`tests/air_compat.rs`), which is a *soft* target subordinate to Tenet 1, this
//! is a **correctness invariant**: if arity's formatting changes what roxygen2
//! renders, arity silently altered the documentation --- a behavior-preservation
//! bug, of the same family as a losslessness or idempotence failure. A divergence
//! is not "a tension to record"; it must be fixed, or explicitly **blocked** in
//! `tests/roxygen_oracle_blocked.toml` with a rationale. The check is `#[ignore]`d
//! only because it shells out to R/roxygen2 (absent in the base CI), *not* because
//! divergence is acceptable: when R is present it is strict and **fails on any
//! unaccounted divergence**. It skips cleanly when R is missing. Invoke it:
//!
//! ```sh
//! task roxygen-oracle
//! # or
//! cargo test --test roxygen_oracle -- --ignored --nocapture
//! ```
//!
//! Methodology --- the **semantic fixed point**: for each corpus file `x`, we
//! compare `roxygen2(arity_format(x))` against `roxygen2(x)` at the Rd *parse
//! tree* level (`tests/oracle/roxygen_oracle.R`, via `tools::parse_Rd`, with
//! cosmetic noise --- srcref, the `% Generated` header, and prose line-wrapping
//! --- normalized away). A match means arity's formatting is **Rd-preserving**:
//! it never changed what the documentation *means*. This is the analog of air-
//! compat's `air(arity(x)) == arity(x)` fixed point.
//!
//! IMPORTANT scope note: this measures *semantic* preservation, not layout
//! quality. A purely cosmetic defect (e.g. a `\describe{}` reflowed into a run-on
//! paragraph in non-markdown mode) renders to the *same* Rd, so it shows up here
//! as a match. Catching those is the job of the formatter fixtures and, once it
//! exists, the CST-to-Rd projector check --- not this semantic gauge.
//!
//! Corpus --- defaults to `tests/oracle/corpus/roxygen/*.R` (complete,
//! roxygen2-processable units). Point `ROXYGEN_ORACLE_CORPUS` at a directory of
//! real `.R` files for a broader run.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use arity::formatter::format;

/// One corpus file's outcome.
enum Outcome {
    /// roxygen2 renders the same Rd from `x` and from `arity_format(x)`.
    Preserving,
    /// The rendered Rd differs --- arity's formatting changed the documentation.
    Divergent,
    /// arity could not format the input (parse error / unsupported).
    SkippedArity,
    /// roxygen2 could not process the input or arity's output.
    SkippedR,
}

struct FileReport {
    key: String,
    outcome: Outcome,
}

#[test]
#[ignore = "roxygen2 differential oracle; run via `task roxygen-oracle`"]
fn roxygen_oracle_report() {
    let Some(rscript) = locate_rscript() else {
        eprintln!("roxygen-oracle: `Rscript` not found on PATH; skipping (this is not a failure).");
        return;
    };
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("oracle")
        .join("roxygen_oracle.R");
    if !driver.is_file() {
        eprintln!(
            "roxygen-oracle: driver {} missing; skipping.",
            driver.display()
        );
        return;
    }

    let corpus = collect_corpus();
    if corpus.is_empty() {
        eprintln!("roxygen-oracle: empty corpus; nothing to measure.");
        return;
    }

    let blocked = load_blocked();
    let mut reports: Vec<FileReport> = Vec::new();

    for (key, path) in &corpus {
        let Ok(raw) = fs::read_to_string(path) else {
            continue;
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

        let (Some(orig_tree), Some(fmt_tree)) = (
            oracle_tree(&rscript, &driver, &raw),
            oracle_tree(&rscript, &driver, &arity_out),
        ) else {
            reports.push(FileReport {
                key: key.clone(),
                outcome: Outcome::SkippedR,
            });
            continue;
        };

        let outcome = if orig_tree == fmt_tree {
            Outcome::Preserving
        } else {
            Outcome::Divergent
        };
        reports.push(FileReport {
            key: key.clone(),
            outcome,
        });
    }

    let report = render_report(&reports, &blocked, &corpus_label());
    print!("{report}");

    let out_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ROXYGEN_ORACLE.md");
    fs::write(&out_path, &report).expect("write ROXYGEN_ORACLE.md");
    eprintln!("roxygen-oracle: wrote {}", out_path.display());

    // Strict gate: R was available, so any divergence that is not explicitly
    // blocked is a behavior-preservation bug. Fail loudly (the report is already
    // written for triage). Block a case in `roxygen_oracle_blocked.toml` only
    // when the divergence is a deliberate, documented choice.
    let unaccounted: Vec<&str> = reports
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Divergent) && !blocked.contains_key(&r.key))
        .map(|r| r.key.as_str())
        .collect();
    assert!(
        unaccounted.is_empty(),
        "roxygen-oracle: {} unaccounted divergence(s) --- arity's formatting changed the \
         rendered Rd for: {}. Fix the formatter, or block each in \
         tests/roxygen_oracle_blocked.toml with a rationale.",
        unaccounted.len(),
        unaccounted.join(", "),
    );
}

// --- corpus ---------------------------------------------------------------

fn corpus_label() -> String {
    match std::env::var("ROXYGEN_ORACLE_CORPUS") {
        Ok(dir) => format!("custom (`ROXYGEN_ORACLE_CORPUS={dir}`)"),
        Err(_) => "bundled (`tests/oracle/corpus/roxygen/*.R`)".to_string(),
    }
}

/// Returns `(identifier, path)` pairs. The identifier is the allowlist key: the
/// file stem for the default corpus, or the corpus-relative path for a custom one.
fn collect_corpus() -> Vec<(String, PathBuf)> {
    if let Ok(dir) = std::env::var("ROXYGEN_ORACLE_CORPUS") {
        let root = PathBuf::from(&dir);
        let mut files = Vec::new();
        collect_r_files(&root, &root, &mut files);
        files.sort();
        return files;
    }

    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("oracle")
        .join("corpus")
        .join("roxygen");
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "R" || e == "r") {
                let key = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                files.push((key, path));
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

// --- R invocation ---------------------------------------------------------

fn locate_rscript() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ARITY_RSCRIPT") {
        return Some(PathBuf::from(p));
    }
    let ok = Command::new("Rscript")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    ok.then(|| PathBuf::from("Rscript"))
}

/// Run the oracle driver's `block-to-tree` op on `input`, returning the canonical
/// Rd-tree text, or `None` if roxygen2 could not process it (non-zero exit).
fn oracle_tree(rscript: &Path, driver: &Path, input: &str) -> Option<String> {
    let mut child = Command::new(rscript)
        .arg(driver)
        .arg("block-to-tree")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(input.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

// --- allowlist ------------------------------------------------------------

/// Loads the deviations allowlist: `key = "reason"` lines (a simple TOML subset,
/// hand-parsed to avoid a dev-dependency). Keys map a corpus case to the reason
/// its divergence is an accepted, deliberate choice rather than a bug.
fn load_blocked() -> BTreeMap<String, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("roxygen_oracle_blocked.toml");
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

// --- reporting ------------------------------------------------------------

fn render_report(
    reports: &[FileReport],
    blocked: &BTreeMap<String, String>,
    corpus_label: &str,
) -> String {
    let mut preserving = 0usize;
    let mut skipped_arity = 0usize;
    let mut skipped_r = 0usize;
    let mut blocked_hits: Vec<(&str, &str)> = Vec::new();
    let mut unaccounted: Vec<&str> = Vec::new();

    for r in reports {
        match r.outcome {
            Outcome::Preserving => preserving += 1,
            Outcome::SkippedArity => skipped_arity += 1,
            Outcome::SkippedR => skipped_r += 1,
            Outcome::Divergent => {
                if let Some(reason) = blocked.get(&r.key) {
                    blocked_hits.push((&r.key, reason));
                } else {
                    unaccounted.push(&r.key);
                }
            }
        }
    }

    let measured = preserving + blocked_hits.len() + unaccounted.len();
    let preserve_pct = if measured == 0 {
        100.0
    } else {
        preserving as f64 / measured as f64 * 100.0
    };

    let mut s = String::new();
    s.push_str("# roxygen2 oracle (differential)\n\n");
    s.push_str("_Generated by `task roxygen-oracle` (`tests/roxygen_oracle.rs`). Do not edit by hand._\n\n");
    s.push_str(
        "A **strict correctness check** (not a soft target): it asserts the semantic fixed point \
         `roxygen2(arity_format(x)) == roxygen2(x)` at the Rd parse-tree level --- arity's \
         formatting must never change what roxygen2 renders. An unaccounted divergence **fails** \
         (when R is present). It checks *meaning*, not layout: a cosmetic defect that renders to the \
         same Rd (e.g. a reflowed `\\describe{}` in non-markdown mode) is preserving here --- those \
         are the formatter fixtures' and the future (pinned, CI-safe) CST-to-Rd projector's job.\n\n",
    );
    s.push_str(&format!("- **Corpus:** {corpus_label}\n"));
    s.push_str(&format!(
        "- **Rd-preserving:** {preserve_pct:.1}%  ({preserving}/{measured} files)\n"
    ));
    s.push_str(&format!(
        "- **Blocked divergences:** {}  ·  **Unaccounted divergences (gate failures):** {}\n",
        blocked_hits.len(),
        unaccounted.len()
    ));
    if skipped_arity + skipped_r > 0 {
        s.push_str(&format!(
            "- **Skipped:** {skipped_arity} (arity could not format) + {skipped_r} (roxygen2 could not process)\n"
        ));
    }
    s.push('\n');

    if !unaccounted.is_empty() {
        unaccounted.sort_unstable();
        s.push_str("## Unaccounted divergences (gate failures)\n\n");
        s.push_str(
            "**These fail the check.** arity's formatting changed the rendered Rd --- that must not \
             happen. Fix the formatter so the case becomes preserving, or, if the divergence is a \
             deliberate and documented choice, block it in `tests/roxygen_oracle_blocked.toml` with \
             a rationale.\n\n",
        );
        s.push_str("| File |\n|---|\n");
        for key in &unaccounted {
            s.push_str(&format!("| `{key}` |\n"));
        }
        s.push('\n');
    }

    if !blocked_hits.is_empty() {
        blocked_hits.sort_by(|a, b| a.0.cmp(b.0));
        s.push_str("## Blocked divergences (accepted, with rationale)\n\n");
        s.push_str("Listed in `tests/roxygen_oracle_blocked.toml`.\n\n");
        s.push_str("| File | Rationale |\n|---|---|\n");
        for (key, reason) in &blocked_hits {
            s.push_str(&format!("| `{key}` | {reason} |\n"));
        }
        s.push('\n');
    }

    s
}
