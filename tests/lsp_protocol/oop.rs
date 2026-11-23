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
        "s3/method_environments",
        "s4/definitions",
        "s4/declaration_identity",
        "s4/use_sites",
        "s4/incomplete",
        "s4/shadowing",
        "s4/dynamic",
        "s4/named_arguments",
        "s4/method_environments",
        "reference_classes/method_environments",
        "reference_classes/member_apis",
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
        "r6/member_apis",
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

#[test]
fn implementation_opens_registered_method_body() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("classes.R");
    let source = "methods::setGeneric('describe', function(x) standardGeneric('describe'))
methods::setMethod('describe', 'numeric', function(x) x)
label <- 'describe'
";
    fs::write(&path, source).unwrap();
    let uri = path_uri(&path);
    let mut harness = Harness::start_push();
    assert_eq!(harness.capabilities["implementationProvider"], true);
    harness.did_open(&uri, source, 1);
    harness.recv_publish_for(&uri, 1);

    for (name, expected) in [("describe", true), ("label <- 'describe", false)] {
        let offset = source.find(name).unwrap() + name.len() - 1;
        let result = request(
            &mut harness,
            "textDocument/implementation",
            json!({ "textDocument": { "uri": uri }, "position": position(source, offset) }),
        );
        if expected {
            assert_eq!(result["uri"], uri);
            let [start, end] = byte_range(source, &result["range"]);
            assert_eq!(&source[start..end], "function(x) x");
        } else {
            assert!(result.is_null(), "{result}");
        }
    }
    harness.shutdown();
}

#[test]
fn namespace_references_follow_open_buffers_and_explicit_methods() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("R/classes.R");
    let namespace_path = dir.path().join("NAMESPACE");
    let source = r#"methods::setClass("Existing")
methods::setClass("Alternate")
public <- function(x) UseMethod("public")
impl <- function(x) x
impl(1)
other <- function(x) x
other(1)
alone <- function(x) x
Generator <- R6::R6Class("Example")
"#;
    let saved = "exportClasses(Existing)\nS3method(public, example, impl)\nS3method(public, single, alone)\n";
    let live = saved.replace("Existing", "Alternate") + "export(Exposed)\n";
    fs::create_dir(path.parent().unwrap()).unwrap();
    fs::write(dir.path().join("DESCRIPTION"), super::COMPLETE_DESCRIPTION).unwrap();
    fs::write(&path, source).unwrap();
    fs::write(path.with_file_name("aliases.R"), "Exposed <- Generator\n").unwrap();
    fs::write(&namespace_path, saved).unwrap();
    let uri = path_uri(&path);
    let namespace_uri = path_uri(&namespace_path);
    let mut harness = Harness::start_push();
    harness.did_open(&uri, source, 1);
    harness.recv_publish_for(&uri, 1);
    harness.did_open(&namespace_uri, &live, 1);
    harness.recv_publish_for(&namespace_uri, 1);

    let definition = request(
        &mut harness,
        "textDocument/definition",
        json!({
            "textDocument": { "uri": namespace_uri },
            "position": position(&live, live.find("Exposed").unwrap() + 1)
        }),
    );
    assert_eq!(definition["uri"], uri, "{definition}");
    assert_eq!(definition["range"]["start"]["line"], 8, "{definition}");

    let references = |harness: &mut Harness, uri: &str, text: &str, name: &str| {
        request(
            harness,
            "textDocument/references",
            json!({
                "textDocument": { "uri": uri },
                "position": position(text, text.find(name).unwrap() + 1),
                "context": { "includeDeclaration": false }
            }),
        )
    };
    let result = references(&mut harness, &namespace_uri, &live, "impl");
    let locations = result.as_array().unwrap();
    assert_eq!(locations.len(), 2, "{result}");
    assert!(
        locations
            .iter()
            .any(|location| location["uri"] == uri && location["range"]["start"]["line"] == 4)
    );
    assert_eq!(
        references(&mut harness, &uri, source, "Alternate")[0]["uri"],
        namespace_uri
    );
    harness.notify(
        "textDocument/didClose",
        json!({ "textDocument": { "uri": namespace_uri } }),
    );

    let result = references(&mut harness, &uri, source, "Alternate");
    assert!(
        result.is_null() || result.as_array().is_some_and(Vec::is_empty),
        "{result}"
    );
    assert_eq!(
        references(&mut harness, &uri, source, "Existing")[0]["uri"],
        namespace_uri
    );
    let result = references(&mut harness, &uri, source, "impl");
    let locations = result.as_array().unwrap();
    assert_eq!(locations.len(), 2, "{result}");
    assert!(
        locations
            .iter()
            .any(|location| location["uri"] == namespace_uri)
    );
    let result = references(&mut harness, &uri, source, "other");
    let locations = result.as_array().unwrap();
    assert_eq!(locations.len(), 1, "{result}");
    assert_eq!(locations[0]["uri"], uri);
    assert_eq!(locations[0]["range"]["start"]["line"], 6);
    let result = references(&mut harness, &uri, source, "alone");
    let locations = result.as_array().unwrap();
    assert_eq!(locations.len(), 1, "{result}");
    assert_eq!(locations[0]["uri"], namespace_uri);
    assert_eq!(locations[0]["range"]["start"]["line"], 2);
    harness.shutdown();
}

