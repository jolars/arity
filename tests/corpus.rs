//! Tier 0 corpus smoke test (real-world R robustness).
//!
//! Runs arity's parser + formatter over a corpus of real `.R` files and asserts
//! the two foundational invariants on every file:
//!
//! 1. **Losslessness** --- `reconstruct(raw) == raw` (the parser preserves all
//!    text). This is checked even for files arity cannot yet format.
//! 2. **Idempotence** --- `format(format(x)) == format(x)`.
//!
//! It does **not** check semantic preservation (that the formatted code *means*
//! the same thing); that needs a trivia-stripped AST comparison and is deferred.
//!
//! Unlike `cargo test`'s curated fixtures, this points at a large, uncurated
//! body of real code (e.g. a checkout of CRAN packages) to surface panics,
//! losslessness violations, and non-idempotence on shapes the fixtures miss.
//!
//! It is `#[ignore]`d so it never runs in `cargo test` and cannot fail CI. Point
//! it at a directory and run it explicitly:
//!
//! ```sh
//! ARITY_CORPUS=/path/to/r/sources task corpus
//! # or
//! ARITY_CORPUS=/path/to/r/sources cargo test --test corpus -- --ignored --nocapture
//! ```
//!
//! Setting `ARITY_CORPUS_REPORT=<path>` switches it to **CI mode**: instead of
//! panicking on failure it writes a tab-separated failure report (one
//! `relative-key \t category \t message` record per line) to that path and
//! returns cleanly, leaving the workflow (`.github/workflows/smoke-test.yml`) to
//! file issues and decide pass/fail. Without it, any failure panics the test.
//!
//! A file arity cannot parse (parse diagnostics) is **skipped**, not failed ---
//! the corpus contains code targeting R features arity may not support yet, and
//! a parse gap is a known limitation rather than a regression. A losslessness
//! violation, an idempotence violation, a non-parse format error, or a panic in
//! either parsing or formatting is a hard failure.

use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use arity::formatter::{FormatError, format};
use arity::parser::reconstruct;

/// Why a single file failed the smoke test.
enum Failure {
    /// `reconstruct(raw) != raw` --- the parser dropped or altered text.
    Lossless,
    /// `format` returned an error other than parse diagnostics (e.g. an
    /// ambiguous construct the formatter could not lay out).
    FormatError(String),
    /// `format(format(x)) != format(x)`.
    Idempotence,
    /// Parsing or formatting panicked.
    Panic(String),
}

impl Failure {
    fn label(&self) -> &'static str {
        match self {
            Failure::Lossless => "losslessness",
            Failure::FormatError(_) => "format error",
            Failure::Idempotence => "idempotence",
            Failure::Panic(_) => "panic",
        }
    }

    /// Machine-readable failure category for the CI report (hyphenated slug,
    /// no spaces --- it travels through GitHub issue markers).
    fn slug(&self) -> &'static str {
        match self {
            Failure::Lossless => "losslessness",
            Failure::FormatError(_) => "format-error",
            Failure::Idempotence => "idempotence",
            Failure::Panic(_) => "panic",
        }
    }

    /// The free-form message (error text / panic message), or empty for the
    /// self-explanatory invariant violations.
    fn message(&self) -> &str {
        match self {
            Failure::Lossless | Failure::Idempotence => "",
            Failure::FormatError(msg) | Failure::Panic(msg) => msg,
        }
    }

    fn detail(&self) -> String {
        match self.message() {
            "" => String::new(),
            msg => format!(": {msg}"),
        }
    }
}

