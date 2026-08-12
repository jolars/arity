//! Differential oracle: arity's DCF parser against R's own `read.dcf`.
//!
//! R's `read.dcf` *is* the definition of what a `DESCRIPTION` means, so the
//! facts arity's parser encodes about it should be checked, not asserted in a
//! comment. This harness runs both over the same inputs and fails on any
//! disagreement outside the two divergences arity keeps on purpose.
//!
//! **The two permitted divergences** (see `TODO.md`; both are pinned by unit
//! tests in `dcf/parser.rs` as well):
//!
//! 1. A field whose own line is empty folds with a leading `\n` in arity
//!    (`Collate:\n a.R` -> `"\na.R"`); R drops the empty leading segment.
//!    Normalized here by stripping one leading `\n` from arity's value.
//! 2. A duplicate field resolves to the **first** occurrence in arity; R takes
//!    the last. Normalized here by comparing a last-wins map.
//! 3. `read.dcf` does **not** trim a field name, so `Package : p` declares a
//!    field literally named `"Package "` and R therefore sees no `Package` at
//!    all. arity trims, which is the lenient direction: it reads a typo'd
//!    header the way it was obviously meant, and the CST still keeps the
//!    whitespace as its own token so a lint can flag it precisely. Normalized
//!    here by trimming R's name.
//!
//! Anything else is a failure. When one of those divergences is closed, delete
//! the corresponding normalization and this harness proves the fix.
//!
//! `#[ignore]`d because it needs R: run via `task dcf-oracle`. A missing
//! `Rscript` is a skip, never a failure.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use arity::dcf;

/// One case's view of the world according to R.
#[derive(Debug, PartialEq, Eq)]
enum Oracle {
    /// `read.dcf` refused the input.
    Error(String),
    /// One map per record, field name -> value.
    Records(Vec<BTreeMap<String, String>>),
}

#[test]
#[ignore = "read.dcf differential oracle; run via `task dcf-oracle`"]
fn dcf_matches_read_dcf() {
    let Some(rscript) = locate_rscript() else {
        eprintln!("dcf-oracle: `Rscript` not found on PATH; skipping (this is not a failure).");
        return;
    };
    let driver = manifest_path("tests/oracle/dcf_oracle.R");
    if !driver.is_file() {
        eprintln!("dcf-oracle: driver {} missing; skipping.", driver.display());
        return;
    }

    let cases = corpus();
    assert!(!cases.is_empty(), "the oracle corpus should not be empty");

    let mut checked = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for (label, input) in &cases {
        let Some(oracle) = run_oracle(&rscript, &driver, input) else {
            eprintln!("dcf-oracle: {label}: driver could not process; skipped.");
            continue;
        };
        checked += 1;
        if let Err(why) = compare(input, &oracle) {
            failures.push(format!("{label}: {why}"));
        }
    }

    eprintln!("dcf-oracle: {checked}/{} cases checked.", cases.len());
    assert!(
        failures.is_empty(),
        "arity disagrees with read.dcf on {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// Compare one input's arity parse against R's verdict.
fn compare(input: &str, oracle: &Oracle) -> Result<(), String> {
    let output = dcf::parse(input);
    let document = output.document();

    match oracle {
        // R rejects the whole file where it hits a structural error. arity
        // never aborts a parse, so the agreement we require is narrower but
        // real: arity must have *noticed* something.
        Oracle::Error(message) => {
            if output.diagnostics.is_empty() {
                return Err(format!(
                    "read.dcf errored ({message:?}) but arity reported no diagnostic"
                ));
            }
            Ok(())
        }
        Oracle::Records(expected) => {
            if !output.diagnostics.is_empty() {
                return Err(format!(
                    "read.dcf accepted the input but arity diagnosed {:?}",
                    output
                        .diagnostics
                        .iter()
                        .map(|d| d.message.as_str())
                        .collect::<Vec<_>>()
                ));
            }

            let actual: Vec<BTreeMap<String, String>> = document
                .records()
                .map(|record| {
                    let mut fields = BTreeMap::new();
                    for field in record.fields() {
                        // Inserting in document order makes the *last*
                        // duplicate win, which is divergence 2's normalization.
                        fields.insert(field.name().to_string(), normalize(&field.folded_value()));
                    }
                    fields
                })
                .collect();

            if actual.len() != expected.len() {
                return Err(format!(
                    "record count: read.dcf {} vs arity {}",
                    expected.len(),
                    actual.len()
                ));
            }
            for (i, (want, got)) in expected.iter().zip(actual.iter()).enumerate() {
                if want != got {
                    return Err(format!(
                        "record {i} differs:\n  read.dcf: {want:?}\n  arity:    {got:?}"
                    ));
                }
            }
            Ok(())
        }
    }
}

/// Divergence 1: drop the empty leading segment a field with an empty own line
/// folds in. Exactly one `\n`, never more — a value legitimately starting with
/// a blank continuation is not a thing DCF can express.
fn normalize(value: &str) -> String {
    value.strip_prefix('\n').unwrap_or(value).to_string()
}

// ---------------------------------------------------------------------------
// Corpus
// ---------------------------------------------------------------------------

/// Every `(label, text)` the oracle runs. Committed fixtures first, then the
/// real `DESCRIPTION`s arity reads, then the untracked roxygen2 reference
/// checkout when it happens to be present, then an inline adversarial table
/// mirroring the parser's own losslessness cases.
fn corpus() -> Vec<(String, String)> {
    let mut cases: Vec<(String, String)> = Vec::new();

    collect_dir(
        &mut cases,
        &manifest_path("crates/arity-parser/tests/fixtures/dcf"),
        "input.dcf",
        "fixture",
    );
    collect_dir(
        &mut cases,
        &manifest_path("tests/fixtures/rindex"),
        "DESCRIPTION",
        "rindex",
    );
    // Reference-only checkout: absent in a fresh clone, and that is normal.
    collect_dir(
        &mut cases,
        &manifest_path("roxygen2-ref/tests/testthat"),
        "DESCRIPTION",
        "roxygen2-ref",
    );

    for (i, text) in ADVERSARIAL.iter().enumerate() {
        cases.push((format!("adversarial[{i}] {text:?}"), (*text).to_string()));
    }
    cases
}

/// Push every `<dir>/*/<file_name>` under `root`, labeled `<tag>/<subdir>`.
fn collect_dir(cases: &mut Vec<(String, String)>, root: &Path, file_name: &str, tag: &str) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join(file_name))
        .filter(|path| path.is_file())
        .collect();
    found.sort();
    for path in found {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let label = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        cases.push((format!("{tag}/{label}"), text));
    }
}