#[test]
fn namespace_export_keeps_conventional_method_binding_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let r_dir = dir.path().join("R");
    fs::create_dir(&r_dir).unwrap();
    fs::write(dir.path().join("DESCRIPTION"), super::COMPLETE_DESCRIPTION).unwrap();
    let path = r_dir.join("a.R");
    let later_path = r_dir.join("z.R");
    let namespace_path = dir.path().join("NAMESPACE");
    let source = "probe <- function(x) UseMethod('probe')\nprobe.example <- function(x) x\n";
    let later = "replacement <- function(x) x\n";
    let namespace = "export(probe)\nexport(probe.example)\n";
    fs::write(&path, source).unwrap();
    fs::write(&later_path, later).unwrap();
    fs::write(&namespace_path, "export(probe)\n").unwrap();
    let uri = path_uri(&path);
    let later_uri = path_uri(&later_path);
    let namespace_uri = path_uri(&namespace_path);
    let mut harness = Harness::start_push();
    harness.did_open(&uri, source, 1);
    harness.recv_publish_for(&uri, 1);
    harness.did_open(&later_uri, later, 1);
    harness.recv_publish_for(&later_uri, 1);
    harness.did_open(&namespace_uri, namespace, 1);
    harness.recv_publish_for(&namespace_uri, 1);

    let definition = |harness: &mut Harness| {
        request(
            harness,
            "textDocument/definition",
            json!({
                "textDocument": { "uri": namespace_uri },
                "position": position(namespace, namespace.find("probe.example").unwrap() + 1)
            }),
        )
    };
    let has_namespace_reference = |harness: &mut Harness| {
        request(
            harness,
            "textDocument/references",
            json!({
                "textDocument": { "uri": uri },
                "position": position(source, source.find("probe.example").unwrap() + 1),
                "context": { "includeDeclaration": false }
            }),
        )
        .as_array()
        .is_some_and(|locations| {
            locations
                .iter()
                .any(|location| location["uri"] == namespace_uri)
        })
    };
    let result = definition(&mut harness);
    assert_eq!(result["uri"], uri, "{result}");
    assert_eq!(result["range"]["start"]["line"], 1, "{result}");
    assert!(has_namespace_reference(&mut harness));

    harness.did_change(
        &later_uri,
        &format!("{later}probe.example <- replacement\n"),
        2,
    );
    harness.recv_publish_for(&later_uri, 2);
    let result = definition(&mut harness);
    assert_eq!(result["uri"], later_uri, "{result}");
    assert_eq!(result["range"]["start"]["line"], 1, "{result}");
    assert!(!has_namespace_reference(&mut harness));
    harness.shutdown();
}

#[test]
fn generic_references_include_cross_system_registrations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("generics.R");
    let source = r#"
