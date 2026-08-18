//! roxygen2 *lint* differential oracle (allowlist-ratcheted, informative).
//!
//! roxygen2 signals warnings/messages for documentation mistakes while parsing
//! and rd-roclet-processing a file. This harness runs roxygen2 (driver op
//! `lint-warnings` in `tests/oracle/roxygen_oracle.R`) over the curated corpus
//! and compares those signals against arity's `documentation/` lint rules,
//! keeping the rules' behavior honest against the reference implementation.
//!
//! The comparison is at the level of **comparable event classes**, not message
//! text (roxygen2's cli-formatted messages are version-dependent) and not raw
//! rule ids (several arity findings are deliberately *stricter* than roxygen2
//! and have no counterpart). roxygen2 7.3.x is silent on: missing/nonexistent/
//! duplicate `@param`, missing `@return`, syntax errors in `@examples`
//! *bodies*, and undocumented `@export`s --- all of those arity findings are
//! excluded from the diff by construction. What remains comparable:
//!
//! | class                 | roxygen2 signal                          | arity finding                                  |
//! |-----------------------|------------------------------------------|------------------------------------------------|
//! | `unknown-tag`         | "is not a known tag"                     | `roxygen-unknown-tag`                          |
//! | `param-two-part`      | "requires two parts"                     | `roxygen-param` name/description findings      |
//! | `title`               | "no name and/or title"                   | `roxygen-title`                                |
//! | `examplesif-condition`| "@examplesIf condition failed to parse"  | `roxygen-examples` on the condition            |
//!
//! Known asymmetries in `title` (arity flags undocumented `@export`s where
//! roxygen2 is silent; roxygen2 warns on blocks arity conservatively refuses
//! to associate) simply keep those files off the allowlist --- visible in the
//! report, never a failure. Gate: files in
//! `tests/oracle/roxygen-lint-allowlist.txt` must keep matching; everything
//! else is backlog. `#[ignore]`d (needs R); run via `task roxygen-lint-oracle`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use arity::config::LintConfig;
use arity::linter::check_document;

const ALLOWLIST_REL: &str = "tests/oracle/roxygen-lint-allowlist.txt";
const REPORT_REL: &str = ".agents/skills/roxygen-parity/ROXYGEN_LINT.md";

const CLASSES: &[&str] = &[
    "unknown-tag",
    "param-two-part",
    "title",
    "examplesif-condition",
];

/// Event counts per comparable class.
type Events = BTreeMap<&'static str, usize>;

fn manifest_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

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