#[test]
#[ignore = "corpus smoke test; run via `task corpus` with ARITY_CORPUS set"]
fn corpus_smoke() {
    let Ok(dir) = std::env::var("ARITY_CORPUS") else {
        eprintln!(
            "corpus: ARITY_CORPUS not set; skipping (this is not a failure). \
             Point it at a directory of real .R files."
        );
        return;
    };

    let root = PathBuf::from(&dir);
    let mut files = Vec::new();
    collect_r_files(&root, &root, &mut files);
    files.sort();

    if files.is_empty() {
        eprintln!("corpus: no .R files under {dir}; nothing to check.");
        return;
    }

    let total = files.len();
    let mut skipped = 0usize;
    let mut failures: Vec<(String, Failure)> = Vec::new();

    for (key, path) in &files {
        let Ok(raw) = fs::read_to_string(path) else {
            // Unreadable / non-UTF-8: not arity's concern.
            continue;
        };

        if let Some(failure) = check_file(&raw) {
            if matches!(failure, Failure::FormatError(ref m) if m == SKIP_PARSE) {
                skipped += 1;
                continue;
            }
            failures.push((key.clone(), failure));
        }
    }

    eprintln!(
        "corpus: {total} files, {checked} checked, {skipped} skipped (unparseable), {failed} failed.",
        checked = total - skipped,
        failed = failures.len(),
    );

    // CI mode: when ARITY_CORPUS_REPORT names a path, write a tab-separated
    // failure record (`relative-key \t slug \t message`) and return cleanly.
    // The workflow turns that into GitHub issues and decides pass/fail, so the
    // test itself must not panic here. Tabs/newlines in messages are flattened
    // to keep one record per line.
    if let Ok(report_path) = std::env::var("ARITY_CORPUS_REPORT") {
        let mut report = String::new();
        for (key, failure) in &failures {
            let message = failure.message().replace(['\t', '\n', '\r'], " ");
            report.push_str(&format!("{key}\t{}\t{message}\n", failure.slug()));
        }
        fs::write(&report_path, report).expect("write ARITY_CORPUS_REPORT");
        eprintln!(
            "corpus: wrote {} failure record(s) to {report_path}",
            failures.len()
        );
        return;
    }

    if !failures.is_empty() {
        let mut summary = String::from("corpus smoke test found failures:\n");
        for (key, failure) in &failures {
            summary.push_str(&format!(
                "  [{}] {key}{}\n",
                failure.label(),
                failure.detail()
            ));
        }
        panic!("{summary}");
    }
}

/// Sentinel detail used to signal "skip: arity can't parse this file" through
/// the `Failure` channel without a separate type.
const SKIP_PARSE: &str = "<unparseable>";

/// Run the Tier 0 checks on one file's text. Returns the first failure, or
/// `None` if the file passes (or is a parse-skip, signaled via [`SKIP_PARSE`]).
fn check_file(raw: &str) -> Option<Failure> {
    // 1. Losslessness --- independent of whether the file is formattable.
    match catch_unwind(AssertUnwindSafe(|| reconstruct(raw))) {
        Ok(round_trip) if round_trip != raw => return Some(Failure::Lossless),
        Ok(_) => {}
        Err(panic) => return Some(Failure::Panic(format!("parse: {}", panic_msg(&panic)))),
    }

    // 2. Format once.
    let first = match catch_unwind(AssertUnwindSafe(|| format(raw))) {
        Ok(Ok(out)) => out,
        Ok(Err(FormatError::ParseErrors { .. })) => {
            return Some(Failure::FormatError(SKIP_PARSE.to_string()));
        }
        Ok(Err(other)) => return Some(Failure::FormatError(other.to_string())),
        Err(panic) => return Some(Failure::Panic(format!("format: {}", panic_msg(&panic)))),
    };

    // 3. Idempotence --- format(format(x)) == format(x).
    match catch_unwind(AssertUnwindSafe(|| format(&first))) {
        Ok(Ok(second)) if second != first => Some(Failure::Idempotence),
        Ok(Ok(_)) => None,
        // The first pass produced output the formatter then rejects: a real bug.
        Ok(Err(err)) => Some(Failure::FormatError(format!("on reformat: {err}"))),
        Err(panic) => Some(Failure::Panic(format!("reformat: {}", panic_msg(&panic)))),
    }
}

fn panic_msg(panic: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic>".to_string()
    }
}

/// Collect `.R`/`.r` files under `dir`, keyed by their path relative to `root`.
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
