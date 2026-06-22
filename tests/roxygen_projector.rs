//! Projector-parity gate --- the primary, **CI-safe** roxygen2 conformance
//! engine (the build target of the `roxygen-parity` effort).
//!
//! For each curated corpus case (`tests/oracle/corpus/roxygen/<name>.R`) we
//! project arity's CST to the parser-owned Rd section subtrees
//! (`arity::roxygen::project_rd::project_to_rd`) and diff it against a **pinned**
//! `<name>.rdtree`, minted once from roxygen2 by `task roxygen-projector-refresh`
//! (the R driver's `block-to-sections` op). *Pinned ⇒ no R at test time ⇒ this
//! runs in plain `cargo test`* and is a hard gate, unlike the R-dependent
//! `#[ignore]`d fixed-point oracle (`tests/roxygen_oracle.rs`).
//!
//! Crucially this compares **structure**, so it sees what the semantic
//! fixed-point check is blind to: a `\describe` the CST never modeled as a block,
//! or a markdown list still flat prose, projects as flat text and *diverges* from
//! the nested pin. That divergence is the backlog that drives parser growth ---
//! the whole point of pinning the projector rather than chasing the formatter.
//!
//! Accounting follows fatou's `parser-parity` model. Every case is either
//! **allowlisted** (`tests/oracle/roxygen-projector-allowlist.txt` --- its
//! projection currently matches the pin, guarded against regression) or part of
//! the **backlog** (a structural divergence to close in the parser, then ratchet
//! in). An allowlisted case that regresses --- or whose pin is missing --- fails
//! the build. Non-allowlisted divergences are the backlog, never a failure.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use arity::roxygen::project_rd::project_to_rd;

const ALLOWLIST_REL: &str = "tests/oracle/roxygen-projector-allowlist.txt";

#[derive(PartialEq)]
enum Outcome {
    /// Projection is byte-identical to the pin.
    Match,
    /// Projection differs from the pin --- a structural gap (the backlog).
    Divergent,
    /// No `<name>.rdtree` pin committed for this case.
    Unpinned,
}

struct Report {
    key: String,
    outcome: Outcome,
}

fn manifest_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// `(stem, .R path, optional .rdtree pin path)` for every curated corpus case.
fn collect_corpus() -> Vec<(String, PathBuf, PathBuf)> {
    let dir = manifest_path("tests/oracle/corpus/roxygen");
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "R" || e == "r") {
                let stem = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let pin = path.with_extension("rdtree");
                out.push((stem, path, pin));
            }
        }
    }
    out.sort();
    out
}

/// Reads a slug-list file (one entry per line; `#` comments and blanks ignored).
fn read_allowlist() -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    let Ok(text) = fs::read_to_string(manifest_path(ALLOWLIST_REL)) else {
        return set;
    };
    for line in text.lines() {
        let line = line.trim();
        if !line.is_empty() && !line.starts_with('#') {
            set.insert(line.to_string());
        }
    }
    set
}

fn evaluate() -> Vec<Report> {
    collect_corpus()
        .into_iter()
        .map(|(key, r_path, pin_path)| {
            let src = fs::read_to_string(&r_path).unwrap_or_default();
            let projected = project_to_rd(&src);
            let outcome = match fs::read_to_string(&pin_path) {
                // The pin file carries a trailing newline from the R driver;
                // the projector emits none. Compare trimmed.
                Ok(pin) if pin.trim_end_matches('\n') == projected => Outcome::Match,
                Ok(_) => Outcome::Divergent,
                Err(_) => Outcome::Unpinned,
            };
            Report { key, outcome }
        })
        .collect()
}

#[test]
fn projector_parity() {
    let reports = evaluate();
    let allow = read_allowlist();

    let (mut matched, mut divergent, mut unpinned) = (0, 0, 0);
    for r in &reports {
        match r.outcome {
            Outcome::Match => matched += 1,
            Outcome::Divergent => divergent += 1,
            Outcome::Unpinned => unpinned += 1,
        }
    }

    write_report(&reports, &allow, matched, divergent, unpinned);

    // Regression guard: every allowlisted case must still match its pin (and the
    // pin must still exist). Non-allowlisted divergences are the backlog.
    let mut regressed: Vec<&str> = Vec::new();
    let mut stale: Vec<&str> = Vec::new();
    let keys: BTreeSet<&str> = reports.iter().map(|r| r.key.as_str()).collect();
    for r in &reports {
        if allow.contains(&r.key) && r.outcome != Outcome::Match {
            regressed.push(&r.key);
        }
    }
    for slug in &allow {
        if !keys.contains(slug.as_str()) {
            stale.push(slug);
        }
    }

    assert!(
        regressed.is_empty() && stale.is_empty(),
        "projector-parity gate failed:\n  \
         {} allowlisted case(s) no longer match their pin: {:?}\n  \
         {} allowlisted case(s) absent from the corpus (stale entry): {:?}\n  \
         A faithful divergence means the CST is wrong --- fix the parser, never the \
         projector. If a pin is outdated, refresh it with `task roxygen-projector-refresh`.",
        regressed.len(),
        regressed,
        stale.len(),
        stale,
    );
}

fn write_report(
    reports: &[Report],
    allow: &BTreeSet<String>,
    matched: usize,
    divergent: usize,
    unpinned: usize,
) {
    let mut md = String::new();
    md.push_str("# roxygen2 projector parity (CST → Rd sections)\n\n");
    md.push_str(
        "_Generated by `cargo test --test roxygen_projector` (`task roxygen-projector`). \
         Do not edit by hand._\n\n",
    );
    md.push_str(
        "The **primary, CI-safe** conformance gate: `project_to_rd(parse(x))` vs a pinned \
         `<name>.rdtree` minted from roxygen2 (`block-to-sections`). It compares Rd **structure**, \
         so it catches what the semantic fixed-point oracle cannot --- a `\\describe`/`\\itemize`/\
         `\\tabular` the CST has not modeled as a block, or markdown still flat prose. \
         Allowlisted cases (`tests/oracle/roxygen-projector-allowlist.txt`) are guarded against \
         regression; **divergent** cases are the backlog: close them in the *parser*, then ratchet \
         in.\n\n",
    );
    md.push_str(&format!(
        "- **Matching (pinned):** {matched}  ({} allowlisted)\n",
        reports
            .iter()
            .filter(|r| r.outcome == Outcome::Match && allow.contains(&r.key))
            .count()
    ));
    md.push_str(&format!("- **Divergent (backlog):** {divergent}\n"));
    if unpinned > 0 {
        md.push_str(&format!("- **Unpinned:** {unpinned} (no `.rdtree`)\n"));
    }
    md.push('\n');

    let backlog: Vec<&str> = reports
        .iter()
        .filter(|r| r.outcome == Outcome::Divergent)
        .map(|r| r.key.as_str())
        .collect();
    if !backlog.is_empty() {
        md.push_str("## Divergent (backlog)\n\n");
        md.push_str(
            "These project structurally differently from roxygen2 --- the parser work to pick \
             off, then ratchet into the allowlist.\n\n| Case |\n|---|\n",
        );
        for key in &backlog {
            md.push_str(&format!("| `{key}` |\n"));
        }
        md.push('\n');
    }

    let out_path = manifest_path(".claude/skills/roxygen-parity/ROXYGEN_PROJECTOR.md");
    let _ = fs::write(&out_path, &md);
}
