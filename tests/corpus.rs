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

use arity::dcf;
use arity::formatter::{DescriptionFormatError, FormatError, format, format_description};
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
    /// A `DESCRIPTION` failure. Kept as its own family so the weekly scan's
    /// per-`(repo, category)` issue dedup never mixes the two grammars.
    Dcf(DcfFailure),
}

/// Why a single `DESCRIPTION` failed.
enum DcfFailure {
    Lossless,
    FormatError(String),
    Idempotence,
    /// `read.dcf` would see a different set of records, fields, or values.
    Meaning(String),
    /// A comment was dropped or invented. `desc` drops them all; we must not.
    CommentLoss,
    Panic(String),
}

impl Failure {
    fn label(&self) -> &'static str {
        match self {
            Failure::Lossless => "losslessness",
            Failure::FormatError(_) => "format error",
            Failure::Idempotence => "idempotence",
            Failure::Panic(_) => "panic",
            Failure::Dcf(DcfFailure::Lossless) => "DESCRIPTION losslessness",
            Failure::Dcf(DcfFailure::FormatError(_)) => "DESCRIPTION format error",
            Failure::Dcf(DcfFailure::Idempotence) => "DESCRIPTION idempotence",
            Failure::Dcf(DcfFailure::Meaning(_)) => "DESCRIPTION meaning",
            Failure::Dcf(DcfFailure::CommentLoss) => "DESCRIPTION comment loss",
            Failure::Dcf(DcfFailure::Panic(_)) => "DESCRIPTION panic",
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
            Failure::Dcf(DcfFailure::Lossless) => "dcf-losslessness",
            Failure::Dcf(DcfFailure::FormatError(_)) => "dcf-format-error",
            Failure::Dcf(DcfFailure::Idempotence) => "dcf-idempotence",
            Failure::Dcf(DcfFailure::Meaning(_)) => "dcf-meaning",
            Failure::Dcf(DcfFailure::CommentLoss) => "dcf-comment-loss",
            Failure::Dcf(DcfFailure::Panic(_)) => "dcf-panic",
        }
    }

    /// The free-form message (error text / panic message), or empty for the
    /// self-explanatory invariant violations.
    fn message(&self) -> &str {
        match self {
            Failure::Lossless
            | Failure::Idempotence
            | Failure::Dcf(
                DcfFailure::Lossless | DcfFailure::Idempotence | DcfFailure::CommentLoss,
            ) => "",
            Failure::FormatError(msg)
            | Failure::Panic(msg)
            | Failure::Dcf(
                DcfFailure::FormatError(msg) | DcfFailure::Panic(msg) | DcfFailure::Meaning(msg),
            ) => msg,
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

    // Deliberately *without* the `is_own_package_root` gate `arity format` uses:
    // the miniature packages under `tests/testthat/` are the most adversarial
    // DCF in any repo, and here we are checking invariants, not addressing an
    // author.
    let mut descriptions = Vec::new();
    collect_descriptions(&root, &root, &mut descriptions);
    descriptions.sort();

    if files.is_empty() && descriptions.is_empty() {
        eprintln!("corpus: no .R or DESCRIPTION files under {dir}; nothing to check.");
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

    let dcf_total = descriptions.len();
    let mut dcf_skipped = 0usize;
    for (key, path) in &descriptions {
        let Ok(raw) = fs::read_to_string(path) else {
            dcf_skipped += 1;
            continue;
        };
        match check_description(&raw) {
            // Input the formatter refuses is not a failure; it is the design.
            // Counting it keeps a shift between buckets visible to triage.
            DcfOutcome::Skipped => dcf_skipped += 1,
            DcfOutcome::Clean => {}
            DcfOutcome::Failed(failure) => failures.push((key.clone(), Failure::Dcf(failure))),
        }
    }

    eprintln!(
        "corpus: {total} files, {checked} checked, {skipped} skipped (unparseable), \
         {dcf_total} DESCRIPTIONs, {dcf_checked} checked, {dcf_skipped} skipped, {failed} failed.",
        checked = total - skipped,
        dcf_checked = dcf_total - dcf_skipped,
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

enum DcfOutcome {
    Clean,
    /// Refused by design, or unreadable. Not a failure.
    Skipped,
    Failed(DcfFailure),
}

/// Run the Tier 0 checks on one `DESCRIPTION`.
fn check_description(raw: &str) -> DcfOutcome {
    // Losslessness holds whether or not the file is formattable.
    match catch_unwind(AssertUnwindSafe(|| dcf::reconstruct(raw))) {
        Ok(round_trip) if round_trip != raw => return DcfOutcome::Failed(DcfFailure::Lossless),
        Ok(_) => {}
        Err(panic) => {
            return DcfOutcome::Failed(DcfFailure::Panic(format!("parse: {}", panic_msg(&panic))));
        }
    }

    let first = match catch_unwind(AssertUnwindSafe(|| format_description(raw))) {
        Ok(Ok(out)) => out,
        // A refusal and a parse error are both "we deliberately did nothing".
        Ok(Err(
            DescriptionFormatError::ParseErrors { .. } | DescriptionFormatError::Declined(_),
        )) => {
            return DcfOutcome::Skipped;
        }
        Err(panic) => {
            return DcfOutcome::Failed(DcfFailure::Panic(format!("format: {}", panic_msg(&panic))));
        }
    };

    if let Some(why) = meaning_changed(raw, &first) {
        return DcfOutcome::Failed(DcfFailure::Meaning(why));
    }
    if comment_texts(raw) != comment_texts(&first) {
        return DcfOutcome::Failed(DcfFailure::CommentLoss);
    }

    match catch_unwind(AssertUnwindSafe(|| format_description(&first))) {
        Ok(Ok(second)) if second != first => DcfOutcome::Failed(DcfFailure::Idempotence),
        Ok(Ok(_)) => DcfOutcome::Clean,
        // The first pass produced output the formatter now rejects: a real bug,
        // not a refusal.
        Ok(Err(err)) => DcfOutcome::Failed(DcfFailure::FormatError(format!("on reformat: {err}"))),
        Err(panic) => DcfOutcome::Failed(DcfFailure::Panic(format!(
            "reformat: {}",
            panic_msg(&panic)
        ))),
    }
}

/// What `read.dcf` would see, reduced to what formatting is allowed to change:
/// record structure, field names, and each value modulo whitespace and (for a
/// dependency field) entry order.
///
/// Deliberately coarse. The fixture suite owns the exact per-class relation;
/// this is the sweep, and a sweep that cries wolf on real packages gets muted.
/// The message names one field, not the whole projection: it travels into a
/// GitHub issue body.
fn meaning_changed(before: &str, after: &str) -> Option<String> {
    let lhs = project_meaning(before);
    let rhs = project_meaning(after);
    if lhs.len() != rhs.len() {
        return Some(format!("record count {} -> {}", lhs.len(), rhs.len()));
    }
    for (index, (want, got)) in lhs.iter().zip(&rhs).enumerate() {
        if want.len() != got.len() {
            return Some(format!("record {index} field count changed"));
        }
        for ((want_name, want_value), (got_name, got_value)) in want.iter().zip(got) {
            if want_name != got_name {
                return Some(format!("record {index}: {want_name:?} -> {got_name:?}"));
            }
            if want_value != got_value {
                return Some(format!(
                    "record {index} field {want_name:?}: {} -> {}",
                    truncate(want_value),
                    truncate(got_value)
                ));
            }
        }
    }
    None
}

fn truncate(value: &str) -> String {
    if value.chars().count() <= 160 {
        return format!("{value:?}");
    }
    let head: String = value.chars().take(160).collect();
    format!("{head:?}...")
}

fn project_meaning(text: &str) -> Vec<Vec<(String, String)>> {
    dcf::parse(text)
        .document()
        .records()
        .map(|record| {
            let mut fields: Vec<(String, String)> = record
                .fields()
                .map(|field| {
                    let name = field.name().to_string();
                    let value = field.folded_value();
                    (name.clone(), normalize_value(&name, &value))
                })
                .collect();
            fields.sort();
            fields
        })
        .collect()
}

fn normalize_value(name: &str, value: &str) -> String {
    if dcf::is_dependency_field(name) {
        // Entries are sorted by design, so only the multiset survives.
        let mut entries: Vec<String> = value
            .split(',')
            .map(collapse_ws)
            .filter(|entry| !entry.is_empty())
            .collect();
        entries.sort();
        return entries.join(",");
    }
    if matches!(name, "Collate" | "Collate.windows" | "Collate.unix") {
        // R reads this field with `scan()` (`tools:::.read_collate_field`),
        // which strips the quotes the canonical style adds, so only the token
        // sequence means anything. Order is execution order and survives.
        return collate_tokens(value);
    }
    if matches!(name, "Authors@R" | "Roxygen") {
        // R code, laid out by the R formatter. Collapsing whitespace is not
        // enough — it respells an empty argument `,,` as `, ,` — so equality
        // here means "formats to the same R".
        return format(value).unwrap_or_else(|_| collapse_ws(value));
    }
    collapse_ws(value)
}

fn collapse_ws(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The `Collate` entries `scan()` would see, each re-quoted so that a token
/// boundary the formatter moved cannot hide inside the join.
fn collate_tokens(value: &str) -> String {
    let mut tokens: Vec<String> = Vec::new();
    let mut chars = value.chars().peekable();
    while let Some(&ch) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        let mut token = String::new();
        if ch == '\'' || ch == '"' {
            chars.next();
            for next in chars.by_ref() {
                if next == ch {
                    break;
                }
                token.push(next);
            }
        } else {
            while let Some(&next) = chars.peek() {
                if next.is_whitespace() {
                    break;
                }
                token.push(next);
                chars.next();
            }
        }
        tokens.push(format!("{token:?}"));
    }
    tokens.join(" ")
}

fn comment_texts(text: &str) -> Vec<String> {
    let parsed = dcf::parse(text);
    let mut out: Vec<String> = parsed
        .cst
        .descendants()
        .filter(|node| node.kind() == dcf::SyntaxKind::COMMENT_LINE)
        .filter_map(|node| {
            node.children_with_tokens()
                .filter_map(|el| el.into_token())
                .find(|tok| tok.kind() == dcf::SyntaxKind::COMMENT)
                .map(|tok| tok.text().trim_end().to_string())
        })
        .collect();
    out.sort();
    out
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

/// Collect files named `DESCRIPTION` under `dir`, keyed relative to `root`.
fn collect_descriptions(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_descriptions(root, &path, out);
        } else if path.file_name().is_some_and(|name| name == "DESCRIPTION") {
            let key = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            out.push((key, path));
        }
    }
}

#[cfg(test)]
mod meaning_tests {
    use super::meaning_changed;

    /// `scan()` is what R reads a `Collate` field with, so it strips the quotes
    /// the canonical style adds. A sweep that called that a meaning change would
    /// report every package whose `Collate` was written bare.
    #[test]
    fn collate_quoting_is_not_a_meaning_change() {
        let before = "Package: p\nCollate: b.R a.R\n";
        let after = "Package: p\nCollate:\n    'b.R'\n    'a.R'\n";
        assert_eq!(meaning_changed(before, after), None);
    }

    #[test]
    fn collate_reordering_is_a_meaning_change() {
        let before = "Package: p\nCollate: b.R a.R\n";
        let after = "Package: p\nCollate:\n    'a.R'\n    'b.R'\n";
        assert!(meaning_changed(before, after).is_some());
    }

    /// Quote-blindness stops at the field's own class: a quote elsewhere is a
    /// byte the sweep still guards.
    #[test]
    fn quotes_in_other_fields_still_count() {
        let before = "Package: p\nTitle: a b\n";
        let after = "Package: p\nTitle: 'a' 'b'\n";
        assert!(meaning_changed(before, after).is_some());
    }
}
