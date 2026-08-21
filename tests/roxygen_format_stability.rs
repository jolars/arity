//! Stage-0 differential guardrail for the roxygen CST re-model.
//!
//! Asserts `format(input)` is byte-identical to a committed baseline over the
//! whole roxygen corpus (the curated dir + the harvested jsonl). The baseline is
//! captured from the *pre-re-model* formatter, so any drift during the
//! logical-content CST migration --- which must be behavior-preserving for the
//! formatter (Tenet 1) --- fails here, independently of the per-fixture formatter
//! snapshots. Regenerate intentionally with
//! `BLESS_ROXYGEN_FORMAT=1 cargo test --test roxygen_format_stability`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use arity::formatter::{FormatStyle, format_with_options};
use arity::parser::ParseOptions;

const BASELINE_REL: &str = "tests/oracle/roxygen-format-baseline.jsonl";
const CURATED_DIR_REL: &str = "tests/oracle/corpus/roxygen";
const HARVEST_CORPUS_REL: &str = "tests/oracle/corpus/roxygen.jsonl";

#[derive(serde::Deserialize)]
struct HarvestInput {
    slug: String,
    input: String,
    #[serde(default)]
    roxygen_markdown_default: bool,
}

#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CuratedOptions {
    #[serde(default)]
    roxygen_markdown_default: bool,
}

struct CaseInput {
    input: String,
    roxygen_markdown_default: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct BaselineEntry {
    key: String,
    formatted: String,
}

fn manifest_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// `key -> input` over the whole roxygen corpus. Keys are namespaced
/// (`curated/<stem>`, `harvest/<slug>`) so the two sources can't collide.
fn collect_inputs() -> BTreeMap<String, CaseInput> {
    let mut inputs = BTreeMap::new();

    let dir = manifest_path(CURATED_DIR_REL);
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("R") {
                let stem = path.file_stem().unwrap().to_string_lossy().to_string();
                let text = fs::read_to_string(&path).expect("read curated corpus .R");
                let options_path = path.with_extension("options.json");
                let options: CuratedOptions = if options_path.is_file() {
                    serde_json::from_str(
                        &fs::read_to_string(options_path).expect("read curated options"),
                    )
                    .expect("parse curated options")
                } else {
                    CuratedOptions::default()
                };
                inputs.insert(
                    format!("curated/{stem}"),
                    CaseInput {
                        input: text,
                        roxygen_markdown_default: options.roxygen_markdown_default,
                    },
                );
            }
        }
    }

    let harvest =
        fs::read_to_string(manifest_path(HARVEST_CORPUS_REL)).expect("read harvest jsonl");
    for line in harvest.lines().filter(|l| !l.trim().is_empty()) {
        let h: HarvestInput = serde_json::from_str(line).expect("parse harvest jsonl line");
        inputs.insert(
            format!("harvest/{}", h.slug),
            CaseInput {
                input: h.input,
                roxygen_markdown_default: h.roxygen_markdown_default,
            },
        );
    }

    inputs
}

/// The formatter output, or a stable error marker (a format error is itself a
/// behavior we want pinned so the re-model can't silently start/stop erroring).
fn formatted_or_marker(case: &CaseInput) -> String {
    let options =
        ParseOptions::default().with_roxygen_markdown_default(case.roxygen_markdown_default);
    match format_with_options(&case.input, FormatStyle::default(), &options) {
        Ok(out) => out,
        Err(e) => format!("<<FORMAT-ERROR: {e:?}>>"),
    }
}

#[test]
fn roxygen_format_is_stable() {
    let inputs = collect_inputs();
    let current: BTreeMap<String, String> = inputs
        .iter()
        .map(|(key, input)| (key.clone(), formatted_or_marker(input)))
        .collect();

    let baseline_path = manifest_path(BASELINE_REL);

    if std::env::var_os("BLESS_ROXYGEN_FORMAT").is_some() || !baseline_path.exists() {
        let mut out = String::new();
        for (key, formatted) in &current {
            let entry = BaselineEntry {
                key: key.clone(),
                formatted: formatted.clone(),
            };
            out.push_str(&serde_json::to_string(&entry).unwrap());
            out.push('\n');
        }
        fs::write(&baseline_path, out).expect("write baseline");
        eprintln!(
            "blessed roxygen format baseline: {} cases -> {BASELINE_REL}",
            current.len()
        );
        return;
    }

    let baseline: BTreeMap<String, String> = fs::read_to_string(&baseline_path)
        .expect("read baseline")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let e: BaselineEntry = serde_json::from_str(l).expect("parse baseline line");
            (e.key, e.formatted)
        })
        .collect();

    let cur_keys: BTreeSet<&String> = current.keys().collect();
    let base_keys: BTreeSet<&String> = baseline.keys().collect();
    assert_eq!(
        cur_keys, base_keys,
        "roxygen corpus key set drifted from the format baseline; \
         re-bless with BLESS_ROXYGEN_FORMAT=1 if the corpus change is intended"
    );

    let diffs: Vec<&String> = baseline
        .iter()
        .filter(|(key, base)| current[*key] != **base)
        .map(|(key, _)| key)
        .collect();

    assert!(
        diffs.is_empty(),
        "formatter output drifted from the pre-re-model baseline on {} case(s): {:?}\n\
         The CST re-model must keep formatter output byte-identical (Tenet 1).",
        diffs.len(),
        diffs
    );
}