Parent <- S7::new_class("Parent")
S7::S4_register(Parent)
methods::setGeneric("probeS4", function(x) methods::standardGeneric("probeS4"))
S7::method(probeS4, Parent) <- function(x) "s4"
other <- function(x) UseMethod("other")
other(1)
"#;
    fs::write(&path, source).unwrap();
    let uri = path_uri(&path);
    let mut harness = Harness::start_push();
    harness.did_open(&uri, source, 1);
    harness.recv_publish_for(&uri, 1);
    let result = request(
        &mut harness,
        "textDocument/references",
        json!({
            "textDocument": { "uri": uri },
            "position": position(source, source.find("probeS4").unwrap() + 1),
            "context": { "includeDeclaration": false }
        }),
    );
    let registration = source.find("S7::method(probeS4").unwrap() + "S7::method(".len();
    assert!(
        result.as_array().unwrap().iter().any(|location| {
            location["uri"] == uri
                && byte_range(source, &location["range"]) == [registration, registration + 7]
        }),
        "{result}"
    );
    harness.shutdown();
}

#[test]
fn generic_references_follow_workspace_aliases_in_live_buffers() {
    let dir = tempfile::tempdir().unwrap();
    let r_dir = dir.path().join("R");
    fs::create_dir(&r_dir).unwrap();
    fs::write(dir.path().join("DESCRIPTION"), super::COMPLETE_DESCRIPTION).unwrap();
    let generic = r_dir.join("generic.R");
    let caller = r_dir.join("caller.R");
    let source = "probe <- function(x) UseMethod('probe')\nalias <- probe\n";
    let calls = "run <- function() {\n  alias(1)\n  alias(2)\n  alternate(3)\n}\n\
                 shadow <- function(alias, alternate) {\n  alias(4)\n  alternate(5)\n}\n";
    fs::write(&generic, source).unwrap();
    fs::write(&caller, calls).unwrap();
    let generic_uri = path_uri(&generic);
    let caller_uri = path_uri(&caller);
    let mut harness = Harness::start_push();
    harness.did_open(&generic_uri, source, 1);
    harness.recv_publish_for(&generic_uri, 1);

    let caller_lines = |harness: &mut Harness| {
        request(
            harness,
            "textDocument/references",
            json!({
                "textDocument": { "uri": generic_uri },
                "position": position(source, 1),
                "context": { "includeDeclaration": false }
            }),
        )
        .as_array()
        .unwrap()
        .iter()
        .filter(|location| location["uri"] == caller_uri)
        .map(|location| location["range"]["start"]["line"].as_u64().unwrap())
        .collect::<BTreeSet<_>>()
    };
    assert_eq!(caller_lines(&mut harness), BTreeSet::from([1, 2]));

    let edited = source.replace("alias", "alternate");
    harness.did_change(&generic_uri, &edited, 2);
    harness.recv_publish_for(&generic_uri, 2);
    assert_eq!(caller_lines(&mut harness), BTreeSet::from([3]));
    harness.shutdown();
}

#[test]
fn oop_presentation_uses_live_document_roles_and_spans() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("presentation.R");
    let source = "G <- R6::R6Class('Display', public = list(read = function(n = 1) n))