/// The same shapes the parser's inline losslessness table covers, so the
/// oracle checks *meaning* on exactly what the unit tests check *bytes* on.
const ADVERSARIAL: &[&str] = &[
    "",
    "\n",
    "\n\n\n",
    "Package: p",
    "Package: p\n",
    "Package:\n",
    "Package: \n",
    ":\n",
    ": v\n",
    "   \n",
    "\t\n",
    "Package: p\r\nVersion: 1\r\n",
    "# c\n",
    "   # c\n",
    "Package: p\n# c\nVersion: 1\n",
    "Collate:\n a.R\n# c\n b.R\n",
    "garbage\n",
    "  orphan\n",
    "Package: p\n\n  orphan\n",
    "Package: p\n\nVersion: 1\n",
    "Package: p\n\n\nVersion: 1\n",
    "Package: p\n   \nVersion: 1\n",
    "Package : p\n",
    "Package: first\nPackage: second\n",
    "Package: p\nPackage: q\n\nPackage: r\n",
    "Built: R 4.5.3; ; 2025-01-01 00:00:00 UTC; unix\n",
    "Date/Publication: 2025-09-12 07:20:14 UTC\n",
    "Collate:\n    'a.R'\n    'b.R'\n",
    "Description: one\n  # two\n",
    "Authors@R: c(person(\"A\", \"B\", role = c(\"aut\", \"cre\")))\n",
    "Roxygen: list(load = \"installed\",\n    markdown = TRUE)\n",
];

// ---------------------------------------------------------------------------
// Driver plumbing
// ---------------------------------------------------------------------------

fn manifest_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn locate_rscript() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("ARITY_RSCRIPT") {
        return Some(PathBuf::from(path));
    }
    Command::new("Rscript")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
        .then(|| PathBuf::from("Rscript"))
}

/// Run the driver over `input`, or `None` when it could not process the case.
fn run_oracle(rscript: &Path, driver: &Path, input: &str) -> Option<Oracle> {
    let mut child = Command::new(rscript)
        .arg(driver)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.as_mut()?.write_all(input.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(parse_report(&String::from_utf8_lossy(&out.stdout)))
}

fn parse_report(text: &str) -> Oracle {
    let mut records: Vec<BTreeMap<String, String>> = Vec::new();
    for line in text.lines() {
        if let Some(message) = line.strip_prefix("ERROR\t") {
            return Oracle::Error(unescape(message));
        }
        if line == "RECORD" {
            records.push(BTreeMap::new());
        } else if let Some(rest) = line.strip_prefix("F\t") {
            let (name, value) = rest.split_once('\t').unwrap_or((rest, ""));
            if let Some(record) = records.last_mut() {
                // Divergence 3: R keeps whitespace between the name and the
                // colon as part of the name; arity trims it.
                record.insert(unescape(name).trim_end().to_string(), unescape(value));
            }
        }
    }
    Oracle::Records(records)
}

/// Inverse of the driver's `escape`.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}