/// `(stem, path)` pairs of the curated corpus (or a custom directory via
/// `ROXYGEN_LINT_ORACLE_CORPUS`, for ad-hoc runs on real packages).
fn collect_corpus() -> Vec<(String, PathBuf)> {
    let dir = std::env::var("ROXYGEN_LINT_ORACLE_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_path("tests/oracle/corpus/roxygen"));
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

fn read_allowlist() -> std::collections::BTreeSet<String> {
    let mut set = std::collections::BTreeSet::new();
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

/// Map one roxygen2 warning/message to its comparable class, if any.
fn classify_r_message(msg: &str) -> Option<&'static str> {
    if msg.contains("is not a known tag") {
        Some("unknown-tag")
    } else if msg.contains("requires two parts") {
        Some("param-two-part")
    } else if msg.contains("no name and/or title") {
        Some("title")
    } else if msg.contains("@examplesIf condition failed to parse") {
        Some("examplesif-condition")
    } else {
        None
    }
}

/// arity's comparable events for one source file.
fn arity_events(path: &Path, src: &str) -> Events {
    let mut events = Events::new();
    let Ok(diags) = check_document(path, src, &LintConfig::default()) else {
        return events;
    };
    for d in diags {
        let class = match d.rule {
            "roxygen-unknown-tag" => Some("unknown-tag"),
            "roxygen-title" => Some("title"),
            "roxygen-param"
                if d.message.body.contains("requires a name and description")
                    || d.message.body.contains("has no description") =>
            {
                Some("param-two-part")
            }
            "roxygen-examples" if d.message.body.contains("`@examplesIf` condition") => {
                Some("examplesif-condition")
            }
            _ => None,
        };
        if let Some(class) = class {
            *events.entry(class).or_default() += 1;
        }
    }
    events
}

/// Run the driver's `lint-warnings-batch` op; one entry per input, aligned by
/// index: `Some(messages)` or `None` (roxygen2 errored / driver failed).
fn batch_warnings(rscript: &Path, driver: &Path, inputs: &[String]) -> Vec<Option<Vec<String>>> {
    let mut result = vec![None; inputs.len()];
    let payload = serde_json::to_string(inputs).expect("serialize batch payload");
    let mut child = match Command::new(rscript)
        .arg(driver)
        .arg("lint-warnings-batch")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return result,
    };
    if child
        .stdin
        .take()
        .and_then(|mut s| s.write_all(payload.as_bytes()).ok())
        .is_none()
    {
        return result;
    }
    let Ok(out) = child.wait_with_output() else {
        return result;
    };
    if !out.status.success() {
        return result;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);

    let mut current: Option<usize> = None;
    let mut body: Vec<String> = Vec::new();
    let flush = |idx: Option<usize>, body: &[String], result: &mut [Option<Vec<String>>]| {
        if let Some(i) = idx
            && i < result.len()
        {
            result[i] = if body.iter().any(|l| l.trim() == "!ERROR") {
                None
            } else {
                Some(
                    body.iter()
                        .filter(|l| !l.trim().is_empty())
                        .cloned()
                        .collect(),
                )
            };
        }
    };
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("@@@") {
            flush(current, &body, &mut result);
            current = rest.trim().parse::<usize>().ok();
            body.clear();
        } else {
            body.push(line.to_string());
        }
    }
    flush(current, &body, &mut result);
    result
}

enum Outcome {
    /// Comparable event counts match.
    Match,
    /// They differ (backlog unless allowlisted, then a regression).
    Mismatch { arity: Events, roxygen2: Events },
    /// roxygen2 could not process the file.
    SkippedR,
}

