//! Shared CLI and wire-protocol baselines for the OOP roadmap.
//!
//! Known gaps are recorded beside the probes, so a baseline cannot be mistaken
//! for the intended semantics of a future implementation.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde_json::{Value, json};

use super::{Harness, path_uri, utf16_position_to_byte};

const SYSTEMS: &[&str] = &["s3", "s4", "reference_classes", "r6", "s7"];
const CATEGORIES: &[&str] = &[
    "definitions",
    "use_sites",
    "incomplete",
    "shadowing",
    "dynamic",
];

fn fixture_names() -> &'static [&'static str] {
    &[
        "s3/definitions",
        "s3/use_sites",
        "s3/incomplete",
        "s3/shadowing",
        "s3/dynamic",
        "s4/definitions",
        "s4/declaration_identity",
        "s4/use_sites",
        "s4/incomplete",
        "s4/shadowing",
        "s4/dynamic",
        "s4/named_arguments",
        "reference_classes/definitions",
        "reference_classes/declaration_identity",
        "reference_classes/use_sites",
        "reference_classes/incomplete",
        "reference_classes/shadowing",
        "reference_classes/dynamic",
        "r6/definitions",
        "r6/declaration_identity",
        "r6/use_sites",
        "r6/incomplete",
        "r6/shadowing",
        "r6/dynamic",
        "r6/issue_183",
        "r6/method_environments",
        "r6/field_functions",
        "r6/receiver_ownership",
        "r6/generator_alias",
        "r6/named_arguments",
        "s7/definitions",
        "s7/use_sites",
        "s7/incomplete",
        "s7/shadowing",
        "s7/dynamic",
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    #[serde(default)]
    parse_errors: bool,
    #[serde(default)]
    known_gaps: Vec<String>,
    probes: Vec<Probe>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    method: ProbeMethod,
    needle: String,
    offset: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ProbeMethod {
    Definition,
    References,
    Completion,
    TypeHierarchy,
    SignatureHelp,
    PrepareRename,
    Rename,
    Highlights,
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/oop")
}

fn cases() -> BTreeMap<String, Case> {
    serde_json::from_str(include_str!("../fixtures/oop/matrix.json")).expect("OOP matrix")
}

fn source(name: &str) -> String {
    fs::read_to_string(fixture_root().join(format!("{name}.R")))
        .unwrap_or_else(|error| panic!("OOP fixture {name}: {error}"))
}

fn probe_offset(source: &str, probe: &Probe) -> usize {
    assert!(probe.offset <= probe.needle.len(), "{probe:?}");
    let mut matches = source.match_indices(&probe.needle);
    let (offset, _) = matches
        .next()
        .unwrap_or_else(|| panic!("missing probe needle {probe:?}"));
    assert!(matches.next().is_none(), "ambiguous probe needle {probe:?}");
    let offset = offset + probe.offset;
    assert!(source.is_char_boundary(offset), "{probe:?}");
    offset
}

fn position(source: &str, offset: usize) -> Value {
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let column = prefix.rsplit('\n').next().unwrap().encode_utf16().count();
    json!({ "line": line, "character": column })
}

fn byte_range(source: &str, range: &Value) -> [usize; 2] {
    let offset = |position: &Value| {
        utf16_position_to_byte(
            source,
            position["line"].as_u64().unwrap() as usize,
            position["character"].as_u64().unwrap() as usize,
        )
    };
    [offset(&range["start"]), offset(&range["end"])]
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Finding {
    start: usize,
    end: usize,
    rule: String,
    severity: u64,
}

fn cli_findings(path: &Path, config: &Path) -> Vec<Finding> {
    let output = Command::new(env!("CARGO_BIN_EXE_arity"))
        .arg("--config")
        .arg(config)
        .args(["lint", "--output", "json"])
        .arg(path)
        .env_remove("ARITY_REMOTE_URL")
        .stdin(Stdio::null())
        .output()
        .expect("run CLI lint");
    assert!(output.stderr.is_empty(), "{output:?}");
    let diagnostics: Vec<Value> = if output.stdout.is_empty() {
        Vec::new()
    } else {
        serde_json::from_slice(&output.stdout).expect("CLI lint JSON")
    };
    assert_eq!(
        output.status.code(),
        Some(i32::from(!diagnostics.is_empty())),
        "{output:?}"
    );
    let mut findings: Vec<_> = diagnostics
        .iter()
        .map(|diagnostic| Finding {
            start: diagnostic["range"]["start"].as_u64().unwrap() as usize,
            end: diagnostic["range"]["end"].as_u64().unwrap() as usize,
            rule: diagnostic["rule"].as_str().unwrap().to_owned(),
            severity: match diagnostic["severity"].as_str().unwrap() {
                "Error" => 1,
                "Warning" => 2,
                "Info" => 3,
                "Hint" => 4,
                other => panic!("unknown severity: {other}"),
            },
        })
        .collect();
    findings.sort();
    findings
}

fn lsp_findings(source: &str, diagnostics: &[Value]) -> Vec<Finding> {
    let mut findings: Vec<_> = diagnostics
        .iter()
        .map(|diagnostic| {
            let [start, end] = byte_range(source, &diagnostic["range"]);
            Finding {
                start,
                end,
                rule: diagnostic["code"].as_str().unwrap().to_owned(),
                severity: diagnostic["severity"].as_u64().unwrap(),
            }
        })
        .collect();
    findings.sort();
    findings
}

fn request(harness: &mut Harness, method: &str, params: Value) -> Value {
    let id = harness.request(method, params);
    harness
        .recv_response(&id)
        .response_result
        .unwrap_or_else(|error| panic!("{method}: {error:?}"))
}

fn symbols(result: &Value, owner: &str, out: &mut Vec<Value>) {
    for symbol in result.as_array().expect("hierarchical document symbols") {
        let name = symbol["name"].as_str().unwrap();
        let name = if owner.is_empty() {
            name.to_owned()
        } else {
            format!("{owner}::{name}")
        };
        out.push(json!([name, symbol["kind"], symbol.get("detail")]));
        if let Some(children) = symbol.get("children") {
            symbols(children, &name, out);
        }
    }
}

fn observe_probe(harness: &mut Harness, uri: &str, source: &str, probe: &Probe) -> Value {
    let position = position(source, probe_offset(source, probe));
    let method = match probe.method {
        ProbeMethod::Definition => "textDocument/definition",
        ProbeMethod::References => "textDocument/references",
        ProbeMethod::Completion => "textDocument/completion",
        ProbeMethod::TypeHierarchy => "textDocument/prepareTypeHierarchy",
        ProbeMethod::SignatureHelp => "textDocument/signatureHelp",
        ProbeMethod::PrepareRename => "textDocument/prepareRename",
        ProbeMethod::Rename => "textDocument/rename",
        ProbeMethod::Highlights => "textDocument/documentHighlight",
    };
    let mut params = json!({ "textDocument": { "uri": uri }, "position": position });
    if matches!(probe.method, ProbeMethod::References) {
        params["context"] = json!({ "includeDeclaration": true });
    }
    if matches!(probe.method, ProbeMethod::Rename) {
        params["newName"] = json!("renamed");
    }
    let result = request(harness, method, params);
    if result.is_null() {
        return Value::Null;
    }
    match probe.method {
        ProbeMethod::Definition | ProbeMethod::References => {
            let locations = match result.as_array() {
                Some(locations) => locations.clone(),
                None => vec![result],
            };
            let locations: Vec<_> = locations
                .iter()
                .map(|location| {
                    assert_eq!(location["uri"], uri, "unexpected cross-fixture definition");
                    let [start, end] = byte_range(source, &location["range"]);
                    json!({ "range": [start, end], "text": &source[start..end] })
                })
                .collect();
            json!(locations)
        }
        ProbeMethod::Completion => {
            let items = result
                .as_array()
                .or_else(|| result["items"].as_array())
                .expect("completion items");
            let mut items: Vec<_> = items
                .iter()
                .map(|item| json!([item["label"], item["kind"]]))
                .collect();
            items.sort_by_key(Value::to_string);
            json!(items)
        }
        ProbeMethod::TypeHierarchy => {
            let mut items = Vec::new();
            for item in result.as_array().expect("type hierarchy items") {
                assert_eq!(item["uri"], uri, "unexpected cross-fixture class");
                let parents = request(harness, "typeHierarchy/supertypes", json!({ "item": item }));
                let children = request(harness, "typeHierarchy/subtypes", json!({ "item": item }));
                let names = |relatives: &Value| {
                    let mut names: Vec<_> = relatives
                        .as_array()
                        .expect("type hierarchy relatives")
                        .iter()
                        .map(|relative| relative["name"].clone())
                        .collect();
                    names.sort_by_key(Value::to_string);
                    names
                };
                let [start, end] = byte_range(source, &item["selectionRange"]);
                items.push(json!({
                    "name": item["name"],
                    "detail": item["detail"],
                    "selection": [start, end],
                    "parents": names(&parents),
                    "children": names(&children),
                }));
            }
            json!(items)
        }
        ProbeMethod::SignatureHelp => {
            let signatures: Vec<_> = result["signatures"]
                .as_array()
                .expect("signature help")
                .iter()
                .map(|signature| signature["label"].clone())
                .collect();
            json!({ "signatures": signatures, "active_parameter": result["activeParameter"] })
        }
        ProbeMethod::PrepareRename | ProbeMethod::Rename => result,
        ProbeMethod::Highlights => json!(
            result
                .as_array()
                .expect("highlights")
                .iter()
                .map(|item| {
                    json!({ "range": byte_range(source, &item["range"]), "kind": item["kind"] })
                })
                .collect::<Vec<_>>()
        ),
    }
}

fn check_system(system: &str) {
    let cases = cases();
    let names: Vec<_> = fixture_names()
        .iter()
        .filter(|name| name.starts_with(&format!("{system}/")))
        .copied()
        .collect();
    assert!(!names.is_empty());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("case.R");
    let config = dir.path().join("arity.toml");
    fs::write(
        &config,
        "cache = false\n[lint]\nselect = [\"undefined-symbol\", \"unused-binding\"]\n\
         [index]\nauto-build = false\ncache-dir = \"index\"\n",
    )
    .unwrap();
    fs::write(&path, source(names[0])).unwrap();
    let uri = path_uri(&path);
    let root_uri = path_uri(dir.path());
    let mut harness = Harness::start_in_workspace(
        json!({ "textDocument": { "documentSymbol": { "hierarchicalDocumentSymbolSupport": true } } }),
        Some(&root_uri),
    );
    let mut report = String::new();
    for (index, name) in names.iter().enumerate() {
        let case = &cases[*name];
        let source = source(name);
        fs::write(&path, &source).unwrap();
        let cli = cli_findings(&path, &config);
        if *name == "r6/issue_183" {
            assert!(cli.is_empty(), "#183: {cli:?}");
        }
        let version = index as i32 + 1;
        if index == 0 {
            harness.did_open(&uri, &source, version);
        } else {
            harness.did_change(&uri, &source, version);
        }
        let diagnostics = harness.recv_publish_for(&uri, version);
        assert_eq!(lsp_findings(&source, &diagnostics), cli, "{name}");
        assert_eq!(fs::read_to_string(&path).unwrap(), source, "{name}");

        writeln!(report, "=== {name} ===").unwrap();
        for gap in &case.known_gaps {
            writeln!(report, "known gap: {gap}").unwrap();
        }
        if cli.is_empty() {
            writeln!(report, "diagnostics: []").unwrap();
        }
        for finding in &cli {
            writeln!(
                report,
                "diagnostic: {} severity={} {}..{} {:?}",
                finding.rule,
                finding.severity,
                finding.start,
                finding.end,
                &source[finding.start..finding.end],
            )
            .unwrap();
        }
        let outline = request(
            &mut harness,
            "textDocument/documentSymbol",
            json!({ "textDocument": { "uri": uri } }),
        );
        let mut outline_symbols = Vec::new();
        symbols(&outline, "", &mut outline_symbols);
        writeln!(report, "symbols: {}", json!(outline_symbols)).unwrap();
        for probe in &case.probes {
            let result = observe_probe(&mut harness, &uri, &source, probe);
            writeln!(report, "{:?} {:?}: {result}", probe.method, probe.needle).unwrap();
        }
        writeln!(report).unwrap();
    }
    harness.shutdown();
    insta::with_settings!({ snapshot_path => "../snapshots" }, {
        insta::assert_snapshot!(system, report);
    });
}

#[test]
fn matrix_is_registered_and_lossless() {
    let cases = cases();
    let registered: BTreeSet<_> = fixture_names().iter().copied().collect();
    assert_eq!(
        registered.len(),
        fixture_names().len(),
        "duplicate OOP fixture"
    );
    assert_eq!(registered, cases.keys().map(String::as_str).collect());
    for system in SYSTEMS {
        for category in CATEGORIES {
            assert!(registered.contains(format!("{system}/{category}").as_str()));
        }
    }
    for name in fixture_names() {
        let source = source(name);
        let parsed = arity::parser::parse(&source);
        assert_eq!(parsed.cst.to_string(), source, "{name}");
        assert_eq!(
            !parsed.diagnostics.is_empty(),
            cases[*name].parse_errors,
            "{name}: {:?}",
            parsed.diagnostics,
        );
        for probe in &cases[*name].probes {
            probe_offset(&source, probe);
        }
    }
}

#[test]
fn s3() {
    check_system("s3");
}

#[test]
fn s4() {
    check_system("s4");
}

#[test]
fn reference_classes() {
    check_system("reference_classes");
}

#[test]
fn r6() {
    check_system("r6");
}

#[test]
fn s7() {
    check_system("s7");
}