obj <- G$new()
obj$read()
";
    fs::write(&path, source).unwrap();
    let uri = path_uri(&path);
    let mut harness = Harness::start_push();
    harness.did_open(&uri, source, 1);
    harness.recv_publish_for(&uri, 1);

    let symbols = request(
        &mut harness,
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": uri } }),
    );
    let class = &symbols[0];
    assert_eq!(class["name"], "Display");
    assert_eq!(class["kind"], 5);
    let [start, end] = byte_range(source, &class["location"]["range"]);
    assert_eq!(&source[start..end], "'Display'");
    assert_eq!(symbols[1]["name"], "read");
    assert_eq!(symbols[1]["kind"], 6);
    assert_eq!(symbols[1]["containerName"], "Display");

    let member_use = source.rfind("read").unwrap();
    let hover = request(
        &mut harness,
        "textDocument/hover",
        json!({ "textDocument": { "uri": uri }, "position": position(source, member_use) }),
    );
    let [start, end] = byte_range(source, &hover["range"]);
    assert_eq!(&source[start..end], "read");
    assert!(
        hover["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("(n = 1)")
    );

    let tokens = request(
        &mut harness,
        "textDocument/semanticTokens/full",
        json!({ "textDocument": { "uri": uri } }),
    );
    let legend = harness.capabilities["semanticTokensProvider"]["legend"]["tokenTypes"]
        .as_array()
        .unwrap();
    let data = tokens["data"].as_array().unwrap();
    let (mut line, mut column) = (0, 0);
    let mut method_use = false;
    for token in data.as_chunks::<5>().0 {
        let delta_line = token[0].as_u64().unwrap();
        line += delta_line;
        let delta_column = token[1].as_u64().unwrap();
        column = if delta_line == 0 {
            column + delta_column
        } else {
            delta_column
        };
        if (line, column) == (2, 4) {
            assert_eq!(token[2], 4);
            assert_eq!(legend[token[3].as_u64().unwrap() as usize], "method");
            assert_eq!(token[4], 0);
            method_use = true;
        }
    }
    assert!(method_use);

    let edited = format!("# shift the source\n{}", source.replace("n = 1", "n = 2"));
    harness.did_change(&uri, &edited, 2);
    harness.recv_publish_for(&uri, 2);
    let hover = request(
        &mut harness,
        "textDocument/hover",
        json!({ "textDocument": { "uri": uri }, "position": position(&edited, edited.rfind("read").unwrap()) }),
    );
    let [start, end] = byte_range(&edited, &hover["range"]);
    assert_eq!(&edited[start..end], "read");
    assert!(
        hover["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("(n = 2)")
    );
    harness.shutdown();
}

#[test]
fn oop_presentation_preserves_declaration_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("presentation.R");
    let source = r#"source("helpers.R")
Counter <- R6::R6Class("Counter", public = base::list(make = function() {
    Counter <- 1
    Counter
}))
Counter$set("public", "read", function() { local <- 1; local })
G <- R6::R6Class("")
Shadow <- R6::R6Class("Shadow", public = list(fake = function() 1))
configuration$set("public", "fake_mutation", function() 1)
registration <- methods::setGeneric("say", function(x) standardGeneric("say"))
registration
"#;
    fs::write(&path, source).unwrap();
    fs::write(
        dir.path().join("helpers.R"),
        "list <- function(...) base::list()\nconfiguration <- base::list(set = function(...) NULL)\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("arity.toml"),
        "[index]\nauto-build = false\ncache-dir = \"index\"\n",
    )
    .unwrap();
    let uri = path_uri(&path);
    let mut harness = Harness::start_in_workspace(
        json!({ "textDocument": { "documentSymbol": { "hierarchicalDocumentSymbolSupport": true } } }),
        Some(&path_uri(dir.path())),
    );
    harness.did_open(&uri, source, 1);
    harness.recv_publish_for(&uri, 1);

    let symbols = request(
        &mut harness,
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": uri } }),
    );
    let symbols = symbols.as_array().unwrap();
    let named = |name: &str| {
        symbols
            .iter()
            .find(|symbol| symbol["name"] == name)
            .unwrap_or_else(|| panic!("missing {name} in {symbols:#?}"))
    };
    let counter = named("Counter");
    assert_eq!(counter["kind"], 5);
    assert_eq!(
        symbols
            .iter()
            .filter(|symbol| symbol["name"] == "Counter")
            .count(),
        1
    );
    assert_eq!(named("G")["kind"], 5);
    assert_eq!(named("registration")["kind"], 13);
    assert_eq!(named("say")["kind"], 12);
    let method = named("read");
    assert_eq!(method["children"][0]["name"], "local");
    assert!(named("Shadow")["children"].is_null());
    assert!(
        symbols
            .iter()
            .all(|symbol| symbol["name"] != "fake_mutation")
    );
    let workspace = request(&mut harness, "workspace/symbol", json!({ "query": "" }));
    let workspace = workspace.as_array().unwrap();
    assert!(
        workspace
            .iter()
            .any(|symbol| symbol["name"] == "registration" && symbol["kind"] == 13)
    );
    assert!(
        workspace
            .iter()
            .all(|symbol| symbol["name"] != "fake" && symbol["name"] != "fake_mutation")
    );

    for offset in [
        source.find("Counter <- 1").unwrap() + 1,
        source.rfind("registration").unwrap() + 1,
    ] {
        let hover = request(
            &mut harness,
            "textDocument/hover",
            json!({ "textDocument": { "uri": uri }, "position": position(source, offset) }),
        );
        assert!(hover.is_null(), "{hover}");
    }
    harness.shutdown();
}