#[test]
#[ignore = "roxygen2 lint differential oracle; run via `task roxygen-lint-oracle`"]
fn roxygen_lint_oracle_report() {
    let Some(rscript) = locate_rscript() else {
        eprintln!("roxygen-lint-oracle: `Rscript` not found; skipping (not a failure).");
        return;
    };
    let driver = manifest_path("tests/oracle/roxygen_oracle.R");
    let corpus = collect_corpus();
    if corpus.is_empty() {
        eprintln!("roxygen-lint-oracle: empty corpus; nothing to measure.");
        return;
    }
    let allow = read_allowlist();

    let sources: Vec<String> = corpus
        .iter()
        .map(|(_, p)| fs::read_to_string(p).unwrap_or_default())
        .collect();
    let r_msgs = batch_warnings(&rscript, &driver, &sources);

    let mut outcomes: Vec<(String, Outcome)> = Vec::new();
    let mut uncovered: BTreeMap<String, usize> = BTreeMap::new();
    for (i, (stem, path)) in corpus.iter().enumerate() {
        let outcome = match &r_msgs[i] {
            None => Outcome::SkippedR,
            Some(msgs) => {
                let mut r_events = Events::new();
                for m in msgs {
                    match classify_r_message(m) {
                        Some(class) => *r_events.entry(class).or_default() += 1,
                        None => *uncovered.entry(m.clone()).or_default() += 1,
                    }
                }
                let ours = arity_events(path, &sources[i]);
                if ours == r_events {
                    Outcome::Match
                } else {
                    Outcome::Mismatch {
                        arity: ours,
                        roxygen2: r_events,
                    }
                }
            }
        };
        outcomes.push((stem.clone(), outcome));
    }

    // Greppable PASS lines for re-seeding the allowlist.
    for (stem, o) in &outcomes {
        if matches!(o, Outcome::Match) {
            println!("PASS {stem}");
        }
    }

    let matches = outcomes
        .iter()
        .filter(|(_, o)| matches!(o, Outcome::Match))
        .count();
    let skipped = outcomes
        .iter()
        .filter(|(_, o)| matches!(o, Outcome::SkippedR))
        .count();
    let mismatches: Vec<&(String, Outcome)> = outcomes
        .iter()
        .filter(|(_, o)| matches!(o, Outcome::Mismatch { .. }))
        .collect();

    let mut md = String::new();
    md.push_str("# roxygen2 lint oracle (differential)\n\n");
    md.push_str(
        "_Generated by `task roxygen-lint-oracle` (`tests/roxygen_lint_oracle.rs`). Do not edit by hand._\n\n",
    );
    md.push_str(
        "Compares arity's `documentation/` lint rules against the warnings/messages roxygen2 \
         itself signals, per corpus file, over the comparable event classes (see the harness \
         module doc; arity findings that are deliberately stricter than roxygen2 are excluded \
         by construction). Allowlisted files are guarded against regression; mismatches are \
         the backlog --- never a build failure (needs R; `#[ignore]`d).\n\n",
    );
    md.push_str(&format!(
        "- **Corpus:** {} files (`tests/oracle/corpus/roxygen/*.R`)\n",
        corpus.len()
    ));
    md.push_str(&format!(
        "- **Matching:** {matches}  ({} allowlisted)  ·  **Mismatching (backlog):** {}  ·  **Skipped (roxygen2 errored):** {skipped}\n\n",
        outcomes
            .iter()
            .filter(|(s, o)| matches!(o, Outcome::Match) && allow.contains(s))
            .count(),
        mismatches.len(),
    ));

    if !mismatches.is_empty() {
        md.push_str("## Mismatches (backlog)\n\n");
        md.push_str("| File | arity | roxygen2 |\n|---|---|---|\n");
        for (stem, o) in &mismatches {
            if let Outcome::Mismatch { arity, roxygen2 } = o {
                md.push_str(&format!(
                    "| `{stem}` | {} | {} |\n",
                    render_events(arity),
                    render_events(roxygen2)
                ));
            }
        }
        md.push('\n');
    }

    if !uncovered.is_empty() {
        md.push_str("## Uncovered roxygen2 signals (future-rule backlog)\n\n");
        md.push_str(
            "roxygen2 messages with no comparable arity rule yet, with occurrence counts.\n\n",
        );
        md.push_str("| Count | Message |\n|---|---|\n");
        let mut items: Vec<(&String, &usize)> = uncovered.iter().collect();
        items.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (msg, count) in items {
            md.push_str(&format!("| {count} | `{}` |\n", msg.replace('`', "'")));
        }
        md.push('\n');
    }

    let out_path = manifest_path(REPORT_REL);
    fs::write(&out_path, &md).expect("write ROXYGEN_LINT.md");
    println!(
        "\nroxygen-lint-oracle: {} files -> {matches} matching, {} mismatching (backlog), {skipped} skipped. Wrote {}",
        corpus.len(),
        mismatches.len(),
        out_path.display(),
    );

    // Gate: allowlisted files must keep matching (and keep existing).
    let by_stem: BTreeMap<&str, &Outcome> = outcomes.iter().map(|(s, o)| (s.as_str(), o)).collect();
    let mut regressions: Vec<&str> = Vec::new();
    let mut missing: Vec<&str> = Vec::new();
    for stem in &allow {
        match by_stem.get(stem.as_str()) {
            None => missing.push(stem),
            Some(Outcome::Match) => {}
            Some(_) => regressions.push(stem),
        }
    }
    assert!(
        regressions.is_empty() && missing.is_empty(),
        "roxygen-lint-oracle allowlist guard failed:\n  {} regressed (no longer matching \
         roxygen2): {:?}\n  {} absent from corpus (stale allowlist entry): {:?}\n  \
         Re-seed via `task roxygen-lint-oracle-seed` after a deliberate change.",
        regressions.len(),
        regressions,
        missing.len(),
        missing,
    );
}

fn render_events(events: &Events) -> String {
    if events.is_empty() {
        return "—".to_string();
    }
    CLASSES
        .iter()
        .filter_map(|c| events.get(c).map(|n| format!("{c}×{n}")))
        .collect::<Vec<_>>()
        .join(", ")
}