#[test]
fn oop_presentation_distinguishes_s7_bindings_and_method_removals() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s7.R");
    let source = r#"C <- S7::new_class("C")
S7::`:=`(describe, S7::new_generic(dispatch_args = "x"))
f <- S7::new_generic("f", "x")
S7::method(f, C) <- function(x) 1
S7::method(f, C) <- NULL
"#;
    fs::write(&path, source).unwrap();
    fs::write(
        dir.path().join("arity.toml"),
        "[compat]\ns7 = \"0.2.2.9000\"\n[index]\nauto-build = false\ncache-dir = \"index\"\n",
    )
    .unwrap();
    let uri = path_uri(&path);
    let mut harness = Harness::start_in_workspace(
        json!({ "textDocument": { "documentSymbol": { "hierarchicalDocumentSymbolSupport": true } } }),
        Some(&path_uri(dir.path())),
    );
    harness.did_open(&uri, source, 1);
    harness.recv_publish_for(&uri, 1);
    let symbols = request(
        &mut harness,
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": uri } }),
    );
    let symbols = symbols.as_array().unwrap();
    let generic = symbols
        .iter()
        .find(|symbol| symbol["name"] == "describe")
        .unwrap();
    assert_eq!(generic["kind"], 12);
    let [start, end] = byte_range(source, &generic["selectionRange"]);
    assert_eq!(&source[start..end], "describe");
    let methods: Vec<_> = symbols
        .iter()
        .filter(|symbol| symbol["name"] == "f" && symbol["kind"] == 6)
        .collect();
    let [method] = methods.as_slice() else {
        panic!("{symbols:#?}")
    };
    let [start, end] = byte_range(source, &method["range"]);
    assert!(source[start..end].contains("function(x) 1"));
    let workspace = request(
        &mut harness,
        "workspace/symbol",
        json!({ "query": "describe" }),
    );
    let [start, end] = byte_range(source, &workspace[0]["location"]["range"]);
    assert_eq!(&source[start..end], "describe");
    harness.shutdown();
}

#[test]
fn call_hierarchy_round_trips_member_method_data_and_callee_spans() {
    let mut h = Harness::start_push();
    let uri = super::doc_uri();
    let source = r#"Parent <- R6::R6Class("Parent", public = list(run = function() 1))
Child <- R6::R6Class("Child", inherit = Parent, public = list(run = function() super$run()))
"#;
    h.did_open(uri, source, 1);
    h.recv_publish_for(uri, 1);
    let id = h.request(
        "textDocument/prepareCallHierarchy",
        json!({
            "textDocument": { "uri": uri },
            "position": position(source, source.rfind("run =").unwrap())
        }),
    );
    let prepared = h.recv_response(&id).response_result.unwrap();
    let child = &prepared[0];
    assert_eq!(child["kind"], 6);
    let id = h.request("callHierarchy/outgoingCalls", json!({ "item": child }));
    let outgoing = h.recv_response(&id).response_result.unwrap();
    let parent = &outgoing[0]["to"];
    assert_eq!(parent["kind"], 6);
    assert!(
        parent["detail"]
            .as_str()
            .unwrap()
            .contains("superclass call")
    );
    let span = &outgoing[0]["fromRanges"][0];
    let [start, end] = byte_range(source, span);
    assert_eq!(&source[start..end], "run");
    let id = h.request("callHierarchy/incomingCalls", json!({ "item": parent }));
    let incoming = h.recv_response(&id).response_result.unwrap();
    assert_eq!(incoming[0]["from"]["data"], child["data"]);
    assert_eq!(incoming[0]["fromRanges"], outgoing[0]["fromRanges"]);
    h.shutdown();
}
